//! 音轨名内联重命名：AR 音轨面板 / 图层页共用。
//!
//! 右键菜单调用 [`begin`] 进入编辑；下一帧 [`show`] 在名称位置放一个
//! 单行 TextEdit，Enter/失焦提交，Esc 取消。

use eframe::egui;

#[derive(Clone)]
struct RenameState {
    idx: usize,
    text: String,
    focus: bool,
}

fn state_id() -> egui::Id {
    egui::Id::unique("track_inline_rename")
}

/// 进入某音轨的内联重命名。
pub(crate) fn begin(ui: &egui::Ui, idx: usize, current: &str) {
    ui.data_mut(|d| {
        d.insert_temp(
            state_id(),
            RenameState {
                idx,
                text: current.to_string(),
                focus: true,
            },
        )
    });
}

/// 该音轨是否正在内联重命名。
pub(crate) fn active(ui: &egui::Ui, idx: usize) -> bool {
    ui.data(|d| {
        d.get_temp::<RenameState>(state_id())
            .is_some_and(|s| s.idx == idx)
    })
}

/// 在 `rect` 绘制重命名编辑框。提交时返回新名称（已 trim、非空）。
pub(crate) fn show(ui: &mut egui::Ui, idx: usize, rect: egui::Rect) -> Option<String> {
    let mut state = ui.data(|d| d.get_temp::<RenameState>(state_id()))?;
    if state.idx != idx {
        return None;
    }

    let font = egui::TextStyle::Body.resolve(ui.style());
    let resp = ui.put(
        rect,
        egui::TextEdit::singleline(&mut state.text)
            .font(font)
            .desired_width(rect.width())
            .margin(egui::Margin::symmetric(4, 0)),
    );
    let just_focused = state.focus;
    if just_focused {
        resp.request_focus();
        state.focus = false;
    }

    let escaped = ui.input(|i| i.key_pressed(egui::Key::Escape));
    let submitted =
        !just_focused && (ui.input(|i| i.key_pressed(egui::Key::Enter)) || resp.lost_focus());

    if escaped {
        ui.data_mut(|d| d.remove::<RenameState>(state_id()));
        return None;
    }
    if submitted {
        ui.data_mut(|d| d.remove::<RenameState>(state_id()));
        let name = state.text.trim().to_string();
        return (!name.is_empty()).then_some(name);
    }
    ui.data_mut(|d| d.insert_temp(state_id(), state));
    None
}
