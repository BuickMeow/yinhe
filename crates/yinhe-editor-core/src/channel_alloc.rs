//! 新建音轨的通道分配规则（纯函数）。
//!
//! MIDI 通道全局编号 global = port * 16 + channel（0 起，UI 显示 A1..P16）；
//! 乐器通道是与 MIDI 通道独立的命名空间（CLAP 乐器插件路由用，0 起，UI 显示 1 起）。
//! 新建音轨对话框与 `Document::add_tracks_batch` 共用这里的规则。

use std::sync::Arc;

/// 全局 MIDI 通道上限（P16，0 起）：16 个 port × 16 个 channel = 256 通道。
pub const GLOBAL_CHANNEL_MAX: u16 = 255;

/// 从 `start`（global channel，0 起）向后顺延分配最多 `count` 条通道，
/// 跨 port 进位（A16 的下一条是 B1），超出 P16 的部分截断。
/// 返回 (port, channel) 列表（均为 0 起），长度 ≤ count。
/// 允许分配到已被占用的通道——模型本来就允许多轨同通道，是否冲突由用户在
/// 对话框预览里自行判断。
pub fn alloc_channels_from(start: u16, count: usize) -> Vec<(u8, u8)> {
    (start..=GLOBAL_CHANNEL_MAX)
        .take(count)
        .map(|g| ((g >> 4) as u8, (g & 0x0F) as u8))
        .collect()
}

/// MIDI 轨自动分配起点（global channel，0 起）：
/// 取现有 MIDI 轨的最大 global channel + 1——无论前面是否有空洞都不回填
/// （已有 A11 就从 A12 开始；A16 已有则从 B1 继续）。没有 MIDI 轨时从 A1 = 0 开始。
/// 256 通道全满（最大已是 P16）时返回 None，调用方应禁止确认并提示。
pub fn auto_midi_channel_start(tracks: &[Arc<yinhe_core::TrackData>]) -> Option<u16> {
    let max = tracks
        .iter()
        .filter(|t| t.kind == yinhe_core::TrackKind::Midi)
        .map(|t| u16::from(t.global_channel()))
        .max();
    match max {
        None => Some(0),
        Some(m) if m < GLOBAL_CHANNEL_MAX => Some(m + 1),
        Some(_) => None,
    }
}

/// 乐器轨自动分配起点（0 起）：现有乐器轨最大 instrument_channel + 1；
/// 没有乐器轨时从 0（UI 显示「乐器通道 1」）开始。
pub fn auto_instrument_channel_start(tracks: &[Arc<yinhe_core::TrackData>]) -> u16 {
    tracks
        .iter()
        .filter(|t| t.kind == yinhe_core::TrackKind::Instrument)
        .filter_map(|t| t.instrument_channel)
        .max()
        .map(|m| m.saturating_add(1))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn midi_track(port: u8, channel: u8) -> Arc<yinhe_core::TrackData> {
        Arc::new(yinhe_core::TrackData::new(port, channel))
    }

    fn instrument_track(instrument_channel: u16) -> Arc<yinhe_core::TrackData> {
        let mut t = yinhe_core::TrackData::new(0, 0);
        t.kind = yinhe_core::TrackKind::Instrument;
        t.instrument_channel = Some(instrument_channel);
        Arc::new(t)
    }

    /// A16 的下一条是 B1（跨 port 进位，global 15 → 16）。
    #[test]
    fn alloc_wraps_across_ports() {
        let alloc = alloc_channels_from(15, 3);
        assert_eq!(alloc, vec![(0, 15), (1, 0), (1, 1)]);
    }

    /// 从 A1 开始分 16 条正好占满 port A。
    #[test]
    fn alloc_fills_first_port() {
        let alloc = alloc_channels_from(0, 16);
        assert_eq!(alloc.len(), 16);
        assert_eq!(alloc[0], (0, 0));
        assert_eq!(alloc[15], (0, 15));
    }

    /// 超出 P16 的部分截断：从 P16 起最多只能分到 P16 一条。
    #[test]
    fn alloc_truncates_at_p16() {
        assert_eq!(alloc_channels_from(255, 3), vec![(15, 15)]);
        // 从 P15 起要 4 条，实际只能给到 P16 两条。
        assert_eq!(alloc_channels_from(254, 4), vec![(15, 14), (15, 15)]);
    }

    /// 自动分配不回填空洞：已有 A11（global 10），前面缺不缺都从 A12 开始。
    #[test]
    fn auto_midi_start_ignores_holes() {
        let tracks = vec![midi_track(0, 10)]; // 只有 A11
        assert_eq!(auto_midi_channel_start(&tracks), Some(11)); // A12
    }

    /// A16 已有则从 B1 继续；没有 MIDI 轨时从 A1 开始。
    #[test]
    fn auto_midi_start_boundaries() {
        assert_eq!(auto_midi_channel_start(&[]), Some(0)); // A1
        let tracks = vec![midi_track(0, 15)]; // A16
        assert_eq!(auto_midi_channel_start(&tracks), Some(16)); // B1
    }

    /// 最大已到 P16 = 256 通道全满，返回 None。
    #[test]
    fn auto_midi_start_full_returns_none() {
        let tracks = vec![midi_track(15, 15)]; // P16
        assert_eq!(auto_midi_channel_start(&tracks), None);
    }

    /// 乐器轨不参与 MIDI 自动起点的计算（两套独立命名空间）。
    #[test]
    fn auto_midi_start_ignores_instrument_tracks() {
        let tracks = vec![instrument_track(3), midi_track(0, 4)]; // A5
        assert_eq!(auto_midi_channel_start(&tracks), Some(5)); // A6
    }

    /// 乐器通道自动起点：没有乐器轨从 0 开始，否则最大 + 1；
    /// MIDI 轨不参与。
    #[test]
    fn auto_instrument_start() {
        assert_eq!(auto_instrument_channel_start(&[]), 0);
        let tracks = vec![midi_track(0, 7), instrument_track(2), instrument_track(5)];
        assert_eq!(auto_instrument_channel_start(&tracks), 6);
    }
}
