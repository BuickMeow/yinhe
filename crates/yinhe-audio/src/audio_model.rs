use std::cmp::Ordering;
use std::sync::Arc;

use std::collections::HashMap;
use xsynth_core::channel::{ChannelAudioEvent, ControlEvent};
use yinhe_core::YinModel;

use yinhe_types::automation::{MidiBinding, ParamDevice, binding_max, builtin_param};
use yinhe_types::{AutomationLane, AutomationTarget, KEY_COUNT, SegmentShape};

/// 播放事件流里的一条通道控制事件。
///
/// RPN/NRPN 不再在生成端特化成高层事件或拆 CC 序列：以原生 u16 参数号
/// 为一等事件，由 dispatch 按后端能力适配（详见各适配函数）：
/// - yinhe-synth：原生 `ControlEvent::Rpn/Nrpn`（f32 全链路，无量化）；
/// - XSynth：0/1/2 → 高层 f32 事件，其余 → CC 序列（出口量化）；
/// - VST/CLAP 插件：标准 CC 序列（101/100/6/38 或 99/98/6/38，出口量化）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum AudioEvent {
    /// 直接透传的 xsynth 通道事件（Raw CC / PitchBend / ProgramChange；
    /// 高层 PBS/FineTune/CoarseTune 仅供历史兼容，事件流不再生成）。
    Channel(ChannelAudioEvent),
    /// 原生 RPN：`parameter` u16 参数号，`value` 归一化 f32（0..=1，
    /// 与自动化 lane 同值域——全链路浮点，出口才按参数值域量化）。
    Rpn { parameter: u16, value: f32 },
    /// 原生 NRPN：`parameter` u16 参数号，`value` 归一化 f32。
    Nrpn { parameter: u16, value: f32 },
}

pub(crate) struct SortedCC {
    /// 事件时刻（tick 域，u32——模型 NoteEvent/AutomationEvent 的 tick 上限）。
    /// 音频内部统一 tick 域：dispatch/chase 比较不再需要 sample 转换。
    pub(crate) tick: u32,
    pub(crate) channel: u32,
    /// 源音轨索引，用于 mute 时过滤自动化事件。
    pub(crate) track: u16,
    /// 源自动化 lane 索引（轨道内，0..lanes.len()-1）。
    /// `u16::MAX` = 非 lane 事件（ProgramChange 展开），只受 skip_track 过滤。
    /// dispatch 时用它查 AM M/S 动态掩码（与 skip_track 并列），
    /// 使旁通切换不再需要重建事件流。
    pub(crate) lane: u16,
    pub(crate) event: AudioEvent,
    /// 插件参数事件：`Some` 时 `event` 为占位（不影响 xsynth 路径），
    /// dispatch 走乐器通道的 `PluginEvent::ParamValue`。
    pub(crate) plugin_param: Option<PluginParamEvent>,
}

/// 插件参数自动化事件（值域归一化 0..1）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PluginParamEvent {
    /// MIDI 全局通道（0..256，与 `TrackData::global_channel()` 对齐）。
    pub(crate) channel: u8,
    pub(crate) param_id: u32,
    /// 归一化值 0..1。
    pub(crate) value: f32,
}

/// `SortedCC.lane` 哨兵：ProgramChange 事件（不受 AM lane 掩码过滤）。
pub(crate) const PC_LANE: u16 = u16::MAX;

/// 活跃音符（已 NoteOn 待 NoteOff）。
///
/// `Ord` 按 `end_tick` 升序，相同 end_tick 再按 (key, channel) 区分。
/// 配合 `BinaryHeap<Reverse<ActiveNote>>` 用作 min-heap，让最早结束的音符在堆顶，
/// NoteOff 检测从 O(V) retain 全扫降到 O(ended × log V) 逐个 pop。
#[derive(Clone, Copy)]
pub(crate) struct ActiveNote {
    pub(crate) key: u8,
    /// 目标 dense 通道：MIDI 音符 = xsynth dense；乐器音符 = 乐器 dense。
    pub(crate) dense: u32,
    /// 乐器音符的 CLAP 内部 MIDI 通道（= 音轨 global_channel 低 4 位）。MIDI 音符忽略。
    pub(crate) clap_channel: u8,
    /// 是否为乐器音符（true → NoteOff 喂乐器实例）。
    pub(crate) is_instrument: bool,
    pub(crate) end_tick: u32,
    /// 源音轨索引：即时 mute 时据此精确 kill 该轨在响音符（不误伤共享通道）。
    pub(crate) track: u16,
}

impl PartialEq for ActiveNote {
    fn eq(&self, other: &Self) -> bool {
        self.end_tick == other.end_tick
            && self.key == other.key
            && self.dense == other.dense
            && self.is_instrument == other.is_instrument
            && self.clap_channel == other.clap_channel
            && self.track == other.track
    }
}
impl Eq for ActiveNote {}
impl PartialOrd for ActiveNote {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for ActiveNote {
    fn cmp(&self, other: &Self) -> Ordering {
        self.end_tick
            .cmp(&other.end_tick)
            .then(self.key.cmp(&other.key))
            .then(self.dense.cmp(&other.dense))
            .then(self.is_instrument.cmp(&other.is_instrument))
            .then(self.clap_channel.cmp(&other.clap_channel))
            .then(self.track.cmp(&other.track))
    }
}

/// 音频线程消费的可听音事件（力度 > 忽略阈值），时刻存 **tick**（u32，与模型一致）。
/// 桶内按 `start_tick` 严格升序（YinModel.notes[key] 本身按 start_tick 排序，
/// tick 天然单调，**无需再 sort**）。
///
/// `key` 不存（桶索引即 key）。`id` 用于 undo/redo 后跨 prepared model
/// 引用同一音符（暂未使用，预留）。
///
/// tick 域化：1 亿音符下每条约 24→16 字节（-0.8GB），且 dispatch 比较
/// 不再需要 tick→sample 转换；只有"渲染段边界"才转 sample（每块少量）。
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct AudibleNote {
    pub start_tick: u32,
    pub end_tick: u32,
    pub id: u32,
    pub track: u16,
    pub velocity: u8,
}

/// `PrepareNotes` 的增量结果：`[key] = Some(新桶)` 表示该 key 桶需要替换，
/// `None` 表示桶未变化（音频线程保留旧数据与旧 cursor）。
pub(crate) type AudibleDelta = Box<[Option<Vec<AudibleNote>>; KEY_COUNT]>;

/// Pre-computed model data, built on a worker thread and applied
/// atomically on the audio thread.
pub(crate) struct PreparedModel {
    pub model: AudioModel,
    pub yin_model: Arc<YinModel>,
    /// `Arc` so the same cc_events can be shared between the renderer thread
    /// (for seek/chase dispatch) and the worker thread (for chase computation)
    /// without cloning the (potentially hundreds of thousands of) events.
    pub cc_events: Arc<Vec<SortedCC>>,
    /// KEY_COUNT 个 key 桶的可听音（vel > 1），时刻为 tick（u32）。
    /// 音频线程的 seek / dispatch 只读这份列表，不再访问 YinModel.notes。
    pub audible_notes: Box<[Vec<AudibleNote>; KEY_COUNT]>,
    pub duration_samples: u64,
}

/// Lightweight per-track snapshot the audio engine actually needs.
///
/// We extract only `(global_channel)` per track plus the CC0 bank-select
/// events used for percussion-mode detection, so the audio thread holds a few
/// KB instead of a full deep clone of the model.
pub(crate) struct AudioModel {
    /// `track_channels[i]` = global channel `(port<<4)|channel` for track `i`.
    /// 轨道音符/CC 的去向由该通道是否挂了插件实例决定（挂 = 插件，未挂 = XSynth）。
    pub track_channels: Vec<u8>,
    /// Bank Select MSB declarations per track, for percussion-mode detection.
    /// `(tick, value)` pairs merged from standalone CC0 automation lanes and
    /// CC0 values folded into `PcEvent.bank_msb` (same-tick CC0+PC), sorted by
    /// tick. Values >= 120 select a drum kit (GS/XG convention), values < 120
    /// select a melodic bank. Empty Vec for tracks with no bank declaration.
    pub track_banks: Vec<Vec<(u32, u8)>>,
}

impl AudioModel {
    pub(crate) fn from_model(model: &YinModel) -> Self {
        let track_channels: Vec<u8> = model.tracks.iter().map(|t| t.global_channel()).collect();
        let track_banks: Vec<Vec<(u32, u8)>> = model
            .tracks
            .iter()
            .map(|t| {
                let mut banks: Vec<(u32, u8)> = Vec::new();
                // 独立 CC0 自动化事件（未被同 tick PC 折叠）。
                if let Some(lane) = t.automation_lanes.iter().find(|l| {
                    matches!(
                        l.target,
                        yinhe_types::AutomationTarget::CC { controller: 0 }
                    )
                }) {
                    banks.extend(
                        lane.events
                            .iter()
                            .map(|e| (e.tick, restore_raw(e.value, 127.0) as u8)),
                    );
                }
                // 同 tick 被 PC 折叠的 CC0（PcEvent.bank_msb）——否则声明会被丢掉。
                banks.extend(
                    t.program_change
                        .iter()
                        .filter_map(|pc| (pc.bank_msb != 0xFF).then_some((pc.tick, pc.bank_msb))),
                );
                banks.sort_by_key(|&(tick, _)| tick);
                banks
            })
            .collect();
        Self {
            track_channels,
            track_banks,
        }
    }

    /// Global channel for a track index, or 0 if out of range.
    pub(crate) fn track_channel(&self, track_idx: usize) -> u8 {
        self.track_channels.get(track_idx).copied().unwrap_or(0)
    }
}

/// Convert a tick value to sample position using the tempo map.
pub(crate) fn tick_to_sample(
    tick: u32,
    segments: &[yinhe_core::TempoSegment],
    tpb: u32,
    sr: f64,
) -> u64 {
    let idx = match segments.binary_search_by_key(&tick, |s| s.start_tick) {
        Ok(i) => i,
        Err(i) => i.saturating_sub(1),
    };
    let seg = &segments[idx];
    let secs = seg.start_time
        + yinhe_core::ticks_to_seconds((tick - seg.start_tick) as u64, tpb, seg.micros_per_quarter);
    (secs * sr) as u64
}

/// Convert a sample position back to the tick domain (floor), for dispatch
///基准/seek。返回满足 `tick_to_sample(t) <= sample` 的**最大** t。
///
/// 浮点误差防护：floor 后向上校验（tick_to_sample 单调，最多修正 1-2 次），
/// 保证 dispatch 不会因低估基准而漏触发已到位置的事件。
pub(crate) fn sample_to_tick(
    sample: u64,
    segments: &[yinhe_core::TempoSegment],
    tpb: u32,
    sr: f64,
) -> u32 {
    if segments.is_empty() {
        return 0;
    }
    let time = sample as f64 / sr;
    // 找 start_time <= time 的段（按 start_time 二分）
    let idx = segments
        .partition_point(|s| s.start_time <= time)
        .saturating_sub(1);
    let seg = &segments[idx];
    let secs_per_tick = if tpb == 0 {
        0.0
    } else {
        seg.micros_per_quarter as f64 / (tpb as f64 * 1_000_000.0)
    };
    let mut t = if secs_per_tick > 0.0 {
        ((time - seg.start_time) / secs_per_tick).floor() as i64 + seg.start_tick as i64
    } else {
        seg.start_tick as i64
    };
    t = t.max(0);
    // 向上校验：确保返回最大满足 tick_to_sample(t) <= sample 的 t。
    // 若 floor 因浮点误差低估，这里补到真实边界（循环最多几次）。
    while tick_to_sample(t as u32, segments, tpb, sr) <= sample && t < u32::MAX as i64 {
        t += 1;
    }
    (t - 1).max(0) as u32
}

/// 判断 lane 是否被 AM M/S 试听状态旁通。
///
/// - mute：该 lane 直接不发送；
/// - solo：音轨内有任意 lane solo 时，未 solo 的 lane 不发送
///   （主音轨音符发声与其他音轨不受影响）。
pub(crate) fn automation_lane_skipped(
    am_ms: &HashMap<(u16, yinhe_types::AutomationTarget), yinhe_types::AmMsState>,
    track_idx: u16,
    lane: &AutomationLane,
    track_has_solo: bool,
) -> bool {
    let st = am_ms.get(&(track_idx, lane.target.clone()));
    let muted = st.is_some_and(|s| s.mute);
    let soloed = st.is_some_and(|s| s.solo);
    muted || (track_has_solo && !soloed)
}

/// 预计算每条音轨的 lane 跳过掩码：`mask[track][lane_idx] = true` 表示该 lane
/// 事件在 dispatch 时被 AM M/S 旁通。切换代价 O(掩码重建)，与事件流规模无关。
///
/// 规则（与 `automation_lane_skipped` 一致）：
/// - mute：该 lane 直接不发送；
/// - solo：音轨内有任意 lane solo 时，未 solo 的 lane 不发送（作用域 = 音轨内）。
pub(crate) fn build_am_lane_skip(
    model: &YinModel,
    am_ms: &HashMap<(u16, yinhe_types::AutomationTarget), yinhe_types::AmMsState>,
) -> Vec<Vec<bool>> {
    model
        .tracks
        .iter()
        .enumerate()
        .map(|(track_idx, track)| {
            let track_idx_u16 = track_idx as u16;
            let track_has_solo = track.automation_lanes.iter().any(|l| {
                am_ms
                    .get(&(track_idx_u16, l.target.clone()))
                    .is_some_and(|s| s.solo)
            });
            track
                .automation_lanes
                .iter()
                .map(|lane| automation_lane_skipped(am_ms, track_idx_u16, lane, track_has_solo))
                .collect()
        })
        .collect()
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
/// 旁通切换无需重建本事件流。
///
/// Returns `Arc<Vec>` so the same events can be shared between the renderer and
/// the worker thread (for chase computation) without cloning.
pub(crate) fn flatten_automation_to_cc_events(
    model: &YinModel,
    density: u32,
) -> Arc<Vec<SortedCC>> {
    let density = density.max(1);
    let mut cc_events = Vec::new();

    for (track_idx, track) in model.tracks.iter().enumerate() {
        let track_idx_u16 = track_idx as u16;
        let channel = track.global_channel() as u32;

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
            event: AudioEvent::Channel(ChannelAudioEvent::Control(ControlEvent::Raw(
                0,
                pc.bank_msb,
            ))),
        });
    }
    if pc.bank_lsb != 0xFF {
        out.push(SortedCC {
            tick,
            channel,
            track,
            lane: PC_LANE,
            plugin_param: None,
            event: AudioEvent::Channel(ChannelAudioEvent::Control(ControlEvent::Raw(
                32,
                pc.bank_lsb,
            ))),
        });
    }
    out.push(SortedCC {
        tick,
        channel,
        track,
        lane: PC_LANE,
        plugin_param: None,
        event: AudioEvent::Channel(ChannelAudioEvent::ProgramChange(pc.program)),
    });
}

/// 同 tick 同 channel 内的分发优先级：0 = 参数/控制类（RPN、CC、PC），
/// 1 = PitchBendValue。数值小的先发，保证 PBS/FineTune/CoarseTune 等 RPN
/// 参数在 PB 使用它们之前就位。
pub(crate) fn dispatch_priority(event: &AudioEvent) -> u8 {
    match event {
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
        AutomationTarget::Param { device, id, .. } => match device {
            // 第三方乐器插件参数：占位 event 只保证排序/去重键完整；dispatch 按
            // `plugin_param` 分支走 `PluginEvent::ParamValue`（值保持归一化）。
            ParamDevice::PluginInstrument {
                channel: plugin_channel,
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
        },
        AutomationTarget::CC { controller } => {
            push_control(
                out,
                tick,
                channel,
                track,
                lane,
                ControlEvent::Raw(*controller, restore_raw(value, 127.0) as u8),
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
        // Tempo 走 `conductor.tempo` 而非 `track.automation_lanes`，
        // 由 `build_tempo_map` 消费，不进入 CC 事件流。
        AutomationTarget::Tempo => {}
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

/// 推入一条 xsynth 通道事件（Raw CC / PitchBend / ProgramChange 等）。
fn push_control(
    out: &mut Vec<SortedCC>,
    tick: u32,
    channel: u32,
    track: u16,
    lane: u16,
    event: ControlEvent,
) {
    push_event(
        out,
        tick,
        channel,
        track,
        lane,
        AudioEvent::Channel(ChannelAudioEvent::Control(event)),
    );
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
fn restore_raw(value: f32, max: f32) -> u16 {
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
            push_control(
                out,
                tick,
                channel,
                track,
                lane,
                ControlEvent::Raw(controller, raw as u8),
            );
        }
        MidiBinding::PitchBend => {
            push_control(
                out,
                tick,
                channel,
                track,
                lane,
                ControlEvent::PitchBendValue((raw as f32 - 8192.0) / 8192.0),
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

/// 计算片段的有效淡入/淡出时长（秒）：合并自身参数与同轨重叠片段的自动交叉淡化。
///
/// 自动交叉淡化规则（同轨两两重叠）：
/// - 另一片段在本片段内部结束（`o.end < c.end`）→ 本片段从自身起点淡入到 `o.end`；
/// - 另一片段在本片段内部开始（`o.start > c.start`）→ 本片段从 `o.start` 淡出到自身终点。
///
/// 同起点重叠不交叉（并排叠加）。结果不超过片段时长。
pub fn effective_fades(clips: &[yinhe_core::AudioClip], index: usize) -> (f64, f64) {
    let Some(c) = clips.get(index) else {
        return (0.0, 0.0);
    };
    let mut fade_in = c.fade_in_seconds.max(0.0);
    let mut fade_out = c.fade_out_seconds.max(0.0);
    for (j, o) in clips.iter().enumerate() {
        if j == index {
            continue;
        }
        let overlap_start = c.start_seconds.max(o.start_seconds);
        let overlap_end = c.end_seconds().min(o.end_seconds());
        if overlap_end <= overlap_start {
            continue;
        }
        if o.start_seconds > c.start_seconds {
            fade_out = fade_out.max(c.end_seconds() - overlap_start);
        }
        if o.end_seconds() < c.end_seconds() {
            fade_in = fade_in.max(overlap_end - c.start_seconds);
        }
    }
    (
        fade_in.min(c.duration_seconds.max(0.0)),
        fade_out.min(c.duration_seconds.max(0.0)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use yinhe_core::{ConductorData, ProjectMeta, TrackData, YinModel};
    use yinhe_types::automation::xsynth_param;
    use yinhe_types::{AutomationEvent, AutomationLane, AutomationTarget, SegmentShape};

    /// 构建 1 轨道模型，给定 automation lanes。
    fn model_with_lanes(lanes: Vec<AutomationLane>) -> YinModel {
        let conductor = ConductorData {
            tempo: AutomationLane {
                target: AutomationTarget::Tempo,
                track: 0,
                events: vec![AutomationEvent {
                    tick: 0,
                    value: 120.0,
                    shape: SegmentShape::Step,
                }],
            },
            time_sig: Vec::new(),
            key_sig: Vec::new(),
            markers: Vec::new(),
            lyrics: Vec::new(),
            chord: Vec::new(),
        };
        let mut t = TrackData::new(0, 0);
        t.automation_lanes = lanes;
        let mut model = YinModel {
            conductor: Arc::new(conductor),
            tracks: vec![Arc::new(t)],
            meta: ProjectMeta {
                ppq: 480,
                ..ProjectMeta::default()
            },
            ..Default::default()
        };
        model.rebuild();
        model
    }

    /// 在 `cc_events` 中找第一个匹配 `pred` 事件的索引。
    fn index_of<F>(events: &[SortedCC], pred: F) -> Option<usize>
    where
        F: Fn(&AudioEvent) -> bool,
    {
        events.iter().position(|e| pred(&e.event))
    }

    /// 回归测试：bank 声明（独立 CC0 自动化 + 被 PC 折叠的 CC0）必须全部进入
    /// track_banks，供鼓/乐器模式检测使用——否则 10 通道用 CC0<120 声明为乐器
    /// 时会被当作默认鼓通道（事件丢失 bug）。
    #[test]
    fn track_banks_merge_cc0_lane_and_folded_pc_bank() {
        use yinhe_types::PcEvent;

        let mut model = model_with_lanes(vec![AutomationLane {
            target: AutomationTarget::CC { controller: 0 },
            track: 0,
            events: vec![AutomationEvent {
                tick: 100,
                value: 0.0, // 乐器 bank
                shape: SegmentShape::Step,
            }],
        }]);
        // 轨道 2：同 tick CC0+PC 被折叠进 PcEvent.bank_msb（独立 lane 不存在）。
        let mut t2 = TrackData::new(0, 9);
        t2.program_change = vec![
            PcEvent {
                tick: 200,
                program: 0,
                bank_msb: 0, // 乐器 bank（XG 风格声明）
                bank_lsb: 0,
            },
            PcEvent {
                tick: 400,
                program: 0,
                bank_msb: 121, // 鼓 bank
                bank_lsb: 0xFF,
            },
        ];
        model.tracks.push(Arc::new(t2));
        // 轨道 3：无任何 bank 声明。
        model.tracks.push(Arc::new(TrackData::new(0, 3)));

        let audio = AudioModel::from_model(&model);
        assert_eq!(audio.track_banks[0], vec![(100, 0)]);
        assert_eq!(audio.track_banks[1], vec![(200, 0), (400, 121)]);
        assert!(audio.track_banks[2].is_empty());
    }

    /// 插件乐器参数 lane 展平成 plugin_param 事件：占位 event 不参与 xsynth 语义，
    /// 值保持归一化 0..1（越界钳制），param_id/乐器通道原样携带。
    #[test]
    fn emit_plugin_instrument_param_keeps_normalized() {
        let model = model_with_lanes(vec![AutomationLane {
            target: AutomationTarget::Param {
                device: ParamDevice::PluginInstrument { channel: 2 },
                id: 42,
                name: "Cutoff".into(),
            },
            track: 0,
            events: vec![
                AutomationEvent {
                    tick: 0,
                    value: 0.25,
                    shape: SegmentShape::Step,
                },
                AutomationEvent {
                    tick: 10,
                    value: 1.7, // 越界：展平钳制到 1.0
                    shape: SegmentShape::Step,
                },
            ],
        }]);
        let events = flatten_automation_to_cc_events(&model, 1);
        assert_eq!(events.len(), 2);
        for e in events.iter() {
            let pp = e.plugin_param.expect("插件参数事件必须带 plugin_param");
            assert_eq!(pp.channel, 2);
            assert_eq!(pp.param_id, 42);
            // 占位 event 恒为 Raw(0,0)：dispatch 按 plugin_param 分支优先处理。
            assert!(matches!(
                e.event,
                AudioEvent::Channel(ChannelAudioEvent::Control(ControlEvent::Raw(0, 0)))
            ));
        }
        assert_eq!(events[0].plugin_param.map(|p| p.value), Some(0.25));
        assert_eq!(events[1].plugin_param.map(|p| p.value), Some(1.0));
    }

    /// 低层 CC：归一化值在 flatten 边界还原成原始整数（CC7 = 100）。
    #[test]
    fn emit_low_level_cc_restores_integer() {
        let model = model_with_lanes(vec![AutomationLane {
            target: AutomationTarget::CC { controller: 7 },
            track: 0,
            events: vec![AutomationEvent {
                tick: 0,
                value: 100.0 / 127.0,
                shape: SegmentShape::Step,
            }],
        }]);
        let events = flatten_automation_to_cc_events(&model, 1);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].channel, 0);
        assert!(matches!(
            events[0].event,
            AudioEvent::Channel(ChannelAudioEvent::Control(ControlEvent::Raw(7, 100)))
        ));
    }

    /// XSynth 内置参数：PB 中心/满值、RPN0 半音数都按原始整数上限还原。
    #[test]
    fn emit_pitch_bend_and_rpn_restore() {
        let pb_lane = AutomationLane {
            target: AutomationTarget::Param {
                device: ParamDevice::ChannelInstrument { channel: 0 },
                id: xsynth_param::PITCH_BEND,
                name: String::new(),
            },
            track: 0,
            events: vec![
                AutomationEvent {
                    tick: 0,
                    value: 8192.0 / 16383.0,
                    shape: SegmentShape::Step,
                },
                AutomationEvent {
                    tick: 10,
                    value: 1.0,
                    shape: SegmentShape::Step,
                },
            ],
        };
        let pbs_lane = AutomationLane {
            target: AutomationTarget::Param {
                device: ParamDevice::ChannelInstrument { channel: 0 },
                id: xsynth_param::PB_SENSITIVITY,
                name: String::new(),
            },
            track: 0,
            events: vec![AutomationEvent {
                tick: 0,
                value: 2.0 / 127.0,
                shape: SegmentShape::Step,
            }],
        };
        let model = model_with_lanes(vec![pb_lane, pbs_lane]);
        let events = flatten_automation_to_cc_events(&model, 1);

        let pb_values: Vec<f32> = events
            .iter()
            .filter_map(|e| match e.event {
                AudioEvent::Channel(ChannelAudioEvent::Control(ControlEvent::PitchBendValue(
                    v,
                ))) => Some(v),
                _ => None,
            })
            .collect();
        assert_eq!(pb_values.len(), 2);
        assert_eq!(pb_values[0], 0.0, "中心 8192 → 无弯音");
        assert!(
            (pb_values[1] - (16383.0 - 8192.0) / 8192.0).abs() < 1e-6,
            "满值 16383 应还原到 +1 附近，实际 {}",
            pb_values[1]
        );

        // RPN0（PBS 内置参数）：原生 Rpn 事件，归一化 f32（2 半音 → 2/127）。
        let pbs = events.iter().find_map(|e| match e.event {
            AudioEvent::Rpn {
                parameter: 0,
                value,
            } => Some(value),
            _ => None,
        });
        assert!(
            pbs.is_some_and(|v| (v - 2.0 / 127.0).abs() < 1e-6),
            "RPN0 半音数 2 应归一化为 2/127，实际 {pbs:?}"
        );
    }

    #[test]
    fn crossfade_overlap_computes_effective_fades() {
        let clip = |id, start, dur, fade_in, fade_out| yinhe_core::AudioClip {
            id,
            source: "s".into(),
            start_seconds: start,
            offset_seconds: 0.0,
            duration_seconds: dur,
            gain: 1.0,
            fade_in_seconds: fade_in,
            fade_out_seconds: fade_out,
            reversed: false,
        };
        // A [0,4) 与 B [3,5) 重叠 [3,4)：A 在重叠区淡出 1s，B 从 3s 处淡入到 4s（1s）。
        let clips = vec![clip(1, 0.0, 4.0, 0.0, 0.0), clip(2, 3.0, 2.0, 0.0, 0.0)];
        assert_eq!(effective_fades(&clips, 0), (0.0, 1.0));
        assert_eq!(effective_fades(&clips, 1), (1.0, 0.0));
        // 自身淡入淡出更大时保留自身。
        let clips = vec![clip(1, 0.0, 4.0, 2.0, 2.0)];
        assert_eq!(effective_fades(&clips, 0), (2.0, 2.0));
        // 不重叠：保持自身（0）。
        let clips = vec![clip(1, 0.0, 2.0, 0.0, 0.0), clip(2, 3.0, 2.0, 0.0, 0.0)];
        assert_eq!(effective_fades(&clips, 0), (0.0, 0.0));
        assert_eq!(effective_fades(&clips, 1), (0.0, 0.0));
        // 完全包含：外片段获得淡入（到内片段尾）+ 淡出（从内片段头到自身尾）。
        let clips = vec![clip(1, 0.0, 10.0, 0.0, 0.0), clip(2, 3.0, 2.0, 0.0, 0.0)];
        let (fi, fo) = effective_fades(&clips, 0);
        assert!((fi - 5.0).abs() < 1e-9, "fi={fi}");
        assert!((fo - 7.0).abs() < 1e-9, "fo={fo}");
    }

    /// 回归测试：同 tick 上 RPN 0 (PBS) 必须排在 PitchBend 之前。
    /// 见 commit 3490e02：若 PB 先于 PBS，PB 会用旧 PBS 计算弯音，导致音高异常。
    #[test]
    fn rpn_pbs_must_precede_pitch_bend_at_same_tick() {
        let lanes = vec![
            AutomationLane {
                target: AutomationTarget::Param {
                    device: ParamDevice::ChannelInstrument { channel: 0 },
                    id: xsynth_param::PITCH_BEND,
                    name: String::new(),
                },
                track: 0,
                events: vec![AutomationEvent {
                    tick: 0,
                    value: 1.0,
                    shape: SegmentShape::Step,
                }],
            },
            AutomationLane {
                target: AutomationTarget::Param {
                    device: ParamDevice::ChannelInstrument { channel: 0 },
                    id: xsynth_param::PB_SENSITIVITY,
                    name: String::new(),
                },
                track: 0,
                events: vec![AutomationEvent {
                    tick: 0,
                    value: 24.0 / 127.0,
                    shape: SegmentShape::Step,
                }],
            },
        ];
        let model = model_with_lanes(lanes);
        let events = flatten_automation_to_cc_events(&model, 1);

        let pbs_idx = index_of(&events, |e| {
            matches!(e, AudioEvent::Rpn { parameter: 0, .. })
        });
        let pb_idx = index_of(&events, |e| {
            matches!(
                e,
                AudioEvent::Channel(ChannelAudioEvent::Control(ControlEvent::PitchBendValue(_)))
            )
        });

        let pbs_idx = pbs_idx.expect("PBS event should exist");
        let pb_idx = pb_idx.expect("PitchBend event should exist");
        assert!(
            pbs_idx < pb_idx,
            "PBS (index {}) must precede PitchBend (index {}) at the same tick, \
             otherwise PB uses stale PBS and pitch is wrong (regression of 3490e02)",
            pbs_idx,
            pb_idx
        );
    }

    /// AR 自动化 lane 的 M/S 试听旁通已是**运行期动态掩码**（dispatch 时查询，
    /// 见 `build_am_lane_skip`），flatten 全量展平所有 lane。
    /// 本测试验证掩码规则：mute 的 lane 被跳过；有 solo 时音轨内只有被 solo
    /// 的 lane 不被跳过（作用域 = 音轨内，主音轨音符与其他音轨不受影响）。
    #[test]
    fn am_ms_builds_lane_skip_mask() {
        let lanes = vec![
            AutomationLane {
                target: AutomationTarget::CC { controller: 7 },
                track: 0,
                events: vec![AutomationEvent {
                    tick: 0,
                    value: 100.0 / 127.0,
                    shape: SegmentShape::Step,
                }],
            },
            AutomationLane {
                target: AutomationTarget::CC { controller: 10 },
                track: 0,
                events: vec![AutomationEvent {
                    tick: 0,
                    value: 64.0 / 127.0,
                    shape: SegmentShape::Step,
                }],
            },
        ];
        let model = model_with_lanes(lanes);
        let cc7 = AutomationTarget::CC { controller: 7 };
        let cc10 = AutomationTarget::CC { controller: 10 };
        let mask_for = |am_ms: &HashMap<
            (u16, yinhe_types::AutomationTarget),
            yinhe_types::AmMsState,
        >| build_am_lane_skip(&model, am_ms);

        // 无旁通：两条 lane 都不跳过（lane 顺序 = 声明顺序）。
        assert_eq!(mask_for(&HashMap::new()), vec![vec![false, false]]);

        // mute CC7：只有 CC7 被跳过。
        let mut mutes = HashMap::new();
        mutes.insert(
            (0u16, cc7.clone()),
            yinhe_types::AmMsState {
                mute: true,
                solo: false,
            },
        );
        assert_eq!(mask_for(&mutes), vec![vec![true, false]]);

        // solo CC10：同轨未 solo 的 CC7 被跳过，CC10 保留。
        let mut solos = HashMap::new();
        solos.insert(
            (0u16, cc10.clone()),
            yinhe_types::AmMsState {
                mute: false,
                solo: true,
            },
        );
        assert_eq!(mask_for(&solos), vec![vec![true, false]]);
    }

    /// 非标准 RPN（原生 Rpn 事件，不再拆 CC 序列）：同 tick 上必须排在 PB 之前。
    #[test]
    fn nonstandard_rpn_native_event_must_precede_pitch_bend() {
        let lanes = vec![
            AutomationLane {
                target: AutomationTarget::Param {
                    device: ParamDevice::ChannelInstrument { channel: 0 },
                    id: xsynth_param::PITCH_BEND,
                    name: String::new(),
                },
                track: 0,
                events: vec![AutomationEvent {
                    tick: 0,
                    value: 1.0,
                    shape: SegmentShape::Step,
                }],
            },
            // RPN 5（非标准）→ 原生 Rpn 事件
            AutomationLane {
                target: AutomationTarget::Rpn { parameter: 5 },
                track: 0,
                events: vec![AutomationEvent {
                    tick: 0,
                    value: 100.0 / 16383.0,
                    shape: SegmentShape::Step,
                }],
            },
        ];
        let model = model_with_lanes(lanes);
        let events = flatten_automation_to_cc_events(&model, 1);

        let rpn_idx = index_of(&events, |e| {
            matches!(e, AudioEvent::Rpn { parameter: 5, .. })
        });
        let pb_idx = index_of(&events, |e| {
            matches!(
                e,
                AudioEvent::Channel(ChannelAudioEvent::Control(ControlEvent::PitchBendValue(_)))
            )
        });

        let rpn_idx = rpn_idx.expect("RPN 5 event should exist");
        let pb_idx = pb_idx.expect("PitchBend event should exist");
        assert!(
            rpn_idx < pb_idx,
            "RPN 5 (index {}) must precede PitchBend (index {}) at the same tick",
            rpn_idx,
            pb_idx
        );
    }

    /// 覆盖 NRPN（原生 Nrpn 事件）：同 tick 上必须排在 PB 之前。
    #[test]
    fn nrpn_native_event_must_precede_pitch_bend() {
        let lanes = vec![
            AutomationLane {
                target: AutomationTarget::Param {
                    device: ParamDevice::ChannelInstrument { channel: 0 },
                    id: xsynth_param::PITCH_BEND,
                    name: String::new(),
                },
                track: 0,
                events: vec![AutomationEvent {
                    tick: 0,
                    value: 1.0,
                    shape: SegmentShape::Step,
                }],
            },
            AutomationLane {
                target: AutomationTarget::Nrpn { parameter: 10 },
                track: 0,
                events: vec![AutomationEvent {
                    tick: 0,
                    value: 100.0 / 16383.0,
                    shape: SegmentShape::Step,
                }],
            },
        ];
        let model = model_with_lanes(lanes);
        let events = flatten_automation_to_cc_events(&model, 1);

        let nrpn_idx = index_of(&events, |e| {
            matches!(e, AudioEvent::Nrpn { parameter: 10, .. })
        });
        let pb_idx = index_of(&events, |e| {
            matches!(
                e,
                AudioEvent::Channel(ChannelAudioEvent::Control(ControlEvent::PitchBendValue(_)))
            )
        });

        let nrpn_idx = nrpn_idx.expect("NRPN event should exist");
        let pb_idx = pb_idx.expect("PitchBend event should exist");
        assert!(
            nrpn_idx < pb_idx,
            "NRPN (index {}) must precede PitchBend (index {}) at the same tick",
            nrpn_idx,
            pb_idx
        );
    }

    /// 同 tick 上 FineTune (RPN 1) / CoarseTune (RPN 2) 也应排在 PB 前。
    #[test]
    fn rpn_fine_and_coarse_tune_precede_pitch_bend() {
        let lanes = vec![
            AutomationLane {
                target: AutomationTarget::Param {
                    device: ParamDevice::ChannelInstrument { channel: 0 },
                    id: xsynth_param::PITCH_BEND,
                    name: String::new(),
                },
                track: 0,
                events: vec![AutomationEvent {
                    tick: 0,
                    value: 1.0,
                    shape: SegmentShape::Step,
                }],
            },
            AutomationLane {
                target: AutomationTarget::Param {
                    device: ParamDevice::ChannelInstrument { channel: 0 },
                    id: xsynth_param::FINE_TUNE,
                    name: String::new(),
                },
                track: 0,
                events: vec![AutomationEvent {
                    tick: 0,
                    value: 9000.0 / 16383.0,
                    shape: SegmentShape::Step,
                }],
            },
            AutomationLane {
                target: AutomationTarget::Param {
                    device: ParamDevice::ChannelInstrument { channel: 0 },
                    id: xsynth_param::COARSE_TUNE,
                    name: String::new(),
                },
                track: 0,
                events: vec![AutomationEvent {
                    tick: 0,
                    value: 70.0 / 127.0,
                    shape: SegmentShape::Step,
                }],
            },
        ];
        let model = model_with_lanes(lanes);
        let events = flatten_automation_to_cc_events(&model, 1);

        let fine_idx = index_of(&events, |e| {
            matches!(e, AudioEvent::Rpn { parameter: 1, .. })
        });
        let coarse_idx = index_of(&events, |e| {
            matches!(e, AudioEvent::Rpn { parameter: 2, .. })
        });
        let pb_idx = index_of(&events, |e| {
            matches!(
                e,
                AudioEvent::Channel(ChannelAudioEvent::Control(ControlEvent::PitchBendValue(_)))
            )
        });

        let fine_idx = fine_idx.expect("FineTune event should exist");
        let coarse_idx = coarse_idx.expect("CoarseTune event should exist");
        let pb_idx = pb_idx.expect("PitchBend event should exist");
        assert!(
            fine_idx < pb_idx && coarse_idx < pb_idx,
            "FineTune (idx {}) and CoarseTune (idx {}) must precede PitchBend (idx {}) at the same tick",
            fine_idx,
            coarse_idx,
            pb_idx
        );
    }
}
