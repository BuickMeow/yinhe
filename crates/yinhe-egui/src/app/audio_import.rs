//! 音频导入：文件对话框 → 内嵌素材 + 时间线片段。
//!
//! 素材以原始文件字节内嵌进工程（存档自包含）；PCM 解码在 `AudioLibrary`
//! 后台线程完成，导入路径不做重活（只同步 probe 时长/采样率等元信息）。

use std::path::PathBuf;
use std::sync::Arc;

use rust_i18n::t;

use yinhe_core::{AudioSource, TrackKind};
use yinhe_editor_core::NewTrackSpec;
use yinhe_editor_core::channel_alloc;
use yinhe_editor_core::history::UndoAction;

use crate::app::App;

/// 支持的音频扩展名（与 symphonia 解码能力一致：wav/mp3/flac/ogg/m4a 等）。
const AUDIO_EXTENSIONS: &[&str] = &[
    "wav", "wave", "mp3", "flac", "ogg", "oga", "m4a", "aac", "mp4", "aiff", "aif",
];

impl App {
    /// 菜单/快捷键入口：弹出文件对话框，选择音频文件并导入。
    pub(crate) fn import_audio_dialog(&mut self) {
        if self.workspace.active_doc.is_none() {
            return;
        }
        let files = rfd::FileDialog::new()
            .add_filter(t!("file_dialog.audio").as_ref(), AUDIO_EXTENSIONS)
            .pick_files();
        let Some(files) = files else {
            return;
        };
        let paths: Vec<PathBuf> = files.into_iter().collect();
        self.import_audio_files(&paths);
    }

    /// 导入给定音频文件到目标音频轨（选中音频轨 → 首个音频轨 → 新建）。
    /// 多个文件在光标起点依次排开。
    pub(crate) fn import_audio_files(&mut self, paths: &[PathBuf]) {
        let Some(idx) = self.workspace.active_doc else {
            return;
        };
        if paths.is_empty() {
            return;
        }

        // 读文件 + probe 元信息（同步；读盘为一次性操作，量级几十 ms）。
        let mut loaded: Vec<(String, Arc<Vec<u8>>, f64)> = Vec::new();
        let mut errors: Vec<String> = Vec::new();
        for path in paths {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("audio")
                .to_string();
            match std::fs::read(path) {
                Ok(bytes) => match yinhe_audio::probe_audio_info(&bytes) {
                    Ok(info) if info.duration_seconds > 0.0 => {
                        loaded.push((name, Arc::new(bytes), info.duration_seconds));
                    }
                    Ok(_) => errors.push(format!("{name}: {}", t!("import_audio.empty"))),
                    Err(e) => errors.push(format!("{name}: {e}")),
                },
                Err(e) => errors.push(format!("{}: {e}", path.display())),
            }
        }

        if !loaded.is_empty() {
            self.insert_imported_audio(idx, &loaded);
        }
        if !errors.is_empty() {
            self.show_error(
                t!("toast.import_audio_failed").to_string(),
                errors.join("\n"),
            );
        }
    }

    /// 把已读取的音频素材插入工程：必要时新建音频轨，再逐个建片段。
    fn insert_imported_audio(&mut self, idx: usize, loaded: &[(String, Arc<Vec<u8>>, f64)]) {
        let before = self.workspace.documents[idx].capture_snapshot();

        // 目标轨道：选中集合里的音频轨优先（取最靠前的），否则首个音频轨。
        let target_track = {
            let doc = &self.workspace.documents[idx];
            let model = &doc.data.model;
            let selected_audio = doc
                .edit
                .track_selected
                .iter()
                .copied()
                .filter(|&t| {
                    model
                        .tracks
                        .get(t as usize)
                        .is_some_and(|tr| tr.kind == TrackKind::Audio)
                })
                .min();
            let first_audio = model.tracks.iter().position(|t| t.kind == TrackKind::Audio);
            selected_audio.map(|t| t as usize).or(first_audio)
        };

        let mut actions: Vec<UndoAction> = Vec::new();
        let track_idx = match target_track {
            Some(t) => t,
            None => {
                // 无音频轨：新建一条（自动音频通道）。
                let start = channel_alloc::auto_audio_channel_start(
                    &self.workspace.documents[idx].data.model.tracks,
                );
                let specs = [NewTrackSpec {
                    kind: TrackKind::Audio,
                    port: 0,
                    channel: 0,
                    instrument_channel: None,
                    audio_channel: Some(start),
                }];
                if let Some(action) = self.workspace.documents[idx].add_tracks_batch(&specs) {
                    actions.push(action);
                }
                self.workspace.documents[idx].data.model.tracks.len() - 1
            }
        };

        // 起点：光标位置（tick → 秒；音频用绝对时间）。
        let mut start_seconds = {
            let doc = &self.workspace.documents[idx];
            doc.edit
                .cursor_tick
                .map(|tick| {
                    doc.data
                        .model
                        .tempo_map
                        .tick_to_seconds(tick.max(0.0) as u64)
                })
                .unwrap_or(0.0)
        };

        for (name, data, duration) in loaded {
            let uuid = uuid::Uuid::new_v4().to_string();
            let source = AudioSource {
                uuid: uuid.clone(),
                name: name.clone(),
                data: Arc::clone(data),
                duration_seconds: *duration,
            };
            self.workspace.documents[idx].add_audio_source(source);
            if let Some(action) = self.workspace.documents[idx].add_audio_clip(
                track_idx,
                &uuid,
                start_seconds,
                0.0,
                *duration,
            ) {
                actions.push(action);
            }
            // 依次排开（0.05s 间隔避免无缝拼接变成交叉淡化）。
            start_seconds += *duration + 0.05;
        }

        if actions.is_empty() {
            return;
        }
        // 统一合成一个 undo（新建轨 + 全部片段一次撤销）。
        let action = UndoAction::Composite(actions);
        self.workspace.documents[idx].push_undo(action, t!("undo.import_audio").as_ref(), before);
        // 目标轨选中（便于后续继续导入/编辑）。
        self.workspace.documents[idx].edit.track_selected.clear();
        self.workspace.documents[idx]
            .edit
            .track_selected
            .insert(track_idx as u16);
        self.workspace.documents[idx].data.bump_revision();

        // 引擎：新建音频轨会翻转 ChannelLayout → teardown 重建；
        // 已有轨导入则走 reload，更新 duration（音频末尾）与片段数据。
        self.notify_audio_model_changed();
    }
}
