/// Build the conductor entry path inside the archive.
pub fn conductor_path(name: &str) -> String {
    format!("conductor/{name}")
}

/// Format a global channel (0..127) as `A01` style label.
/// port = global_channel / 16 → letter 'A' + port (A..H)
/// raw  = global_channel % 16 → 1-indexed two-digit "01".."16"
pub fn channel_label(global_channel: u8) -> String {
    let port = global_channel / 16;
    let raw = global_channel % 16;
    format!("{}{:02}", (b'A' + port) as char, raw + 1)
}

/// Build the directory prefix for a single track.
pub fn track_prefix(global_channel: u8, uuid: &str) -> String {
    format!("channels/{}/{}", channel_label(global_channel), uuid)
}

/// Build the full path for a track notes entry.
pub fn track_notes_path(global_channel: u8, uuid: &str) -> String {
    format!("{}/notes.zst", track_prefix(global_channel, uuid))
}

/// Build the full path for a CC entry for one track.
pub fn cc_path(global_channel: u8, uuid: &str, cc_num: u8) -> String {
    format!(
        "{}/cc_{cc_num:03}.zst",
        track_prefix(global_channel, uuid)
    )
}

/// Build the full path for a pitch bend entry for one track.
pub fn pitch_path(global_channel: u8, uuid: &str) -> String {
    format!("{}/pitch.zst", track_prefix(global_channel, uuid))
}

/// Build the full path for a program change entry for one track.
pub fn pc_path(global_channel: u8, uuid: &str) -> String {
    format!("{}/pc.zst", track_prefix(global_channel, uuid))
}

/// Build the full path for an RPN entry for one track.
pub fn rpn_path(global_channel: u8, uuid: &str, rpn_num: u8) -> String {
    format!("{}/rpn_{rpn_num}.zst", track_prefix(global_channel, uuid))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_helpers() {
        assert_eq!(conductor_path("tempo.zst"), "conductor/tempo.zst");
        // global_channel = 0 → port 'A', raw_ch 0 → label "A01"
        assert_eq!(track_notes_path(0, "abc"), "channels/A01/abc/notes.zst");
        // global_channel = 17 = port 1 ('B') + raw_ch 1 (label "02") → "B02"
        assert_eq!(cc_path(17, "abc", 7), "channels/B02/abc/cc_007.zst");
        assert_eq!(pitch_path(0, "abc"), "channels/A01/abc/pitch.zst");
        assert_eq!(pc_path(0, "abc"), "channels/A01/abc/pc.zst");
        assert_eq!(rpn_path(0, "abc", 0), "channels/A01/abc/rpn_0.zst");
    }

    #[test]
    fn channel_label_format() {
        assert_eq!(channel_label(0), "A01");
        assert_eq!(channel_label(15), "A16");
        assert_eq!(channel_label(16), "B01");
        assert_eq!(channel_label(17), "B02");
        assert_eq!(channel_label(127), "H16");
    }
}
