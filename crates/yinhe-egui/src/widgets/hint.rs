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
//
// 优先级：控件提示（`set`/`hover`，高）> 面板/区域提示（`set_region`，低）。
// 区域提示不会覆盖同帧已写入的控件提示，因此各视图的动态区域写入即使晚于
// 控件渲染，也不会把按钮提示顶掉。
//
// `on_hover_text`（鼠标旁 tooltip）仅保留给独立子窗口/对话框与 AM 拖动数值。

/// 控件/模式栏提示（高优先级）。
const P_CONTROL: u8 = 1;
/// 面板/区域提示（低优先级）。
const P_REGION: u8 = 0;

#[derive(Clone, Default)]
struct Hint {
    priority: u8,
    text: String,
}

fn slot_id() -> egui::Id {
    egui::Id::new("status_hint_slot")
}

fn shown_id() -> egui::Id {
    egui::Id::new("status_hint_shown")
}

fn write(ctx: &egui::Context, text: impl Into<String>, priority: u8) {
    let text = text.into();
    ctx.data_mut(|d| {
        let keep = d
            .get_temp::<Hint>(slot_id())
            .is_some_and(|h| h.priority > priority);
        if !keep {
            d.insert_temp(slot_id(), Hint { priority, text });
        }
    });
}

/// 设置控件/模式栏提示（高优先级，同帧覆盖区域提示）。
pub(crate) fn set(ctx: &egui::Context, text: impl Into<String>) {
    write(ctx, text, P_CONTROL);
}

/// 设置面板/区域提示（低优先级，不覆盖同帧控件提示）。
pub(crate) fn set_region(ctx: &egui::Context, text: impl Into<String>) {
    write(ctx, text, P_REGION);
}

/// 控件悬停时设置提示（高优先级）。
pub(crate) fn hover(ctx: &egui::Context, resp: &egui::Response, text: impl Into<String>) {
    if resp.hovered() {
        set(ctx, text);
    }
}

/// 清空区域提示：仅当当前是区域提示（或无）时清除，不误删控件提示。
pub(crate) fn clear_region(ctx: &egui::Context) {
    ctx.data_mut(|d| {
        if d.get_temp::<Hint>(slot_id())
            .is_none_or(|h| h.priority == P_REGION)
        {
            d.remove_temp::<Hint>(slot_id());
        }
    });
}

/// 帧末提交：把本帧写入值快照为下帧显示值并清空槽。
pub(crate) fn commit(ctx: &egui::Context) {
    let v = ctx
        .data_mut(|d| d.remove_temp::<Hint>(slot_id()))
        .map(|h| h.text);
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

    /// 同帧区域清空覆盖此前的区域 set（控件提示不受影响，见下）。
    #[test]
    fn clear_region_clears_region_set() {
        let ctx = egui::Context::default();
        set_region(&ctx, "region");
        clear_region(&ctx);
        commit(&ctx);
        assert_eq!(current(&ctx), None);
    }

    /// 区域提示不覆盖同帧控件提示（与写入顺序无关）。
    #[test]
    fn control_hint_survives_region_write() {
        let ctx = egui::Context::default();
        set(&ctx, "control");
        set_region(&ctx, "region");
        commit(&ctx);
        assert_eq!(current(&ctx), Some("control".to_string()));

        // 反向顺序：区域先写，控件后写仍生效。
        let ctx = egui::Context::default();
        set_region(&ctx, "region");
        set(&ctx, "control");
        commit(&ctx);
        assert_eq!(current(&ctx), Some("control".to_string()));
    }

    /// clear_region 不误删控件提示；无控件时清掉区域提示。
    #[test]
    fn clear_region_respects_control_priority() {
        let ctx = egui::Context::default();
        set(&ctx, "control");
        clear_region(&ctx);
        commit(&ctx);
        assert_eq!(current(&ctx), Some("control".to_string()));

        let ctx = egui::Context::default();
        set_region(&ctx, "region");
        clear_region(&ctx);
        commit(&ctx);
        assert_eq!(current(&ctx), None);
    }
}
