use std::collections::HashMap;
use std::sync::Arc;

use yinhe_core::YinModel;
use yinhe_types::AutomationLane;

use super::*;

/// Pre-computed model data, built on a worker thread and applied
/// atomically on the audio thread.
pub(crate) struct PreparedModel {
    pub model: AudioModel,
    pub yin_model: Arc<YinModel>,
    /// `Arc` so the same cc_events can be shared between the renderer thread
    /// (for seek/chase dispatch) and the worker thread (for chase computation)
    /// without cloning the (potentially hundreds of thousands of) events.
    pub cc_events: Arc<Vec<SortedCC>>,
    pub duration_samples: u64,
}

/// Lightweight per-track snapshot the audio engine actually needs.
///
/// We extract only `(global_channel)` per track plus the CC0 bank-select
/// events used for percussion-mode detection, so the audio thread holds a few
/// KB instead of a full deep clone of the model.
pub(crate) struct AudioModel {
    /// `track_channels[i]` = 音轨 `i` 的源 MIDI 通道 `(port<<4)|channel`；
    /// 非 MIDI 轨（音频/文件夹/Master）为 `None`，不再回退到 A01。
    /// 轨道音符/CC 的去向由该通道是否挂了插件实例决定（挂 = 插件，未挂 = XSynth）。
    pub track_channels: Vec<Option<u8>>,
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
    /// 通道级「默认乐器」的鼓通道声明（按源 MIDI 通道 0..256）：
    /// `Some(true)` = 鼓组，`Some(false)` = 显式旋律通道，`None` = 未声明。
    /// 这是界面上「鼓通道」开关的唯一去路——没有它，关鼓会被 GM 默认值
    /// 和 `bank == 128` 锁定语义吃掉（鼓通道改为旋律时声音不变）。
    pub channel_percussion: Vec<Option<bool>>,
}

impl AudioModel {
    pub(crate) fn from_model(model: &YinModel) -> Self {
        let track_channels: Vec<Option<u8>> = model
            .tracks
            .iter()
            .map(|t| t.midi_target_channel())
            .collect();
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
        // 通道级默认乐器里的鼓声明：显式 `is_drum`。
        // 未设置（None）= 不表态，交给 GM 约定（每 port 的 9 通道默认鼓）。
        let channel_percussion = (0..yinhe_types::MIDI_CHANNEL_COUNT)
            .map(|ch| {
                model
                    .channel_instruments
                    .get(ch)
                    .copied()
                    .flatten()
                    .map(|inst| inst.is_drum)
            })
            .collect();
        Self {
            track_channels,
            track_banks,
            clip_fades,
            channel_percussion,
        }
    }

    /// 音轨的源 MIDI 通道；非 MIDI 轨返回 `None`。
    pub(crate) fn track_channel(&self, track_idx: usize) -> Option<u8> {
        self.track_channels.get(track_idx).copied().flatten()
    }

    /// 逐通道鼓组模式声明，按「后写覆盖先写」的顺序展开。
    ///
    /// 覆盖顺序（与 MIDI 实际语义一致）：
    /// 1. GM 约定：每 port 的第 9 通道（索引 9/25/…）默认鼓组；
    /// 2. 通道级默认乐器（界面上的「鼓通道」开关 + 默认乐器行）；
    /// 3. 轨内 Bank Select 声明（CC0 自动化 lane / PC 折叠的 bank_msb）。
    ///
    /// CPU（`ChannelSet` / `CpuSynth`）共用这一份，
    /// 避免三条路径各写一遍、某个后端漏掉某个来源（鼓通道开关此前就只在
    /// CC0 事件流里生效，被 bank==128 锁定语义吃掉）。
    pub(crate) fn percussion_declarations(&self) -> Vec<(u16, bool)> {
        let mut out: Vec<(u16, bool)> = Vec::new();
        for port in 0..16u16 {
            out.push((port * 16 + 9, true));
        }
        for (ch, declared) in self.channel_percussion.iter().enumerate() {
            if let Some(on) = declared {
                out.push((ch as u16, *on));
            }
        }
        for (track_idx, banks) in self.track_banks.iter().enumerate() {
            if banks.is_empty() {
                continue;
            }
            let Some(ch) = self.track_channel(track_idx) else {
                continue;
            };
            let src = u16::from(ch);
            for &(_, value) in banks {
                out.push((src, value >= 120));
            }
        }
        out
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
