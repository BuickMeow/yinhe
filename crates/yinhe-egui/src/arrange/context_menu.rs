//! AR / 通道 AR 的音乐区右键菜单。
//!
//! 两个视图共用同一套菜单项（动作由 `App` 统一落地）：选框内提供编辑动作，
//! 选框外提供粘贴 / 插入 / 自动化等背景动作。菜单只产出命令，不直接改
//! `Document`，避免在渲染借用期内制造第二套 undo/音频同步路径。

use eframe::egui;
use rust_i18n::t;

use crate::chrome::insert_actions::InsertAction;
use crate::chrome::transport_bar::EditAction;
use crate::widgets::action_menu::PopupRow;

/// 时间线右键菜单动作。
pub(crate) enum TimelineMenuAction {
    /// 复用编辑动作分发（复制/粘贴/删除/变速…）。
    Edit { action: EditAction, tick: f64 },
    /// 复用上下文动作分发（编组 / 添加自动化 / 轨道属性…）。
    Context { id: &'static str, tick: f64 },
    /// 插入菜单动作（依赖光标 tick）。
    Insert { action: InsertAction, tick: f64 },
    /// 打开右侧批处理面板。
    Batch,
    /// 在点击轨道的点击位置新建一个空 MIDI Clip。
    NewMidiClip { track: u16, tick: f64 },
    /// 清空 AR / 通道 AR 选框。
    Deselect,
}

/// 菜单打开时的上下文（右键落点）。
#[derive(Clone)]
pub(crate) struct TimelineMenuCtx {
    /// 指针是否落在已有选框内。
    pub inside_selection: bool,
    /// 当前是否有选区内容。
    pub has_selection: bool,
    /// 右键命中轨（Conductor / Bus 行可能为 None）。
    pub track: Option<u16>,
    /// 该轨是否允许新建 MIDI Clip。
    pub can_new_clip: bool,
    /// 右键位置的 tick（已吸附）。
    pub tick: f64,
}

impl TimelineMenuCtx {
    fn menu_tick(&self) -> f64 {
        self.tick.max(0.0)
    }
}

fn item(
    ui: &mut egui::Ui,
    label: impl Into<egui::WidgetText>,
    action_id: &str,
    actions: &mut Vec<TimelineMenuAction>,
    action: TimelineMenuAction,
) {
    if ui
        .add(crate::widgets::menu::menu_item_action(
            ui, false, label, action_id,
        ))
        .clicked()
    {
        actions.push(action);
        ui.close();
    }
}

fn edit_item(
    ui: &mut egui::Ui,
    label: impl Into<egui::WidgetText>,
    action: EditAction,
    ctx: &TimelineMenuCtx,
    actions: &mut Vec<TimelineMenuAction>,
) {
    item(
        ui,
        label,
        action.action_id(),
        actions,
        TimelineMenuAction::Edit {
            action,
            tick: ctx.menu_tick(),
        },
    );
}

fn context_item(
    ui: &mut egui::Ui,
    label: impl Into<egui::WidgetText>,
    id: &'static str,
    ctx: &TimelineMenuCtx,
    actions: &mut Vec<TimelineMenuAction>,
) {
    item(
        ui,
        label,
        id,
        actions,
        TimelineMenuAction::Context {
            id,
            tick: ctx.menu_tick(),
        },
    );
}

/// 绘制时间线右键菜单。
pub(crate) fn show(
    ui: &mut egui::Ui,
    ctx: &TimelineMenuCtx,
    actions: &mut Vec<TimelineMenuAction>,
) {
    ui.set_min_width(168.0);
    ui.set_max_width(200.0);

    if ctx.inside_selection && ctx.has_selection {
        edit_item(ui, t!("menu.cut"), EditAction::Cut, ctx, actions);
        edit_item(ui, t!("menu.copy"), EditAction::Copy, ctx, actions);
        edit_item(ui, t!("menu.paste"), EditAction::Paste, ctx, actions);
        edit_item(
            ui,
            t!("menu.paste_at_original"),
            EditAction::PasteAtOriginal,
            ctx,
            actions,
        );
        ui.separator();
        edit_item(ui, t!("menu.delete"), EditAction::Delete, ctx, actions);
        edit_item(
            ui,
            t!("menu.duplicate"),
            EditAction::Duplicate,
            ctx,
            actions,
        );
        ui.separator();
        edit_item(
            ui,
            t!("menu.octave_up"),
            EditAction::TransposeUp,
            ctx,
            actions,
        );
        edit_item(
            ui,
            t!("menu.octave_down"),
            EditAction::TransposeDown,
            ctx,
            actions,
        );
        edit_item(
            ui,
            t!("menu.length_double"),
            EditAction::LengthDouble,
            ctx,
            actions,
        );
        edit_item(
            ui,
            t!("menu.length_half"),
            EditAction::LengthHalf,
            ctx,
            actions,
        );
        edit_item(
            ui,
            t!("sel.flip_horizontal"),
            EditAction::FlipHorizontal,
            ctx,
            actions,
        );
        edit_item(
            ui,
            t!("sel.flip_vertical"),
            EditAction::FlipVertical,
            ctx,
            actions,
        );
        ui.separator();
        edit_item(ui, t!("menu.humanize"), EditAction::Humanize, ctx, actions);
        edit_item(ui, t!("menu.fill_gaps"), EditAction::FillGaps, ctx, actions);
        edit_item(
            ui,
            t!("menu.merge_notes"),
            EditAction::MergeNotes,
            ctx,
            actions,
        );
        edit_item(
            ui,
            t!("menu.grid_split"),
            EditAction::GridSplit,
            ctx,
            actions,
        );
        ui.separator();
        item(
            ui,
            t!("panel.tab.batch"),
            "",
            actions,
            TimelineMenuAction::Batch,
        );
        item(
            ui,
            t!("menu.select_none"),
            "",
            actions,
            TimelineMenuAction::Deselect,
        );
        return;
    }

    edit_item(ui, t!("menu.paste"), EditAction::Paste, ctx, actions);
    edit_item(
        ui,
        t!("menu.paste_at_original"),
        EditAction::PasteAtOriginal,
        ctx,
        actions,
    );
    ui.separator();

    ui.menu_button(t!("menu.insert").as_ref(), |ui| {
        ui.set_min_width(168.0);
        ui.set_max_width(220.0);
        for action in InsertAction::ALL {
            if ui
                .add(crate::widgets::menu::menu_item_button(
                    ui,
                    false,
                    t!(action.label_key()).as_ref(),
                ))
                .clicked()
            {
                actions.push(TimelineMenuAction::Insert {
                    action,
                    tick: ctx.menu_tick(),
                });
                ui.close();
            }
        }
    });

    if ctx.track.is_some() {
        context_item(
            ui,
            t!("arrange.add_automation"),
            yinhe_editor_core::shortcuts::ACTION_ADD_AUTOMATION,
            ctx,
            actions,
        );
    }
    if ctx.can_new_clip {
        item(
            ui,
            t!("menu.new_clip"),
            "",
            actions,
            TimelineMenuAction::NewMidiClip {
                track: ctx.track.unwrap_or(0),
                tick: ctx.menu_tick(),
            },
        );
    }
    ui.separator();
    edit_item(
        ui,
        t!("menu.select_all"),
        EditAction::SelectAll,
        ctx,
        actions,
    );
    context_item(
        ui,
        t!("arrange.group_tracks"),
        yinhe_editor_core::shortcuts::ACTION_TRACK_GROUP,
        ctx,
        actions,
    );
    context_item(
        ui,
        t!("arrange.track_properties"),
        yinhe_editor_core::shortcuts::ACTION_TRACK_PROPERTIES,
        ctx,
        actions,
    );
    if ctx.has_selection {
        ui.separator();
        item(
            ui,
            t!("menu.select_none"),
            "",
            actions,
            TimelineMenuAction::Deselect,
        );
    }
}
