use std::sync::mpsc;

use rust_i18n::t;

/// 截断字符串到指定最大显示宽度（按字符数估算），超出部分替换为省略号。
/// 返回值是截断后的字符串。注意这是近似值，不同字体宽度略有差异。
fn truncate_name(name: &str, max_chars: usize) -> String {
    if name.chars().count() <= max_chars {
        return name.to_string();
    }
    let truncated: String = name.chars().take(max_chars.saturating_sub(1)).collect();
    format!("{}…", truncated)
}

/// State of the archive picker dialog.
pub(crate) enum ArchivePickerState {
    /// Background thread is opening the archive.
    Opening {
        path: String,
        rx: mpsc::Receiver<
            Result<(yinhe_archive::Archive, Vec<yinhe_archive::ArchiveEntry>), String>,
        >,
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
    ui: &mut eframe::egui::Ui,
) -> ArchivePickerAction {
    match state {
        ArchivePickerState::Opening { path, rx } => {
            match rx.try_recv() {
                Ok(Ok((archive, entries))) => {
                    if entries.is_empty() {
                        return ArchivePickerAction::Error(
                            t!("dialog.archive.no_midi").to_string(),
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
                Ok(Err(e)) => {
                    ArchivePickerAction::Error(t!("dialog.archive.open_failed", e = e).to_string())
                }
                Err(_) => {
                    // Still loading — show spinner
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(t!("dialog.archive.scanning").as_ref());
                    });
                    ArchivePickerAction::None
                }
            }
        }
        ArchivePickerState::Opened(picker) => {
            let mut action = ArchivePickerAction::None;

            let filename = std::path::Path::new(&picker.path)
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_else(|| picker.path.clone());
            let display_name = truncate_name(&filename, 45);
            let source_resp = ui.label(
                eframe::egui::RichText::new(
                    t!("dialog.archive.source", name = display_name).as_ref(),
                )
                .strong()
                .size(13.0),
            );
            if filename.len() != display_name.len() {
                source_resp.on_hover_text(&filename);
            }
            ui.add_space(4.0);

            let search_response = ui.horizontal(|ui| {
                use egui_material_icons::icons::ICON_SEARCH;
                ui.label(
                    eframe::egui::RichText::new(ICON_SEARCH.codepoint)
                        .family(ICON_SEARCH.font_family())
                        .size(14.0)
                        .color(eframe::egui::Color32::GRAY),
                );
                ui.add(
                    eframe::egui::TextEdit::singleline(&mut picker.search_query)
                        .hint_text(t!("dialog.archive.search_hint").as_ref())
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
                            crate::theme::ROW_SELECTED_BG
                        } else {
                            eframe::egui::Color32::TRANSPARENT
                        };

                        let response = ui.add_sized(
                            [ui.available_width(), row_height],
                            eframe::egui::Button::new("").fill(bg),
                        );

                        if response.hovered() && !is_selected {
                            let rect = response.rect;
                            ui.painter()
                                .rect_filled(rect, 0.0, crate::theme::ROW_SELECTED_BG);
                        }

                        if response.clicked() {
                            picker.selected_idx = Some(entry_idx);
                        }
                        if response.double_clicked() {
                            let entry = picker.entries[entry_idx].clone();
                            action = ArchivePickerAction::LoadFile {
                                archive: picker.archive.clone(),
                                entry,
                            };
                            return;
                        }

                        let response_rect = response.rect;
                        let prefix = if is_selected { "▶ " } else { "  " };
                        let display_name = truncate_name(&entry.name, 55);
                        let text = format!("{}{}", prefix, display_name);
                        let size_text = format_size(entry.size);

                        ui.painter().text(
                            response_rect.left_center() + eframe::egui::vec2(8.0, 0.0),
                            eframe::egui::Align2::LEFT_CENTER,
                            &text,
                            eframe::egui::FontId::proportional(13.0),
                            ui.visuals().text_color(),
                        );

                        if display_name.len() != entry.name.len() {
                            let name_rect = eframe::egui::Rect::from_min_size(
                                response_rect.left_center() + eframe::egui::vec2(8.0, 0.0),
                                eframe::egui::vec2(
                                    response_rect.width() * 0.75,
                                    response_rect.height(),
                                ),
                            );
                            let name_resp = ui.interact(
                                name_rect,
                                ui.next_auto_id(),
                                eframe::egui::Sense::hover(),
                            );
                            name_resp.on_hover_text(&entry.name);
                        }
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
                    eframe::egui::RichText::new(
                        t!("dialog.archive.file_count", n = picker.filtered.len()).as_ref(),
                    )
                    .size(12.0)
                    .color(eframe::egui::Color32::GRAY),
                );
                ui.with_layout(
                    eframe::egui::Layout::right_to_left(eframe::egui::Align::Center),
                    |ui| {
                        if ui.button(t!("common.cancel").as_ref()).clicked() {
                            action = ArchivePickerAction::Cancel;
                        }
                        let confirm_enabled = picker.selected_idx.is_some();
                        if ui
                            .add_enabled(
                                confirm_enabled,
                                eframe::egui::Button::new(t!("common.confirm").as_ref()),
                            )
                            .clicked()
                            && let Some(idx) = picker.selected_idx
                        {
                            let entry = picker.entries[idx].clone();
                            action = ArchivePickerAction::LoadFile {
                                archive: picker.archive.clone(),
                                entry,
                            };
                        }
                    },
                );
            });

            if ui.input(|i| i.key_pressed(eframe::egui::Key::Escape)) {
                action = ArchivePickerAction::Cancel;
            }

            action
        }
    }
}

pub(crate) fn show_viewport(
    ctx: &eframe::egui::Context,
    state: &mut Option<ArchivePickerState>,
) -> ArchivePickerAction {
    if state.is_none() {
        return ArchivePickerAction::None;
    }
    let viewport_id = eframe::egui::ViewportId::from_hash_of("archive_picker_dialog");

    let taken_state = std::rc::Rc::new(std::cell::RefCell::new(std::mem::replace(
        state.as_mut().unwrap(),
        ArchivePickerState::Opening {
            path: String::new(),
            rx: std::sync::mpsc::channel().1,
        },
    )));
    let action = std::rc::Rc::new(std::cell::RefCell::new(ArchivePickerAction::None));
    let ctx_clone = ctx.clone();
    let taken_state_cb = taken_state.clone();
    let action_cb = action.clone();

    ctx_clone.show_viewport_immediate(
        viewport_id,
        crate::chrome::dialog::viewport_builder(
            t!("dialog.archive.title").as_ref(),
            [560.0, 400.0],
            true,
        ),
        move |vctx, _class| {
            let close_requested = vctx.input(|i| i.viewport().close_requested());
            let vctx_cmd = vctx.clone();
            eframe::egui::CentralPanel::default()
                .frame(eframe::egui::Frame {
                    fill: crate::theme::APP_BG,
                    ..Default::default()
                })
                .show(vctx, |ui| {
                    let mut close = close_requested;
                    crate::chrome::dialog::title_bar(
                        ui,
                        t!("dialog.archive.title").as_ref(),
                        &mut close,
                    );
                    if close {
                        vctx_cmd.send_viewport_cmd(eframe::egui::ViewportCommand::Visible(false));
                        *action_cb.borrow_mut() = ArchivePickerAction::Cancel;
                    } else {
                        eframe::egui::Frame::new()
                            .inner_margin(eframe::egui::Margin {
                                left: 12,
                                right: 12,
                                top: 0,
                                bottom: 12,
                            })
                            .show(ui, |ui| {
                                let result = show(&mut taken_state_cb.borrow_mut(), ui);
                                *action_cb.borrow_mut() = result;
                            });
                    }
                });
        },
    );

    if let Some(taken_state) = std::rc::Rc::into_inner(taken_state) {
        *state = Some(taken_state.into_inner());
    }

    std::rc::Rc::into_inner(action)
        .map(|rc| rc.into_inner())
        .unwrap_or(ArchivePickerAction::None)
}

// ── 密码输入对话框 ──

/// 密码输入对话框状态。
pub(crate) struct PasswordPrompt {
    pub path: String,
    pub password: String,
    /// `true` 表示之前提交的密码错误，需要重新输入。
    pub wrong: bool,
    /// `true` 时明文显示密码，`false` 时圆点遮蔽。
    pub show_password: bool,
}

impl PasswordPrompt {
    pub(crate) fn new(path: String, wrong: bool) -> Self {
        Self {
            path,
            password: String::new(),
            wrong,
            show_password: false,
        }
    }

    fn clone_state(&self) -> PasswordPrompt {
        PasswordPrompt {
            path: self.path.clone(),
            password: self.password.clone(),
            wrong: self.wrong,
            show_password: self.show_password,
        }
    }
}

/// 密码输入对话框返回的动作。
pub(crate) enum PasswordPromptAction {
    None,
    Cancel,
    /// 用户确认密码，重新打开压缩包。
    Confirm {
        path: String,
        password: String,
    },
}

/// 显示密码输入对话框 viewport。
pub(crate) fn show_password_prompt_viewport(
    ctx: &eframe::egui::Context,
    state: &mut Option<PasswordPrompt>,
) -> PasswordPromptAction {
    if state.is_none() {
        return PasswordPromptAction::None;
    }
    let viewport_id = eframe::egui::ViewportId::from_hash_of("archive_password_prompt_dialog");

    let taken_state = std::rc::Rc::new(std::cell::RefCell::new(
        state.as_mut().unwrap().clone_state(),
    ));
    let action = std::rc::Rc::new(std::cell::RefCell::new(PasswordPromptAction::None));
    let ctx_clone = ctx.clone();
    let taken_state_cb = taken_state.clone();
    let action_cb = action.clone();

    ctx_clone.show_viewport_immediate(
        viewport_id,
        crate::chrome::dialog::viewport_builder(
            t!("dialog.archive.password_title").as_ref(),
            [460.0, 160.0],
            false,
        ),
        move |vctx, _class| {
            let close_requested = vctx.input(|i| i.viewport().close_requested());
            let vctx_cmd = vctx.clone();
            eframe::egui::CentralPanel::default()
                .frame(eframe::egui::Frame {
                    fill: crate::theme::APP_BG,
                    ..Default::default()
                })
                .show(vctx, |ui| {
                    let mut close = close_requested;
                    crate::chrome::dialog::title_bar(
                        ui,
                        t!("dialog.archive.password_title").as_ref(),
                        &mut close,
                    );
                    if close {
                        vctx_cmd.send_viewport_cmd(eframe::egui::ViewportCommand::Visible(false));
                        *action_cb.borrow_mut() = PasswordPromptAction::Cancel;
                    } else {
                        eframe::egui::Frame::new()
                            .inner_margin(eframe::egui::Margin {
                                left: 12,
                                right: 12,
                                top: 0,
                                bottom: 12,
                            })
                            .show(ui, |ui| {
                                let result =
                                    show_password_prompt(&mut taken_state_cb.borrow_mut(), ui);
                                *action_cb.borrow_mut() = result;
                            });
                    }
                });
        },
    );

    // 把对话框中的状态写回（密码文本框内容）。
    if let Some(taken_state) = std::rc::Rc::into_inner(taken_state) {
        let inner = taken_state.into_inner();
        if let Some(s) = state.as_mut() {
            s.password = inner.password;
            s.wrong = inner.wrong;
        }
    }

    std::rc::Rc::into_inner(action)
        .map(|rc| rc.into_inner())
        .unwrap_or(PasswordPromptAction::None)
}

/// 渲染密码输入对话框内容。返回动作（None 表示对话框继续开启）。
fn show_password_prompt(
    prompt: &mut PasswordPrompt,
    ui: &mut eframe::egui::Ui,
) -> PasswordPromptAction {
    let filename = std::path::Path::new(&prompt.path)
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| prompt.path.clone());

    // 主体内容 + 底部按钮行（吸底）
    // 按钮闭包与输入框闭包不能同时可变借用 `prompt`，所以按钮点击只记录标志，
    // 动作在 helper 调用结束后根据最新 `prompt` 构造。
    let path = prompt.path.clone();
    let password_len = std::rc::Rc::new(std::cell::Cell::new(prompt.password.len()));
    let password_len_cb = password_len.clone();
    let cancel_clicked = std::rc::Rc::new(std::cell::Cell::new(false));
    let cancel_cb = cancel_clicked.clone();
    let confirm_clicked = std::rc::Rc::new(std::cell::Cell::new(false));
    let confirm_cb = confirm_clicked.clone();

    crate::chrome::dialog::content_with_bottom_buttons(
        ui,
        36.0,
        |ui| {
            ui.add_space(6.0);
            let display_name = truncate_name(&filename, 40);
            let prompt_resp = ui.label(
                eframe::egui::RichText::new(
                    t!("dialog.archive.password_prompt", name = display_name).as_ref(),
                )
                .size(13.0),
            );
            if filename.len() != display_name.len() {
                prompt_resp.on_hover_text(&filename);
            }

            if prompt.wrong {
                ui.add_space(2.0);
                ui.label(
                    eframe::egui::RichText::new(t!("dialog.archive.password_wrong").as_ref())
                        .size(12.0)
                        .color(crate::theme::ERROR_TEXT),
                );
            }

            ui.add_space(6.0);
            // 密码输入框 + 眼睛切换按钮：回车确认，Esc 取消
            ui.horizontal(|ui| {
                let resp = ui.add(
                    eframe::egui::TextEdit::singleline(&mut prompt.password)
                        .password(!prompt.show_password)
                        .hint_text(t!("dialog.archive.password_hint").as_ref())
                        .desired_width(f32::INFINITY),
                );
                resp.request_focus();

                // 眼睛图标：切换明文/圆点显示
                use egui_material_icons::icons::{ICON_VISIBILITY, ICON_VISIBILITY_OFF};
                let icon = if prompt.show_password {
                    ICON_VISIBILITY_OFF
                } else {
                    ICON_VISIBILITY
                };
                let icon_color = ui.visuals().text_color();
                let btn_resp = ui.add(
                    eframe::egui::Button::new(
                        eframe::egui::RichText::new(icon)
                            .size(16.0)
                            .color(icon_color),
                    )
                    .frame(false),
                );
                if btn_resp.clicked() {
                    prompt.show_password = !prompt.show_password;
                }
            });

            password_len_cb.set(prompt.password.len());
            ui.add_space(8.0);
            ui.separator();
        },
        |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.with_layout(
                    eframe::egui::Layout::right_to_left(eframe::egui::Align::Center),
                    |ui| {
                        if ui.button(t!("common.cancel").as_ref()).clicked() {
                            cancel_cb.set(true);
                        }
                        let confirm_enabled = password_len.get() > 0;
                        if ui
                            .add_enabled(
                                confirm_enabled,
                                eframe::egui::Button::new(t!("common.confirm").as_ref()),
                            )
                            .clicked()
                        {
                            confirm_cb.set(true);
                        }
                    },
                );
            });
        },
    );

    let mut action = PasswordPromptAction::None;
    if confirm_clicked.get() {
        action = PasswordPromptAction::Confirm {
            path,
            password: prompt.password.clone(),
        };
    } else if cancel_clicked.get() {
        action = PasswordPromptAction::Cancel;
    }

    // 回车确认
    if ui.input(|i| i.key_pressed(eframe::egui::Key::Enter)) && !prompt.password.is_empty() {
        action = PasswordPromptAction::Confirm {
            path: prompt.path.clone(),
            password: prompt.password.clone(),
        };
    }
    // Esc 取消
    if ui.input(|i| i.key_pressed(eframe::egui::Key::Escape)) {
        action = PasswordPromptAction::Cancel;
    }

    action
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
