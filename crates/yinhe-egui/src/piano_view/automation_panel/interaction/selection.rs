use yinhe_types::{AnchorSelRect, AutomationTarget};

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
    /// 替换所有选框为一组新选框（如多选框整体偏移后回写）
    ReplaceAll(Vec<AnchorSelRect>),
    /// 保持现有选框
    Keep,
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
