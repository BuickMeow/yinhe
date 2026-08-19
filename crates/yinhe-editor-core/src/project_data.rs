use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use yinhe_core::{TrackInfo, YinModel};
use yinhe_types::KEY_COUNT;
use yinhe_yin::{MappingFile, ProjectFile};

/// 全局文档代次：每个 `ProjectData` 拿到全局唯一的初始 `revision`。
///
/// 所有 GPU layer 缓存键（AR notes、PR notes、AM curve）都以 revision 为成分，
/// 但缓存键里没有文档身份。此前新文档恒为 0：打开 .yin 替换空 Untitled、或
/// 先后打开两个同轨道数的工程时，缓存键与上一文档逐位相同，渲染器跳过重建，
/// 显示上一文档（或空）纹理，必须滚动改变视口 hash 才能刷新（AR 加载不刷新 bug）。
/// 全局唯一初始值让跨文档碰撞在构造层面不可能发生。
static NEXT_DOC_REVISION: AtomicU64 = AtomicU64::new(1);

/// Persistent project data. This is the source of truth for saving,
/// and the content of undo/redo snapshots.
///
/// `Arc<YinModel>` ensures snapshot clone is O(1) — actual data copy
/// only happens on `Arc::make_mut` (copy-on-write).
///
/// 项目元数据（name / artist / description / ppq / compression_level）
/// 只存一份在 `model.meta` 中，UI 直接读写，不再维护平行副本。
/// `ProjectFile` 仅在 save/load 时与 `model.meta` 互转。
#[derive(Clone)]
pub struct ProjectData {
    pub model: Arc<YinModel>,

    /// The original `project.json` structure, preserved for faithful round-tripping
    /// (保留 SoundFont 等非 meta 字段)。meta 字段在 save 时从 `model.meta` 重建。
    pub project_file: ProjectFile,
    /// The original `mapping.json` structure, preserved for faithful round-tripping.
    pub mapping_file: MappingFile,

    /// Monotonic counter bumped on every YinModel mutation or snapshot restore.
    /// Used as pianoroll layer-cache key so GPU re-renders when data changes.
    pub revision: u64,
}

impl ProjectData {
    /// Bump the revision counter to invalidate GPU layer caches.
    pub fn bump_revision(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    /// Per-key note revision counters from the model. Consumers compare these
    /// to detect which key buckets need incremental GPU re-upload.
    pub fn note_revisions(&self) -> &[u64; KEY_COUNT] {
        &self.model.note_revisions
    }

    /// Rebuild derived indices on the YinModel after mutations.
    ///
    /// O(N) where N = total notes. Call after `Arc::make_mut`.
    pub fn rebuild_model(&mut self) {
        let model = Arc::make_mut(&mut self.model);
        model.rebuild();
    }

    /// Rebuild only the dirty buckets on the YinModel.
    ///
    /// Much cheaper than `rebuild_model()` when only a few buckets changed.
    /// Skips `Arc::make_mut` on unmodified buckets, avoiding deep clones.
    pub fn rebuild_model_dirty(&mut self) {
        let model = Arc::make_mut(&mut self.model);
        model.rebuild_dirty();
    }

    /// Rebuild only `tempo_map` from `conductor.tempo` / `conductor.time_sig`.
    ///
    /// Use after Tempo automation edits (notes untouched). O(tempo_events),
    /// near-instant even for 100M-note projects.
    pub fn rebuild_tempo_map(&mut self) {
        let model = Arc::make_mut(&mut self.model);
        model.rebuild_tempo_map();
    }

    /// Get a reference to the YinModel.
    pub fn model(&self) -> &YinModel {
        &self.model
    }

    /// Rebuild `track_info_cache` from current model.
    pub fn track_info(&self) -> Vec<TrackInfo> {
        self.model
            .tracks
            .iter()
            .enumerate()
            .map(|(i, track)| TrackInfo {
                index: i as u16,
                name: track.name.clone(),
                note_count: self.model.track_note_count.get(i).copied().unwrap_or(0),
                event_count: track
                    .automation_lanes
                    .iter()
                    .map(|l| l.events.len() as u64)
                    .sum(),
                port: track.port,
                channel: track.channel,
            })
            .collect()
    }

    /// Rebuild `pc_map_cache` from current control events.
    pub fn pc_map_cache(&self) -> HashMap<u8, u8> {
        let mut pc_map = HashMap::new();
        for track in &self.model.tracks {
            for pc in &track.program_change {
                pc_map.entry(track.channel).or_insert(pc.program);
            }
        }
        pc_map
    }

    /// Sync `model.meta` back into `project_file` before saving.
    ///
    /// 只同步 meta 字段（name/artist/description/ppq/compression_level），
    /// 保留 `project_file` 中的非 meta 字段（SoundFont 配置等）。
    pub fn sync_project_file(&mut self) {
        let meta = &self.model.meta;
        self.project_file.name = meta.name.clone();
        self.project_file.artist = meta.artist.clone();
        self.project_file.description = meta.description.clone();
        self.project_file.ppq = meta.ppq;
        self.project_file.compression_level = meta.compression_level;
    }

    /// Rebuild `mapping_file` from current model tracks.
    pub fn sync_mapping_file(&mut self) {
        self.mapping_file = MappingFile::from_tracks(&self.model.tracks);
    }

    /// Construct a `ProjectData` with a given model and file structures.
    ///
    /// 初始 `revision` 取全局唯一值（见 `NEXT_DOC_REVISION` 注释）：
    /// 新文档绝不能与任何已存在文档共享缓存键。
    pub fn new(model: Arc<YinModel>, project_file: ProjectFile, mapping_file: MappingFile) -> Self {
        Self {
            model,
            project_file,
            mapping_file,
            revision: NEXT_DOC_REVISION.fetch_add(1, Ordering::Relaxed),
        }
    }
}
