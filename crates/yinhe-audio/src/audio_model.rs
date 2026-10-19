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
    /// xsynth 回退路径的**音符**事件（NoteOn/NoteOff）直通；控制事件不再走这里。
    Channel(ChannelAudioEvent),
    /// 原始 MIDI CC：`controller` u8 号，`value` f32（0..127 域，**不量化**）。
    /// 引擎内部统一浮点；出口按消费者量化——yinhe-synth 用 f32，其余
    /// （xsynth / VST3 / CLAP / yinhe-dsp）round 成 u8。
    Cc { controller: u8, value: f32 },
    /// Pitch Bend：`value` f32（-1..1）。出口转 14-bit（xsynth/插件）或 f32（yinhe-synth）。
    PitchBend(f32),
    /// Program Change：`value` u8 音色号。
    ProgramChange(u8),
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
    /// 目标乐器实例 uid（同一通道可叠加多个乐器，按 uid 精确定位）。
    pub(crate) uid: u64,
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
/// `key` 不存（桶索引即 key），`id` 不存（音频 dispatch/seek 不需要，
/// 主模型的 `NoteBucket` 已持有同一 `Arc`）。
///
/// tick 域化：1 亿音符下每条约 24→12 字节，且 dispatch 比较
/// 不再需要 tick→sample 转换；只有"渲染段边界"才转 sample（每块少量）。
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct AudibleNote {
    pub start_tick: u32,
    pub end_tick: u32,
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
    /// 每条音轨各片段的有效淡入/淡出（秒），`[track][clip]` 与
    /// `YinModel.tracks[].audio_clips` 索引对齐。模型准备阶段一次性算好，
    /// 播放线程按索引 O(1) 查询。随 AudioModel 快照整体替换而失效。
    pub clip_fades: Vec<Vec<(f64, f64)>>,
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
        let clip_fades: Vec<Vec<(f64, f64)>> = model
            .tracks
            .iter()
            .map(|t| effective_fades_all(&t.audio_clips))
            .collect();
        Self {
            track_channels,
            track_banks,
            clip_fades,
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
    // 退化 tempo（ppq/mpq = 0）：tick 与 sample 无映射关系，返回段起点。
    // 旧实现会进入下面的回补循环：每步 tick_to_sample 都返回同一值，
    // 最坏需要 ~2^32 次迭代才退出。
    if secs_per_tick <= 0.0 {
        return seg.start_tick;
    }
    // clamp 到 tick 上限：超出 u32 tick 域的大 sample 不再经 i64→u32 截断
    // 成任意值（旧值因 `t < u32::MAX` 守卫不会进入回补循环）。
    let mut t = (((time - seg.start_time) / secs_per_tick).floor() as i64 + seg.start_tick as i64)
        .clamp(0, u32::MAX as i64);
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
fn agg_key(cc: &SortedCC) -> Option<AggKey> {
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
fn agg_value(cc: &SortedCC) -> f32 {
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
    for (ch, inst) in model.channel_instruments.iter().enumerate() {
        if let Some(inst) = inst {
            let pc = yinhe_types::PcEvent {
                tick: 0,
                program: inst.program,
                bank_msb: inst.bank_msb,
                bank_lsb: 0,
            };
            push_program_change(&pc, ch as u32, u16::MAX, &mut cc_events);
        }
    }

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
                    id: 0,
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
                id: 0,
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
                device: ParamDevice::PluginInstrument { channel: 2, uid: 7 },
                id: 42,
                name: "Cutoff".into(),
            },
            track: 0,
            events: vec![
                AutomationEvent {
                    id: 0,
                    tick: 0,
                    value: 0.25,
                    shape: SegmentShape::Step,
                },
                AutomationEvent {
                    id: 0,
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
            assert_eq!(pp.uid, 7);
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

    /// 通道级默认乐器：tick 0 先下发默认 PC，MIDI 内 PC 同日生效时在其后覆盖。
    #[test]
    fn channel_instrument_default_pc_before_midi_pc() {
        let mut model = model_with_lanes(vec![]);
        Arc::make_mut(&mut model.tracks[0]).program_change = vec![yinhe_types::PcEvent {
            tick: 0,
            program: 9,
            bank_msb: 0,
            bank_lsb: 0xFF,
        }];
        model.channel_instruments[0] = Some(yinhe_core::ChannelInstrument {
            bank_msb: 0,
            program: 42,
        });
        let events = flatten_automation_to_cc_events(&model, 1);
        let pcs: Vec<u8> = events
            .iter()
            .filter_map(|e| match &e.event {
                AudioEvent::ProgramChange(p) => Some(*p),
                _ => None,
            })
            .collect();
        assert_eq!(pcs, vec![42, 9], "默认 PC 先发，MIDI PC 覆盖它");

        // 没有 MIDI PC 时只剩默认。
        Arc::make_mut(&mut model.tracks[0]).program_change.clear();
        let events = flatten_automation_to_cc_events(&model, 1);
        let pcs: Vec<u8> = events
            .iter()
            .filter_map(|e| match &e.event {
                AudioEvent::ProgramChange(p) => Some(*p),
                _ => None,
            })
            .collect();
        assert_eq!(pcs, vec![42]);
    }

    /// 低层 CC：归一化值在 flatten 边界还原成原始值域（CC7 = 100，f32 不量化）。
    #[test]
    fn emit_low_level_cc_restores_integer() {
        let model = model_with_lanes(vec![AutomationLane {
            target: AutomationTarget::CC { controller: 7 },
            track: 0,
            events: vec![AutomationEvent {
                id: 0,
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
            AudioEvent::Cc {
                controller: 7,
                value
            } if (value - 100.0).abs() < 1e-4
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
                    id: 0,
                    tick: 0,
                    value: 8192.0 / 16383.0,
                    shape: SegmentShape::Step,
                },
                AutomationEvent {
                    id: 0,
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
                id: 0,
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
                AudioEvent::PitchBend(v) => Some(v),
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

    /// 回归对照：`effective_fades_all`（预计算，进 AudioModel 快照）必须与
    /// 旧逐 index 调用的结果逐位一致——含空/单片段、重叠方向、包含、同起点叠加。
    #[test]
    fn fade_precompute_matches_per_index() {
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
        let cases: Vec<Vec<yinhe_core::AudioClip>> = vec![
            vec![],
            vec![clip(1, 0.0, 4.0, 2.0, 2.0)],
            vec![clip(1, 0.0, 4.0, 0.0, 0.0), clip(2, 3.0, 2.0, 0.0, 0.0)],
            vec![clip(1, 0.0, 10.0, 0.0, 0.0), clip(2, 3.0, 2.0, 0.0, 0.0)],
            vec![
                clip(1, 0.0, 4.0, 0.0, 0.0),
                clip(2, 3.0, 2.0, 0.0, 0.0),
                clip(3, 3.0, 3.0, 0.5, 0.0),
                clip(4, 5.0, 2.0, 0.0, 1.5),
            ],
            vec![clip(1, 0.0, 2.0, 0.0, 0.0), clip(2, 3.0, 2.0, 0.0, 0.0)],
        ];
        for clips in &cases {
            let all = effective_fades_all(clips);
            assert_eq!(all.len(), clips.len());
            for (i, fade) in all.iter().enumerate() {
                assert_eq!(
                    *fade,
                    yinhe_core::effective_fades(clips, i),
                    "片段 {i} 的预计算结果应与逐次计算一致"
                );
            }
        }
    }

    /// `sample_to_tick`：空 tempo map / sample 0 / 精确边界。
    #[test]
    fn sample_to_tick_zero_and_exact_boundaries() {
        assert_eq!(sample_to_tick(0, &[], 480, 48_000.0), 0);
        // 120 BPM + 60 BPM 两段：48000 sample 前 50 sample/tick，之后 100 sample/tick。
        let segments = vec![
            yinhe_core::TempoSegment {
                start_tick: 0,
                start_time: 0.0,
                micros_per_quarter: 500_000,
            },
            yinhe_core::TempoSegment {
                start_tick: 960,
                start_time: 1.0,
                micros_per_quarter: 1_000_000,
            },
        ];
        assert_eq!(sample_to_tick(0, &segments, 480, 48_000.0), 0);
        assert_eq!(sample_to_tick(49, &segments, 480, 48_000.0), 0);
        assert_eq!(sample_to_tick(50, &segments, 480, 48_000.0), 1);
        // tempo 段边界：tick 960 == sample 48000，两侧各差 1 sample。
        assert_eq!(sample_to_tick(47_999, &segments, 480, 48_000.0), 959);
        assert_eq!(sample_to_tick(48_000, &segments, 480, 48_000.0), 960);
        assert_eq!(sample_to_tick(48_001, &segments, 480, 48_000.0), 960);
    }

    /// `sample_to_tick` 单调不降：跨变速段密集采样。
    #[test]
    fn sample_to_tick_monotonic_across_tempo_change() {
        let segments = vec![
            yinhe_core::TempoSegment {
                start_tick: 0,
                start_time: 0.0,
                micros_per_quarter: 500_000,
            },
            yinhe_core::TempoSegment {
                start_tick: 960,
                start_time: 1.0,
                micros_per_quarter: 250_000,
            },
        ];
        let mut prev = 0u32;
        for sample in (0..240_000u64).step_by(37) {
            let t = sample_to_tick(sample, &segments, 480, 48_000.0);
            assert!(
                t >= prev,
                "sample {sample}: t={t} < prev={prev}（必须单调不降）"
            );
            prev = t;
        }
    }

    /// 极值：
    /// - ppq/mpq 退化（0）时立即返回段起点（旧实现会 ~2^32 次空转）；
    /// - sample 超出 u32 tick 域时饱和，不经 i64→u32 截断返回任意值。
    #[test]
    fn sample_to_tick_extremes_saturate() {
        let degenerate = vec![yinhe_core::TempoSegment {
            start_tick: 123,
            start_time: 0.0,
            micros_per_quarter: 0,
        }];
        assert_eq!(sample_to_tick(0, &degenerate, 480, 48_000.0), 123);
        assert_eq!(sample_to_tick(u64::MAX, &degenerate, 480, 48_000.0), 123);
        assert_eq!(sample_to_tick(1_000_000, &degenerate, 0, 48_000.0), 123);

        let segments = vec![yinhe_core::TempoSegment {
            start_tick: 0,
            start_time: 0.0,
            micros_per_quarter: 500_000,
        }];
        let tick_ceiling_sample = tick_to_sample(u32::MAX, &segments, 480, 48_000.0);
        // 超出 tick 域的 sample 与 tick 上限处的查询同值（饱和到 u32::MAX 附近），
        // 而不是旧实现的 i64→u32 截断垃圾值。
        assert_eq!(
            sample_to_tick(u64::MAX, &segments, 480, 48_000.0),
            sample_to_tick(tick_ceiling_sample, &segments, 480, 48_000.0),
        );
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
                    id: 0,
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
                    id: 0,
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
        let pb_idx = index_of(&events, |e| matches!(e, AudioEvent::PitchBend(_)));

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
                    id: 0,
                    tick: 0,
                    value: 100.0 / 127.0,
                    shape: SegmentShape::Step,
                }],
            },
            AutomationLane {
                target: AutomationTarget::CC { controller: 10 },
                track: 0,
                events: vec![AutomationEvent {
                    id: 0,
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
                    id: 0,
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
                    id: 0,
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
        let pb_idx = index_of(&events, |e| matches!(e, AudioEvent::PitchBend(_)));

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
                    id: 0,
                    tick: 0,
                    value: 1.0,
                    shape: SegmentShape::Step,
                }],
            },
            AutomationLane {
                target: AutomationTarget::Nrpn { parameter: 10 },
                track: 0,
                events: vec![AutomationEvent {
                    id: 0,
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
        let pb_idx = index_of(&events, |e| matches!(e, AudioEvent::PitchBend(_)));

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
                    id: 0,
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
                    id: 0,
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
                    id: 0,
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
        let pb_idx = index_of(&events, |e| matches!(e, AudioEvent::PitchBend(_)));

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

    // -----------------------------------------------------------------------
    // 同 tick 同通道同目标取 max（聚合辅助）
    // -----------------------------------------------------------------------

    /// 构造测试用 SortedCC。
    fn scc(tick: u32, channel: u32, event: AudioEvent) -> SortedCC {
        SortedCC {
            tick,
            channel,
            track: 0,
            lane: 0,
            plugin_param: None,
            event,
        }
    }

    /// 同 tick 同通道同 CC：聚合为一个代表，值取 max（两个顺序都生效）。
    #[test]
    fn aggregate_max_reps_cc_takes_max_both_orders() {
        let asc = vec![
            scc(
                10,
                0,
                AudioEvent::Cc {
                    controller: 7,
                    value: 40.0,
                },
            ),
            scc(
                10,
                0,
                AudioEvent::Cc {
                    controller: 7,
                    value: 100.0,
                },
            ),
        ];
        let reps = aggregate_max_reps(&asc, |_| true);
        assert_eq!(reps.len(), 1, "同 tick 同通道同 CC 只保留一条");
        assert_eq!(agg_value(&asc[reps[0]]), 100.0, "取 max");

        let desc = vec![
            scc(
                10,
                0,
                AudioEvent::Cc {
                    controller: 7,
                    value: 100.0,
                },
            ),
            scc(
                10,
                0,
                AudioEvent::Cc {
                    controller: 7,
                    value: 40.0,
                },
            ),
        ];
        let reps = aggregate_max_reps(&desc, |_| true);
        assert_eq!(reps.len(), 1);
        assert_eq!(agg_value(&desc[reps[0]]), 100.0, "顺序颠倒仍取 max");
    }

    /// 不同语义键互不干扰；不聚合事件（ProgramChange）原样保留；输出顺序 =
    /// 组内首见顺序（同 tick 内 RPN 先于 PitchBend 的优先级不被破坏）。
    #[test]
    fn aggregate_max_reps_keeps_distinct_keys_and_order() {
        let events = vec![
            // 组 1：CC7（首见）
            scc(
                10,
                0,
                AudioEvent::Cc {
                    controller: 7,
                    value: 40.0,
                },
            ),
            // 不聚合：PC 原地保留
            scc(10, 0, AudioEvent::ProgramChange(5)),
            // 组 2：CC10（首见）
            scc(
                10,
                0,
                AudioEvent::Cc {
                    controller: 10,
                    value: 80.0,
                },
            ),
            // 组 1 的代表更新（值更大）
            scc(
                10,
                0,
                AudioEvent::Cc {
                    controller: 7,
                    value: 100.0,
                },
            ),
        ];
        let reps = aggregate_max_reps(&events, |_| true);
        let out: Vec<&AudioEvent> = reps.iter().map(|&i| &events[i].event).collect();
        assert_eq!(
            out,
            vec![
                &AudioEvent::Cc {
                    controller: 7,
                    value: 100.0
                },
                &AudioEvent::ProgramChange(5),
                &AudioEvent::Cc {
                    controller: 10,
                    value: 80.0
                },
            ],
            "代表事件按首见顺序发送，CC7/CC10 互不干扰"
        );
    }

    /// 过滤先于聚合：被过滤（mute/旁通）的事件不参与 max；整组被过滤则不发。
    #[test]
    fn aggregate_max_reps_filters_before_max() {
        let events = vec![
            scc(
                10,
                0,
                AudioEvent::Cc {
                    controller: 7,
                    value: 40.0,
                },
            ),
            scc(
                10,
                0,
                AudioEvent::Cc {
                    controller: 7,
                    value: 100.0,
                },
            ),
        ];
        // 过滤掉第二条（值 100 的"被旁通"事件）→ 代表降级为 40
        let reps = aggregate_max_reps(
            &events,
            |cc| !matches!(cc.event, AudioEvent::Cc { value, .. } if value == 100.0),
        );
        assert_eq!(reps.len(), 1);
        assert_eq!(agg_value(&events[reps[0]]), 40.0);

        // 全部过滤 → 整组不发
        let reps = aggregate_max_reps(&events, |_| false);
        assert!(reps.is_empty());
    }

    /// 不同 tick 的同键事件不聚合（后写覆盖由发送顺序决定）。
    #[test]
    fn aggregate_max_reps_does_not_merge_across_ticks() {
        let events = vec![
            scc(
                10,
                0,
                AudioEvent::Cc {
                    controller: 7,
                    value: 100.0,
                },
            ),
            scc(
                20,
                0,
                AudioEvent::Cc {
                    controller: 7,
                    value: 40.0,
                },
            ),
        ];
        let reps = aggregate_max_reps(&events, |_| true);
        assert_eq!(reps, vec![0, 1]);
    }

    /// xsynth 高层 Raw CC / PitchBendValue 与内部浮点 CC / PB 同目标聚合；
    /// RPN 按参数号独立（不与 PB 混）。
    #[test]
    fn aggregate_max_reps_unifies_high_level_and_float_events() {
        let events = vec![
            scc(
                10,
                0,
                AudioEvent::Channel(ChannelAudioEvent::Control(ControlEvent::Raw(7, 40))),
            ),
            scc(
                10,
                0,
                AudioEvent::Cc {
                    controller: 7,
                    value: 100.0,
                },
            ),
            scc(
                10,
                0,
                AudioEvent::Channel(ChannelAudioEvent::Control(ControlEvent::PitchBendValue(
                    0.25,
                ))),
            ),
            scc(10, 0, AudioEvent::PitchBend(0.5)),
            scc(
                10,
                0,
                AudioEvent::Rpn {
                    parameter: 0,
                    value: 48.0 / 127.0,
                },
            ),
        ];
        let reps = aggregate_max_reps(&events, |_| true);
        let out: Vec<f32> = reps.iter().map(|&i| agg_value(&events[i])).collect();
        assert_eq!(out, vec![100.0, 0.5, 48.0 / 127.0]);
    }

    /// 插件参数事件按 (uid, param_id) 取 max；不同参数互不干扰。
    #[test]
    fn aggregate_max_reps_plugin_params_take_max() {
        let pp = |uid: u64, param_id: u32, value: f32| {
            Some(PluginParamEvent {
                channel: 0,
                uid,
                param_id,
                value,
            })
        };
        let placeholder =
            || AudioEvent::Channel(ChannelAudioEvent::Control(ControlEvent::Raw(0, 0)));
        let events = vec![
            SortedCC {
                tick: 10,
                channel: 0,
                track: 0,
                lane: 0,
                plugin_param: pp(1, 2, 0.2),
                event: placeholder(),
            },
            SortedCC {
                tick: 10,
                channel: 0,
                track: 1,
                lane: 0,
                plugin_param: pp(1, 2, 0.9),
                event: placeholder(),
            },
            SortedCC {
                tick: 10,
                channel: 0,
                track: 2,
                lane: 0,
                plugin_param: pp(1, 3, 0.4),
                event: placeholder(),
            },
        ];
        let reps = aggregate_max_reps(&events, |_| true);
        assert_eq!(reps.len(), 2, "同参数取 max，不同参数各自保留");
        let vals: Vec<f32> = reps.iter().map(|&i| agg_value(&events[i])).collect();
        assert_eq!(vals, vec![0.9, 0.4]);
    }
}
