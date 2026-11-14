//! 保存快照、后台保存与 MIDI / 音频导出。

use std::sync::mpsc;

use eframe::egui;
use rust_i18n::t;

use crate::app::App;

/// 离线保存所需的文档快照（用户保存与自动保存共用）。
/// 抓取后不再借用 `App`，可整体移动到后台线程。
pub(crate) struct SaveSnapshot {
    pub model: std::sync::Arc<yinhe_core::YinModel>,
    pub project_file: yinhe_yin::ProjectFile,
    pub mapping_file: yinhe_yin::MappingFile,
    pub mixer: yinhe_mixer::MixerParams,
}

impl App {
    /// 保存前把编辑态同步进 model/project_file，并抓取可离线保存的快照。
    /// 保存线程只持有快照，不再借用 `self`；保存与自动保存共用。
    pub(crate) fn take_save_snapshot(
        &mut self,
        idx: usize,
        sync_plugin_state: bool,
    ) -> SaveSnapshot {
        let doc = &mut self.workspace.documents[idx];
        doc.sync_overrides_to_model();
        doc.data.sync_project_file();
        doc.data.sync_mapping_file();
        doc.sync_sf_overrides_to_project_file();

        // 插件实例不是 Send（CLAP/VST3 带原生窗口/COM 对象），无法搬到后台线程；
        // 自动保存跳过这一步，避免每 5 分钟在主线程逐个插件调 save_state。
        // 手工保存仍然同步，保证插件状态落盘。
        if sync_plugin_state {
            // 混音台插件：保存前把实例的 CLAP state 写回 InsertRef（旁通标志同步）。
            if let Some(rack) = self.mixer_racks.get_mut(idx) {
                rack.sync_states_to(&mut self.workspace.documents[idx].mixer);
            }
            // 乐器插件：同样把实例 state 写回 mixer.instruments[channel]。
            if let Some(irack) = self.instrument_racks.get_mut(idx) {
                irack.sync_states_to(&mut self.workspace.documents[idx].mixer);
            }
        }
        let doc = &self.workspace.documents[idx];
        SaveSnapshot {
            model: doc.data.model.clone(),
            project_file: doc.data.project_file.clone(),
            mapping_file: doc.data.mapping_file.clone(),
            mixer: doc.mixer.clone(),
        }
    }

    /// Spawn a background thread to save the project.
    pub(crate) fn save_project_async(&mut self, idx: usize, path: String) {
        // 同一时刻只允许一个保存任务：否则两个后台线程会同时写同一路径，
        // 且 `jobs.save_rx` 会被后来的覆盖，前一次结果丢失、脏标记错乱。
        if self.jobs.is_saving() {
            tracing::warn!("[save] 上一次保存尚未完成，忽略本次请求: {path}");
            return;
        }
        let snap = self.take_save_snapshot(idx, true);
        let path_for_thread = path.clone();
        // 发起时的状态标记：保存期间的新编辑不应被误标为已保存
        let (saved_marker, saved_mixer_revision) = self
            .workspace
            .documents
            .get(idx)
            .map(|d| (d.undo_marker(), d.mixer_revision))
            .unwrap_or((None, 0));

        let (tx, rx) = mpsc::channel();
        let (progress_tx, progress_rx) = mpsc::channel();
        let pause = self.jobs.save_pause.clone();
        pause.store(false, std::sync::atomic::Ordering::Relaxed);
        std::thread::spawn(move || {
            let result = yinhe_yin::save_yin_with_files_progress(
                &snap.model,
                &path_for_thread,
                &snap.project_file,
                &snap.mapping_file,
                Some(&snap.mixer),
                |p| {
                    while pause.load(std::sync::atomic::Ordering::Relaxed) {
                        std::thread::sleep(std::time::Duration::from_millis(20));
                    }
                    let _ = progress_tx.send(p);
                },
            );
            let result = result.map_err(|e| e.to_string());
            if let Err(e) = &result {
                tracing::error!("Failed to save project: {}", e);
            }
            // 回传 (目标文档, 版本快照, 路径, 结果)：失败不得标记已保存/
            // 不得改路径/不得执行延迟动作（此前失败被当成功，可能丢数据）。
            let _ = tx.send((
                idx,
                saved_marker,
                saved_mixer_revision,
                path_for_thread,
                result,
            ));
        });

        self.jobs.save_rx = Some(rx);
        self.jobs.save_progress_rx = Some(progress_rx);
    }

    pub(crate) fn save_as_dialog(&mut self) {
        if let Some(idx) = self.workspace.active_doc {
            self.save_as_dialog_for(idx);
        }
    }

    /// 另存为（指定文档；未保存确认弹窗的目标可能是非 active 文档）。
    /// 路径/文件名在**保存成功后**才写入（见 poll 的保存完成处理）。
    /// 返回是否已发起保存（`false` = 用户取消了文件对话框）。
    pub(crate) fn save_as_dialog_for(&mut self, idx: usize) -> bool {
        let default_name = if let Some(doc) = self.workspace.documents.get(idx) {
            format!("{}.yin", doc.file_name)
        } else {
            t!("file_dialog.untitled").to_string()
        };
        if let Some(path) = rfd::FileDialog::new()
            .add_filter(t!("file_dialog.yinhe_project").as_ref(), &["yin"])
            .set_file_name(&default_name)
            .save_file()
        {
            let mut path_str = path.to_string_lossy().to_string();
            // Ensure .yin extension
            if !path_str.ends_with(".yin") {
                path_str.push_str(".yin");
            }
            self.save_project_async(idx, path_str);
            true
        } else {
            false
        }
    }

    pub(super) fn export_midi_dialog(&mut self) {
        if self.jobs.is_midi_exporting() {
            return;
        }
        let default_name = if let Some(idx) = self.workspace.active_doc {
            format!("{}.mid", self.workspace.documents[idx].file_name)
        } else {
            t!("file_dialog.export_mid").to_string()
        };
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("MIDI", &["mid", "midi"])
            .set_file_name(&default_name)
            .save_file()
        {
            let Some(idx) = self.workspace.active_doc else {
                return;
            };
            let path_str = path.to_string_lossy().to_string();
            let (model, opts) = {
                let doc = &self.workspace.documents[idx];
                (
                    std::sync::Arc::clone(&doc.data.model),
                    yinhe_midi::MidiExportOptions {
                        encoding: self.audio_settings.midi_export_encoding,
                        rpn_full: self.audio_settings.midi_export_rpn_full,
                        curve_interpolate: self.audio_settings.midi_export_curve_interpolate,
                        curve_density: self.audio_settings.midi_export_curve_density,
                        strip_empty_tracks: self.audio_settings.midi_export_strip_empty_tracks,
                        dedup_overlaps: self.audio_settings.midi_export_dedup_overlaps,
                    },
                )
            };
            let live = std::sync::Arc::new(std::sync::Mutex::new(
                crate::app::export_state::MidiExportLive { progress: None },
            ));
            self.jobs.midi_export.live = std::sync::Arc::clone(&live);
            self.jobs.midi_export.path = path_str;
            let (tx, rx) = mpsc::channel();
            self.jobs.midi_export.rx = Some(rx);
            self.jobs.midi_export.running = true;
            let path_buf = path.clone();
            let pause = self.jobs.midi_export.pause.clone();
            pause.store(false, std::sync::atomic::Ordering::Relaxed);
            let spawned = std::thread::Builder::new()
                .name("midi-export".into())
                .spawn(move || {
                    let result = yinhe_memtrace::with_tag(yinhe_memtrace::AllocTag::Midi, || {
                        yinhe_midi::write_to_file_with_progress(&model, &path_buf, &opts, |p| {
                            while pause.load(std::sync::atomic::Ordering::Relaxed) {
                                std::thread::sleep(std::time::Duration::from_millis(20));
                            }
                            if let Ok(mut l) = live.lock() {
                                l.progress = Some(p);
                            }
                        })
                    })
                    .map_err(|e| e.to_string());
                    let _ = tx.send(result);
                });
            if let Err(e) = spawned {
                self.jobs.midi_export.running = false;
                self.jobs.midi_export.rx = None;
                self.notifications.error(
                    t!("toast.midi_export_failed"),
                    t!("toast.export_thread_start_failed", e = e),
                );
            }
        }
    }

    pub(super) fn export_audio_dialog(&mut self, ctx: &egui::Context) {
        if self.workspace.active_doc.is_none() {
            return;
        }

        if self.jobs.is_exporting() {
            return; // already exporting
        }

        // Show export settings dialog first
        self.jobs.export.show_bit_depth = true;
        crate::chrome::dialog::raise_viewport(
            ctx,
            egui::ViewportId::from_hash_of("export_settings_dialog"),
        );
    }

    /// Called after the bit-depth dialog is confirmed.
    /// Opens the file-save dialog and starts the export.
    ///
    /// 含插件链的导出在渲染线程内复用**实时引擎**（insert/乐器/PDC 全量参与）；
    /// GPU 合成器模式仍走独立 GPU 导出线程（不经混音台，不含插件链）。
    pub(crate) fn start_export(&mut self) {
        let idx = match self.workspace.active_doc {
            Some(idx) => idx,
            None => return,
        };

        // 真实计时起点：点击「开始导出」按钮的瞬间（包含后续文件对话框等待）。
        let button_time = std::time::Instant::now();
        let default_name = format!("{}.wav", self.workspace.documents[idx].file_name);
        let path = match rfd::FileDialog::new()
            .add_filter("WAV", &["wav"])
            .set_file_name(&default_name)
            .save_file()
        {
            Some(p) => p,
            None => return,
        };
        let mut path_str = path.to_string_lossy().to_string();
        if !path_str.ends_with(".wav") {
            path_str.push_str(".wav");
        }

        let sr = if self.jobs.export.sample_rate > 0 {
            self.jobs.export.sample_rate
        } else {
            self.audio_settings.sample_rate
        };
        // 导出复用实时引擎（含插件链与 PDC），采样率必须与当前设备一致：
        // 插件实例按设备采样率激活，无法在不重载插件的情况下换采样率。
        if sr != self.audio_settings.sample_rate {
            self.notifications.error(
                t!("toast.export_sample_rate_mismatch"),
                t!(
                    "toast.export_sample_rate_mismatch_desc",
                    sr = self.audio_settings.sample_rate
                ),
            );
            return;
        }

        let bit_depth = self.jobs.export.bit_depth;
        let layer_count = if self.jobs.export.layer_count == 0 {
            None
        } else {
            Some(self.jobs.export.layer_count as usize)
        };
        let export_progress = self.jobs.export.progress.clone();
        let cancel_flag = self.jobs.export.cancel.clone();
        let pause_flag = self.jobs.export.pause.clone();
        // 记下输出路径：中止卡“打开文件夹”按钮用。
        self.jobs.export.last_output_path = Some(path_str.clone());
        // 音频完成后写 WAV LIST/INFO；元数据取发起导出时的快照。
        self.jobs.export.tags = if self.jobs.export.write_metadata {
            self.workspace.active_doc.map(|i| {
                (
                    path_str.clone(),
                    yinhe_core::SongInfo::from_meta(&self.workspace.documents[i].data.model.meta),
                )
            })
        } else {
            None
        };
        cancel_flag.store(false, std::sync::atomic::Ordering::Relaxed);
        pause_flag.store(false, std::sync::atomic::Ordering::Relaxed);
        // Reset progress state（计时起点为按钮点击时刻，保证壁钟时间真实）。
        if let Ok(mut p) = export_progress.lock() {
            p.reset();
            p.started_at = Some(button_time);
        }

        // 含插件链的导出：交给渲染线程复用实时引擎。
        if let Some(h) = &self.audio_state.handle {
            // 导出结束后恢复用户设置的层数（导出设置只影响本次导出）。
            let restore_layer_count = if self.audio_settings.xsynth_layers == 0 {
                None
            } else {
                Some(self.audio_settings.xsynth_layers as usize)
            };
            h.handle.send(yinhe_audio::AudioCommand::ExportStart {
                path: std::path::PathBuf::from(&path_str),
                bit_depth,
                layer_count,
                restore_layer_count,
                progress: export_progress,
                cancel: cancel_flag,
                pause: pause_flag,
            });
            self.jobs.export.running = true;
        }
    }
}
