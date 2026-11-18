use super::*;

impl Document {
    /// 新工程/导入 MIDI 的默认混音台：不预设任何总线，降低学习曲线；
    /// 需要总线时由用户在 MIX 或 AR 手动添加。
    fn default_mixer() -> MixerParams {
        let mut mixer = MixerParams::default();
        mixer.ensure_len();
        mixer
    }

    pub fn empty() -> Self {
        Self::from_empty_model(YinModel::new_empty_with_16_tracks())
    }

    /// 空白工作区（桌面/手机「新建工程」与手机启动占位共用）：
    /// 只保留 Conductor，其余音轨由 AR 的「+」创建。
    pub fn empty_conductor_only() -> Self {
        let mut model = YinModel::new_empty_with_16_tracks();
        model.tracks.truncate(1);
        Self::from_empty_model(model)
    }

    fn from_empty_model(mut model: YinModel) -> Self {
        model.rebuild();

        let num_tracks = model.tracks.len();
        let conductor_track_idx = Some(0);

        let data = ProjectData::new(
            Arc::new(model.clone()),
            ProjectFile::from_meta(&model.meta),
            MappingFile::default(),
        );
        let track_info_cache = data.track_info();

        let mut doc = Document {
            data,
            edit: EditState {
                track_visible: vec![true; num_tracks],
                track_pianoroll_visible: vec![true; num_tracks],
                track_locked: vec![false; num_tracks],
                track_overrides: model
                    .tracks
                    .iter()
                    .map(|t| TrackOverride {
                        muted: t.muted,
                        soloed: t.soloed,
                    })
                    .collect(),
                track_cache: TrackCache {
                    info: track_info_cache,
                    colors: (0..num_tracks)
                        .map(|i| track_color(&model.tracks[i], i, conductor_track_idx))
                        .collect(),
                    conductor_idx: conductor_track_idx,
                    ..Default::default()
                },
                ..Default::default()
            },
            history: UndoStack::new(),
            file_name: "Untitled".into(),
            file_path: None,
            mixer: Self::default_mixer(),
            doc_id: next_doc_id(),
            mixer_dirty: false,
            mixer_revision: 0,
            density: Default::default(),
            density_pr: Default::default(),
        };
        doc.default_empty_project_own_lc();
        doc
    }

    /// Create a Document from a freshly parsed YinModel.
    /// Path is used only to derive the file name; data ownership comes from
    /// the model. Inserts a conductor track at index 0 if absent.
    pub fn from_model(
        path: &str,
        model: YinModel,
        quantize_arrange: QuantizePreset,
        quantize_pianoroll: QuantizePreset,
        project_file: ProjectFile,
        mapping_file: MappingFile,
        mixer: Option<MixerParams>,
    ) -> Result<Self, String> {
        yinhe_memtrace::with_tag(yinhe_memtrace::AllocTag::Ui, || {
            let file_name = std::path::Path::new(path)
                .file_stem()
                .and_then(|n| n.to_str())
                .map(|s| s.to_string())
                .unwrap_or_default();

            let mut model = model;
            // 自动化事件 id 不落盘：文档构建时兜底统一发号（幂等，已发号的
            // 只是重新编号；覆盖各平台加载路径与直接构造模型的调用方）。
            model.renumber_automation_ids();

            // Detect conductor track; insert one if missing.
            //（插入后需 rebuild 让 `track_note_count` 与轨号一致；新建的
            // conductor 一定在 0 号位，无需再判定。）
            let conductor_track_idx = if model.ensure_conductor_track() {
                model.rebuild();
                Some(0)
            } else {
                detect_conductor_from_model(&model)
            };
            model.normalize_folder_structure();
            // 权限清理：Folder 不承载任何内容，Master 不承载音符/音频片段；
            // 历史版本误拖入的内容在加载时一次性移除。
            model.sanitize_track_content();

            let num_tracks = model.tracks.len();
            let track_colors_cache = (0..num_tracks)
                .map(|i| track_color(&model.tracks[i], i, conductor_track_idx))
                .collect();

            // 无混音数据的导入（.mid / 压缩包内 MIDI / 老 .yin 无混音段）走新工程
            // 默认混音台：不预设总线；.yin 工程自带混音数据（Some）时原样保留。
            let mut mixer = mixer.unwrap_or_else(Self::default_mixer);
            mixer.ensure_len();
            let mut data = ProjectData::new(Arc::new(model), project_file, mapping_file);
            data.rebuild_model();

            let track_info_cache = data.track_info();
            let pc_map_cache = data.pc_map_cache();
            let track_overrides: Vec<TrackOverride> = data
                .model
                .tracks
                .iter()
                .map(|t| TrackOverride {
                    muted: t.muted,
                    soloed: t.soloed,
                })
                .collect();

            let mut doc = Document {
                data,
                edit: EditState {
                    quantize_arrange,
                    quantize_pianoroll,
                    track_visible: vec![true; num_tracks],
                    track_pianoroll_visible: vec![true; num_tracks],
                    track_locked: vec![false; num_tracks],
                    track_overrides,
                    track_cache: TrackCache {
                        info: track_info_cache,
                        colors: track_colors_cache,
                        pc_map: pc_map_cache,
                        conductor_idx: conductor_track_idx,
                    },
                    ..Default::default()
                },
                history: UndoStack::new(),
                file_name,
                file_path: None,
                mixer,
                mixer_dirty: false,
                mixer_revision: 0,
                doc_id: next_doc_id(),
                density: Default::default(),
                density_pr: Default::default(),
            };
            doc.migrate_shared_audio_home_lcs();
            doc.ensure_audio_targets();
            doc.ensure_rack_track_lcs();
            doc.adopt_loaded_freezes();
            Ok(doc)
        })
    }

    /// Load a `.yin` file. 工程内每通道音色库覆盖会填入 `edit.project_sf`。
    pub fn from_yin_path(
        path: &str,
        quantize_arrange: QuantizePreset,
        quantize_pianoroll: QuantizePreset,
    ) -> std::io::Result<Self> {
        let (model, sf, mapping, mixer) =
            yinhe_yin::load_yin_with_sf(path).map_err(|e| match e {
                yinhe_yin::YinError::Io(io) => io,
                other => std::io::Error::new(std::io::ErrorKind::InvalidData, other.to_string()),
            })?;
        let project_file = yinhe_yin::ProjectFile::from_meta(&model.meta);
        let mut doc = Self::from_model(
            path,
            model,
            quantize_arrange,
            quantize_pianoroll,
            project_file,
            mapping,
            mixer,
        )
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        doc.apply_sf_overrides(&sf.overrides);
        doc.sync_sf_overrides_to_project_file();
        doc.file_path = Some(path.to_string());
        Ok(doc)
    }

    /// 把工程内的每通道音色库覆盖装入编辑态（加载工程后调用）。
    pub fn apply_sf_overrides(&mut self, overrides: &[yinhe_yin::SfChannelOverride]) {
        self.edit.project_sf.overrides = overrides
            .iter()
            .map(|co| {
                let entries = co
                    .entries
                    .iter()
                    .map(|e| crate::config::SfEntry {
                        path: e.path.clone(),
                        name: e.name.clone(),
                        enabled: e.enabled,
                    })
                    .collect();
                (co.channel, entries)
            })
            .collect();
    }

    /// 把编辑态的每通道音色库覆盖写回 `project_file`（保存前调用）。
    pub fn sync_sf_overrides_to_project_file(&mut self) {
        self.data.project_file.sf_channel_overrides = self
            .edit
            .project_sf
            .overrides
            .iter()
            .map(|(channel, entries)| yinhe_yin::SfChannelOverride {
                channel: *channel,
                entries: entries
                    .iter()
                    .map(|e| yinhe_yin::SfEntryJson {
                        path: e.path.clone(),
                        name: e.name.clone(),
                        enabled: e.enabled,
                    })
                    .collect(),
            })
            .collect();
    }

    pub fn recode_track_names(&mut self, _encoding: yinhe_midi::MidiImportEncoding) {
        // TODO: implement track name re-encoding for YinModel
        self.data.bump_revision();
    }
}
