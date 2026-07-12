use crate::app::App;
use crate::file_loader::LoadResult;
use yinhe_editor_core::document::Document;
use yinhe_editor_core::quantize::QuantizePreset;

impl App {
    /// Poll all async operations: file loading, save completion, export completion.
    pub(in crate::app) fn poll_async_operations(&mut self) {
        // Poll async file loading
        match self.file_loader.poll_loading() {
            LoadResult::ModelLoaded { path, model } => {
                let (quantize_arrange, quantize_pianoroll) = self
                    .active_doc
                    .and_then(|idx| self.documents.get(idx))
                    .map(|doc| (doc.edit.quantize_arrange, doc.edit.quantize_pianoroll))
                    .unwrap_or((QuantizePreset::Fraction(1, 4), QuantizePreset::Fraction(1, 16)));
                match Document::from_model(&path, model, quantize_arrange, quantize_pianoroll, yinhe_yin::ProjectFile::default(), yinhe_yin::MappingFile::default()) {
                    Ok(mut doc) => {
                        doc.mark_loaded(); // Loaded from file, not a fresh empty doc
                        let insert_idx = self.documents.len();
                        self.documents.push(doc);
                        self.active_doc = Some(insert_idx);
                        self.teardown_audio();
                    }
                    Err(msg) => {
                        self.load_error = Some(msg);
                    }
                }
            }
            LoadResult::ModelFromYin {
                path,
                model,
                file_name,
                sf,
                mapping,
            } => {
                let (quantize_arrange, quantize_pianoroll) = self
                    .active_doc
                    .and_then(|idx| self.documents.get(idx))
                    .map(|doc| (doc.edit.quantize_arrange, doc.edit.quantize_pianoroll))
                    .unwrap_or((QuantizePreset::Fraction(1, 4), QuantizePreset::Fraction(1, 16)));
                let project_file = yinhe_yin::ProjectFile::from_meta_with_sf(
                    &model.meta,
                    sf.mode,
                    sf.overrides.clone(),
                );
                let result = Document::from_model(&path, model, quantize_arrange, quantize_pianoroll, project_file, mapping)
                    .ok()
                    .map(|mut d| {
                        d.file_path = Some(path.clone());
                        d.mark_loaded(); // Loaded from file, not a fresh empty doc

                        d.edit.project_sf.overrides = sf
                            .overrides
                            .iter()
                            .map(|po| {
                                let entries = po
                                    .entries
                                    .iter()
                                    .map(|e| yinhe_editor_core::SfEntry {
                                        path: e.path.clone(),
                                        name: e.name.clone(),
                                        enabled: e.enabled,
                                    })
                                    .collect();
                                (po.port, entries)
                            })
                            .collect();

                        (d, sf.mode)
                    });
                if let Some((doc, sf_project_mode)) = result {
                    self.audio_settings.global_sf_config.global_enabled = !sf_project_mode;
                    let insert_idx = self.documents.len();
                    self.documents.push(doc);
                    self.active_doc = Some(insert_idx);
                    self.teardown_audio();
                } else {
                    self.load_error = Some(format!(
                        "无法打开「{}」：可能不是有效的 .yin 文件，或其内嵌 MIDI 缺少 Conductor 轨道。",
                        file_name
                    ));
                }
            }
            LoadResult::ArchiveError(msg) => {
                self.load_error = Some(msg);
            }
            LoadResult::NotReady => {}
        }

        // Poll async save completion
        if let Some(rx) = &self.save_rx {
            if rx.try_recv().is_ok() {
                self.save_rx = None;
                // Mark the active document as saved
                if let Some(idx) = self.active_doc {
                    self.documents[idx].mark_saved();
                }
                // If there's a deferred action, execute it now
                if self.pending_unsaved.is_some() {
                    let ctx = egui::Context::default();
                    self.execute_pending_file_action(&ctx);
                }
            }
        }

        // Poll async export completion
        if let Some(rx) = &self.export.rx {
            if let Ok(result) = rx.try_recv() {
                self.export.rx = None;
                match result {
                    Ok((path, elapsed, speed)) => {
                        self.export.completed = Some(crate::dialogs::export::ExportCompleted {
                            file_path: path,
                            elapsed_secs: elapsed,
                            overall_speed: speed,
                        });
                    }
                    Err(e) => {
                        self.load_error = Some(e);
                    }
                }
            }
        }
    }

    /// Sync `automation_event_density` to the audio engine when it changes.
    pub(in crate::app) fn sync_automation_density(&mut self) {
        let density = self.audio_settings.automation_event_density;
        if density != self.last_automation_density {
            self.last_automation_density = density;
            if let Some(audio) = &self.audio {
                audio.handle.send(yinhe_audio::AudioCommand::SetAutomationDensity {
                    density,
                });
            }
        }
    }

    /// Refresh system resource monitoring (CPU, memory) if enough time has elapsed.
    pub(crate) fn refresh_system_stats(&mut self) {
        self.sys_monitor.refresh_if_needed();
    }
}
