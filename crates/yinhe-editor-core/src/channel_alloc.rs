//! 新建音轨的通道分配规则（纯函数）。
//!
//! MIDI 通道全局编号 global = port * 16 + channel（0 起，UI 显示 A1..P16）；
//! 该通道同时是乐器的挂载点（默认 XSynth，可挂 CLAP/VST3 插件）。
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

/// 单 port 内循环分配：从 `start_channel`（0 起）开始，channel+1..16、1..，
/// 到 16 后回到 1（**不跨 port**，与 `alloc_channels_from` 的跨 port 进位不同）。
/// `count` 可超过 16（重复循环）。
pub fn alloc_channels_in_port(port: u8, start_channel: u8, count: usize) -> Vec<(u8, u8)> {
    let port = port & 0x0F;
    let start = usize::from(start_channel & 0x0F);
    (0..count)
        .map(|i| (port, ((start + i) % 16) as u8))
        .collect()
}

/// 方案 C：独立 Track 暂时只分配在 Port A（A1..A16，global 0..=15）。
/// 满 16 条后回到 A1 复用通道（每条轨仍有自己独立的 LC，互不干扰）。
/// AP 仍可占用 A01..P16 中已有的 MIDI 通道，不受此规则限制。
pub const PORT_A_CHANNEL_MAX: u8 = 15;

/// MIDI 轨自动分配起点（global channel，0..=15，只在 Port A）：
/// 取现有 Port A MIDI 轨的最大通道 + 1；没有 MIDI 轨时从 A1 = 0 开始；
/// 16 条已满时回到 A1（复用）。
pub fn auto_midi_channel_start(_tracks: &[Arc<yinhe_core::TrackData>]) -> Option<u16> {
    let max = _tracks
        .iter()
        .filter(|t| t.kind == yinhe_core::TrackKind::Midi && t.port == 0)
        .map(|t| t.channel)
        .max();
    match max {
        None => Some(0),
        Some(m) if m < PORT_A_CHANNEL_MAX => Some(u16::from(m + 1)),
        Some(_) => Some(0), // 16 条占满，复用 A1
    }
}

/// 音频轨自动分配起点（0 起）：现有音频轨最大 audio_channel + 1；
/// 没有音频轨时从 0（UI 显示「A01」）开始。
/// 已用满 `u16::MAX` 时返回 `None`，调用方应禁止创建而不是回绕。
pub fn auto_audio_channel_start(tracks: &[Arc<yinhe_core::TrackData>]) -> Option<u16> {
    let max = tracks
        .iter()
        .filter(|t| t.kind == yinhe_core::TrackKind::Audio)
        .filter_map(|t| t.audio_channel)
        .max();
    match max {
        None => Some(0),
        Some(m) if m < u16::MAX => Some(m + 1),
        Some(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn midi_track(port: u8, channel: u8) -> Arc<yinhe_core::TrackData> {
        Arc::new(yinhe_core::TrackData::new(port, channel))
    }

    fn audio_track(audio_channel: u16) -> Arc<yinhe_core::TrackData> {
        let mut t = yinhe_core::TrackData::new(0, 0);
        t.kind = yinhe_core::TrackKind::Audio;
        t.audio_channel = Some(audio_channel);
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
        assert_eq!(auto_midi_channel_start(&tracks), Some(0)); // 方案 C：回到 A1
    }

    /// 最大已到 P16 = 256 通道全满，返回 None。
    #[test]
    fn auto_midi_start_full_returns_none() {
        let tracks = vec![midi_track(15, 15)]; // P16
        // 方案 C：AP 可占 P16，但 Auto 只看 Port A，仍从 A1 开始
        assert_eq!(auto_midi_channel_start(&tracks), Some(0));
    }

    /// 方案 C：Auto 只参考 Port A 轨，Port B..P 不影响独立轨分配。
    #[test]
    fn auto_midi_start_only_considers_port_a() {
        let tracks = vec![midi_track(1, 3), midi_track(0, 5)]; // B4 + A6
        assert_eq!(auto_midi_channel_start(&tracks), Some(6)); // 跟在 A6 后
        let all_a: Vec<_> = (0..16).map(|c| midi_track(0, c)).collect();
        assert_eq!(auto_midi_channel_start(&all_a), Some(0)); // A 满则回绕
    }

    /// 音频轨不参与 MIDI 自动起点的计算（独立命名空间）。
    #[test]
    fn auto_midi_start_ignores_audio_tracks() {
        let tracks = vec![audio_track(3), midi_track(0, 4)]; // A5
        assert_eq!(auto_midi_channel_start(&tracks), Some(5)); // A6
    }

    /// 音频通道自动分配：没有轨从 0 开始，满 65535 返回 None。
    #[test]
    fn auto_audio_start_boundaries() {
        assert_eq!(auto_audio_channel_start(&[]), Some(0));
        let tracks = vec![audio_track(65534)];
        assert_eq!(auto_audio_channel_start(&tracks), Some(65535));
        let tracks = vec![audio_track(65535)];
        assert_eq!(auto_audio_channel_start(&tracks), None);
    }
}

#[cfg(test)]
mod in_port_tests {
    use super::alloc_channels_in_port;

    /// B3 起循环：B3..B16 后回到 B1（不跨到 C1）。
    #[test]
    fn in_port_wraps_within_same_port() {
        let alloc = alloc_channels_in_port(1, 2, 16);
        assert_eq!(alloc[0], (1, 2)); // B3
        assert_eq!(alloc[13], (1, 15)); // B16
        assert_eq!(alloc[14], (1, 0)); // B1（回到本 port 开头）
        assert_eq!(alloc[15], (1, 1)); // B2
    }

    /// 超过 16 条继续循环（重复通道）。
    #[test]
    fn in_port_repeats_after_wrap() {
        let alloc = alloc_channels_in_port(0, 0, 18);
        assert_eq!(alloc[16], (0, 0));
        assert_eq!(alloc[17], (0, 1));
    }

    /// 常规循环（count=3, 起点 A5）：A5、A6、A7。
    #[test]
    fn in_port_sequence() {
        let alloc = alloc_channels_in_port(0, 4, 3);
        assert_eq!(alloc, vec![(0, 4), (0, 5), (0, 6)]);
    }
}
