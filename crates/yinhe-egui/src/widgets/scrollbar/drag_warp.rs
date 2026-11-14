//! 滚动条拖动到窗口边缘时的指针瞬移。

use eframe::egui;

/// 指针距离窗口边缘小于此值时，拖动中把指针瞬移到窗口中心。
const EDGE_WARP_MARGIN: f32 = 48.0;

/// 防撞边只检查滚动条主轴方向的窗口边缘。
///
/// 横向滚动条贴在窗口底部，拖动时 y 本来就靠近底边；若检查四边会每帧
/// 触发瞬移并吞掉横向位移。纵向滚动条同理只检查左右（主轴为 Y 时检查上下）。
#[derive(Clone, Copy)]
pub(crate) enum WarpAxis {
    X,
    Y,
    Both,
}

/// 滚动条拖动中的指针防撞边状态。
#[derive(Clone, Copy)]
struct DragWarpState {
    /// 按下时的位置，松手后把指针放回这里（只在实际发生过瞬移时）。
    anchor: egui::Pos2,
    /// 上一帧刚瞬移过：下一帧的绝对位移是瞬移造成的反向跳动，必须丢弃。
    pending: bool,
    /// 本次拖动是否发生过瞬移。
    warped: bool,
}

/// 滚动条拖动的位移：优先使用当前帧绝对位移；当指针接近窗口边缘时，
/// 把它瞬移到窗口中心，并丢弃下一帧由瞬移产生的反向位移。
///
/// 隐藏指针还不够——指针仍会实际移动到屏幕/窗口边缘并停在那里。
/// 瞬移后指针始终有继续移动的空间，拖动可以无限继续。
pub(crate) fn drag_delta_with_edge_warp(
    ui: &egui::Ui,
    id: egui::Id,
    resp: &egui::Response,
    axis: WarpAxis,
) -> egui::Vec2 {
    let anchor = ui
        .input(|i| {
            i.pointer
                .press_origin()
                .or_else(|| i.pointer.interact_pos())
        })
        .unwrap_or_default();
    let mut st = ui.data_mut(|d| {
        d.get_persisted::<DragWarpState>(id)
            .unwrap_or(DragWarpState {
                anchor,
                pending: false,
                warped: false,
            })
    });
    let mut delta = resp.drag_delta();
    if st.pending {
        // 瞬移产生的 PointerMoved 会表现为一个巨大的反向 delta，丢弃它。
        delta = egui::Vec2::ZERO;
        st.pending = false;
    }
    let vp = ui.ctx().viewport_rect();
    let pos = ui.input(|i| i.pointer.interact_pos());
    let near_edge = pos.is_none_or(|p| match axis {
        WarpAxis::X => p.x < vp.min.x + EDGE_WARP_MARGIN || p.x > vp.max.x - EDGE_WARP_MARGIN,
        WarpAxis::Y => p.y < vp.min.y + EDGE_WARP_MARGIN || p.y > vp.max.y - EDGE_WARP_MARGIN,
        WarpAxis::Both => {
            p.x < vp.min.x + EDGE_WARP_MARGIN
                || p.x > vp.max.x - EDGE_WARP_MARGIN
                || p.y < vp.min.y + EDGE_WARP_MARGIN
                || p.y > vp.max.y - EDGE_WARP_MARGIN
        }
    });
    if near_edge {
        ui.ctx()
            .send_viewport_cmd(egui::ViewportCommand::CursorPosition(vp.center()));
        st.pending = true;
        st.warped = true;
    }
    ui.data_mut(|d| d.insert_persisted(id, st));
    delta
}

/// 松手后清理防撞边状态：若拖动中瞬移过，把可见指针放回按下位置。
pub(crate) fn finish_edge_warp(ui: &egui::Ui, id: egui::Id) {
    if ui.input(|i| i.pointer.primary_down()) {
        return;
    }
    let st = ui.data_mut(|d| d.get_persisted::<DragWarpState>(id));
    ui.data_mut(|d| d.remove::<DragWarpState>(id));
    if let Some(st) = st
        && st.warped
    {
        ui.ctx()
            .send_viewport_cmd(egui::ViewportCommand::CursorPosition(st.anchor));
    }
}
