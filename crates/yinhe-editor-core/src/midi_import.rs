//! MIDI import: append a parsed `YinModel` to the current project.
//!
//! Design notes:
//! - Import is always an append at the project content end. Existing content
//!   never moves, so an import cannot shift notes/tempo of the current song.
//! - The source model owns its own PPQ; only the source is rescaled, never the
//!   destination.
//! - New tracks are appended after the existing ones (no O(notes) track remap).
//! - Imported note ids are allocated contiguously, which lets undo remove them
//!   by id range without keeping a second copy of the notes.
//! - Redo of an import is intentionally unavailable; `Document::undo` clears
//!   the redo stack for this action.

use std::collections::HashSet;
use std::sync::Arc;

use yinhe_core::{ConductorData, TrackData, TrackKind, YinModel};
use yinhe_types::{MAX_KEY, Note};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportChannelMode {
    /// Keep the source (port, channel). Tracks sharing a channel share the
    /// instrument and mixer strip — usually what a multi-track MIDI expects.
    KeepSource,
    /// Assign the first free global channel to each imported track, so the
    /// imported part gets its own instrument instead of merging into an
    /// existing channel.
    AllocateFree,
}

#[derive(Clone, Debug)]
pub struct MidiImportOptions {
    /// One flag per parsed source track (index = `YinModel::tracks` order).
    /// Empty or length-mismatched = include everything.
    pub include: Vec<bool>,
    pub channel_mode: ImportChannelMode,
}

impl Default for MidiImportOptions {
    fn default() -> Self {
        Self {
            include: Vec::new(),
            channel_mode: ImportChannelMode::KeepSource,
        }
    }
}

#[derive(Clone, Debug)]
pub struct MidiImportTrackInfo {
    pub name: String,
    pub port: u8,
    pub channel: u8,
    pub note_count: u64,
    pub has_automation: bool,
}

#[derive(Clone, Debug)]
pub struct MidiImportSummary {
    pub tracks: Vec<MidiImportTrackInfo>,
    pub ppq: u32,
    pub note_count: u64,
    pub length_ticks: u64,
    pub tempo_events: usize,
    pub time_sig_events: usize,
}

impl MidiImportSummary {
    pub fn from_model(model: &YinModel) -> Self {
        let tracks = model
            .tracks
            .iter()
            .enumerate()
            .map(|(i, t)| MidiImportTrackInfo {
                name: t.name.clone(),
                port: t.port,
                channel: t.channel,
                note_count: model.track_note_count.get(i).copied().unwrap_or(0),
                has_automation: !t.automation_lanes.is_empty() || !t.program_change.is_empty(),
            })
            .collect();
        Self {
            tracks,
            ppq: model.meta.ppq,
            note_count: model.note_count,
            length_ticks: model.tick_length,
            tempo_events: model.conductor.tempo.events.len(),
            time_sig_events: model.conductor.time_sig.len(),
        }
    }

    /// Summary as JSON (mobile FFI dialog).
    pub fn to_json(&self) -> String {
        let tracks: Vec<serde_json::Value> = self
            .tracks
            .iter()
            .map(|t| {
                serde_json::json!({
                    "name": t.name,
                    "port": t.port,
                    "channel": t.channel,
                    "notes": t.note_count,
                    "automation": t.has_automation,
                })
            })
            .collect();
        serde_json::json!({
            "tracks": tracks,
            "ppq": self.ppq,
            "note_count": self.note_count,
            "length_ticks": self.length_ticks,
            "tempo_events": self.tempo_events,
            "time_sig_events": self.time_sig_events,
        })
        .to_string()
    }
}

/// Undo metadata for one import.
pub struct MidiImportMerged {
    pub tracks_before: Vec<Arc<TrackData>>,
    pub conductor_before: ConductorData,
    /// Imported note ids are `[note_id_start, note_id_end)`.
    pub note_id_start: u32,
    pub note_id_end: u32,
    /// Keys that received imported notes (undo only scans these buckets).
    pub note_keys: Vec<u8>,
    pub first_track: usize,
    pub last_track: usize,
    pub start_tick: u32,
    pub end_tick: u32,
    pub note_count: u64,
}

fn unique_name(used: &mut HashSet<String>, wanted: &str, fallback_idx: usize) -> String {
    let base = if wanted.trim().is_empty() {
        format!("Track {}", fallback_idx + 1)
    } else {
        wanted.to_string()
    };
    if used.insert(base.clone()) {
        return base;
    }
    for n in 2.. {
        let candidate = format!("{base} ({n})");
        if used.insert(candidate.clone()) {
            return candidate;
        }
    }
    base
}

fn append_tempo(dst: &mut YinModel, src: &ConductorData, offset: u32) {
    if src.tempo.events.is_empty() {
        return;
    }
    let mut imported = src.tempo.events.clone();
    if let Some(first) = imported.first().cloned()
        && first.tick > offset
    {
        imported.insert(
            0,
            yinhe_types::AutomationEvent {
                tick: offset,
                ..first
            },
        );
    }
    let ticks: HashSet<u32> = imported.iter().map(|e| e.tick).collect();
    let conductor = Arc::make_mut(&mut dst.conductor);
    conductor.tempo.events.retain(|e| !ticks.contains(&e.tick));
    conductor.tempo.events.extend(imported);
    conductor.tempo.events.sort_by_key(|e| e.tick);
}

fn append_time_sig(dst: &mut YinModel, src: &ConductorData, offset: u32) {
    if src.time_sig.is_empty() {
        return;
    }
    let mut imported = src.time_sig.clone();
    if let Some(first) = imported.first().cloned()
        && first.tick > offset
    {
        imported.insert(
            0,
            yinhe_types::TimeSigEvent {
                tick: offset,
                numerator: first.numerator,
                denominator: first.denominator,
            },
        );
    }
    let ticks: HashSet<u32> = imported.iter().map(|e| e.tick).collect();
    let conductor = Arc::make_mut(&mut dst.conductor);
    conductor.time_sig.retain(|e| !ticks.contains(&e.tick));
    conductor.time_sig.extend(imported);
    conductor.time_sig.sort_by_key(|e| e.tick);
}

fn append_key_sig(dst: &mut YinModel, src: &ConductorData) {
    let ticks: HashSet<u32> = src.key_sig.iter().map(|e| e.tick).collect();
    let conductor = Arc::make_mut(&mut dst.conductor);
    conductor.key_sig.retain(|e| !ticks.contains(&e.tick));
    conductor.key_sig.extend(src.key_sig.iter().cloned());
    conductor.key_sig.sort_by_key(|e| e.tick);
}

fn append_markers(dst: &mut YinModel, src: &ConductorData) {
    if src.markers.is_empty() {
        return;
    }
    let conductor = Arc::make_mut(&mut dst.conductor);
    conductor.markers.extend(src.markers.iter().cloned());
    conductor.markers.sort_by_key(|e| e.tick);
}

fn append_conductor_lyrics_chords(dst: &mut YinModel, src: &ConductorData) {
    let conductor = Arc::make_mut(&mut dst.conductor);
    conductor.lyrics.extend(src.lyrics.iter().cloned());
    conductor.lyrics.sort_by_key(|e| e.tick);
    conductor.chord.extend(src.chord.iter().cloned());
    conductor.chord.sort_by_key(|e| e.tick);
}

fn event_end(src: &YinModel) -> u32 {
    let mut end = 0u32;
    for ev in &src.conductor.tempo.events {
        end = end.max(ev.tick);
    }
    for ev in &src.conductor.time_sig {
        end = end.max(ev.tick);
    }
    for ev in &src.conductor.key_sig {
        end = end.max(ev.tick);
    }
    for ev in &src.conductor.markers {
        end = end.max(ev.tick);
    }
    for ev in &src.conductor.lyrics {
        end = end.max(ev.tick);
    }
    for ev in &src.conductor.chord {
        end = end.max(ev.tick);
    }
    for track in &src.tracks {
        for lane in &track.automation_lanes {
            if let Some(ev) = lane.events.last() {
                end = end.max(ev.tick);
            }
        }
        if let Some(ev) = track.program_change.last() {
            end = end.max(ev.tick);
        }
        if let Some(ev) = track.lyrics.last() {
            end = end.max(ev.tick);
        }
        if let Some(ev) = track.chord.last() {
            end = end.max(ev.tick);
        }
    }
    end
}

/// Merge `src` into `dst`.
///
/// `progress` is called with 0.0..=1.0 while notes are copied.
/// Returns `None` when no track is included.
pub fn merge_midi_into_model(
    dst: &mut YinModel,
    src: YinModel,
    opts: &MidiImportOptions,
    mut progress: impl FnMut(f32),
) -> Option<MidiImportMerged> {
    let tracks_before = dst.tracks.clone();
    let conductor_before = (*dst.conductor).clone();
    let offset = dst.content_end_tick();
    let dst_ppq = dst.meta.ppq.max(1);

    let mut src = src;
    src.rescale_and_offset_ticks(dst_ppq, offset);

    let include = if opts.include.len() == src.tracks.len() {
        opts.include.clone()
    } else {
        vec![true; src.tracks.len()]
    };

    let mut used_names: HashSet<String> = dst.tracks.iter().map(|t| t.name.clone()).collect();
    let mut used_channels: HashSet<u8> = dst.tracks.iter().map(|t| t.global_channel()).collect();

    let mut mapping: Vec<Option<usize>> = vec![None; src.tracks.len()];
    let mut first_track = None;
    let mut last_track = 0usize;
    for (i, track) in src.tracks.iter().enumerate() {
        if !include.get(i).copied().unwrap_or(true) {
            continue;
        }
        let mut new_track = (**track).clone();
        new_track.uuid = uuid::Uuid::new_v4().to_string();
        new_track.notes.clear();
        new_track.kind = TrackKind::Midi;
        new_track.muted = false;
        new_track.soloed = false;
        let new_idx = dst.tracks.len();
        new_track.name = unique_name(&mut used_names, &new_track.name, new_idx);
        if opts.channel_mode == ImportChannelMode::AllocateFree
            && let Some(ch) = (0..=u8::MAX).find(|ch| !used_channels.contains(ch))
        {
            new_track.port = ch >> 4;
            new_track.channel = ch & 0x0F;
            used_channels.insert(ch);
        }
        for lane in &mut new_track.automation_lanes {
            lane.track = new_idx as u16;
        }
        mapping[i] = Some(new_idx);
        first_track.get_or_insert(new_idx);
        last_track = new_idx;
        dst.tracks.push(Arc::new(new_track));
    }
    let first_track = first_track?;
    let new_track_count = dst.tracks.len() - tracks_before.len();
    dst.push_empty_track_stats(new_track_count);

    let mut imported_end = event_end(&src).max(offset);

    append_tempo(dst, &src.conductor, offset);
    append_time_sig(dst, &src.conductor, offset);
    append_key_sig(dst, &src.conductor);
    append_markers(dst, &src.conductor);
    append_conductor_lyrics_chords(dst, &src.conductor);

    let note_id_start = dst.next_note_id;
    let mut note_count = 0u64;
    let mut note_keys: Vec<u8> = Vec::new();
    for key in 0..=MAX_KEY {
        let mut notes: Vec<Note> = Vec::new();
        for n in src.notes[key as usize].iter() {
            let Some(Some(new_track)) = mapping.get(n.track as usize).copied() else {
                continue;
            };
            let mut note = *n;
            note.id = dst.alloc_note_id();
            note.track = new_track as u16;
            if note.end_tick <= note.start_tick {
                note.end_tick = note.start_tick.saturating_add(1);
            }
            imported_end = imported_end.max(note.end_tick);
            notes.push(note);
        }
        if !notes.is_empty() {
            note_count += notes.len() as u64;
            note_keys.push(key);
            Arc::make_mut(&mut dst.notes[key as usize]).insert_batch_sorted(notes);
            dst.mark_dirty(key);
        }
        progress((key as f32 + 1.0) / 128.0);
    }
    let note_id_end = dst.next_note_id;

    // Note buckets are merged sorted, and track stats arrays were grown before
    // insertion. `rebuild_dirty` updates only touched keys; conductor lists
    // changed independently, so rebuild the tempo map explicitly.
    dst.rebuild_dirty();
    // 纯事件导入（标记/速度没有音符）也要把时间线扩展到导入末尾。
    dst.tick_length = dst.tick_length.max(imported_end as u64);
    dst.rebuild_tempo_map();
    dst.sync_automation_lane_tracks();

    Some(MidiImportMerged {
        tracks_before,
        conductor_before,
        note_id_start,
        note_id_end,
        note_keys,
        first_track,
        last_track,
        start_tick: offset,
        end_tick: imported_end,
        note_count,
    })
}

/// Restore the model to the state stored in an import undo entry.
pub fn revert_midi_import(
    model: &mut YinModel,
    tracks_before: Vec<Arc<TrackData>>,
    conductor_before: ConductorData,
    note_id_start: u32,
    note_id_end: u32,
    note_keys: &[u8],
) {
    yinhe_core::remove_notes_by_id_range(model, note_id_start, note_id_end, note_keys);
    model.tracks = tracks_before;
    // 统计数组必须跟着轨道一起收缩：否则下一次 push_empty_track_stats 的新轨
    // 统计会落到旧长度之后，track_note_count 与 tracks 索引错位。
    model.track_note_count.truncate(model.tracks.len());
    model.track_audible_count.truncate(model.tracks.len());
    model.conductor = Arc::new(conductor_before);
    model.rebuild_dirty();
    model.rebuild_tempo_map();
    model.sync_automation_lane_tracks();
}

#[cfg(test)]
mod tests {
    use super::*;
    use yinhe_core::NoteEvent;
    use yinhe_core::TrackData;

    fn source_model() -> YinModel {
        let mut model = YinModel {
            tracks: vec![Arc::new(TrackData::new(0, 0))],
            ..Default::default()
        };
        Arc::make_mut(&mut model.tracks[0]).name = "Piano".into();
        model.load_track_notes(vec![vec![
            NoteEvent {
                id: 0,
                start_tick: 0,
                end_tick: 480,
                key: 60,
                velocity: 100,
            },
            NoteEvent {
                id: 0,
                start_tick: 480,
                end_tick: 960,
                key: 64,
                velocity: 90,
            },
        ]]);
        model.rebuild();
        model
    }

    #[test]
    fn append_uses_content_end_and_selects_new_track() {
        let mut dst = YinModel {
            tracks: vec![Arc::new(TrackData::new(0, 0))],
            ..Default::default()
        };
        dst.load_track_notes(vec![vec![NoteEvent {
            id: 0,
            start_tick: 0,
            end_tick: 1000,
            key: 50,
            velocity: 100,
        }]]);
        dst.rebuild();
        let merged = merge_midi_into_model(
            &mut dst,
            source_model(),
            &MidiImportOptions::default(),
            |_| {},
        )
        .expect("应导入");
        assert_eq!(merged.start_tick, 1000);
        assert_eq!(merged.first_track, 1);
        assert_eq!(dst.tracks.len(), 2);
        assert_eq!(dst.tracks[1].name, "Piano");
        assert_eq!(dst.note_count, 3);
        assert_eq!(dst.tick_length, 1960);
        // 两个导入音符的 id 连续，供 undo 按区间删除。
        assert_eq!(merged.note_id_end - merged.note_id_start, 2);
        let ids: Vec<u32> = dst.notes[60]
            .iter()
            .chain(dst.notes[64].iter())
            .map(|n| n.id)
            .collect();
        assert!(
            ids.iter()
                .all(|&id| id >= merged.note_id_start && id < merged.note_id_end)
        );
    }

    #[test]
    fn undo_removes_imported_notes_and_tracks() {
        let mut dst = YinModel {
            tracks: vec![Arc::new(TrackData::new(0, 0))],
            ..Default::default()
        };
        dst.load_track_notes(vec![vec![NoteEvent {
            id: 0,
            start_tick: 0,
            end_tick: 1000,
            key: 50,
            velocity: 100,
        }]]);
        dst.rebuild();
        let merged = merge_midi_into_model(
            &mut dst,
            source_model(),
            &MidiImportOptions::default(),
            |_| {},
        )
        .expect("应导入");
        revert_midi_import(
            &mut dst,
            merged.tracks_before,
            merged.conductor_before,
            merged.note_id_start,
            merged.note_id_end,
            &merged.note_keys,
        );
        assert_eq!(dst.tracks.len(), 1);
        assert_eq!(dst.note_count, 1);
        assert_eq!(dst.tick_length, 1000);
        assert_eq!(dst.notes[60].len(), 0);
        assert_eq!(dst.notes[64].len(), 0);
        assert_eq!(
            dst.track_note_count.len(),
            dst.tracks.len(),
            "undo 后轨道统计数组必须与轨道数一致"
        );
    }

    #[test]
    fn source_ppq_is_rescaled_not_destination() {
        let mut dst = YinModel {
            tracks: vec![Arc::new(TrackData::new(0, 0))],
            ..Default::default()
        };
        dst.meta.ppq = 960;
        dst.load_track_notes(vec![vec![NoteEvent {
            id: 0,
            start_tick: 0,
            end_tick: 1920,
            key: 50,
            velocity: 100,
        }]]);
        dst.rebuild();
        let mut src = source_model();
        src.meta.ppq = 480;
        let merged = merge_midi_into_model(&mut dst, src, &MidiImportOptions::default(), |_| {})
            .expect("应导入");
        assert_eq!(dst.meta.ppq, 960);
        assert_eq!(merged.start_tick, 1920);
        let n = dst.notes[60].iter().next().copied().expect("保留");
        assert_eq!(n.start_tick, 1920);
        assert_eq!(n.end_tick, 2880);
    }

    #[test]
    fn alloc_free_channel_avoids_existing() {
        let mut dst = YinModel {
            tracks: vec![Arc::new(TrackData::new(0, 0))],
            ..Default::default()
        };
        dst.rebuild();
        let mut src = source_model();
        src.tracks[0] = Arc::new({
            let mut t = (*src.tracks[0]).clone();
            t.channel = 0;
            t
        });
        let opts = MidiImportOptions {
            include: vec![],
            channel_mode: ImportChannelMode::AllocateFree,
        };
        merge_midi_into_model(&mut dst, src, &opts, |_| {}).expect("应导入");
        assert_eq!(dst.tracks[1].global_channel(), 1);
    }

    #[test]
    fn tempo_is_appended_at_end_and_undone() {
        use yinhe_types::{AutomationEvent, SegmentShape};

        let mut dst = YinModel {
            tracks: vec![Arc::new(TrackData::new(0, 0))],
            ..Default::default()
        };
        dst.load_track_notes(vec![vec![NoteEvent {
            id: 0,
            start_tick: 0,
            end_tick: 1000,
            key: 50,
            velocity: 100,
        }]]);
        dst.rebuild();

        let mut src = source_model();
        Arc::make_mut(&mut src.conductor)
            .tempo
            .events
            .push(AutomationEvent {
                id: 0,
                tick: 0,
                value: 90.0,
                shape: SegmentShape::Step,
            });
        let merged = merge_midi_into_model(&mut dst, src, &MidiImportOptions::default(), |_| {})
            .expect("应导入");
        assert_eq!(dst.conductor.tempo.events.len(), 1);
        assert_eq!(dst.conductor.tempo.events[0].tick, 1000);
        assert_eq!(dst.conductor.tempo.events[0].value, 90.0);

        revert_midi_import(
            &mut dst,
            merged.tracks_before,
            merged.conductor_before,
            merged.note_id_start,
            merged.note_id_end,
            &merged.note_keys,
        );
        assert!(
            dst.conductor.tempo.events.is_empty(),
            "undo 应恢复导入前速度表"
        );
    }
}
