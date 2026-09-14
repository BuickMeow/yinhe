//! 音频片段编辑操作：添加/移动/裁剪/分割/复制/删除/增益/淡入淡出/反向。
//!
//! 所有时间参数均为**秒**（绝对时间轴，不受 BPM 影响）；吸附/网格换算由
//! UI 层完成，这里只接受精确值。每个操作返回 `UndoAction::AudioClips`
//! （整轨片段快照），由调用方 push 进 undo 栈。

use std::sync::Arc;

use yinhe_core::{AudioClip, AudioSource};

use crate::history::UndoAction;

use super::Document;

impl Document {
    /// 添加音频素材到素材池（只增不减，不产生 undo：删除片段/轨道不会丢素材）。
    /// 返回共享引用供调用方立即建片段。
    pub fn add_audio_source(&mut self, source: AudioSource) -> Arc<AudioSource> {
        let model = Arc::make_mut(&mut self.data.model);
        let source = Arc::new(source);
        model.audio_sources.push(Arc::clone(&source));
        source
    }

    /// 在轨道末尾插入一个音频片段（完整时长或指定时长）。
    /// `offset_seconds` 是素材内起点；`duration_seconds` 是播放时长。
    pub fn add_audio_clip(
        &mut self,
        track_idx: usize,
        source_uuid: &str,
        start_seconds: f64,
        offset_seconds: f64,
        duration_seconds: f64,
    ) -> Option<UndoAction> {
        if duration_seconds <= 0.0 {
            return None;
        }
        let source = self.data.model.audio_source(source_uuid)?;
        let duration = duration_seconds.min((source.duration_seconds - offset_seconds).max(0.0));
        if duration <= 0.0 {
            return None;
        }
        let model = Arc::make_mut(&mut self.data.model);
        if track_idx >= model.tracks.len() {
            return None;
        }
        let id = model.alloc_audio_clip_id();
        let track = Arc::make_mut(&mut model.tracks[track_idx]);
        let before = track.audio_clips.clone();
        track.audio_clips.push(AudioClip {
            id,
            source: source_uuid.to_string(),
            start_seconds: start_seconds.max(0.0),
            offset_seconds: offset_seconds.max(0.0),
            duration_seconds: duration,
            gain: 1.0,
            fade_in_seconds: 0.0,
            fade_out_seconds: 0.0,
            reversed: false,
        });
        sort_clips(&mut track.audio_clips);
        let after = track.audio_clips.clone();
        self.data.bump_revision();
        Some(UndoAction::AudioClips {
            track_idx,
            before,
            after,
        })
    }

    /// 通用片段列表替换：供 UI 拖拽预览/一次性批量编辑使用。
    /// `new_clips` 必须属于 `track_idx`；返回 undo action（无需变更时 None）。
    pub fn replace_audio_clips(
        &mut self,
        track_idx: usize,
        new_clips: Vec<AudioClip>,
    ) -> Option<UndoAction> {
        let model = Arc::make_mut(&mut self.data.model);
        let track = model.tracks.get_mut(track_idx)?;
        let track = Arc::make_mut(track);
        let before = track.audio_clips.clone();
        if before == new_clips {
            return None;
        }
        track.audio_clips = new_clips;
        sort_clips(&mut track.audio_clips);
        let after = track.audio_clips.clone();
        self.data.bump_revision();
        Some(UndoAction::AudioClips {
            track_idx,
            before,
            after,
        })
    }

    /// 移动一组片段（时间平移，起点不允许 < 0）。
    /// `delta_seconds` 是时间轴平移量；轨道间移动由 UI 用 add+remove 组合。
    pub fn move_audio_clips(
        &mut self,
        track_idx: usize,
        ids: &[u32],
        delta_seconds: f64,
    ) -> Option<UndoAction> {
        let model = Arc::make_mut(&mut self.data.model);
        let track = model.tracks.get_mut(track_idx)?;
        let track = Arc::make_mut(track);
        let before = track.audio_clips.clone();
        for clip in track.audio_clips.iter_mut() {
            if ids.contains(&clip.id) {
                clip.start_seconds = (clip.start_seconds + delta_seconds).max(0.0);
            }
        }
        if track.audio_clips == before {
            return None;
        }
        sort_clips(&mut track.audio_clips);
        let after = track.audio_clips.clone();
        self.data.bump_revision();
        Some(UndoAction::AudioClips {
            track_idx,
            before,
            after,
        })
    }

    /// 裁剪片段左边缘到 `new_start_seconds`（保持尾端不动）。
    pub fn trim_audio_clip_start(
        &mut self,
        track_idx: usize,
        id: u32,
        new_start_seconds: f64,
    ) -> Option<UndoAction> {
        let model = Arc::make_mut(&mut self.data.model);
        let track = model.tracks.get_mut(track_idx)?;
        let track = Arc::make_mut(track);
        let before = track.audio_clips.clone();
        let clip = track.audio_clips.iter_mut().find(|c| c.id == id)?;
        let new_start = new_start_seconds
            .clamp(0.0, clip.end_seconds() - MIN_CLIP_SECONDS)
            .max(clip.start_seconds - clip.offset_seconds);
        let delta = new_start - clip.start_seconds;
        clip.start_seconds = new_start;
        clip.offset_seconds = (clip.offset_seconds + delta).max(0.0);
        clip.duration_seconds = (clip.duration_seconds - delta).max(MIN_CLIP_SECONDS);
        if track.audio_clips == before {
            return None;
        }
        sort_clips(&mut track.audio_clips);
        let after = track.audio_clips.clone();
        self.data.bump_revision();
        Some(UndoAction::AudioClips {
            track_idx,
            before,
            after,
        })
    }

    /// 裁剪片段右边缘到 `new_end_seconds`（保持起点不动）。
    pub fn trim_audio_clip_end(
        &mut self,
        track_idx: usize,
        id: u32,
        new_end_seconds: f64,
    ) -> Option<UndoAction> {
        // 先只读查询素材长度（再 make_mut 改片段，避免借用冲突）。
        let source_len = self
            .data
            .model
            .tracks
            .get(track_idx)
            .and_then(|t| t.audio_clips.iter().find(|c| c.id == id))
            .and_then(|c| self.data.model.audio_source(&c.source))
            .map(|s| s.duration_seconds)
            .unwrap_or(f64::MAX);
        let model = Arc::make_mut(&mut self.data.model);
        let track = model.tracks.get_mut(track_idx)?;
        let track = Arc::make_mut(track);
        let before = track.audio_clips.clone();
        let clip = track.audio_clips.iter_mut().find(|c| c.id == id)?;
        let max_duration = (source_len - clip.offset_seconds).max(MIN_CLIP_SECONDS);
        clip.duration_seconds = (new_end_seconds - clip.start_seconds)
            .clamp(MIN_CLIP_SECONDS, max_duration.max(MIN_CLIP_SECONDS));
        if track.audio_clips == before {
            return None;
        }
        let after = track.audio_clips.clone();
        self.data.bump_revision();
        Some(UndoAction::AudioClips {
            track_idx,
            before,
            after,
        })
    }

    /// 在 `at_seconds` 处分割片段（该时刻必须严格在片段内部）。
    /// 返回新片段（后半段）的 id。
    pub fn split_audio_clip(
        &mut self,
        track_idx: usize,
        id: u32,
        at_seconds: f64,
    ) -> Option<(UndoAction, u32)> {
        let model = Arc::make_mut(&mut self.data.model);
        if track_idx >= model.tracks.len() {
            return None;
        }
        let new_id = model.alloc_audio_clip_id();
        let track = Arc::make_mut(&mut model.tracks[track_idx]);
        let before = track.audio_clips.clone();
        let pos = track.audio_clips.iter().position(|c| c.id == id)?;
        let clip = track.audio_clips[pos].clone();
        if at_seconds <= clip.start_seconds + MIN_CLIP_SECONDS
            || at_seconds >= clip.end_seconds() - MIN_CLIP_SECONDS
        {
            return None;
        }
        let split_offset = at_seconds - clip.start_seconds;
        let mut left = clip.clone();
        left.duration_seconds = split_offset;
        // 分割后左右两段的淡入淡出不应跨过分割点：左段保留淡入，右段保留淡出。
        left.fade_out_seconds = left.fade_out_seconds.min(left.duration_seconds);
        let mut right = clip;
        right.id = new_id;
        right.start_seconds = at_seconds;
        right.offset_seconds += split_offset;
        right.duration_seconds -= split_offset;
        right.fade_in_seconds = right.fade_in_seconds.min(right.duration_seconds);
        track.audio_clips[pos] = left;
        track.audio_clips.insert(pos + 1, right);
        sort_clips(&mut track.audio_clips);
        let after = track.audio_clips.clone();
        self.data.bump_revision();
        Some((
            UndoAction::AudioClips {
                track_idx,
                before,
                after,
            },
            new_id,
        ))
    }

    /// 删除一组片段。
    pub fn delete_audio_clips(&mut self, track_idx: usize, ids: &[u32]) -> Option<UndoAction> {
        let model = Arc::make_mut(&mut self.data.model);
        let track = model.tracks.get_mut(track_idx)?;
        let track = Arc::make_mut(track);
        let before = track.audio_clips.clone();
        track.audio_clips.retain(|c| !ids.contains(&c.id));
        if track.audio_clips == before {
            return None;
        }
        let after = track.audio_clips.clone();
        self.data.bump_revision();
        Some(UndoAction::AudioClips {
            track_idx,
            before,
            after,
        })
    }

    /// 复制一组片段到 `delta_seconds` 之后（原片段保留）。返回新片段 id 列表。
    pub fn duplicate_audio_clips(
        &mut self,
        track_idx: usize,
        ids: &[u32],
        delta_seconds: f64,
    ) -> Option<(UndoAction, Vec<u32>)> {
        let model = Arc::make_mut(&mut self.data.model);
        if track_idx >= model.tracks.len() {
            return None;
        }
        let before = model.tracks[track_idx].audio_clips.clone();
        let originals: Vec<AudioClip> = model.tracks[track_idx]
            .audio_clips
            .iter()
            .filter(|c| ids.contains(&c.id))
            .cloned()
            .collect();
        if originals.is_empty() {
            return None;
        }
        let mut new_ids = Vec::with_capacity(originals.len());
        let mut new_clips = Vec::with_capacity(originals.len());
        for clip in originals {
            let id = model.alloc_audio_clip_id();
            new_ids.push(id);
            new_clips.push(AudioClip {
                id,
                start_seconds: (clip.start_seconds + delta_seconds).max(0.0),
                ..clip
            });
        }
        let track = Arc::make_mut(&mut model.tracks[track_idx]);
        track.audio_clips.extend(new_clips);
        sort_clips(&mut track.audio_clips);
        let after = track.audio_clips.clone();
        self.data.bump_revision();
        Some((
            UndoAction::AudioClips {
                track_idx,
                before,
                after,
            },
            new_ids,
        ))
    }

    /// 设置片段增益/淡入/淡出（秒）。
    pub fn set_audio_clip_params(
        &mut self,
        track_idx: usize,
        id: u32,
        gain: f32,
        fade_in_seconds: f64,
        fade_out_seconds: f64,
    ) -> Option<UndoAction> {
        let model = Arc::make_mut(&mut self.data.model);
        let track = model.tracks.get_mut(track_idx)?;
        let track = Arc::make_mut(track);
        let before = track.audio_clips.clone();
        let clip = track.audio_clips.iter_mut().find(|c| c.id == id)?;
        clip.gain = gain.max(0.0);
        clip.fade_in_seconds = fade_in_seconds.clamp(0.0, clip.duration_seconds);
        clip.fade_out_seconds = fade_out_seconds.clamp(0.0, clip.duration_seconds);
        if track.audio_clips == before {
            return None;
        }
        let after = track.audio_clips.clone();
        self.data.bump_revision();
        Some(UndoAction::AudioClips {
            track_idx,
            before,
            after,
        })
    }

    /// 反转一组片段的播放方向。
    pub fn toggle_audio_clips_reverse(
        &mut self,
        track_idx: usize,
        ids: &[u32],
    ) -> Option<UndoAction> {
        let model = Arc::make_mut(&mut self.data.model);
        let track = model.tracks.get_mut(track_idx)?;
        let track = Arc::make_mut(track);
        let before = track.audio_clips.clone();
        for clip in track.audio_clips.iter_mut() {
            if ids.contains(&clip.id) {
                clip.reversed = !clip.reversed;
            }
        }
        if track.audio_clips == before {
            return None;
        }
        let after = track.audio_clips.clone();
        self.data.bump_revision();
        Some(UndoAction::AudioClips {
            track_idx,
            before,
            after,
        })
    }

    /// 归一化：按素材峰值把片段增益设为 `1 / peak`（0 dB 峰值）。
    /// `peak` 由 UI 从波形峰值数据查询后传入（PCM 不在模型里）。
    pub fn normalize_audio_clip(
        &mut self,
        track_idx: usize,
        id: u32,
        peak: f32,
    ) -> Option<UndoAction> {
        if peak <= 1e-6 {
            return None;
        }
        let target = 1.0 / peak;
        let model = Arc::make_mut(&mut self.data.model);
        let track = model.tracks.get_mut(track_idx)?;
        let track = Arc::make_mut(track);
        let before = track.audio_clips.clone();
        let clip = track.audio_clips.iter_mut().find(|c| c.id == id)?;
        clip.gain = target;
        if track.audio_clips == before {
            return None;
        }
        let after = track.audio_clips.clone();
        self.data.bump_revision();
        Some(UndoAction::AudioClips {
            track_idx,
            before,
            after,
        })
    }
}

/// 最短片段时长（秒），防止裁剪/分割产生零长片段。
pub const MIN_CLIP_SECONDS: f64 = 0.001;

/// 按 start_seconds 排序（同起点按 id 稳定）。
fn sort_clips(clips: &mut [AudioClip]) {
    clips.sort_by(|a, b| {
        a.start_seconds
            .partial_cmp(&b.start_seconds)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.id.cmp(&b.id))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc_with_source() -> (Document, String) {
        let mut doc = Document::empty();
        let source = doc.add_audio_source(AudioSource {
            uuid: "src-1".into(),
            name: "test.wav".into(),
            data: Arc::new(vec![0u8; 16]),
            duration_seconds: 10.0,
        });
        (doc, source.uuid.clone())
    }

    #[test]
    fn add_clip_clamps_to_source_length() {
        let (mut doc, uuid) = doc_with_source();
        let action = doc.add_audio_clip(1, &uuid, 1.0, 2.0, 100.0);
        assert!(action.is_some());
        let clips = &doc.model().tracks[1].audio_clips;
        assert_eq!(clips.len(), 1);
        assert!((clips[0].duration_seconds - 8.0).abs() < 1e-9);
        assert!((clips[0].start_seconds - 1.0).abs() < 1e-9);
    }

    #[test]
    fn split_clip_creates_two_halves_with_offsets() {
        let (mut doc, uuid) = doc_with_source();
        doc.add_audio_clip(1, &uuid, 0.0, 0.0, 4.0).unwrap();
        let id = doc.model().tracks[1].audio_clips[0].id;
        let (_, new_id) = doc.split_audio_clip(1, id, 1.5).unwrap();
        let clips = &doc.model().tracks[1].audio_clips;
        assert_eq!(clips.len(), 2);
        assert!((clips[0].duration_seconds - 1.5).abs() < 1e-9);
        assert_eq!(clips[1].id, new_id);
        assert!((clips[1].start_seconds - 1.5).abs() < 1e-9);
        assert!((clips[1].offset_seconds - 1.5).abs() < 1e-9);
        assert!((clips[1].duration_seconds - 2.5).abs() < 1e-9);
    }

    #[test]
    fn split_clip_keeps_only_side_of_crossing_fades() {
        let (mut doc, uuid) = doc_with_source();
        doc.add_audio_clip(1, &uuid, 0.0, 0.0, 4.0).unwrap();
        let id = doc.model().tracks[1].audio_clips[0].id;
        doc.set_audio_clip_params(1, id, 1.0, 3.5, 3.5).unwrap();
        doc.split_audio_clip(1, id, 1.0).unwrap();
        let clips = &doc.model().tracks[1].audio_clips;
        assert!(clips[0].fade_out_seconds <= clips[0].duration_seconds);
        assert!(clips[1].fade_in_seconds <= clips[1].duration_seconds);
    }

    #[test]
    fn trim_start_moves_offset_and_keeps_end() {
        let (mut doc, uuid) = doc_with_source();
        doc.add_audio_clip(1, &uuid, 1.0, 0.0, 4.0).unwrap();
        let id = doc.model().tracks[1].audio_clips[0].id;
        doc.trim_audio_clip_start(1, id, 2.0).unwrap();
        let clip = &doc.model().tracks[1].audio_clips[0];
        assert!((clip.start_seconds - 2.0).abs() < 1e-9);
        assert!((clip.offset_seconds - 1.0).abs() < 1e-9);
        assert!((clip.duration_seconds - 3.0).abs() < 1e-9);
        assert!((clip.end_seconds() - 5.0).abs() < 1e-9);
    }

    #[test]
    fn delete_and_duplicate_clips() {
        let (mut doc, uuid) = doc_with_source();
        doc.add_audio_clip(1, &uuid, 0.0, 0.0, 2.0).unwrap();
        let id = doc.model().tracks[1].audio_clips[0].id;
        let (action, new_ids) = doc.duplicate_audio_clips(1, &[id], 1.0).unwrap();
        assert_eq!(new_ids.len(), 1);
        assert_eq!(doc.model().tracks[1].audio_clips.len(), 2);
        // undo 复制
        action.reversed().redo(&mut doc);
        assert_eq!(doc.model().tracks[1].audio_clips.len(), 1);
        // 删除
        doc.delete_audio_clips(1, &[id]).unwrap();
        assert!(doc.model().tracks[1].audio_clips.is_empty());
    }
}
