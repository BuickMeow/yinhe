use crate::TimelineViewBase;

/// 钢琴卷帘时间轴方向（横向 / 纵向瀑布流，二选一）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Orientation {
    /// 横向：时间轴 = 屏幕 X（左→右），音高 = 屏幕 Y（高音在上）。
    #[default]
    Horizontal,
    /// 纵向瀑布流：时间轴 = 屏幕 Y（上→下），音高 = 屏幕 X（低音在左）。
    Vertical,
}

impl Orientation {
    /// 切换为另一个方向。
    pub fn toggled(self) -> Self {
        match self {
            Orientation::Horizontal => Orientation::Vertical,
            Orientation::Vertical => Orientation::Horizontal,
        }
    }
}

/// Piano roll view state: manages coordinate transforms between
/// tick/key space and screen pixel space.
///
/// 方向语义（orientation）统一描述为「主轴 / 副轴」：
/// - 主轴（main）始终是**时间轴**（tick），屏幕方向随 orientation 变化：
///   横向 = X（左→右，起点在键盘列右侧），纵向 = Y（上→下，起点在内容区顶部）。
/// - 副轴（cross）始终是**音高轴**（key）：横向 = Y（高音在上），纵向 = X（低音在左）。
///
/// 交互/绘制代码应优先使用 `*_main_*` / `*_cross_*` 语义访问器，而非直接的
/// `tick_to_x` / `key_to_y`（后者是横向假设的薄封装，仅供横向专用代码使用）。
#[derive(Clone, Debug)]
pub struct PianoRollView {
    /// Shared horizontal timeline state.
    pub base: TimelineViewBase,
    /// 时间轴方向（横向 / 纵向瀑布流）。
    pub orientation: Orientation,
    /// Pixels per MIDI key (vertical zoom).
    pub key_height: f32,
    /// 上次记录的**副轴视口尺寸**（横向 = 视口高度；纵向 = 音乐区宽度）。
    /// 音高缩放是相对的：副轴视口尺寸变化（如窗口最大化）时按比例换算
    /// key_height/副轴 scroll，保持屏幕上显示的键数不变。
    /// 0.0 表示尚未初始化（首次渲染时默认显示 64 键并居中）。
    pub viewport_h: f32,
}

impl Default for PianoRollView {
    fn default() -> Self {
        Self {
            base: TimelineViewBase {
                pixels_per_tick: 0.15,
                scroll_x: 0.0,
                scroll_y: 0.0,
                left_panel_width: 60.0,
                dirty: true,
                track_panel_row_height: 40.0,
                track_panel_scroll_y: 0.0,
                follow_target: None,
            },
            key_height: 12.0,
            viewport_h: 0.0,
            orientation: Orientation::Horizontal,
        }
    }
}

impl PianoRollView {
    // ── 方向 ────────────────────────────────────────────────────────────────

    /// 当前时间轴方向。
    #[inline]
    pub fn orientation(&self) -> Orientation {
        self.orientation
    }

    #[inline]
    pub fn is_vertical(&self) -> bool {
        self.orientation == Orientation::Vertical
    }

    /// 切换方向。会重置副轴视口初始化状态，让下一帧 `clamp_scroll` 重新初始化。
    pub fn set_orientation(&mut self, o: Orientation) {
        if self.orientation == o {
            return;
        }
        self.orientation = o;
        self.viewport_h = 0.0;
        self.base.dirty = true;
    }

    // ── 旧横向语义 accessor（仅横向假设代码使用）────────────────────────────

    /// Convenience alias for the keyboard width.
    #[inline]
    pub fn keyboard_width(&self) -> f32 {
        self.base.left_panel_width
    }

    /// Total height of all 128 keys in pixels.
    pub fn total_key_height(&self) -> f32 {
        128.0 * self.key_height
    }

    /// Convert a MIDI tick to screen x coordinate（横向语义：相对内容区左缘）。
    #[inline]
    pub fn tick_to_x(&self, tick: f64) -> f32 {
        self.base.tick_to_x(tick)
    }

    /// Convert a MIDI key (0-127) to screen y coordinate（横向语义）。
    /// Key 127 (G9) is at the top, key 0 (C-1) is at the bottom.
    pub fn key_to_y(&self, key: u8) -> f32 {
        let bottom = self.total_key_height() - self.base.scroll_y;
        bottom - (key as f32 + 1.0) * self.key_height
    }

    /// Convert screen x to MIDI tick（横向语义：相对内容区左缘）。
    #[inline]
    pub fn x_to_tick(&self, x: f32) -> f64 {
        self.base.x_to_tick(x)
    }

    /// Convert screen y to MIDI key（横向语义）。
    pub fn y_to_key(&self, y: f32) -> u8 {
        let bottom = self.total_key_height() - self.base.scroll_y;
        let key_f = ((bottom - y) / self.key_height).clamp(0.0, 128.0);
        (key_f.ceil() as u8).saturating_sub(1)
    }

    /// The tick range visible on screen（横向语义，需音乐区宽度）。
    #[inline]
    pub fn visible_tick_range(&self, width: f32) -> (f64, f64) {
        self.base.visible_tick_range(width)
    }

    // ── 主轴（时间轴）语义访问器 ────────────────────────────────────────────

    /// 主轴视口长度：横向 = 音乐区宽度；纵向 = 内容区高度。
    #[inline]
    pub fn main_axis_len(&self, w: f32, h: f32) -> f32 {
        match self.orientation {
            Orientation::Horizontal => (w - self.keyboard_width()).max(0.0),
            Orientation::Vertical => h,
        }
    }

    /// tick → 主轴像素（相对音乐区左缘 / 内容区顶部，0 = 时间轴起点）。
    #[inline]
    pub fn tick_to_main_px(&self, tick: f64) -> f32 {
        tick as f32 * self.base.pixels_per_tick - self.main_scroll_val()
    }

    /// 主轴像素 → tick。
    #[inline]
    pub fn main_px_to_tick(&self, px: f32) -> f64 {
        ((px + self.main_scroll_val()) / self.base.pixels_per_tick) as f64
    }

    /// 主轴滚动值（&mut）：横向 = scroll_x（tick 滚动），纵向 = scroll_y。
    pub fn main_scroll(&mut self) -> &mut f32 {
        match self.orientation {
            Orientation::Horizontal => &mut self.base.scroll_x,
            Orientation::Vertical => &mut self.base.scroll_y,
        }
    }

    #[inline]
    pub fn main_scroll_val(&self) -> f32 {
        match self.orientation {
            Orientation::Horizontal => self.base.scroll_x,
            Orientation::Vertical => self.base.scroll_y,
        }
    }

    /// 主轴可见 tick 范围（给定主轴视口长度）。
    pub fn visible_main_range(&self, main_size: f32) -> (f64, f64) {
        let start = self.main_px_to_tick(0.0).max(0.0);
        let end = self.main_px_to_tick(main_size);
        (start, end)
    }

    /// 主轴上围绕 `px` 缩放（时间轴缩放）。
    pub fn zoom_main_around(&mut self, px: f32, factor: f32) {
        let old = self.base.pixels_per_tick;
        self.base.pixels_per_tick = (self.base.pixels_per_tick * factor).clamp(0.001, 10.0);
        // Keep the tick under the pointer stationary
        let tick = (px + self.main_scroll_val()) / old;
        *self.main_scroll() = tick * self.base.pixels_per_tick - px;
        self.base.dirty = true;
    }

    /// Clamp 主轴滚动到 [0, max]（总 tick 域）。
    pub fn clamp_main_scroll(&mut self, main_size: f32, total_ticks: f64) {
        let max_scroll = (total_ticks as f32 * self.base.pixels_per_tick - main_size).max(0.0);
        let s = self.main_scroll();
        *s = s.clamp(0.0, max_scroll);
    }

    // ── 副轴（音高轴）语义访问器 ────────────────────────────────────────────

    /// 副轴视口长度：横向 = 内容区高度；纵向 = 音乐区宽度。
    #[inline]
    pub fn cross_axis_len(&self, w: f32, h: f32) -> f32 {
        match self.orientation {
            Orientation::Horizontal => h,
            Orientation::Vertical => (w - self.keyboard_width()).max(0.0),
        }
    }

    /// key → 副轴像素。
    /// 横向：y（顶部 = key 127）；纵向：x（左缘 = key 0）。
    #[inline]
    pub fn key_to_cross_px(&self, key: u8) -> f32 {
        match self.orientation {
            Orientation::Horizontal => self.key_to_y(key),
            Orientation::Vertical => key as f32 * self.key_height - self.base.scroll_x,
        }
    }

    /// 副轴像素 → key。
    #[inline]
    pub fn cross_px_to_key(&self, px: f32) -> u8 {
        match self.orientation {
            Orientation::Horizontal => self.y_to_key(px),
            Orientation::Vertical => {
                let key_f = ((px + self.base.scroll_x) / self.key_height).clamp(0.0, 127.999);
                key_f.floor() as u8
            }
        }
    }

    /// 副轴滚动值（&mut）：横向 = scroll_y（key 滚动），纵向 = scroll_x。
    pub fn cross_scroll(&mut self) -> &mut f32 {
        match self.orientation {
            Orientation::Horizontal => &mut self.base.scroll_y,
            Orientation::Vertical => &mut self.base.scroll_x,
        }
    }

    #[inline]
    pub fn cross_scroll_val(&self) -> f32 {
        match self.orientation {
            Orientation::Horizontal => self.base.scroll_y,
            Orientation::Vertical => self.base.scroll_x,
        }
    }

    /// 副轴可见 key 范围（含 1 键 padding，用于粗剔除）。
    pub fn visible_cross_range(&self, cross_size: f32) -> (u8, u8) {
        match self.orientation {
            Orientation::Horizontal => {
                let top = self.y_to_key(0.0);
                let bottom = self.y_to_key(cross_size);
                let (lo, hi) = (bottom.min(top), top.max(bottom));
                (lo.saturating_sub(1).min(127), hi.saturating_add(1).min(127))
            }
            Orientation::Vertical => {
                let lo = (self.base.scroll_x / self.key_height).floor() as i64;
                let hi = ((self.base.scroll_x + cross_size) / self.key_height).ceil() as i64;
                let lo = lo.clamp(0, 127) as u8;
                let hi = (hi - 1).clamp(0, 127) as u8;
                (lo.saturating_sub(1).min(127), hi.saturating_add(1).min(127))
            }
        }
    }

    /// 副轴上围绕 `px` 缩放（音高缩放）。`cross_size` 为副轴视口长度。
    pub fn zoom_cross_around(&mut self, px: f32, factor: f32, cross_size: f32) {
        // 相对缩放：最小 = 128 键一屏（缩小极限），最大 = 12 键一屏（放大极限）。
        let min_kh = (cross_size / 128.0).max(0.0);
        let max_kh = (cross_size / 12.0).max(min_kh);
        let old = self.key_height;
        self.key_height = (self.key_height * factor).clamp(min_kh, max_kh);
        let new_scroll = (*self.cross_scroll() + px) / old * self.key_height - px;
        *self.cross_scroll() = new_scroll;
        self.base.dirty = true;
    }

    /// Clamp 副轴滚动到 [0, max]（128 键域）。
    pub fn clamp_cross_scroll(&mut self, cross_size: f32) {
        let max_scroll = (self.total_key_height() - cross_size).max(0.0);
        let s = self.cross_scroll();
        *s = s.clamp(0.0, max_scroll);
    }

    // ── 通用 —— 相对视口尺寸的处理 ─────────────────────────────────────────

    /// Hash of all fields that affect GPU rendering output.
    /// Used as cache key for GPU layers.  Includes only fields that
    /// change the visual output — adding a new field here is mandatory
    /// when it affects rendering.
    pub fn render_hash(&self) -> u64 {
        crate::hash::hash_f32s(&[
            self.base.pixels_per_tick,
            self.base.scroll_x,
            self.base.scroll_y,
            self.base.left_panel_width,
            self.key_height,
            self.orientation as u8 as f32,
        ])
    }

    /// Clamp scroll so the view doesn't go out of bounds.
    /// 内部按 orientation 拆分为主轴（时间）与副轴（音高）两个方向的 clamp。
    pub fn clamp_scroll(&mut self, width: f32, height: f32, total_ticks: f64) {
        let old_main = self.main_scroll_val();
        let old_cross = self.cross_scroll_val();
        let old_kh = self.key_height;

        let main_size = self.main_axis_len(width, height);
        let cross_size = self.cross_axis_len(width, height);

        // 主轴（时间）
        self.clamp_main_scroll(main_size, total_ticks);

        // 副轴（音高）：缩放是相对的：视口尺寸变化（如窗口最大化）时按比例
        // 换算 key_height/scroll，保持显示的键数不变（居中位置也自动保持）。
        if cross_size > 0.0 {
            if self.viewport_h > 0.0 && cross_size != self.viewport_h {
                let ratio = cross_size / self.viewport_h;
                self.key_height =
                    (self.key_height * ratio).clamp(cross_size / 128.0, cross_size / 12.0);
                let s = self.cross_scroll();
                *s *= ratio;
            } else if self.viewport_h == 0.0 {
                // 首次初始化：默认显示 64 键，视口居中。
                self.key_height = cross_size / 64.0;
                *self.cross_scroll() = (self.total_key_height() - cross_size).max(0.0) / 2.0;
            }
        }
        self.viewport_h = cross_size;

        // 128 键总高未超过副轴视口（含略超几像素的浮点误差）时吸附填满。
        let total = self.total_key_height();
        if cross_size > 0.0 && total < cross_size + 5.0 {
            self.key_height = cross_size / 128.0;
        }

        // 副轴（音高）clamp
        self.clamp_cross_scroll(cross_size);

        if old_main != self.main_scroll_val()
            || old_cross != self.cross_scroll_val()
            || old_kh != self.key_height
        {
            self.base.dirty = true;
        }
    }

    /// Zoom around a pointer position on the horizontal axis (屏幕 X 轴)。
    /// 横向 = 时间缩放（沿 X）；纵向 = 音高缩放（沿 X，副轴尺寸用视口值）。
    #[inline]
    pub fn zoom_around_x(&mut self, pointer_x: f32, zoom_factor: f32) {
        if self.is_vertical() {
            let cross_size = if self.viewport_h > 0.0 {
                self.viewport_h
            } else {
                self.key_height * 64.0
            };
            self.zoom_cross_around(pointer_x, zoom_factor, cross_size);
        } else {
            self.base.zoom_around_x(pointer_x, zoom_factor);
        }
    }

    /// Zoom around a pointer position on the vertical axis (屏幕 Y 轴)。
    /// 横向 = 音高缩放（沿 Y）；纵向 = 时间缩放（沿 Y）。
    /// 因此这两个方法在任意方向下都按「屏幕轴」语义工作。
    pub fn zoom_around_y(&mut self, pointer_y: f32, zoom_factor: f32, viewport_height: f32) {
        if self.is_vertical() {
            self.zoom_main_around(pointer_y, zoom_factor);
        } else {
            self.zoom_cross_around(pointer_y, zoom_factor, viewport_height);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_view() -> PianoRollView {
        PianoRollView {
            base: TimelineViewBase {
                pixels_per_tick: 0.15,
                scroll_x: 0.0,
                scroll_y: 0.0,
                left_panel_width: 60.0,
                dirty: false,
                track_panel_row_height: 40.0,
                track_panel_scroll_y: 0.0,
                follow_target: None,
            },
            key_height: 12.0,
            viewport_h: 0.0,
            orientation: Orientation::Horizontal,
        }
    }

    fn make_vertical() -> PianoRollView {
        let mut v = make_view();
        v.orientation = Orientation::Vertical;
        v
    }

    #[test]
    fn test_orientation_toggle() {
        assert_eq!(Orientation::Horizontal.toggled(), Orientation::Vertical);
        assert_eq!(Orientation::Vertical.toggled(), Orientation::Horizontal);
        assert_eq!(Orientation::default(), Orientation::Horizontal);
    }

    #[test]
    fn test_default_values() {
        let v = PianoRollView::default();
        assert_eq!(v.key_height, 12.0);
        assert_eq!(v.base.pixels_per_tick, 0.15);
        assert_eq!(v.base.left_panel_width, 60.0);
        assert_eq!(v.viewport_h, 0.0);
        assert_eq!(v.orientation, Orientation::Horizontal);
        assert!(v.base.dirty);
    }

    #[test]
    fn test_keyboard_width() {
        let v = make_view();
        assert_eq!(v.keyboard_width(), 60.0);
    }

    #[test]
    fn test_total_key_height() {
        let v = make_view();
        assert_eq!(v.total_key_height(), 128.0 * 12.0);
    }

    // ── 横向：主轴 = X，副轴 = Y（与原实现一致的回归） ──

    #[test]
    fn test_tick_to_x_origin() {
        let v = make_view();
        let x = v.tick_to_x(0.0);
        assert!((x - 60.0).abs() < 0.01);
    }

    #[test]
    fn test_key_to_y_key_60() {
        let v = make_view();
        let y = v.key_to_y(60);
        assert!((y - 804.0).abs() < 0.01);
    }

    #[test]
    fn test_y_to_key_roundtrip() {
        let v = make_view();
        for key in [0, 12, 36, 60, 72, 127] {
            let y = v.key_to_y(key);
            let back = v.y_to_key(y + 6.0);
            assert_eq!(back, key, "key {} roundtrip failed", key);
        }
    }

    #[test]
    fn test_axis_lens_horizontal() {
        let v = make_view();
        assert!((v.main_axis_len(1100.0, 500.0) - 1040.0).abs() < 0.01);
        assert!((v.cross_axis_len(1100.0, 500.0) - 500.0).abs() < 0.01);
        let (lo, hi) = v.visible_cross_range(500.0);
        assert!(lo < hi && hi <= 127);
    }

    // ── 纵向：主轴 = Y，副轴 = X ──

    #[test]
    fn test_axis_lens_vertical() {
        let v = make_vertical();
        assert!((v.main_axis_len(1100.0, 500.0) - 500.0).abs() < 0.01);
        assert!((v.cross_axis_len(1100.0, 500.0) - 1040.0).abs() < 0.01);
    }

    #[test]
    fn test_vertical_main_px_roundtrip() {
        let mut v = make_vertical();
        v.base.pixels_per_tick = 0.15;
        v.base.scroll_y = 100.0;
        for tick in [0.0, 480.0, 960.0, 12345.0] {
            let px = v.tick_to_main_px(tick);
            let back = v.main_px_to_tick(px);
            assert!((back - tick).abs() < 0.1, "tick {tick} roundtrip: {back}");
        }
    }

    #[test]
    fn test_vertical_tick0_at_top() {
        let v = make_vertical();
        // 纵向：tick 0 在主轴原点（顶部），tick 增大朝下（y 增大）。
        let px0 = v.tick_to_main_px(0.0);
        let px1 = v.tick_to_main_px(480.0);
        assert!((px0 - 0.0).abs() < 0.01);
        assert!((px1 - 480.0 * 0.15).abs() < 0.01);
        assert!(px1 > px0);
    }

    #[test]
    fn test_vertical_key_to_cross_px() {
        let v = make_vertical();
        // 纵向：key 0（低音）在左，key 127（高音）在右。
        let x0 = v.key_to_cross_px(0);
        let x127 = v.key_to_cross_px(127);
        assert!((x0 - 0.0).abs() < 0.01);
        assert!((x127 - 127.0 * 12.0).abs() < 0.01);
        assert!(x127 > x0);
    }

    #[test]
    fn test_vertical_cross_px_to_key_roundtrip() {
        let mut v = make_vertical();
        v.base.scroll_x = 12.0;
        for key in [0u8, 12, 36, 60, 72, 127] {
            let x = v.key_to_cross_px(key);
            let back = v.cross_px_to_key(x + v.key_height * 0.5);
            assert_eq!(back, key, "key {key} roundtrip failed");
        }
    }

    #[test]
    fn test_vertical_visible_main_range() {
        let mut v = make_vertical();
        v.base.pixels_per_tick = 0.15;
        v.base.scroll_y = 150.0;
        let (start, end) = v.visible_main_range(500.0);
        assert!((start - 1000.0).abs() < 1.0);
        assert!((end - (1000.0 + 3333.33)).abs() < 1.0);
    }

    #[test]
    fn test_vertical_visible_cross_range() {
        let mut v = make_vertical();
        v.base.scroll_x = 0.0;
        v.key_height = 12.0;
        let (lo, hi) = v.visible_cross_range(1200.0);
        assert_eq!(lo, 0);
        assert_eq!(hi, 100);
    }

    #[test]
    fn test_zoom_main_preserves_pointer() {
        let mut v = make_vertical();
        v.base.pixels_per_tick = 0.15;
        v.base.scroll_y = 150.0;
        let px = 300.0;
        let tick_before = v.main_px_to_tick(px);
        v.zoom_main_around(px, 2.0);
        let tick_after = v.main_px_to_tick(px);
        assert!((tick_before - tick_after).abs() < 1.0);
    }

    #[test]
    fn test_zoom_cross_around_horizontal_equals_zoom_around_y() {
        // 横向：zoom_cross_around == 旧 zoom_around_y（回归）。
        let mut a = make_view();
        let mut b = make_view();
        let py = 200.0;
        a.zoom_around_y(py, 2.0, 500.0);
        b.zoom_cross_around(py, 2.0, 500.0);
        assert!((a.key_height - b.key_height).abs() < 0.001);
        assert!((a.base.scroll_y - b.base.scroll_y).abs() < 0.001);
    }

    #[test]
    fn test_zoom_cross_preserves_pointer() {
        let mut v = make_vertical();
        v.key_height = 12.0;
        v.base.scroll_x = 60.0;
        let px = 200.0;
        let key_before = v.cross_px_to_key(px);
        v.zoom_cross_around(px, 2.0, 1040.0);
        let key_after = v.cross_px_to_key(px);
        assert_eq!(key_before, key_after);
    }

    #[test]
    fn test_clamp_scroll_vertical() {
        let mut v = make_vertical();
        v.base.pixels_per_tick = 0.15;
        v.base.scroll_y = 99999.0;
        v.base.scroll_x = 99999.0;
        v.clamp_scroll(1100.0, 500.0, 10000.0);
        // 主轴（scroll_y）clamp 到总 tick 域
        assert!(v.base.scroll_y <= 10000.0 * 0.15 - 500.0 + 0.01);
        // 副轴（scroll_x）首次初始化为 64 键居中
        let cross_size = 1100.0 - 60.0;
        assert!((v.key_height - cross_size / 64.0).abs() < 0.01);
        assert!(v.base.scroll_x <= (128.0 * v.key_height - cross_size) / 2.0 + 0.01);
    }

    #[test]
    fn test_set_orientation_resets_viewport() {
        let mut v = make_view();
        v.viewport_h = 800.0;
        v.set_orientation(Orientation::Vertical);
        assert_eq!(v.orientation, Orientation::Vertical);
        assert_eq!(v.viewport_h, 0.0);
        assert!(v.base.dirty);
        // set 相同方向不重置
        let dirty = v.base.dirty;
        v.set_orientation(Orientation::Vertical);
        assert_eq!(v.viewport_h, 0.0);
        assert_eq!(v.base.dirty, dirty);
    }

    /// 切换方向后 clamp_scroll 能重新初始化（不 panic、键数正常）。
    #[test]
    fn test_clamp_scroll_after_switching_orientation() {
        let mut v = make_view();
        v.clamp_scroll(1100.0, 600.0, 10000.0);
        v.set_orientation(Orientation::Vertical);
        v.clamp_scroll(1100.0, 600.0, 10000.0);
        let cross_size = 1100.0 - 60.0;
        assert!((v.key_height - cross_size / 64.0).abs() < 0.01);
        assert!(v.base.scroll_x >= 0.0);
    }
}
