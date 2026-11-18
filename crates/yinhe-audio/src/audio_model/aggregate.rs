use std::collections::HashMap;
use std::sync::Arc;

use yinhe_core::YinModel;
use yinhe_types::AutomationTarget;
use yinhe_types::automation::{MidiBinding, ParamDevice, SegmentShape, binding_max, builtin_param};

use super::*;

/// 同 tick 同通道自动化的语义聚合键：键相同的事件是同一控制目标，播放时
/// 取最大值发送（多条自动化数据仍全部保留，不合并模型；导出不受影响）。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum AggKey {
    /// MIDI CC：按控制器号（xsynth Raw CC 事件与内部浮点 CC 同键）。
    Cc { controller: u8 },
    /// Pitch Bend：按变体（xsynth PitchBendValue 事件与内部浮点 PB 同键）。
    PitchBend,
    /// 原生 RPN：按参数号。
    Rpn { parameter: u16 },
    /// 原生 NRPN：按参数号。
    Nrpn { parameter: u16 },
    /// xsynth 高层 PitchBendSensitivity：独立键（tones 域，与归一化 RPN0
    /// 值域不同，不混用 max）。
    ChannelPitchBendSensitivity,
    /// xsynth 高层 FineTune：独立键（cents 域）。
    ChannelFineTune,
    /// xsynth 高层 CoarseTune：独立键（semitones 域）。
    ChannelCoarseTune,
    /// 插件参数：按乐器实例 uid + 参数 id（分组维度已含通道）。
    PluginParam { uid: u64, param_id: u32 },
}

/// 事件的聚合键（`None` = 不参与聚合，如 ProgramChange 原样发送）。
pub(super) fn agg_key(cc: &SortedCC) -> Option<AggKey> {
    // 插件参数事件：`event` 是占位（无 xsynth 语义），按 (uid, param_id) 聚合。
    if let Some(pp) = &cc.plugin_param {
        return Some(AggKey::PluginParam {
            uid: pp.uid,
            param_id: pp.param_id,
        });
    }
    match &cc.event {
        AudioEvent::Cc { controller, .. } => Some(AggKey::Cc {
            controller: *controller,
        }),
        AudioEvent::PitchBend(_) => Some(AggKey::PitchBend),
        AudioEvent::Rpn { parameter, .. } => Some(AggKey::Rpn {
            parameter: *parameter,
        }),
        AudioEvent::Nrpn { parameter, .. } => Some(AggKey::Nrpn {
            parameter: *parameter,
        }),
        // xsynth 高层事件：Raw CC 与内部浮点 CC 同目标；PitchBendValue 与
        // 内部浮点 PB 同目标。
        AudioEvent::Channel(ChannelAudioEvent::Control(ControlEvent::Raw(controller, _))) => {
            Some(AggKey::Cc {
                controller: *controller,
            })
        }
        AudioEvent::Channel(ChannelAudioEvent::Control(
            ControlEvent::PitchBendValue(_) | ControlEvent::PitchBend(_),
        )) => Some(AggKey::PitchBend),
        AudioEvent::Channel(ChannelAudioEvent::Control(ControlEvent::PitchBendSensitivity(_))) => {
            Some(AggKey::ChannelPitchBendSensitivity)
        }
        AudioEvent::Channel(ChannelAudioEvent::Control(ControlEvent::FineTune(_))) => {
            Some(AggKey::ChannelFineTune)
        }
        AudioEvent::Channel(ChannelAudioEvent::Control(ControlEvent::CoarseTune(_))) => {
            Some(AggKey::ChannelCoarseTune)
        }
        _ => None,
    }
}

/// 事件的聚合比较值（取 max 用）：各变体内部的归一化值 / 具体值。
pub(super) fn agg_value(cc: &SortedCC) -> f32 {
    if let Some(pp) = &cc.plugin_param {
        return pp.value;
    }
    match &cc.event {
        AudioEvent::Cc { value, .. }
        | AudioEvent::Rpn { value, .. }
        | AudioEvent::Nrpn { value, .. } => *value,
        AudioEvent::PitchBend(v) => *v,
        AudioEvent::Channel(ChannelAudioEvent::Control(ControlEvent::Raw(_, v))) => f32::from(*v),
        AudioEvent::Channel(ChannelAudioEvent::Control(
            ControlEvent::PitchBendValue(v)
            | ControlEvent::PitchBend(v)
            | ControlEvent::PitchBendSensitivity(v)
            | ControlEvent::FineTune(v)
            | ControlEvent::CoarseTune(v),
        )) => *v,
        // 不参与聚合的事件（agg_key 返回 None）：不会被比较。
        _ => 0.0,
    }
}

/// 对一段已按 `(tick, channel, priority)` 排序的事件做「同 tick 同通道同目标
/// 取 max」聚合：返回应发送的事件下标（进入 reps 的顺序 = 组内首见事件的顺序）。
///
/// 不重排事件：同 tick 内 RPN/参数类先于 PitchBend 的既有优先级由首见顺序
/// 天然保持；组内代表 = 值最大者，并列取首见。
///
/// `keep` 在聚合前过滤（mute 轨 / AM M/S 旁通 lane / 后端不消费的事件）：
/// 被过滤的事件不参与取 max 也不输出——整组被过滤则整组不发。
pub(crate) fn aggregate_max_reps<F>(events: &[SortedCC], mut keep: F) -> Vec<usize>
where
    F: FnMut(&SortedCC) -> bool,
{
    let mut reps: Vec<usize> = Vec::new();
    let mut groups: HashMap<(u32, u32, AggKey), usize> = HashMap::new();
    for (i, cc) in events.iter().enumerate() {
        if !keep(cc) {
            continue;
        }
        let Some(key) = agg_key(cc) else {
            reps.push(i);
            continue;
        };
        match groups.get(&(cc.tick, cc.channel, key)) {
            Some(&pos) => {
                if agg_value(cc) > agg_value(&events[reps[pos]]) {
                    reps[pos] = i;
                }
            }
            None => {
                groups.insert((cc.tick, cc.channel, key), reps.len());
                reps.push(i);
            }
        }
    }
    reps
}

/// Flatten automation lanes + program changes into sorted, deduped SortedCC events.
///
/// Standard RPN 0/1/2 are sent as high-level xsynth events (PitchBendSensitivity,
/// FineTune, CoarseTune). Non-standard RPN and NRPN use the raw CC sequence.
///
/// `density`: Linear/Curve 段在播放时按多少 tick 间隔展开中间事件。1 = 每 tick 一个事件
/// （最平滑），值越大中间事件越少。Step 段不受影响（保持值到下一点）。
///
/// 所有 lane 的事件**全部展平**（AM M/S 旁通由 dispatch 时的动态掩码负责），
/// 旁通切换无需重建本事件流。同 tick 同通道同目标（见 [`AggKey`]）的"取 max"
/// 聚合发生在消费端（dispatch / GPU 事件表 / chase），此处不合并数据。
///
/// Returns `Arc<Vec>` so the same events can be shared between the renderer and
/// the worker thread (for chase computation) without cloning.
pub(crate) fn flatten_automation_to_cc_events(
    model: &YinModel,
    density: u32,
) -> Arc<Vec<SortedCC>> {
    let density = density.max(1);
    let mut cc_events = Vec::new();

    // 通道级默认乐器：先于所有 MIDI 事件下发（tick 0），任一 MIDI PC 覆盖它。
    // track 用 u16::MAX：通道级配置不被任何单轨的 skip_track 过滤。
    //
    // 鼓通道模式由 `AudioModel::percussion_declarations` 的 PercussionMode
    // 事件声明，这里只下发 bank + program；鼓通道不下发 bank
    // （鼓组不占真实 bank 号，随便发会被 xsynth 当成普通 bank 查不到音色）。
    for (ch, inst) in model.channel_instruments.iter().enumerate() {
        if let Some(inst) = inst {
            let pc = yinhe_types::PcEvent {
                tick: 0,
                program: inst.program,
                bank_msb: if inst.is_drum { 0xFF } else { inst.bank_msb },
                bank_lsb: if inst.is_drum { 0xFF } else { inst.bank_lsb },
            };
            push_program_change(&pc, ch as u32, u16::MAX, &mut cc_events);
        }
    }

    for (track_idx, track) in model.tracks.iter().enumerate() {
        let track_idx_u16 = track_idx as u16;
        let channel = track
            .midi_target_channel()
            .map(u32::from)
            .unwrap_or_else(|| u32::from(track.global_channel()));

        for (lane_idx, lane) in track.automation_lanes.iter().enumerate() {
            let lane_idx_u16 = lane_idx as u16;
            let n = lane.events.len();
            for (i, e) in lane.events.iter().enumerate() {
                // tick 域：事件时刻直接存模型的 tick（u32），不再转 sample。
                emit_automation_event(
                    &lane.target,
                    e.value,
                    e.tick,
                    channel,
                    track_idx_u16,
                    lane_idx_u16,
                    &mut cc_events,
                );

                // Linear/Curve 段：在当前事件与下一事件之间按 density 间隔展开中间事件
                if i + 1 < n {
                    let next = &lane.events[i + 1];
                    let tick1 = e.tick;
                    let tick2 = next.tick;
                    if tick2 > tick1 && !matches!(e.shape, SegmentShape::Step) {
                        let v1 = e.value;
                        let v2 = next.value;
                        let span = (tick2 - tick1) as f32;
                        let mut t = tick1.saturating_add(density);
                        while t < tick2 {
                            let frac = (t - tick1) as f32 / span;
                            let f = e.shape.interpolate(frac);
                            let v = v1 + (v2 - v1) * f;
                            emit_automation_event(
                                &lane.target,
                                v,
                                t,
                                channel,
                                track_idx_u16,
                                lane_idx_u16,
                                &mut cc_events,
                            );
                            t = t.saturating_add(density);
                        }
                    }
                }
            }
        }

        for e in &track.program_change {
            push_program_change(e, channel, track_idx_u16, &mut cc_events);
        }
    }

    // 排序：同 tick 同 channel 下，RPN/参数类事件必须排在 PitchBendValue 之前。
    // 原因：xsynth 收到 PitchBendValue 时会按当前 PBS 立即计算弯音并作用于已响 voice，
    // 若 PBS 尚未更新，PB 会用旧 PBS 算出错误音高。见 commit 3490e02。
    // sort_by_key 稳定，同 priority 仍按插入顺序。
    cc_events.sort_by_key(|e| (e.tick, e.channel, dispatch_priority(&e.event)));
    // 去重限定在同一 lane：不同 lane 的同名同值事件必须各自保留，
    // 否则 dispatch 按 lane 查掩码时会把正常 lane 的事件误判成被旁通。
    cc_events.dedup_by(|a, b| {
        a.channel == b.channel
            && a.lane == b.lane
            && a.event == b.event
            && a.plugin_param == b.plugin_param
    });
    Arc::new(cc_events)
}

/// Program Change 展开为 CC 事件序列（bank select + ProgramChange）。
/// 供 flatten（播放事件流）与查询式 chase 共用。
/// 事件标记 `lane = PC_LANE`（非 lane 事件，只受 skip_track 过滤）。
pub(crate) fn push_program_change(
    pc: &yinhe_types::PcEvent,
    channel: u32,
    track: u16,
    out: &mut Vec<SortedCC>,
) {
    let tick = pc.tick;
    if pc.bank_msb != 0xFF {
        out.push(SortedCC {
            tick,
            channel,
            track,
            lane: PC_LANE,
            plugin_param: None,
            event: AudioEvent::Cc {
                controller: 0,
                value: f32::from(pc.bank_msb),
            },
        });
    }
    if pc.bank_lsb != 0xFF {
        out.push(SortedCC {
            tick,
            channel,
            track,
            lane: PC_LANE,
            plugin_param: None,
            event: AudioEvent::Cc {
                controller: 32,
                value: f32::from(pc.bank_lsb),
            },
        });
    }
    out.push(SortedCC {
        tick,
        channel,
        track,
        lane: PC_LANE,
        plugin_param: None,
        event: AudioEvent::ProgramChange(pc.program),
    });
}

/// 同 tick 同 channel 内的分发优先级：0 = 参数/控制类（RPN、CC、PC），
/// 1 = PitchBend。数值小的先发，保证 PBS/FineTune/CoarseTune 等 RPN
/// 参数在 PB 使用它们之前就位。
pub(crate) fn dispatch_priority(event: &AudioEvent) -> u8 {
    match event {
        AudioEvent::PitchBend(_) => 1,
        AudioEvent::Channel(ChannelAudioEvent::Control(ControlEvent::PitchBendValue(_))) => 1,
        _ => 0,
    }
}

/// 将单个 automation 值转换成 XSynth 事件并推入 `out`。
/// 事件时刻为 tick（u32，与模型一致）。
/// `lane`：源自动化 lane 索引（轨道内），dispatch 查 AM M/S 动态掩码用。
/// 供 `flatten_automation_to_cc_events`（播放事件流）与查询式 chase 共用。
///
/// 值域边界：lane 事件 `value` 归一化 0..1（Tempo 例外），引擎内部链路吃原始
/// 整数 CC/PB/RPN，这里按 target 的 MIDI 绑定还原（全链路唯一换算点）。
pub(crate) fn emit_automation_event(
    target: &AutomationTarget,
    value: f32,
    tick: u32,
    channel: u32,
    track: u16,
    lane: u16,
    out: &mut Vec<SortedCC>,
) {
    match target {
        AutomationTarget::Param { .. } => {
            emit_param_event(target, value, tick, track, lane, out);
        }
        // Tempo 走 `conductor.tempo` 而非 `track.automation_lanes`，
        // 由 `build_tempo_map` 消费，不进入 CC 事件流。
        AutomationTarget::Tempo => {}
        _ => emit_midi_event(target, value, tick, channel, track, lane, out),
    }
}

/// `AutomationTarget::Param`（乐器/内置设备参数）事件：按设备类型还原为
/// 插件参数事件或 MIDI 绑定事件。
fn emit_param_event(
    target: &AutomationTarget,
    value: f32,
    tick: u32,
    track: u16,
    lane: u16,
    out: &mut Vec<SortedCC>,
) {
    let AutomationTarget::Param { device, id, .. } = target else {
        return;
    };
    match device {
        // 第三方乐器插件参数：占位 event 只保证排序/去重键完整；dispatch 按
        // `plugin_param` 分支走 `PluginEvent::ParamValue`（值保持归一化）。
        ParamDevice::PluginInstrument {
            channel: plugin_channel,
            uid,
        } => {
            out.push(SortedCC {
                tick,
                // 排序键用 MIDI 通道（占位 event 无 xsynth 语义，只求稳定顺序）。
                channel: u32::from(*plugin_channel),
                track,
                lane,
                event: AudioEvent::Channel(ChannelAudioEvent::Control(ControlEvent::Raw(0, 0))),
                plugin_param: Some(PluginParamEvent {
                    channel: *plugin_channel,
                    uid: *uid,
                    param_id: *id,
                    value: value.clamp(0.0, 1.0),
                }),
            });
        }
        // 内置设备参数（XSynth）：按 MIDI 绑定还原为对应的原始整数事件。
        // 回放时 dispatch 统一"广播给 insert 链上订阅的效果器 + 走常规
        // 路径透传乐器插件"（CC 广播方案）。
        ParamDevice::ChannelInstrument {
            channel: device_channel,
        } => {
            let Some(info) = builtin_param(device, *id) else {
                return; // 表外 id：无 MIDI 绑定，不产生事件。
            };
            emit_midi_binding(
                info.midi,
                value,
                tick,
                u32::from(*device_channel),
                track,
                lane,
                out,
            );
        }
    }
}

/// `AutomationTarget::CC / Rpn / Nrpn`（MIDI 原生通路）事件。
fn emit_midi_event(
    target: &AutomationTarget,
    value: f32,
    tick: u32,
    channel: u32,
    track: u16,
    lane: u16,
    out: &mut Vec<SortedCC>,
) {
    match target {
        AutomationTarget::CC { controller } => {
            // 浮点 CC 进引擎事件流（不量化）；出口按消费者取整。
            push_event(
                out,
                tick,
                channel,
                track,
                lane,
                AudioEvent::Cc {
                    controller: *controller,
                    value: restore_raw(value, 127.0) as f32,
                },
            );
        }
        AutomationTarget::Rpn { parameter } => {
            push_event(
                out,
                tick,
                channel,
                track,
                lane,
                AudioEvent::Rpn {
                    parameter: *parameter,
                    // 归一化 f32 原样进入事件流：不在此处量化，出口按值域换算。
                    value: value.clamp(0.0, 1.0),
                },
            );
        }
        AutomationTarget::Nrpn { parameter } => {
            push_event(
                out,
                tick,
                channel,
                track,
                lane,
                AudioEvent::Nrpn {
                    parameter: *parameter,
                    value: value.clamp(0.0, 1.0),
                },
            );
        }
        AutomationTarget::PitchBend => {
            push_event(
                out,
                tick,
                channel,
                track,
                lane,
                // 归一化 0..1 → 弯音 -1..1（0.5 = 中心）。
                AudioEvent::PitchBend(value.clamp(0.0, 1.0) * 2.0 - 1.0),
            );
        }
        _ => {}
    }
}

/// 推入一条非插件参数的事件。
fn push_event(
    out: &mut Vec<SortedCC>,
    tick: u32,
    channel: u32,
    track: u16,
    lane: u16,
    event: AudioEvent,
) {
    out.push(SortedCC {
        tick,
        channel,
        track,
        lane,
        plugin_param: None,
        event,
    });
}

/// RPN/NRPN 参数号的原始整数上限：RPN 0/2 是 7-bit，其余 14-bit
/// （与 `yinhe_types::automation::display_max` 同一规则，出口量化用）。
pub(crate) fn rpn_raw_max(parameter: u16) -> f32 {
    match parameter {
        0 | 2 => 127.0,
        _ => 16383.0,
    }
}

/// 归一化值 → 原始整数（四舍五入 + 钳制）。
pub(super) fn restore_raw(value: f32, max: f32) -> u16 {
    (value * max).round().clamp(0.0, max) as u16
}

/// 按 MIDI 绑定把归一化值还原成原始整数事件。
fn emit_midi_binding(
    midi: MidiBinding,
    value: f32,
    tick: u32,
    channel: u32,
    track: u16,
    lane: u16,
    out: &mut Vec<SortedCC>,
) {
    let raw = restore_raw(value, binding_max(midi));
    match midi {
        MidiBinding::Cc(controller) => {
            push_event(
                out,
                tick,
                channel,
                track,
                lane,
                AudioEvent::Cc {
                    controller,
                    value: raw as f32,
                },
            );
        }
        MidiBinding::PitchBend => {
            push_event(
                out,
                tick,
                channel,
                track,
                lane,
                AudioEvent::PitchBend((raw as f32 - 8192.0) / 8192.0),
            );
        }
        MidiBinding::Rpn(parameter) => {
            push_event(
                out,
                tick,
                channel,
                track,
                lane,
                AudioEvent::Rpn {
                    parameter,
                    value: value.clamp(0.0, 1.0),
                },
            );
        }
    }
}

/// 预计算一条音轨全部片段的有效淡入/淡出（秒），索引与 `audio_clips` 对齐。
///
/// 纯离线/模型准备阶段使用：播放线程改为按索引 O(1) 查 `AudioModel.clip_fades`，
/// 不再对每个片段做整轨线性扫描（旧实现为每块 O(n²)）。
pub(crate) fn effective_fades_all(clips: &[yinhe_core::AudioClip]) -> Vec<(f64, f64)> {
    (0..clips.len())
        .map(|i| yinhe_core::effective_fades(clips, i))
        .collect()
}
