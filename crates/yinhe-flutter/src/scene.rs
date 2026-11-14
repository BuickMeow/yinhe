//! 渲染场景：文档快照（模型 + PR/AR 双视口 + 颜色）与演示内容生成。
//!
//! 渲染线程只读快照；Flutter 手势（经 FFI 下发）与首帧初始化写视口。
//! 不跨线程借用 `Document`，避免把 editor-core 的内部可变性带进渲染线程。

use std::collections::HashSet;
use std::sync::{Arc, Mutex, OnceLock};

use yinhe_core::YinModel;
use yinhe_editor_core::Document;
use yinhe_editor_core::clipboard::{AutomationClip, NotesClipboard, PasteMode};
use yinhe_editor_core::follow::total_ticks_padded;
use yinhe_editor_core::quantize::QuantizePreset;
use yinhe_types::NoteSource;
use yinhe_types::automation::{AutomationEvent, AutomationTarget};
use yinhe_types::time_format::{
    build_time_sig_segments, compute_bar_offsets, compute_measure_divisor,
    format_tick_bar_beat_with_time_sig, measure_ticks,
};
use yinhe_types::{ArrangementView, HiddenFilter, HiddenNotes, PianoRollView, TimelineViewBase};

mod automation;
mod edit;
mod events;
mod file;
mod project;
mod recent;
mod settings;
mod view;

pub(crate) use automation::*;
pub(crate) use edit::*;
pub(crate) use events::*;
pub(crate) use file::*;
pub(crate) use project::*;
pub(crate) use recent::*;
pub(crate) use settings::*;
pub(crate) use view::*;

/// 视图模式（与 Dart 约定一致：0=AR 走带，1=PR 卷帘）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewMode {
    Arrangement = 0,
    Pianoroll = 1,
}

/// 小节线感知吸附上下文：网格相对当前小节起点对齐，拍号变化段不会错位
/// （与桌面 `yinhe_editor_core::quantize::snap_tick*` 同规则）。
#[derive(Clone)]
pub struct SnapCtx {
    ppq: u32,
    num: u8,
    den_pow: u8,
    events: Vec<yinhe_types::TimeSigEvent>,
}

impl SnapCtx {
    pub fn new(s: &RenderScene) -> Self {
        match s.model.as_ref() {
            Some(m) => Self {
                ppq: s.ppq.max(1),
                num: m.tempo_map.time_sig_default.0,
                den_pow: m.tempo_map.time_sig_default.1,
                events: m.tempo_map.time_sig_events.clone(),
            },
            None => Self {
                ppq: s.ppq.max(1),
                num: 4,
                den_pow: 2,
                events: Vec::new(),
            },
        }
    }

    fn args(&self) -> (u32, u8, u8, &[yinhe_types::TimeSigEvent]) {
        (self.ppq, self.num, self.den_pow, &self.events)
    }

    /// 就近吸附（round）。
    pub fn round(&self, q: QuantizePreset, tick: f64) -> f64 {
        let (tpb, num, den, ev) = self.args();
        yinhe_editor_core::quantize::snap_tick(tick, q, tpb, Some((tpb, num, den, ev))).max(0.0)
    }

    /// 向上吸附（ceil）。
    pub fn ceil(&self, q: QuantizePreset, tick: f64) -> f64 {
        let (tpb, num, den, ev) = self.args();
        yinhe_editor_core::quantize::snap_tick_ceil(tick, q, tpb, Some((tpb, num, den, ev)))
            .max(0.0)
    }

    /// 向下吸附（floor）。
    pub fn floor(&self, q: QuantizePreset, tick: f64) -> f64 {
        let (tpb, num, den, ev) = self.args();
        yinhe_editor_core::quantize::snap_tick_floor(tick, q, tpb, Some((tpb, num, den, ev)))
            .max(0.0)
    }

    /// 新选框的最小右端：一格量化，但不超过下一个小节线。
    pub fn min_selection_end(&self, start: f64, interval: f64) -> f64 {
        let (tpb, num, den, ev) = self.args();
        yinhe_editor_core::quantize::selection_min_end(start, interval, Some((tpb, num, den, ev)))
    }
}

/// AR 轨道面板宽度（逻辑像素，与旧安卓壳 PANEL_W 一致）。
/// AR 轨道面板宽度（168 → 134，约 -20%，给内容区更多横向空间）。
pub const AR_TRACK_PANEL_W: f32 = 134.0;

/// AR 行高（逻辑像素，触屏友好的旧壳值）。
pub const AR_LANE_HEIGHT: f32 = 56.0;

/// 手机 PR 首次进入时一屏显示的键数：约 2⅔ 个八度。
///
/// 桌面默认 64 键，在手机上每行只有 ~14dp，手指很难点准；32 键约为其两倍高。
pub const INITIAL_VISIBLE_KEYS: f32 = 32.0;

/// 轨道面板用的精简轨道信息（Dart 浮层绘制）。
pub struct TrackInfoLite {
    /// 轨道稳定标识（轨道重排/删除后 Dart 仍能记住展开状态）。
    pub uuid: String,
    pub name: String,
    pub port: u8,
    pub channel: u8,
    pub color: [f32; 4],
    pub muted: bool,
    pub soloed: bool,
    /// 所属通道的默认乐器（-1 = 未设置）。
    pub instrument_bank: i32,
    pub instrument_program: i32,
    /// 是否 Master（Conductor）轨。
    pub master: bool,
}

/// AR 拖拽时跟随移动的自动化竖线预览（按 lane 分组）。
#[derive(Clone, Debug)]
pub struct AutoGhostLane {
    /// 移动后的轨道行。
    pub track: u16,
    pub target: AutomationTarget,
    /// 与正常折叠 lane 相同的值域上限（Tempo 用 BPM）。
    pub max_val: f32,
    /// 移动后的事件（tick 已加上拖动量）。
    pub events: Vec<AutomationEvent>,
    /// 源轨道：用于隐藏原位置的选中竖线。
    pub src_track: u16,
    /// 源位置被选中的 tick（已排序）。
    pub src_ticks: Vec<u32>,
}

/// 渲染线程与 Flutter 视口手势共享的场景数据。
/// PR/AR 拖拽预览：ghost 音符 + 原位置隐藏集合 + AR 自动化竖线 ghost。
///
/// `Arc` 让渲染线程快照只做引用计数，不逐帧深拷贝（大选区时尤其重要）。
#[derive(Clone, Default)]
pub struct DragPreview {
    pub ghost: Arc<Vec<(u32, u32, u8, u16)>>,
    pub hidden: Arc<HiddenNotes>,
    /// AR 自动化竖线 ghost（跟随选框移动）。
    pub auto_ghost: Arc<Vec<AutoGhostLane>>,
    /// hidden 集合的稳定 hash；只在 hidden 真正变化时重算。
    pub hidden_hash: u64,
    /// ghost 每次变化自增，供 ghost 层缓存键使用。
    pub revision: u64,
    /// ghost 使用选中态配色（选择工具拖动）还是原色（铅笔/刷子）。
    pub selected: bool,
}

impl DragPreview {
    pub fn set_ghost(&mut self, ghost: Vec<(u32, u32, u8, u16)>, selected: bool) {
        self.ghost = Arc::new(ghost);
        self.selected = selected;
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn set_auto_ghost(&mut self, lanes: Vec<AutoGhostLane>) {
        if lanes.is_empty() && self.auto_ghost.is_empty() {
            return;
        }
        self.auto_ghost = Arc::new(lanes);
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn set_hidden_arc(&mut self, hidden: Arc<HiddenNotes>) {
        if Arc::ptr_eq(&self.hidden, &hidden) {
            return;
        }
        let hash = hidden.hidden_hash();
        if hash == self.hidden_hash && hidden.hidden_count() == self.hidden.hidden_count() {
            return;
        }
        self.hidden = hidden;
        self.hidden_hash = hash;
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn clear_hidden(&mut self) {
        if self.hidden.is_empty() && self.hidden_hash == 0 {
            return;
        }
        self.hidden = Arc::new(HiddenNotes::default());
        self.hidden_hash = 0;
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn clear(&mut self) {
        let had = !self.ghost.is_empty()
            || !self.hidden.is_empty()
            || !self.auto_ghost.is_empty()
            || self.selected;
        self.ghost = Arc::new(Vec::new());
        self.hidden = Arc::new(HiddenNotes::default());
        self.hidden_hash = 0;
        self.auto_ghost = Arc::new(Vec::new());
        self.selected = false;
        if had {
            self.revision = self.revision.wrapping_add(1);
        }
    }
}

/// PR 底部 AM 面板在主纹理中的绘制状态（Dart 布局后写入）。
#[derive(Clone, Debug)]
pub struct AutomationPanelState {
    pub visible: bool,
    /// 绘制区逻辑尺寸（独立 AM 纹理，不含 combo 列和顶部手柄）。
    pub w: f32,
    pub h: f32,
    pub active_track: usize,
    /// `None` = Velocity；`Some(i)` = 当前轨第 i 条可绘制自动化 lane。
    pub lane: Option<usize>,
    /// 铅笔工具下才在 wgpu 画锚点。
    pub show_anchors: bool,
    /// 拖动预览：old tick -> new tick/value（wgpu ghost 层）。
    pub ghost: Vec<(u32, u32, f32)>,
    /// AM 选区：当前 lane 上被选中的锚点 tick（Flutter 框选后同步）。
    pub selected_ticks: Vec<u32>,
}

impl Default for AutomationPanelState {
    fn default() -> Self {
        Self {
            visible: false,
            w: 0.0,
            h: 0.0,
            active_track: 0,
            lane: None,
            show_anchors: false,
            ghost: Vec::new(),
            selected_ticks: Vec::new(),
        }
    }
}

pub struct RenderScene {
    /// 当前文档模型（None = 无文档，渲染清屏）。
    pub model: Option<Arc<YinModel>>,
    /// PR 视口（逻辑点坐标系）。
    pub pr_view: PianoRollView,
    /// AR 视口。
    pub arr_view: ArrangementView,
    /// 当前视图。
    pub view_mode: ViewMode,
    /// 轨道颜色（RGBA per track）。
    pub track_colors: Vec<[f32; 4]>,
    /// AR 可见轨（`EditState::track_visible`，不受 PR 显示过滤影响）。
    pub track_visible: Vec<bool>,
    /// PR 可见轨（`EditState::pr_visible()`：PR 专属显隐 ∪ 主音轨）。
    pub pr_track_visible: Vec<bool>,
    /// 总 tick 数（视口 clamp 用）。
    pub total_ticks: f64,
    /// 轨道数（AR 布局/vendor 层构建用）。
    pub num_tracks: usize,
    /// 轨道面板信息（名称/端口/颜色/M/S）。
    pub tracks: Vec<TrackInfoLite>,
    /// 每拍 tick 数（网格对齐用）。
    pub ppq: u32,
    /// “显示轨道”过滤：Some = 只画该轨（曲名框选择）。
    pub display_track: Option<usize>,
    /// PR 其他轨道去色：当前编辑轨保留彩色，其余轨道转黑白。
    pub gray_other_tracks: bool,
    /// Riff 拖拽预览：只画一个“原选区 + 重复范围”的总框，隐藏持久选框。
    pub riff_preview: bool,
    /// 当前工具（PR：5 种；AR 只用选框/手形）。
    pub tool: Tool,
    /// 当前编辑轨（AR 进入 PR 的轨；工具写入目标）。
    pub active_track: Option<usize>,
    /// PR 拖拽 ghost/hidden 预览。
    pub preview: DragPreview,
    /// 非播放态的播放光标位置（点击跳转用，tick）。
    pub cursor_tick: f64,
    /// 最近一帧渲染时的播放头 x（与同帧视口 scroll 完全一致）。
    /// UI 直接读它，避免“先滚视口、后按新 tick 再算播放头”造成单帧左右抽动。
    pub last_playhead_x: f32,
    /// 覆盖层（选框/预览）修订号与内容（画布内容逻辑坐标）。
    pub overlay_rev: u32,
    pub overlay_box: Option<[f32; 4]>,
    pub overlay_cells: Vec<[f32; 4]>,
    /// 文档本体（编辑操作：轨道排序等）。
    pub document: Option<Document>,
    /// 当前文档的“来源路径”（加载时记录；保存后不改成临时文件路径）。
    /// 用于主页再次选择同一文件时直接回到工作区，不重新解析。
    pub source_path: Option<String>,
    /// 文档/选区世代号：每次 refresh_snapshot 自增，Dart 据此刷新轨道/自动化浮层。
    pub doc_generation: u32,
    /// 文档模型身份：只在替换文档（打开/新建）时自增，编辑不变。
    /// 供音符层缓存判断“这是不是一个全新的模型”，避免 revision 重置后复用旧实例。
    pub model_epoch: u64,
    /// 当前快照对应模型的内容修订号。
    ///
    /// 后台文档任务运行期间 `document` 会被临时取走（渲染仍用 `model`），
    /// 渲染层不能直接读 `document.data.revision`，否则会误判为 revision 0。
    pub model_revision: u64,
    /// PR 底部 AM 面板的 wgpu 叠加区域。
    pub automation_panel: AutomationPanelState,
    /// 播放跟随：用户手动平移/缩放后暂停，等播放头连续离开视口后恢复。
    pub follow_paused: bool,
    pub follow_out_since: Option<std::time::Instant>,
    pub follow_last_at: Option<std::time::Instant>,
    /// 恢复跟随后的一小段平滑动画（到期后改为每帧精确居中）。
    pub follow_anim_until: Option<std::time::Instant>,
    /// 选框四边的视觉偏移（像素）：量化跳变时从旧位置缓动到目标。
    pub box_anim: [f32; 4],
    /// 选框目标位置（内容像素）；`None` = 当前没有活动选框。
    pub box_target: Option<[f32; 4]>,
    /// ghost 预览的整体像素偏移（只作用于 GHOST_BIT 实例）。
    pub ghost_dx: f32,
    pub ghost_dy: f32,
    /// PR 离散音高缩放的输入累积器（1.0 = 无累积）。
    pub pr_key_zoom_accum: f32,
    /// 选框锚点拖动时的视觉偏移（四条边，像素）：和选框预览使用同一套缓动。
    pub sel_anim: [f32; 4],
    pub sel_target: Option<[f32; 4]>,
    /// 正在拖动选框控制点：期间不能丢掉 `sel_target`，否则 overlay 会退回
    /// 未提交的旧矩形，产生「新边 ↔ 旧边」来回抖动。
    pub sel_drag_active: bool,
    /// 上一次动画推进时间（指数衰减用）。
    pub(crate) anim_last: Option<std::time::Instant>,
}

/// 工具（移动端子集：与桌面 `tools_panel::Tool` 的编号对齐）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Select = 0,
    Pan = 1,
    Pencil = 2,
    Eraser = 3,
    Brush = 4,
    /// 直线工具：拖起止点，按量化列直线插值生成音符。
    Line = 5,
}

impl Default for RenderScene {
    fn default() -> Self {
        Self {
            model: None,
            pr_view: PianoRollView::default(),
            arr_view: ArrangementView::default(),
            view_mode: ViewMode::Pianoroll,
            track_colors: Vec::new(),
            track_visible: Vec::new(),
            pr_track_visible: Vec::new(),
            total_ticks: 0.0,
            num_tracks: 0,
            tracks: Vec::new(),
            ppq: 480,
            display_track: None,
            gray_other_tracks: false,
            riff_preview: false,
            tool: Tool::Select,
            active_track: None,
            preview: DragPreview::default(),
            cursor_tick: 0.0,
            last_playhead_x: -1.0,
            overlay_rev: 0,
            overlay_box: None,
            overlay_cells: Vec::new(),
            document: None,
            source_path: None,
            doc_generation: 0,
            model_epoch: 0,
            model_revision: 0,
            automation_panel: AutomationPanelState::default(),
            follow_paused: false,
            follow_out_since: None,
            follow_last_at: None,
            follow_anim_until: None,
            box_anim: [0.0; 4],
            box_target: None,
            ghost_dx: 0.0,
            ghost_dy: 0.0,
            pr_key_zoom_accum: 1.0,
            sel_anim: [0.0; 4],
            sel_target: None,
            sel_drag_active: false,
            anim_last: None,
        }
    }
}

/// 预览动画时间常数：约 45ms 衰减到 5%。
///
/// 量化跳变本身是离散的，动画只负责抹掉台阶；时间常数过长会明显落后手指。
const PREVIEW_ANIM_TAU: f32 = 0.016;

/// 小于此像素值直接归零，避免无限小尾巴。
const PREVIEW_ANIM_EPS: f32 = 0.08;

impl RenderScene {
    /// 设定选框目标；把上一次目标到本次目标的跳变量并入视觉偏移。
    ///
    /// 选框每跨过一个量化格都会跳一段，这里让视觉位置从旧格平滑追到新格。
    pub fn set_box_target(&mut self, target: [f32; 4]) {
        if self.box_target == Some(target) {
            return;
        }
        if let Some(prev) = self.box_target {
            for i in 0..4 {
                self.box_anim[i] += prev[i] - target[i];
            }
        }
        self.box_target = Some(target);
        self.overlay_box = Some(self.animated_box());
        self.anim_last = Some(std::time::Instant::now());
    }

    /// 只清除临时拖动框，保留持久选框（Riff 预览用）。
    pub fn clear_box_preview(&mut self) {
        self.box_target = None;
        self.box_anim = [0.0; 4];
        self.overlay_box = None;
    }

    /// 不参与缓动的选框（铅笔等连续预览）。
    pub fn set_box_static(&mut self, target: [f32; 4]) {
        self.box_target = None;
        self.box_anim = [0.0; 4];
        self.overlay_box = Some(target);
    }

    /// 设定选框锚点拖动的目标矩形（像素），把跳变量并入视觉偏移。
    pub fn set_sel_target(&mut self, target: [f32; 4]) {
        if self.sel_target == Some(target) {
            return;
        }
        if let Some(prev) = self.sel_target {
            for i in 0..4 {
                self.sel_anim[i] += prev[i] - target[i];
            }
        }
        self.sel_target = Some(target);
        self.anim_last = Some(std::time::Instant::now());
    }

    /// 固定选框某条边（0=左 1=上 2=右 3=下）：预览时该边不参与缓动。
    pub fn hold_box_edge(&mut self, edge: usize) {
        if edge < 4 {
            self.box_anim[edge] = 0.0;
            if self.box_target.is_some() {
                self.overlay_box = Some(self.animated_box());
            }
        }
    }

    /// 固定选区某条边（锚点拖动时对边不动）。
    pub fn hold_sel_edge(&mut self, edge: usize) {
        if edge < 4 {
            self.sel_anim[edge] = 0.0;
        }
    }

    /// ghost 目标发生跳变：把像素差累加到缓动偏移上。
    pub fn nudge_ghost(&mut self, dx: f32, dy: f32) {
        if dx == 0.0 && dy == 0.0 {
            return;
        }
        self.ghost_dx += dx;
        self.ghost_dy += dy;
        self.anim_last = Some(std::time::Instant::now());
    }

    /// 清空选框与 ghost 动画状态（提交/取消时调用）。
    pub fn clear_preview_anim(&mut self) {
        self.box_target = None;
        self.box_anim = [0.0; 4];
        self.overlay_box = None;
        self.ghost_dx = 0.0;
        self.ghost_dy = 0.0;
        self.sel_anim = [0.0; 4];
        self.sel_target = None;
        self.sel_drag_active = false;
        self.riff_preview = false;
        self.anim_last = None;
    }

    fn animated_box(&self) -> [f32; 4] {
        match self.box_target {
            Some(t) => [
                t[0] + self.box_anim[0],
                t[1] + self.box_anim[1],
                t[2] + self.box_anim[2],
                t[3] + self.box_anim[3],
            ],
            None => [0.0; 4],
        }
    }

    /// 每帧推进预览缓动。返回 true = 选框视觉位置变化（需通知 Flutter 重取）。
    pub fn tick_preview_anim(&mut self) -> bool {
        let box_animating =
            self.box_target.is_some() && self.box_anim.iter().any(|v| v.abs() > PREVIEW_ANIM_EPS);
        let ghost_animating =
            self.ghost_dx.abs() > PREVIEW_ANIM_EPS || self.ghost_dy.abs() > PREVIEW_ANIM_EPS;
        let sel_animating =
            self.sel_target.is_some() && self.sel_anim.iter().any(|v| v.abs() > PREVIEW_ANIM_EPS);
        if !box_animating && !ghost_animating && !sel_animating {
            // 收尾：把残留的极小值归零。
            let had_box = self.box_target.is_some() && self.box_anim != [0.0; 4];
            self.box_anim = [0.0; 4];
            self.ghost_dx = 0.0;
            self.ghost_dy = 0.0;
            let had_sel = self.sel_target.is_some() && self.sel_anim != [0.0; 4];
            self.sel_anim = [0.0; 4];
            if self.sel_target.is_some() && !self.sel_drag_active {
                self.sel_target = None;
            }
            if had_box {
                self.overlay_box = self.box_target;
                return true;
            }
            return had_sel;
        }
        let now = std::time::Instant::now();
        let dt = self
            .anim_last
            .map(|t| (now - t).as_secs_f32())
            .unwrap_or(1.0 / 60.0)
            .clamp(0.001, 0.05);
        self.anim_last = Some(now);
        let k = (-dt / PREVIEW_ANIM_TAU).exp();
        for v in &mut self.box_anim {
            *v = if v.abs() <= PREVIEW_ANIM_EPS {
                0.0
            } else {
                *v * k
            };
        }
        self.ghost_dx = if self.ghost_dx.abs() <= PREVIEW_ANIM_EPS {
            0.0
        } else {
            self.ghost_dx * k
        };
        self.ghost_dy = if self.ghost_dy.abs() <= PREVIEW_ANIM_EPS {
            0.0
        } else {
            self.ghost_dy * k
        };
        for v in &mut self.sel_anim {
            *v = if v.abs() <= PREVIEW_ANIM_EPS {
                0.0
            } else {
                *v * k
            };
        }
        if self.sel_target.is_some() && self.sel_anim == [0.0; 4] && !self.sel_drag_active {
            self.sel_target = None;
        }
        if self.sel_target.is_some() {
            return true;
        }
        if self.box_target.is_some() {
            let b = self.animated_box();
            if self.overlay_box != Some(b) {
                self.overlay_box = Some(b);
                return true;
            }
        }
        false
    }
}

static SCENE: OnceLock<Mutex<RenderScene>> = OnceLock::new();

/// 全局场景（渲染线程每帧取快照；Flutter 手势直接写视口）。
pub fn scene() -> &'static Mutex<RenderScene> {
    SCENE.get_or_init(|| Mutex::new(RenderScene::default()))
}

/// 纯选区变化：只刷新覆盖层，不重建音符层/轨道快照。
pub(crate) fn selection_changed(s: &mut RenderScene) {
    s.overlay_rev = s.overlay_rev.wrapping_add(1);
}

/// 从文档重算轨道快照（模型/颜色/显隐/轨道面板信息/ppq）。
pub(crate) fn refresh_snapshot(s: &mut RenderScene) {
    // 内容已变：选区物化/拖动 ghost 缓存都作废（ghost 里存的是旧位置/旧 id）。
    crate::selection_materializer::invalidate();
    s.doc_generation = s.doc_generation.wrapping_add(1);
    // 覆盖层（选框/预览）随快照变化一起失效（undo/编辑/加载后重取）。
    s.overlay_rev = s.overlay_rev.wrapping_add(1);
    // 文档修订号：编辑后（视图未变）也必须让音符层缓存失效。
    if let Some(doc) = s.document.as_mut() {
        doc.data.bump_revision();
    }
    let Some(doc) = s.document.as_ref() else {
        return;
    };
    let model = doc.data.model.clone();
    let colors = doc.edit.track_cache.colors.clone();
    s.model_revision = doc.data.revision;
    // 内容结束后实时保留 64 小节空余（编辑后立即可滚动/点击）。
    s.total_ticks = total_ticks_padded(model.tick_length, model.meta.ppq).max(1.0);
    s.model = Some(model.clone());
    s.track_colors = colors.clone();
    // AR / PR 各自一套显隐：PR 的「显示轨道」过滤不能影响 AR。
    s.track_visible = doc.edit.track_visible.clone();
    s.pr_track_visible = doc.edit.pr_visible();
    s.num_tracks = model.tracks.len();
    s.ppq = model.meta.ppq;
    s.tracks = model
        .tracks
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let inst = model
                .channel_instruments
                .get(t.channel as usize)
                .copied()
                .flatten();
            TrackInfoLite {
                uuid: t.uuid.clone(),
                name: t.name.to_string(),
                port: t.port,
                channel: t.channel,
                instrument_bank: inst.map_or(-1, |v| i32::from(v.bank_msb)),
                instrument_program: inst.map_or(-1, |v| i32::from(v.program)),
                master: t.kind == yinhe_core::TrackKind::Master || i == 0,
                color: colors.get(i).copied().unwrap_or([0.5, 0.5, 0.5, 1.0]),
                muted: doc
                    .edit
                    .track_overrides
                    .get(i)
                    .map(|o| o.muted)
                    .unwrap_or(false),
                soloed: doc
                    .edit
                    .track_overrides
                    .get(i)
                    .map(|o| o.soloed)
                    .unwrap_or(false),
            }
        })
        .collect();
}

/// 应用内音符剪贴板（复制快照 + 选区，O(1) 结构共享）。
static CLIPBOARD: Mutex<Option<NotesClipboard>> = Mutex::new(None);

/// 应用内 AM 锚点剪贴板（物化事件，跨 lane/文档粘贴安全）。
static AUTOMATION_CLIPBOARD: Mutex<Option<AutomationClip>> = Mutex::new(None);

/// Tempo 曲线值域上界：当前最大 Tempo 事件（BPM），至少 1。
///
/// AM/AR 的 Tempo 曲线都按它归一化：最大值贴着满高，新增更大的 Tempo 后
/// 所有曲线比例自动重算（读 JSON / 每帧渲染都会重新取）。
pub(crate) fn tempo_max_bpm(model: &YinModel) -> f32 {
    model
        .conductor
        .tempo
        .events
        .iter()
        .map(|e| e.value)
        .fold(0.0_f32, f32::max)
        .max(1.0)
}

/// 极简 JSON 字符串转义（双引号/反斜杠/控制字符）。
pub(crate) fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {}
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests;
