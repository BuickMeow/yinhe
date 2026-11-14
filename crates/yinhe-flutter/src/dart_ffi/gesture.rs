use super::*;

/// 拖拽边缘自动滚动（Flutter 每帧调用）。返回 1 = 已滚动。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_drag_tick() -> i32 {
    // 编辑边缘自动滚动 + 松手惯性（两者都由帧驱动）。
    edit::tick_scroll() | inertia::tick()
}

/// 单指按下：按当前工具开始编辑。返回 1 = 进入编辑，0 = 交给平移。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_edit_begin(x: f32, y: f32) -> i32 {
    edit::begin(x, y)
}

/// 以选框语义起手（不修改全局工具；长按拖选框用）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_begin_marquee(x: f32, y: f32) -> i32 {
    edit::begin_marquee(x, y)
}

/// 单指落在音符右端：进入改长度（返回 0 则按普通平移处理）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_begin_note_resize(x: f32, y: f32) -> i32 {
    edit::begin_note_resize(x, y)
}

/// 单指落在音符中部：进入移动（返回 0 则按普通平移处理）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_begin_note_move(x: f32, y: f32) -> i32 {
    edit::begin_note_move(x, y)
}

/// 命中音符的像素矩形 JSON（删除淡出动画用）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_note_rect_at(x: f32, y: f32) -> *mut std::ffi::c_char {
    to_c_string(edit::note_rect_at(x, y))
}

/// 单指落在选框控制点：进入选框缩放（返回 0 则继续后续命中）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_begin_selection_handle(x: f32, y: f32) -> i32 {
    edit::begin_selection_handle(x, y)
}

/// 单指落在已有选框内：直接移动选框（返回 0 则继续后续命中）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_begin_selection_move(x: f32, y: f32) -> i32 {
    edit::begin_selection_move(x, y)
}

/// 时间标尺长按：以标尺位置起手插入/粘贴定位。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_begin_ruler_insert(x: f32, y: f32) -> i32 {
    edit::begin_ruler_insert(x, y)
}

/// 本次长按是否来自时间标尺。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_insert_from_ruler() -> i32 {
    edit::insert_from_ruler()
}

/// 长按点是否落在已有选框内。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_selection_box_at(x: f32, y: f32) -> i32 {
    edit::selection_box_at(x, y)
}

/// 单指移动：更新编辑拖拽。返回 1 = 仍在编辑，0 = 已取消（应转为平移）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_edit_update(x: f32, y: f32) -> i32 {
    edit::update(x, y)
}

/// 单指释放：提交编辑。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_edit_end(x: f32, y: f32) {
    edit::end(x, y);
}

/// 取消当前编辑拖拽（双指介入/模态窗口/切后台）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_edit_cancel() -> i32 {
    edit::cancel();
    1
}

/// Flutter 长按识别完成：把待定选框/空白态推进到移动态/粘贴态。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_edit_long_press() -> i32 {
    edit::long_press()
}

/// Flutter 手势应用：先平移，再围绕当前焦点缩放，最后统一 clamp。
///
/// `mode`：0=只平移 1=PR 时间轴 2=PR 音高轴 3=AR 时间轴 4=AR 行高。
/// PR 单击：命中音符删除，否则量化位置插入。返回 1 = 有改动。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_tap_note(x: f32, y: f32) -> i32 {
    edit::tap_note(x, y)
}

/// 长按位置上下文 JSON（只用于弹菜单，不进入拖拽态）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_long_press_context_at(x: f32, y: f32) -> *mut std::ffi::c_char {
    to_c_string(edit::context_at(x, y))
}

#[unsafe(no_mangle)]
pub extern "C" fn yinhe_viewport_gesture(
    mode: i32,
    pan_x: f32,
    pan_y: f32,
    scale: f32,
    focus_x: f32,
    focus_y: f32,
) -> i32 {
    scene::viewport_gesture(mode, pan_x, pan_y, scale, focus_x, focus_y)
}

/// 按逻辑像素平移视口（鼠标右键拖拽等）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_pan_by(dx: f32, dy: f32) -> i32 {
    scene::pan_by(dx, dy)
}

/// 以 Flutter 手势结束速度启动物理惯性（逻辑像素/秒）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_start_fling(vx: f32, vy: f32) {
    inertia::start(vx, vy);
}

/// 停止惯性。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_stop_fling() {
    inertia::stop();
}

/// 取消编辑并停止惯性（模态窗口打开/App 切后台时调用）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_reset_interaction() {
    edit::cancel();
    inertia::stop();
}

/// 切换视图：0=AR 走带，1=PR 卷帘。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_set_view_mode(mode: i32) {
    if let Ok(mut s) = scene::scene().lock() {
        s.view_mode = if mode == 0 {
            scene::ViewMode::Arrangement
        } else {
            scene::ViewMode::Pianoroll
        };
        // 切视图后选框数据换了一套：强制 Flutter 重取覆盖层（避免残留）。
        s.overlay_rev = s.overlay_rev.wrapping_add(1);
        // 清掉上一视图的拖动 ghost（AR 自动化竖线不能残留到 PR）。
        s.preview.clear();
    }
}

/// 滚轮/触控板：ctrl=1 缩放（键盘/面板区副轴，其余主轴），否则平移。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_scroll_at(x: f32, y: f32, dx: f32, dy: f32, ctrl: i32) -> i32 {
    scene::scroll_at(x, y, dx, dy, ctrl != 0)
}
