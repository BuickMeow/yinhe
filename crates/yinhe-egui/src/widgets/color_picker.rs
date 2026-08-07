use eframe::egui;
use egui::ecolor::Hsva;
use egui::{Mesh, Pos2, Rect, Sense, Shape, Stroke, StrokeKind};

/// 自定义颜色编辑按钮：点击弹出调色板窗口。
///
/// 完全自定义布局（egui 自带 `color_picker_hsva_2d` 的数值区与色板
/// 是一个整体，无法在中间插入 HSV 数值行；egui_extras 也无调色板）：
/// ```text
/// [R][G][B][A]     ← RGBA 数值行（0-255，含 alpha）
/// [H][S][V]        ← HSV 数值行（H 0-360°、S/V 0-100%）
/// [SV 色板]        ← 饱和度/明度平面（点击/拖动取色）
/// [色相条]         ← 横向渐变（点击/拖动取色）
/// ```
pub(crate) fn color_edit_button(ui: &mut egui::Ui, color: &mut egui::Color32) -> egui::Response {
    let popup_id = ui.auto_id_with("color_picker_popup");

    let mut btn = ui.add(
        egui::Button::new("  ")
            .fill(*color)
            .stroke(Stroke::new(1.0, egui::Color32::from_gray(100)))
            .min_size(egui::vec2(28.0, 20.0))
            .corner_radius(3.0),
    );

    egui::Popup::from_toggle_button_response(&btn)
        .id(popup_id)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            let mut hsva = Hsva::from(*color);
            let mut changed = false;

            // ── RGBA 数值行（0-255，含 alpha）──
            let mut rgba = hsva.to_srgba_unmultiplied();
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for (i, name) in ["R", "G", "B", "A"].iter().enumerate() {
                    ui.label(*name);
                    changed |= ui
                        .add(egui::DragValue::new(&mut rgba[i]).range(0..=255))
                        .changed();
                }
            });
            if changed {
                hsva = Hsva::from_srgba_unmultiplied(rgba);
            }

            // ── HSV 数值行（H 0-360°、S/V 0-100%）──
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.label("H");
                let mut h = hsva.h * 360.0;
                changed |= ui
                    .add(egui::DragValue::new(&mut h).range(0.0..=360.0).suffix("°"))
                    .changed();
                hsva.h = h / 360.0;
                ui.label("S");
                let mut s = hsva.s * 100.0;
                changed |= ui
                    .add(egui::DragValue::new(&mut s).range(0.0..=100.0).suffix("%"))
                    .changed();
                hsva.s = s / 100.0;
                ui.label("V");
                let mut v = hsva.v * 100.0;
                changed |= ui
                    .add(egui::DragValue::new(&mut v).range(0.0..=100.0).suffix("%"))
                    .changed();
                hsva.v = v / 100.0;
            });

            // ── SV 色板 ──
            changed |= sv_panel(ui, &mut hsva);

            // ── 色相条 ──
            changed |= hue_bar(ui, &mut hsva);

            if changed {
                *color = egui::Color32::from(hsva);
                btn.mark_changed();
            }
        });

    btn
}

const PANEL_W: f32 = 240.0;
const PANEL_H: f32 = 150.0;
const BAR_H: f32 = 16.0;

/// 饱和度/明度平面：横向 S 0→1，纵向 V 1→0（当前色相的渐变网格）。
fn sv_panel(ui: &mut egui::Ui, hsva: &mut Hsva) -> bool {
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(PANEL_W, PANEL_H), Sense::click_and_drag());
    if ui.is_rect_visible(rect) {
        ui.painter().add(Shape::mesh(sv_mesh(rect, hsva.h)));
        ui.painter().rect_stroke(
            rect,
            0.0,
            Stroke::new(1.0, egui::Color32::from_gray(80)),
            StrokeKind::Inside,
        );
        // 当前点标记（白圈黑边，明暗底都可见）
        let pos = Pos2::new(
            rect.min.x + hsva.s * rect.width(),
            rect.min.y + (1.0 - hsva.v) * rect.height(),
        );
        ui.painter().circle_filled(pos, 5.0, egui::Color32::WHITE);
        ui.painter()
            .circle_stroke(pos, 5.0, Stroke::new(1.0, egui::Color32::from_gray(30)));
    }
    let mut changed = false;
    if (resp.dragged() || resp.clicked())
        && let Some(p) = resp.interact_pointer_pos()
    {
        let s = ((p.x - rect.min.x) / rect.width()).clamp(0.0, 1.0);
        let v = 1.0 - ((p.y - rect.min.y) / rect.height()).clamp(0.0, 1.0);
        if (hsva.s - s).abs() > 0.001 || (hsva.v - v).abs() > 0.001 {
            hsva.s = s;
            hsva.v = v;
            changed = true;
        }
    }
    changed
}

/// 横向色相条：左 0° → 右 360°（S=1, V=1 的纯色渐变）。
fn hue_bar(ui: &mut egui::Ui, hsva: &mut Hsva) -> bool {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(PANEL_W, BAR_H), Sense::click_and_drag());
    if ui.is_rect_visible(rect) {
        ui.painter().add(Shape::mesh(hue_mesh(rect)));
        ui.painter().rect_stroke(
            rect,
            0.0,
            Stroke::new(1.0, egui::Color32::from_gray(80)),
            StrokeKind::Inside,
        );
        let x = rect.min.x + hsva.h * rect.width();
        let pos = Pos2::new(x, rect.center().y);
        ui.painter().circle_filled(pos, 5.0, egui::Color32::WHITE);
        ui.painter()
            .circle_stroke(pos, 5.0, Stroke::new(1.0, egui::Color32::from_gray(30)));
    }
    let mut changed = false;
    if (resp.dragged() || resp.clicked())
        && let Some(p) = resp.interact_pointer_pos()
    {
        let h = ((p.x - rect.min.x) / rect.width()).clamp(0.0, 1.0);
        if (hsva.h - h).abs() > 0.001 {
            hsva.h = h;
            changed = true;
        }
    }
    changed
}

/// SV 渐变网格：32×24 段，顶点色 = HSV(h, s, v)。
fn sv_mesh(rect: Rect, h: f32) -> Mesh {
    const COLS: usize = 32;
    const ROWS: usize = 24;
    let mut mesh = Mesh::default();
    for iy in 0..=ROWS {
        let v = 1.0 - iy as f32 / ROWS as f32;
        let y = rect.min.y + iy as f32 / ROWS as f32 * rect.height();
        for ix in 0..=COLS {
            let s = ix as f32 / COLS as f32;
            let x = rect.min.x + ix as f32 / COLS as f32 * rect.width();
            mesh.colored_vertex(
                Pos2::new(x, y),
                egui::Color32::from(Hsva::new(h, s, v, 1.0)),
            );
        }
    }
    for iy in 0..ROWS {
        for ix in 0..COLS {
            let i0 = (iy * (COLS + 1) + ix) as u32;
            mesh.add_triangle(i0, i0 + 1, (i0 + COLS as u32) + 1);
            mesh.add_triangle(i0 + 1, (i0 + COLS as u32) + 2, (i0 + COLS as u32) + 1);
        }
    }
    mesh
}

/// 色相条渐变：64 段，顶点色 = HSV(h, 1, 1)。
fn hue_mesh(rect: Rect) -> Mesh {
    const SEGS: usize = 64;
    let mut mesh = Mesh::default();
    for i in 0..=SEGS {
        let h = i as f32 / SEGS as f32;
        let x = rect.min.x + i as f32 / SEGS as f32 * rect.width();
        let c = egui::Color32::from(Hsva::new(h, 1.0, 1.0, 1.0));
        mesh.colored_vertex(Pos2::new(x, rect.min.y), c);
        mesh.colored_vertex(Pos2::new(x, rect.max.y), c);
    }
    for i in 0..SEGS {
        let i0 = (i * 2) as u32;
        mesh.add_triangle(i0, i0 + 1, i0 + 2);
        mesh.add_triangle(i0 + 1, i0 + 3, i0 + 2);
    }
    mesh
}
