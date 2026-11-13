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
    /// 工程最终封面（仅一张）；保存时写入 `.yin` 的可选 cover 段。
    pub cover: Option<yinhe_core::CoverArt>,
    /// 累计前台工作时间（秒）；运行时更新，保存时写入 project_file。
    pub work_time_secs: u64,

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
        self.project_file.default_bpm = meta.default_bpm;
        self.project_file.default_time_sig = meta.default_time_sig;
        self.project_file.album = meta.album.clone();
        self.project_file.album_artist = meta.album_artist.clone();
        self.project_file.genre = meta.genre.clone();
        self.project_file.year = meta.year.clone();
        self.project_file.track_no = meta.track_no;
        self.project_file.disc_no = meta.disc_no;
        self.project_file.copyright = meta.copyright.clone();
        self.project_file.artists = meta.artists.clone();
        self.project_file.work_time_secs = self.work_time_secs;
        self.project_file.cover = self.cover.clone();
    }

    /// Rebuild `mapping_file` from current model tracks.
    ///
    /// `channel_instruments` 不在轨道里，`MappingFile::from_tracks` 无法
    /// 从轨道重建；保存前必须从 model 补回，否则通道默认乐器会被清空。
    pub fn sync_mapping_file(&mut self) {
        let channel_instruments = self.model.channel_instruments;
        let synth_params: Vec<_> = self
            .model
            .synth_params
            .iter()
            .enumerate()
            .filter(|(_, p)| !p.is_default())
            .map(|(ch, p)| yinhe_yin::ChannelSynthParams {
                channel: ch.min(255) as u8,
                params: *p,
            })
            .collect();
        self.mapping_file = MappingFile::from_tracks(&self.model.tracks);
        self.mapping_file.channel_instruments = channel_instruments;
        self.mapping_file.synth_params = synth_params;
    }

    /// Construct a `ProjectData` with a given model and file structures.
    ///
    /// 初始 `revision` 取全局唯一值（见 `NEXT_DOC_REVISION` 注释）：
    /// 新文档绝不能与任何已存在文档共享缓存键。
    pub fn new(
        mut model: Arc<YinModel>,
        project_file: ProjectFile,
        mapping_file: MappingFile,
    ) -> Self {
        // 旧工程的音频片段只有秒域时间：打开时按当前 tempo 补一次 beat
        // 锚点，之后改 BPM 才会跟随。无音频片段时是 O(tracks) 空扫。
        Arc::make_mut(&mut model).sync_audio_clip_beats();
        let cover = project_file.cover.clone();
        let work_time_secs = model.meta.work_time_secs;
        Self {
            model,
            project_file,
            mapping_file,
            cover,
            work_time_secs,
            revision: NEXT_DOC_REVISION.fetch_add(1, Ordering::Relaxed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yinhe_core::ChannelInstrument;

    #[test]
    fn work_time_roundtrips_through_project_file() {
        let mut model = YinModel::default();
        model.meta.work_time_secs = 42;
        let mut data = ProjectData::new(
            Arc::new(model),
            ProjectFile::default(),
            MappingFile::default(),
        );
        assert_eq!(data.work_time_secs, 42, "加载时从 meta 恢复");

        data.work_time_secs = 99;
        data.sync_project_file();
        assert_eq!(
            data.project_file.work_time_secs, 99,
            "保存时写入 project.json"
        );
    }

    #[test]
    fn sync_project_file_keeps_project_defaults() {
        let mut model = YinModel::default();
        model.meta.default_bpm = 90.0;
        model.meta.default_time_sig = (3, 2);
        let mut data = ProjectData::new(
            Arc::new(model),
            ProjectFile::default(),
            MappingFile::default(),
        );

        data.sync_project_file();

        assert_eq!(data.project_file.default_bpm, 90.0);
        assert_eq!(data.project_file.default_time_sig, (3, 2));
    }

    #[test]
    fn sync_mapping_file_keeps_channel_instruments() {
        let inst = ChannelInstrument {
            bank_msb: 0,
            bank_lsb: 0,
            program: 24,
            is_drum: false,
        };
        let mut model = YinModel::default();
        model.channel_instruments[3] = Some(inst);
        let mut data = ProjectData::new(
            Arc::new(model),
            ProjectFile::default(),
            MappingFile::default(),
        );

        data.sync_mapping_file();

        assert_eq!(data.mapping_file.channel_instruments[3], Some(inst));
    }
    #[test]
    fn new_project_data_migrates_legacy_audio_clips() {
        use yinhe_core::{AudioClip, TrackData};
        let mut model = YinModel::default();
        let mut track = TrackData::new(0, 0);
        track.kind = yinhe_core::TrackKind::Audio;
        track.audio_channel = Some(0);
        track.audio_clips.push(AudioClip {
            id: 1,
            source: "s".into(),
            start_seconds: 1.0,
            offset_seconds: 0.0,
            duration_seconds: 2.0,
            gain: 1.0,
            fade_in_seconds: 0.0,
            fade_out_seconds: 0.0,
            reversed: false,
            start_beat: None,
            warp: None,
        });
        model.tracks.push(Arc::new(track));

        let data = ProjectData::new(
            Arc::new(model),
            ProjectFile::default(),
            MappingFile::default(),
        );
        let clip = &data.model().tracks[0].audio_clips[0];
        assert!(clip.is_warped(), "旧工程加载时应迁移到 beat 主时基");
        assert!(clip.warp.as_ref().unwrap().auto);
    }
}
