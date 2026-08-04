//! GPU cull 模式下的音符 buffer 上传逻辑。
//!
//! 策略：先尝试增量 per-key 上传，失败则回退全量上传。
//! - hidden_notes 变了 → 必须全量上传（影响 per-key 内容）
//! - revision 变了且 per-key revision 匹配 → 跳过（已上传）
//! - revision 变了且部分 key 不同 → 尝试增量（count 必须匹配）
//! - revision 变了且 count 不匹配 → 全量上传
//! - 仅 track_visible 变了（note_key 变化但 revision/hidden 未变）→ 必须全量上传

use std::collections::HashSet;

use yinhe_types::NoteSource;
use yinhe_wgpu::InstanceRenderer;

/// GPU cull 上传所需的状态（含跨帧缓存的 revision/hash）。
pub struct GpuUploadState<'a> {
    pub pianoroll: &'a mut InstanceRenderer,
    pub midi: Option<&'a dyn NoteSource>,
    pub revision: u64,
    pub note_revisions: &'a [u64; 128],
    pub track_visible: &'a [bool],
    pub hidden_notes: &'a HashSet<(u16, u32, u8)>,
    /// 跨帧缓存：上次完整上传的 note_key.value()。变化时触发上传。
    pub last_cull_revision: &'a mut u64,
    /// 跨帧缓存：上次 revision（用于增量检测）。
    pub last_cull_revision_only: &'a mut u64,
    /// 跨帧缓存：上次 hidden_notes hash（用于增量检测）。
    pub last_hidden_hash: &'a mut u64,
}

/// 执行 GPU cull buffer 上传（仅 `use_gpu_cull = true` 时调用）。
pub fn upload(state: GpuUploadState) {
    let GpuUploadState {
        pianoroll,
        midi,
        revision,
        note_revisions,
        track_visible,
        hidden_notes,
        last_cull_revision,
        last_cull_revision_only,
        last_hidden_hash,
    } = state;

    // If cull isn't ready yet (e.g. just enabled, or MIDI just loaded),
    // force a full upload by invalidating the last revision.
    let cull_was_ready = pianoroll.cull_ready();
    if !cull_was_ready {
        *last_cull_revision = 0;
    }
    let note_key = yinhe_wgpu::NoteBufferKey::new(revision, track_visible, hidden_notes);
    if note_key.value() == *last_cull_revision {
        return;
    }

    let Some(midi_src) = midi else {
        *last_cull_revision = note_key.value();
        *last_cull_revision_only = revision;
        *last_hidden_hash = yinhe_wgpu::hash_hidden(hidden_notes);
        return;
    };

    if !cull_was_ready {
        // First-time upload or MIDI just loaded: force full upload.
        let (all_notes, offsets) =
            yinhe_wgpu::build_all_notes(midi_src, hidden_notes, track_visible);
        pianoroll.upload_all_notes_for_cull(&all_notes, &offsets, note_revisions);
    } else {
        let revision_changed = revision != *last_cull_revision_only;
        let hidden_changed = yinhe_wgpu::hash_hidden(hidden_notes) != *last_hidden_hash;

        if hidden_changed && !revision_changed {
            // Only hidden_notes changed → must full upload
            let (all_notes, offsets) =
                yinhe_wgpu::build_all_notes(midi_src, hidden_notes, track_visible);
            pianoroll.upload_all_notes_for_cull(&all_notes, &offsets, note_revisions);
        } else if revision_changed {
            // Revision changed → try incremental per-key upload
            let uploaded = pianoroll.uploaded_key_revisions();
            let dirty_keys: Vec<u8> = (0u8..128)
                .filter(|&k| note_revisions[k as usize] != uploaded[k as usize])
                .collect();

            if !dirty_keys.is_empty() {
                // Try incremental: build + upload each dirty key
                let mut all_ok = true;
                for &key in &dirty_keys {
                    let key_notes =
                        yinhe_wgpu::build_key_notes(midi_src, key, hidden_notes, track_visible);
                    if !pianoroll.try_incremental_key_upload(
                        key,
                        &key_notes,
                        note_revisions[key as usize],
                    ) {
                        all_ok = false;
                        break;
                    }
                }

                if !all_ok {
                    // Fallback: full upload (some key's count changed)
                    let (all_notes, offsets) =
                        yinhe_wgpu::build_all_notes(midi_src, hidden_notes, track_visible);
                    pianoroll.upload_all_notes_for_cull(&all_notes, &offsets, note_revisions);
                }
            }
            // dirty_keys.is_empty(): revision bumped but no key revisions changed
            // (e.g. conductor-only edit) → 只更新 tracking，不重传。
        } else {
            // Only track_visible changed (note_key differs but revision and
            // hidden_notes are unchanged) → must full upload.
            let (all_notes, offsets) =
                yinhe_wgpu::build_all_notes(midi_src, hidden_notes, track_visible);
            pianoroll.upload_all_notes_for_cull(&all_notes, &offsets, note_revisions);
        }
    }

    *last_cull_revision = note_key.value();
    *last_cull_revision_only = revision;
    *last_hidden_hash = yinhe_wgpu::hash_hidden(hidden_notes);
}
