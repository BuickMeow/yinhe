use eframe::egui;

/// 空状态提示：右侧栏/对话框无内容时的弱化提示文字。
/// 统一"提示文字"的间距/颜色/字号（原散落在 5 个文件的重复代码）。
pub(crate) fn empty_hint(ui: &mut egui::Ui, text: &str) {
    ui.add_space(8.0);
    ui.label(
        egui::RichText::new(text)
            .color(crate::theme::text_disabled())
            .size(crate::theme::BODY_FONT),
    );
}

// ── 统一悬停提示（mode bar 左侧讲解行）──
//
// 主窗口内所有控件的悬停提示都写入同一个 ctx 数据槽；每帧末 `commit` 把本帧
// 写入值快照为 `shown` 并清空槽；`mode_bar` 读取 `shown`（上一帧末快照）渲染。
// 一帧延迟在连续悬停下不可见，同时天然消除残留提示（无需各区域手动清空）。
// `on_hover_text`（鼠标旁 tooltip）仅保留给独立子窗口/对话框与 AM 拖动数值。

fn slot_id() -> egui::Id {
    egui::Id::new("status_hint_slot")
}

fn shown_id() -> egui::Id {
    egui::Id::new("status_hint_shown")
}

/// 设置本帧讲解行提示（同帧多次调用后者覆盖前者）。
pub(crate) fn set(ctx: &egui::Context, text: impl Into<String>) {
    ctx.data_mut(|d| d.insert_temp(slot_id(), text.into()));
}

/// 清空本帧讲解行提示（覆盖此前写入）。
pub(crate) fn clear(ctx: &egui::Context) {
    ctx.data_mut(|d| d.remove_temp::<String>(slot_id()));
}

/// 控件悬停时设置提示。
pub(crate) fn hover(ctx: &egui::Context, resp: &egui::Response, text: impl Into<String>) {
    if resp.hovered() {
        set(ctx, text);
    }
}

/// 帧末提交：把本帧写入值快照为下帧显示值并清空槽。
pub(crate) fn commit(ctx: &egui::Context) {
    let v = ctx.data_mut(|d| d.remove_temp::<String>(slot_id()));
    ctx.data_mut(|d| d.insert_temp(shown_id(), v));
}

/// 读取要显示的讲解行提示（上一帧末快照）。
pub(crate) fn current(ctx: &egui::Context) -> Option<String> {
    ctx.data(|d| d.get_temp::<Option<String>>(shown_id()))
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 提交后可见、无写入帧自动清空（不残留）。
    #[test]
    fn commit_snapshots_then_clears_when_unwritten() {
        let ctx = egui::Context::default();
        commit(&ctx);
        assert_eq!(current(&ctx), None, "从未写入应为 None");

        set(&ctx, "hello");
        commit(&ctx);
        assert_eq!(current(&ctx), Some("hello".to_string()));

        commit(&ctx);
        assert_eq!(current(&ctx), None, "无写入帧提交后应清空，避免残留");
    }

    /// 同帧 clear 覆盖此前的 set。
    #[test]
    fn clear_overrides_earlier_set() {
        let ctx = egui::Context::default();
        set(&ctx, "a");
        clear(&ctx);
        commit(&ctx);
        assert_eq!(current(&ctx), None);
    }
}
