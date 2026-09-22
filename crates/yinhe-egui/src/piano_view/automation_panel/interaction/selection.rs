use yinhe_types::{AnchorSelRect, AutomationLane, AutomationPanelView, AutomationTarget};

/// 计算两个 sel_rect 的并集（用于 Shift/Cmd+点击或框选扩展选区）。
/// - tick 范围：取 min/max
/// - value 范围：若任一为 None（垂直全选），结果为 None；否则取 min/max
pub(crate) fn union_anchor_sel_rect(a: AnchorSelRect, b: AnchorSelRect) -> AnchorSelRect {
    let ts = a
        .tick_start
        .min(a.tick_end)
        .min(b.tick_start)
        .min(b.tick_end);
    let te = a
        .tick_start
        .max(a.tick_end)
        .max(b.tick_start)
        .max(b.tick_end);
    let value_range = match (a.value_range, b.value_range) {
        (None, _) | (_, None) => None,
        (Some((va1, va2)), Some((vb1, vb2))) => {
            let vmin = va1.min(va2).min(vb1).min(vb2);
            let vmax = va1.max(va2).max(vb1).max(vb2);
            Some((vmin, vmax))
        }
    };
    AnchorSelRect {
        tick_start: ts,
        tick_end: te,
        value_range,
    }
}

/// 持续化选框变更操作。
#[derive(Clone, Debug)]
pub(crate) enum SelRectOp {
    /// 替换所有选框为单个新选框（非 shift 框选完成 / 点击锚点设置单点选框）
    Set(AnchorSelRect),
    /// 追加一个新选框（shift+框选完成时累加）
    Append(AnchorSelRect),
    /// 替换所有选框为一组新选框（如多选框整体偏移后回写；成员身份保持不变）
    ReplaceAll(Vec<AnchorSelRect>),
    /// 替换所有选框并重置成员（Alt 复制后跟随副本：副本 id 未知，
    /// 退回矩形态按新选框重采样）。
    ReplaceAllResetMembers(Vec<AnchorSelRect>),
    /// 保持现有选框
    Keep,
}

/// 解析面板当前绑定的 lane：Tempo → `tempo_lane`；否则按 target 查找。
/// 速度面板没有 lane（调用方自行跳过）。
pub(crate) fn panel_lane<'a>(
    panel: &AutomationPanelView,
    automation_lanes: &'a [AutomationLane],
    tempo_lane: Option<&'a AutomationLane>,
) -> Option<&'a AutomationLane> {
    if panel.selected_target == AutomationTarget::Tempo {
        tempo_lane
    } else {
        automation_lanes
            .iter()
            .find(|l| l.target == panel.selected_target)
    }
}

/// 应用选框变更并同步锚点成员（PR 面板与 AR 展开 lane 共用）。
///
/// - `Set`：替换选框 → 清成员后按新选框物化；
/// - `Append`：追加选框（shift 加选）→ 保留旧成员，新选框待物化；
/// - `ReplaceAll`：拖动平移选框 → 成员身份不变（只物化未物化部分，通常为 no-op）；
/// - `ReplaceAllResetMembers`：Alt 复制 → 清成员后按新选框重采样（矩形态）；
/// - `Clear` 走 [`AutomationPanelView::clear_anchor_selection`]。
///
/// `lane` 为当前面板绑定的 lane（速度面板/无 lane 时传 None，仅改选框）。
pub(crate) fn apply_sel_rect_op(
    panel: &mut AutomationPanelView,
    op: SelRectOp,
    lane: Option<&AutomationLane>,
) {
    match op {
        SelRectOp::Set(r) => {
            panel.anchor_sel_rects = vec![r];
            panel.anchor_members = None;
            panel.materialized_anchor_rects = 0;
        }
        SelRectOp::Append(r) => panel.anchor_sel_rects.push(r),
        SelRectOp::ReplaceAll(rects) => panel.anchor_sel_rects = rects,
        SelRectOp::ReplaceAllResetMembers(rects) => {
            panel.anchor_sel_rects = rects;
            panel.anchor_members = None;
            panel.materialized_anchor_rects = 0;
        }
        SelRectOp::Keep => {}
    }
    if let Some(lane) = lane {
        panel.materialize_anchor_pending(lane);
    }
    panel.dirty = true;
}

/// Select 工具的选区变更操作（由 interaction 返回，caller 应用到 `panel`）。
#[derive(Clone, Debug)]
pub(crate) enum SelOp {
    /// 设置选框（替换或新建）
    Set(SelRectOp),
    /// 清空选框（点击空白处 < 3px）
    Clear,
    /// 开始新的框选（非加选模式 press）：清空共享音符选区（doc.edit.selected），
    /// 触发 App 层三视图选框互斥，使其他视图的选框立即消失。
    ClearNoteSelection,
}

/// 右键点击锚点时记录的编辑信息。
#[derive(Clone, Debug)]
pub(crate) struct RightClickAnchor {
    pub track_idx: u16,
    pub lane_idx: usize,
    pub old_tick: u32,
    pub target: AutomationTarget,
}
