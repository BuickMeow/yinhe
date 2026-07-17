//! Track structure operations: add, remove, move.

use std::sync::Arc;

use crate::history::UndoAction;

use super::Document;

impl Document {
    /// Insert a new MIDI track after `after_idx`. Returns UndoAction.
    /// The new track gets port 0, channel = first unused channel on port 0.
    pub fn add_track(&mut self, after_idx: usize) -> Option<UndoAction> {
        let model = &self.data.model;
        let num_tracks = model.tracks.len();
        if after_idx >= num_tracks {
            return None;
        }
        // Don't allow adding after conductor if it would insert before conductor
        let insert_idx = after_idx + 1;

        // Find a free channel on port 0
        let used_channels: std::collections::HashSet<u8> = model.tracks.iter()
            .filter(|t| t.port == 0)
            .map(|t| t.channel)
            .collect();
        let channel = (0..16u8).find(|c| !used_channels.contains(c)).unwrap_or(0);

        let mut new_track = yinhe_core::TrackData::new(0, channel);
        new_track.name = format!("A{}", channel + 1);

        let tracks_before: Vec<Arc<yinhe_core::TrackData>> = model.tracks.clone();

        let model = Arc::make_mut(&mut self.data.model);
        model.tracks.insert(insert_idx, Arc::new(new_track));

        // Remap notes: track >= insert_idx gets +1
        let note_remap: Vec<u16> = (0..tracks_before.len())
            .map(|i| if i >= insert_idx { (i + 1) as u16 } else { i as u16 })
            .collect();
        let note_remap_inverse: Vec<u16> = (0..model.tracks.len())
            .map(|i| if i == insert_idx { u16::MAX } else if i < insert_idx { i as u16 } else { (i - 1) as u16 })
            .collect();

        let tracks_after: Vec<Arc<yinhe_core::TrackData>> = model.tracks.clone();

        // Apply remap to notes
        for bucket in model.notes.iter_mut() {
            let bucket = Arc::make_mut(bucket);
            for note in bucket.iter_mut() {
                note.track = note_remap[note.track as usize];
            }
        }

        model.rebuild();
        self.data.bump_revision();

        // Update edit state
        self.sync_track_caches();
        self.edit.track_selected.clear();
        self.edit.track_selected.insert(insert_idx as u16);

        Some(UndoAction::TrackStructure {
            tracks_before,
            tracks_after,
            note_remap,
            note_remap_inverse,
        })
    }

    /// Remove the track at `idx`. Notes belonging to it are deleted.
    pub fn remove_track(&mut self, idx: usize) -> Option<UndoAction> {
        let model = &self.data.model;
        if idx >= model.tracks.len() {
            return None;
        }
        // Don't remove conductor track
        if self.edit.conductor_track_idx == Some(idx as u16) {
            return None;
        }
        // Don't remove if only 2 tracks (conductor + 1)
        if model.tracks.len() <= 2 {
            return None;
        }

        let tracks_before: Vec<Arc<yinhe_core::TrackData>> = model.tracks.clone();

        let model = Arc::make_mut(&mut self.data.model);
        model.tracks.remove(idx);

        // Remap: track < idx stays, track == idx is deleted (u16::MAX), track > idx gets -1
        let note_remap: Vec<u16> = (0..tracks_before.len())
            .map(|i| if i == idx { u16::MAX } else if i > idx { (i - 1) as u16 } else { i as u16 })
            .collect();
        let note_remap_inverse: Vec<u16> = (0..model.tracks.len())
            .map(|i| if i < idx { i as u16 } else { (i + 1) as u16 })
            .collect();

        let tracks_after: Vec<Arc<yinhe_core::TrackData>> = model.tracks.clone();

        // Apply remap: delete notes on removed track, shift others
        for bucket in model.notes.iter_mut() {
            let bucket = Arc::make_mut(bucket);
            bucket.retain(|n| note_remap[n.track as usize] != u16::MAX);
            for note in bucket.iter_mut() {
                note.track = note_remap[note.track as usize];
            }
        }
        // Mark all buckets dirty since we may have removed notes from any
        for k in 0..128 {
            model.mark_dirty(k as u8);
        }
        model.rebuild();
        let num_tracks = model.tracks.len();
        self.data.bump_revision();

        // Update edit state
        self.sync_track_caches();
        self.edit.track_selected.clear();
        // Select the track that took its place (or last track)
        let new_sel = idx.min(num_tracks - 1) as u16;
        self.edit.track_selected.insert(new_sel);

        Some(UndoAction::TrackStructure {
            tracks_before,
            tracks_after,
            note_remap,
            note_remap_inverse,
        })
    }

    /// Move track at `from_idx` to `to_idx`. Other tracks shift to fill the gap.
    pub fn move_track(&mut self, from_idx: usize, to_idx: usize) -> Option<UndoAction> {
        let model = &self.data.model;
        let num_tracks = model.tracks.len();
        if from_idx >= num_tracks || to_idx >= num_tracks || from_idx == to_idx {
            return None;
        }
        // Don't move conductor track
        if self.edit.conductor_track_idx == Some(from_idx as u16) ||
           self.edit.conductor_track_idx == Some(to_idx as u16) {
            return None;
        }

        let tracks_before: Vec<Arc<yinhe_core::TrackData>> = model.tracks.clone();

        let model = Arc::make_mut(&mut self.data.model);
        let track = model.tracks.remove(from_idx);
        model.tracks.insert(to_idx, track);

        // Build remap table
        let note_remap: Vec<u16> = (0..tracks_before.len())
            .map(|i| {
                if i == from_idx { to_idx as u16 }
                else if from_idx < to_idx && i > from_idx && i <= to_idx { (i - 1) as u16 }
                else if from_idx > to_idx && i >= to_idx && i < from_idx { (i + 1) as u16 }
                else { i as u16 }
            })
            .collect();
        let note_remap_inverse: Vec<u16> = (0..model.tracks.len())
            .map(|i| {
                if i == to_idx { from_idx as u16 }
                else if from_idx < to_idx && i >= from_idx && i < to_idx { (i + 1) as u16 }
                else if from_idx > to_idx && i > to_idx && i <= from_idx { (i - 1) as u16 }
                else { i as u16 }
            })
            .collect();

        let tracks_after: Vec<Arc<yinhe_core::TrackData>> = model.tracks.clone();

        // Apply remap to notes
        for bucket in model.notes.iter_mut() {
            let bucket = Arc::make_mut(bucket);
            for note in bucket.iter_mut() {
                note.track = note_remap[note.track as usize];
            }
        }

        model.rebuild();
        self.data.bump_revision();

        // Update edit state
        self.edit.track_info_cache = self.data.track_info();
        self.edit.track_selected.clear();
        self.edit.track_selected.insert(to_idx as u16);

        Some(UndoAction::TrackStructure {
            tracks_before,
            tracks_after,
            note_remap,
            note_remap_inverse,
        })
    }
}
