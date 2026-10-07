//! 右栏多栏选项卡停靠布局的渲染与交互。
//!
//! 布局：若干栏垂直堆叠，每栏 = 选项卡头（横向）+ 内容区，栏间是可拖拽的
//! 分割线。拖动选项卡可实现并入其它栏 / 在栏间分裂新栏。

use eframe::egui;
use rust_i18n::t;

use yinhe_editor_core::right_panel_layout::{PanelColumn, PanelKind, RightPanelLayout};

use crate::theme;

/// 选项卡栏高度（比选项卡本身高，选项卡在其中垂直居中）。
const TAB_BAR_H: f32 = theme::ROW_H_LIST;
/// 选项卡本身高度。
const TAB_H: f32 = theme::ROW_H_COMPACT;
/// 栏间分割线厚度（与项目其它分割线一致）。
const SPLIT_H: f32 = theme::SPLIT_GAP;
/// 判定「分裂出新栏」的边界命中带（上一行底端附近）。
const SPLIT_ZONE: f32 = 12.0;
/// 选项卡文字左右内边距。
const TAB_PAD: f32 = theme::PAD_X;
/// 标签栏 / 内容区与栏边缘的水平边距（分割线不缩进）。
const PAD_X: f32 = theme::PAD_X;
/// 内容区上下边距。
const PAD_Y: f32 = theme::PAD_Y;
/// 每栏内容区最小高度（分配高度时的下限）。
const COL_MIN_H: f32 = 60.0;

/// 拖动中的选项卡状态（持久化到 egui memory）。
#[derive(Clone, Copy, Debug)]
struct DragTab {
    kind: PanelKind,
    /// 当前指针落点预览。
    drop: Option<DropTarget>,
}

/// 拖动落点。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DropTarget {
    /// 并入第 `column` 栏、插到第 `insert` 个选项卡处（0..=tabs.len()）。
    Merge { column: usize, insert: usize },
    /// 在第 `at` 处分裂出新栏（插到该栏上方；`at == columns.len()` 插到末尾）。
    Split(usize),
}

/// 第 `at` 处分裂线的 y 坐标：上一行底端；`at == 0` 取右栏外顶部。
fn boundary_y(geoms: &[ColumnGeom], content_rect: egui::Rect, at: usize) -> f32 {
    if at == 0 {
        content_rect.min.y
    } else {
        geoms[at - 1].rect.max.y
    }
}

/// 一栏的布局几何。
struct ColumnGeom {
    /// 栏整体矩形（含选项卡头）。
    rect: egui::Rect,
    /// 选项卡头矩形。
    header: egui::Rect,
    /// 内容区矩形（含边距）。
    content: egui::Rect,
    /// 内容区实际可用矩形（已扣左右/上下边距）。
    content_inner: egui::Rect,
}

/// 按权重把 `rect` 垂直分配给各栏（栏间预留分割线）。
fn compute_geoms(rect: egui::Rect, n: usize, weights: &[f32]) -> Vec<ColumnGeom> {
    if n == 0 {
        return Vec::new();
    }
    let total_h = rect.height() - SPLIT_H * (n.saturating_sub(1)) as f32;
    let total_h = total_h.max(COL_MIN_H * n as f32);
    let wsum: f32 = weights.iter().copied().filter(|w| *w > 0.0).sum::<f32>();
    let wsum = if wsum <= 0.0 { n as f32 } else { wsum };
    let mut geoms = Vec::with_capacity(n);
    let mut y = rect.min.y;
    for i in 0..n {
        let w = weights.get(i).copied().unwrap_or(1.0).max(0.0);
        let h = (total_h * w / wsum).max(COL_MIN_H);
        let col_rect =
            egui::Rect::from_min_size(egui::pos2(rect.min.x, y), egui::vec2(rect.width(), h));
        let header =
            egui::Rect::from_min_size(col_rect.min, egui::vec2(col_rect.width(), TAB_BAR_H));
        let content = egui::Rect::from_min_max(
            egui::pos2(col_rect.min.x, col_rect.min.y + TAB_BAR_H),
            col_rect.max,
        );
        let content_inner = content.shrink2(egui::vec2(PAD_X, PAD_Y));
        geoms.push(ColumnGeom {
            rect: col_rect,
            header,
            content,
            content_inner,
        });
        y = col_rect.max.y + SPLIT_H;
    }
    geoms
}

/// 选项卡标题文案（本地化）。
fn tab_label(kind: PanelKind) -> String {
    rust_i18n::t!(kind.label_key()).to_string()
}

/// 渲染多栏布局。`render_content` 负责按选项卡类型渲染内容区。
///
/// 返回是否有布局变化（用于触发持久化保存）。
#[allow(clippy::too_many_arguments)]
pub(crate) fn show(
    ui: &mut egui::Ui,
    content_rect: egui::Rect,
    layout: &mut RightPanelLayout,
    mut render_content: impl FnMut(&mut egui::Ui, PanelKind, egui::Rect),
) -> bool {
    let mut changed = false;
    let n = layout.columns.len();
    if n == 0 {
        return false;
    }
    let weights: Vec<f32> = layout.columns.iter().map(|c| c.height_weight).collect();
    let geoms = compute_geoms(content_rect, n, &weights);

    // 拖动状态。
    let drag_id = ui.id().with("rpanel_drag_tab");
    let mut drag: Option<DragTab> = ui.data_mut(|d| d.get_persisted(drag_id)).unwrap_or(None);
    let pointer = ui.input(|i| i.pointer.clone());
    let pointer_pos = pointer.hover_pos();

    // 各栏选项卡 chip 矩形（本帧绘制时收集，供落点判定/插入线使用）。
    let mut tab_rects: Vec<Vec<egui::Rect>> = vec![Vec::new(); n];

    // ── 各栏：选项卡头 + 内容 ──
    for (ci, col) in layout.columns.iter_mut().enumerate() {
        let g = &geoms[ci];

        // 内容区（先画，头覆盖其上无妨）。边距留给标签卡/内容，分割线不缩进。
        ui.scope_builder(egui::UiBuilder::new().max_rect(g.content_inner), |ui| {
            ui.set_clip_rect(g.content);
            if let Some(kind) = col.active_kind() {
                render_content(ui, kind, g.content_inner);
            }
        });

        // 选项卡头。
        paint_header(ui, g, col, ci, &mut drag, &mut changed, &mut tab_rects[ci]);
    }

    // ── 栏间分割线（拖动改权重） ──
    for (i, g) in geoms.iter().enumerate().take(n.saturating_sub(1)) {
        let handle_rect = egui::Rect::from_min_size(
            egui::pos2(content_rect.min.x, g.rect.max.y),
            egui::vec2(content_rect.width(), SPLIT_H),
        );
        let resp = crate::widgets::split_handle::horizontal(ui, ("rpanel_split", i), handle_rect);
        crate::widgets::hint::hover(ui.ctx(), &resp, t!("hint.panel.column_split"));
        if resp.dragged() {
            let dy = resp.drag_delta().y;
            // 把高度从下一栏挪给上一栏。
            let (a, b) = (
                layout.columns[i].height_weight,
                layout.columns[i + 1].height_weight,
            );
            // 权重增量需按「两栏像素总高」换算：单栏高度会让位移被放大。
            let pair_h = g.rect.height() + geoms[i + 1].rect.height();
            let total = (a + b).max(0.001);
            let unit = total / pair_h.max(1.0);
            layout.columns[i].height_weight = (a + dy * unit).max(0.05);
            layout.columns[i + 1].height_weight = (b - dy * unit).max(0.05);
            changed = true;
        }
    }

    // ── 拖动落点预览 ──
    if drag.is_some() {
        if let Some(pos) = pointer_pos {
            let drop = compute_drop(&geoms, content_rect, &tab_rects, pos);
            // 只画合并/分裂高亮。
            match drop {
                Some(DropTarget::Merge { column, insert }) => {
                    // 目标栏轻描边 + 插入位置竖线。
                    ui.painter().rect_stroke(
                        geoms[column].header,
                        3.0,
                        egui::Stroke::new(1.0, theme::accent_active()),
                        egui::StrokeKind::Inside,
                    );
                    let x = insert_x(&geoms[column].header, &tab_rects[column], insert);
                    let cy = geoms[column].header.center().y;
                    ui.painter().line_segment(
                        [
                            egui::pos2(x, cy - TAB_H * 0.5),
                            egui::pos2(x, cy + TAB_H * 0.5),
                        ],
                        egui::Stroke::new(2.0, theme::accent_active()),
                    );
                }
                Some(DropTarget::Split(at)) => {
                    let y = boundary_y(&geoms, content_rect, at);
                    ui.painter().line_segment(
                        [
                            egui::pos2(content_rect.min.x, y),
                            egui::pos2(content_rect.max.x, y),
                        ],
                        egui::Stroke::new(3.0, theme::accent_active()),
                    );
                }
                None => {}
            }
            if let Some(d) = drag.as_mut() {
                d.drop = drop;
            }
        }

        // 释放：应用落点。
        if pointer.any_released() {
            let drop = drag.and_then(|d| d.drop);
            if let Some(target) = drop {
                let kind = drag.unwrap().kind;
                match target {
                    DropTarget::Merge { column, insert } => layout.move_tab(kind, column, insert),
                    DropTarget::Split(at) => layout.split_new_column(kind, at),
                }
                changed = true;
            }
            drag = None;
        }
    }
    // 失焦清理。
    if drag.is_some() && !pointer.any_down() && !pointer.any_released() {
        drag = None;
    }
    ui.data_mut(|d| d.insert_persisted(drag_id, drag));

    changed
}

/// 画一栏的选项卡头并处理点击/拖动；`chips` 收集本栏选项卡矩形。
fn paint_header(
    ui: &mut egui::Ui,
    g: &ColumnGeom,
    col: &mut PanelColumn,
    ci: usize,
    drag: &mut Option<DragTab>,
    changed: &mut bool,
    chips: &mut Vec<egui::Rect>,
) {
    // 头背景。
    ui.painter().rect_filled(g.header, 0.0, theme::control_bg());

    // 面板区提示：悬停选项卡栏（控件 hover 会以高优先级覆盖）。
    if ui.rect_contains_pointer(g.header) {
        crate::widgets::hint::set_region(ui.ctx(), t!("hint.panel.tab_bar"));
    }

    let font = egui::FontId::proportional(crate::scaling::scaled_font(ui.ctx(), theme::SMALL_FONT));
    let mut x = g.header.min.x + PAD_X;
    for (ti, &kind) in col.tabs.iter().enumerate() {
        let label = tab_label(kind);
        // 宽度按文字自适应：量出 galley 宽度 + 左右内边距。
        let text_w = ui
            .painter()
            .layout_no_wrap(label.clone(), font.clone(), theme::text_secondary())
            .size()
            .x;
        let remaining = g.header.max.x - PAD_X - x;
        let tab_w = (text_w + TAB_PAD * 2.0).min(remaining.max(0.0));
        if tab_w < 16.0 {
            break;
        }
        // 选项卡在栏内垂直居中（栏比选项卡高）。
        let chip_y = g.header.center().y - TAB_H * 0.5;
        let tab_rect = egui::Rect::from_min_size(egui::pos2(x, chip_y), egui::vec2(tab_w, TAB_H));
        chips.push(tab_rect);
        x += tab_w + 2.0;

        let selected = ti == col.active;
        let resp = ui.interact(
            tab_rect,
            ui.id().with(("rpanel_tab", ci, ti)),
            egui::Sense::click_and_drag(),
        );
        let hovered = resp.hovered();

        // 无强调色：选中 = 淡色底；hover = 轻微增益底。
        if selected {
            ui.painter().rect_filled(
                tab_rect,
                egui::CornerRadius::same(4),
                theme::control_selected_bg(),
            );
        } else if hovered {
            ui.painter().rect_filled(
                tab_rect,
                egui::CornerRadius::same(4),
                theme::hover_color(theme::control_bg()),
            );
        }

        let color = if selected {
            theme::text_bright()
        } else {
            theme::text_secondary()
        };
        ui.painter().text(
            tab_rect.center(),
            egui::Align2::CENTER_CENTER,
            &label,
            font.clone(),
            color,
        );

        if resp.clicked() {
            col.active = ti;
            *changed = true;
        }
        // 开始拖动。
        if resp.drag_started() {
            *drag = Some(DragTab { kind, drop: None });
        }
    }
}

/// 插入位置竖线的 x：按 `insert` 落在相邻两 chip 之间（或首尾）。
fn insert_x(header: &egui::Rect, chips: &[egui::Rect], insert: usize) -> f32 {
    if chips.is_empty() {
        return header.min.x + PAD_X;
    }
    if insert == 0 {
        chips[0].left() - 1.0
    } else if insert >= chips.len() {
        chips[chips.len() - 1].right() + 1.0
    } else {
        (chips[insert - 1].right() + chips[insert].left()) * 0.5
    }
}

/// 根据指针位置计算落点：
/// - 指针停在某栏标签栏 → 并入该栏，并按 chip 中点求插入位置（优先）；
/// - 否则落在「上一行底端」附近（第一行取右栏外顶部）→ 分裂出新栏；
/// - 兜底：落在某栏内容区 → 追加到该栏末尾。
fn compute_drop(
    geoms: &[ColumnGeom],
    content_rect: egui::Rect,
    tab_rects: &[Vec<egui::Rect>],
    pos: egui::Pos2,
) -> Option<DropTarget> {
    // 1. 标签栏优先：移动到该栏并找插入位置。
    for (ci, g) in geoms.iter().enumerate() {
        if g.header.contains(pos) {
            let insert = tab_rects[ci]
                .iter()
                .filter(|r| r.center().x < pos.x)
                .count();
            return Some(DropTarget::Merge { column: ci, insert });
        }
    }

    // 2. 两栏中间：拖出标签栏后，落在上一行底端（第一行按右栏外顶部）。
    let mut best: Option<(usize, f32)> = None;
    for at in 0..=geoms.len() {
        let d = (pos.y - boundary_y(geoms, content_rect, at)).abs();
        if best.is_none_or(|(_, bd)| d < bd) {
            best = Some((at, d));
        }
    }
    if let Some((at, d)) = best
        && d <= SPLIT_ZONE
    {
        return Some(DropTarget::Split(at));
    }

    // 3. 兜底：内容区追加到该栏末尾。
    for (ci, g) in geoms.iter().enumerate() {
        if g.content.contains(pos) {
            return Some(DropTarget::Merge {
                column: ci,
                insert: tab_rects[ci].len(),
            });
        }
    }
    None
}
