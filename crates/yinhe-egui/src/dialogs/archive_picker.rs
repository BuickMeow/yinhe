use std::sync::mpsc;

/// State of the archive picker dialog.
pub(crate) enum ArchivePickerState {
    /// Background thread is opening the archive.
    Opening {
        path: String,
        rx: mpsc::Receiver<Result<(yinhe_archive::Archive, Vec<yinhe_archive::ArchiveEntry>), String>>,
    },
    /// Archive is open and ready for selection.
    Opened(ArchivePicker),
}

/// The archive picker dialog state.
pub(crate) struct ArchivePicker {
    pub path: String,
    pub archive: yinhe_archive::Archive,
    pub entries: Vec<yinhe_archive::ArchiveEntry>,
    pub selected_idx: Option<usize>,
    pub search_query: String,
    pub filtered: Vec<usize>,
}

impl ArchivePicker {
    fn recompute_filter(&mut self) {
        let q = self.search_query.to_lowercase();
        self.filtered = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| q.is_empty() || e.name.to_lowercase().contains(&q))
            .map(|(i, _)| i)
            .collect();
        if let Some(idx) = self.selected_idx {
            if !self.filtered.contains(&idx) {
                self.selected_idx = self.filtered.first().copied();
            }
        } else {
            self.selected_idx = self.filtered.first().copied();
        }
    }
}

/// Format a byte size into a human-readable string.
fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// Action returned by the archive picker.
pub(crate) enum ArchivePickerAction {
    None,
    Cancel,
    Error(String),
    LoadFile {
        archive: yinhe_archive::Archive,
        entry: yinhe_archive::ArchiveEntry,
    },
}

/// Show the archive picker dialog content inside an existing Ui.
/// Returns an action for the caller to perform.
pub(crate) fn show(
    state: &mut ArchivePickerState,
    ctx: &eframe::egui::Context,
) -> ArchivePickerAction {
    match state {
        ArchivePickerState::Opening { path, rx } => {
            match rx.try_recv() {
                Ok(Ok((archive, entries))) => {
                    if entries.is_empty() {
                        return ArchivePickerAction::Error(
                            "压缩包中没有找到 MIDI 文件".to_string(),
                        );
                    }
                    if entries.len() == 1 {
                        let entry = entries[0].clone();
                        return ArchivePickerAction::LoadFile { archive, entry };
                    }
                    let mut picker = ArchivePicker {
                        path: path.clone(),
                        archive,
                        entries,
                        selected_idx: None,
                        search_query: String::new(),
                        filtered: Vec::new(),
                    };
                    picker.recompute_filter();
                    *state = ArchivePickerState::Opened(picker);
                    ArchivePickerAction::None
                }
                Ok(Err(e)) => ArchivePickerAction::Error(format!("打开压缩包失败: {}", e)),
                Err(_) => {
                    // Still loading — show spinner
                    eframe::egui::CentralPanel::default().show(ctx, |ui| {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("正在扫描压缩包...");
                        });
                    });
                    ArchivePickerAction::None
                }
            }
        }
        ArchivePickerState::Opened(picker) => {
            let mut action = ArchivePickerAction::None;

            eframe::egui::CentralPanel::default().show(ctx, |ui| {
                let filename = std::path::Path::new(&picker.path)
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_else(|| picker.path.clone());
                ui.label(
                    eframe::egui::RichText::new(format!("来源: {}", filename))
                        .strong()
                        .size(13.0),
                );
                ui.add_space(4.0);

                let search_response = ui.horizontal(|ui| {
                    ui.label("🔍");
                    ui.add(
                        eframe::egui::TextEdit::singleline(&mut picker.search_query)
                            .hint_text("搜索文件...")
                            .desired_width(f32::INFINITY),
                    )
                });
                if search_response.response.changed() {
                    picker.recompute_filter();
                }
                ui.add_space(4.0);

                let row_height = 22.0;
                let available_height = ui.available_height() - 40.0;
                eframe::egui::ScrollArea::vertical()
                    .max_height(available_height)
                    .show_rows(ui, row_height, picker.filtered.len(), |ui, row_range| {
                        for row_idx in row_range {
                            let &entry_idx = &picker.filtered[row_idx];
                            let entry = &picker.entries[entry_idx];
                            let is_selected = picker.selected_idx == Some(entry_idx);

                            let bg = if is_selected {
                                eframe::egui::Color32::from_rgba_premultiplied(40, 80, 160, 200)
                            } else {
                                eframe::egui::Color32::TRANSPARENT
                            };

                            let response = ui.add_sized(
                                [ui.available_width(), row_height],
                                eframe::egui::Button::new("").fill(bg),
                            );

                            if response.hovered() && !is_selected {
                                let rect = response.rect;
                                ui.painter().rect_filled(
                                    rect,
                                    0.0,
                                    eframe::egui::Color32::from_rgba_premultiplied(255, 255, 255, 20),
                                );
                            }

                            if response.clicked() {
                                picker.selected_idx = Some(entry_idx);
                            }
                            if response.double_clicked() {
                                let entry = picker.entries[entry_idx].clone();
                                let archive = std::mem::replace(
                                    &mut picker.archive,
                                    yinhe_archive::Archive::open(&picker.path).unwrap(),
                                );
                                action = ArchivePickerAction::LoadFile { archive, entry };
                                return;
                            }

                            let response_rect = response.rect;
                            let prefix = if is_selected { "▶ " } else { "  " };
                            let text = format!("{}{}", prefix, entry.name);
                            let size_text = format_size(entry.size);

                            ui.painter().text(
                                response_rect.left_center() + eframe::egui::vec2(8.0, 0.0),
                                eframe::egui::Align2::LEFT_CENTER,
                                &text,
                                eframe::egui::FontId::proportional(13.0),
                                if is_selected {
                                    eframe::egui::Color32::WHITE
                                } else {
                                    ui.visuals().text_color()
                                },
                            );
                            ui.painter().text(
                                response_rect.right_center() + eframe::egui::vec2(-8.0, 0.0),
                                eframe::egui::Align2::RIGHT_CENTER,
                                &size_text,
                                eframe::egui::FontId::proportional(11.0),
                                eframe::egui::Color32::GRAY,
                            );
                        }
                    });

                ui.add_space(4.0);
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label(
                        eframe::egui::RichText::new(format!("{} 个文件", picker.filtered.len()))
                            .size(12.0)
                            .color(eframe::egui::Color32::GRAY),
                    );
                    ui.with_layout(eframe::egui::Layout::right_to_left(eframe::egui::Align::Center), |ui| {
                        if ui.button("取消").clicked() {
                            action = ArchivePickerAction::Cancel;
                        }
                        let confirm_enabled = picker.selected_idx.is_some();
                        if ui.add_enabled(confirm_enabled, eframe::egui::Button::new("确认")).clicked() {
                            if let Some(idx) = picker.selected_idx {
                                let entry = picker.entries[idx].clone();
                                let archive = std::mem::replace(
                                    &mut picker.archive,
                                    yinhe_archive::Archive::open(&picker.path).unwrap(),
                                );
                                action = ArchivePickerAction::LoadFile { archive, entry };
                            }
                        }
                    });
                });
            });

            if ctx.input(|i| i.key_pressed(eframe::egui::Key::Escape)) {
                action = ArchivePickerAction::Cancel;
            }

            action
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_size_bytes() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1023), "1023 B");
    }

    #[test]
    fn format_size_kilobytes() {
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(1024 * 1024 - 1), "1024.0 KB");
    }

    #[test]
    fn format_size_megabytes() {
        assert_eq!(format_size(1024 * 1024), "1.0 MB");
        assert_eq!(format_size(1024 * 1024 * 5), "5.0 MB");
    }
}
