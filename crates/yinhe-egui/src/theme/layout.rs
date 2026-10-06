// ── Layout constants ──
pub const TITLE_BAR_H: f32 = 32.0;
pub const RULER_H: f32 = 24.0;
pub const SCROLLBAR_H: f32 = 24.0;
/// 垂直滚动条宽度（与水平滚动条高度一致，对称设计）。
pub const SCROLLBAR_W: f32 = 24.0;
pub const SPLIT_GAP: f32 = 2.0;
pub const SPLIT_HANDLE_W: f32 = 2.0;
/// 钢琴卷帘内容层（调式色带等）不透明度。固定值，不再暴露为设置项。
pub const CONTENT_OPACITY: f32 = 0.7;

// ── 字号体系 ──
// 所有 UI 字号集中于此。高 DPI 铺垫：未来支持高分屏时在此统一乘缩放因子，
// 避免散落的魔法数字。部分常量同值但语义不同（主题系统可独立调色/调字）。
pub const MODE_LABEL_FONT: f32 = 9.5; // 模式栏讲解行/性能数字（超小字）
pub const SMALL_LABEL_FONT: f32 = 10.0; // 最弱提示（路径/计数/标尺刻度）
pub const SMALL_FONT: f32 = 11.0; // 表格/字段标签/小按钮
pub const BODY_FONT: f32 = 12.0; // 正文/字段值/事件标题
pub const TOOLTIP_FONT: f32 = 12.0; // 拖拽 tooltip
pub const ICON_FONT_SM: f32 = 12.0; // 小图标（关闭按钮）
pub const SUB_TITLE_FONT: f32 = 13.0; // 子标题/对话框标题
pub const ICON_FONT: f32 = 14.0; // 常规图标
pub const ICON_FONT_LG: f32 = 16.0; // 大图标（密码可见性等）
pub const ICON_BTN_FONT: f32 = 18.0; // 图标按钮（transport/轨道 + 等）
pub const ICON_FONT_XL: f32 = 24.0; // 超大图标（空状态装饰）
pub const PANEL_TOGGLE_FONT: f32 = MODE_LABEL_FONT + 2.0; // 自动化面板 toggle/+/- 图标
pub const TRANSPORT_BTN_SIZE: f32 = 32.0;
pub const TRANSPORT_BTN_FONT: f32 = ICON_BTN_FONT;
pub const TIMECODE_FONT: f32 = 12.0;
pub const FILE_MENU_FONT: f32 = 14.0;

// ── Layout defaults ──
pub const MIN_ARR_HEIGHT: f32 = 60.0;
pub const SPLIT_CLAMP_MIN: f32 = 0.1;
pub const SPLIT_CLAMP_MAX: f32 = 0.7;
#[allow(dead_code)]
pub const MIN_KEYBOARD_WIDTH: f32 = 30.0;
#[allow(dead_code)]
pub const MAX_KEYBOARD_RATIO: f32 = 0.4;

// ── Cursor / playhead ──
pub const CURSOR_WIDTH: f32 = 2.0;

// ── Right panel ──
pub const RIGHT_PANEL_MIN_WIDTH: f32 = 160.0;

// ── 面板排版 token（右栏/信息面板统一，见 widgets::rows）──
/// 标准水平内边距（列表行/栏内容）。
pub const PAD_X: f32 = 8.0;
/// 标准垂直内边距。
pub const PAD_Y: f32 = 4.0;
/// 间距梯级：紧凑（控件行内）。
pub const GAP_TIGHT: f32 = 2.0;
/// 间距梯级：小（相邻字段）。
pub const GAP_SM: f32 = 4.0;
/// 间距梯级：段（章节之间/分隔线两侧）。
pub const GAP: f32 = 8.0;
/// 列表/树行高（与控制高一致）。
pub const ROW_H_LIST: f32 = 24.0;
/// 紧凑行高：表格单元格 / 键值行。
pub const ROW_H_COMPACT: f32 = 20.0;
/// 列表行高亮圆角。
pub const ROW_RADIUS: f32 = 4.0;
/// 表单固定标签列宽（让各行值起始 x 对齐）。
pub const FIELD_LABEL_W: f32 = 72.0;
/// 树/层级缩进步长。
pub const INDENT_STEP: f32 = 16.0;

// ── Automation panel ──
pub const AUTO_PANEL_SPLIT_H: f32 = SPLIT_HANDLE_W;
pub const AUTO_PANEL_COMBO_WIDTH_RATIO: f32 = 1.0;

// ── System monitoring ──
pub const SYS_REFRESH_INTERVAL_SECS: f64 = 0.5;
pub const MEM_POPUP_SIZE: [f32; 2] = [280.0, 390.0];

// ── 属性浮动面板（工程设置浮窗）──
pub const PROJECT_SETTINGS_POPUP_SIZE: [f32; 2] = [520.0, 560.0];

// ── Dialog progress bars ──
#[allow(dead_code)]
pub const PROGRESS_BAR_WIDTH: f32 = 280.0;
