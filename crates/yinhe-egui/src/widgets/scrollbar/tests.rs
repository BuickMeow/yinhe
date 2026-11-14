use super::density::fill_density_pixels;
use super::vertical::snap_cell_size;
use super::*;
use eframe::egui;
use yinhe_editor_core::density::{NO_TRACK, NoteDensityView};

#[test]
fn snap_cell_size_continuous_or_stepped() {
    // 连续：仅 clamp。
    assert_eq!(snap_cell_size(37.3, 16.0, 120.0, None), 37.3);
    assert_eq!(snap_cell_size(5.0, 16.0, 120.0, None), 16.0);
    // 离散：吸附到 min + k·step，并 clamp。
    assert_eq!(snap_cell_size(37.3, 16.0, 120.0, Some(8.0)), 40.0);
    assert_eq!(snap_cell_size(42.0, 16.0, 120.0, Some(8.0)), 40.0);
    assert_eq!(snap_cell_size(0.0, 16.0, 120.0, Some(8.0)), 16.0);
    assert_eq!(snap_cell_size(200.0, 16.0, 120.0, Some(8.0)), 120.0);
}

/// 跑一帧滚动条，返回背景拖拽返回值。
fn run_frame(
    ctx: &egui::Context,
    raw: egui::RawInput,
    rect: egui::Rect,
    scroll_x: &mut f32,
    ppt: &mut f32,
    dirty: &mut bool,
) -> f32 {
    let mut out = 0.0f32;
    ctx.run_ui(raw, |ui| {
        out = show(
            ui,
            rect,
            300.0,
            scroll_x,
            ppt,
            1000.0,
            None,
            None,
            dirty,
            yinhe_types::Orientation::Horizontal,
        );
    })
    .textures_delta
    .clear();
    out
}

/// 性能标定（手动运行）：
/// `cargo test -p yinhe-egui --release bench_density_draw -- --ignored --nocapture`
///
/// 模拟最坏场景：2000px 宽滚动条、1000 万 tick 全曲（约 5 桶/列）、
/// 50% 非零填充的黑乐谱密度，测每帧绘制（含 egui tessellation）。
#[test]
#[ignore = "性能标定，手动运行"]
fn bench_density_draw() {
    use std::time::Instant;
    let ctx = egui::Context::default();
    let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(2000.0, 24.0));
    let mut scroll_x = 0.0f32;
    let mut ppt = 0.0002f32; // 2000px 视口 ≈ 全曲 1000 万 tick
    let mut dirty = false;
    let n_buckets = 9766; // 10_000_000 / 1024
    let n_keys = 128;
    let mut rows = vec![0u32; n_keys * n_buckets];
    let mut dominant = vec![NO_TRACK; n_keys * n_buckets];
    for (i, v) in rows.iter_mut().enumerate() {
        if i % 2 == 0 {
            *v = (i % 200) as u32 + 1;
            dominant[i] = (i % 16) as u32;
        }
    }
    let colors: Vec<[f32; 4]> = (0..16)
        .map(|i| [i as f32 / 16.0, 0.5, 1.0 - i as f32 / 16.0, 1.0])
        .collect();
    let mut run_frames = |n: usize, bump_version: bool| -> std::time::Duration {
        let start = Instant::now();
        let mut ver = 7u64;
        for _ in 0..n {
            if bump_version {
                ver += 1;
            }
            let view = NoteDensityView {
                rows: &rows,
                dominant: &dominant,
                n_keys,
                n_buckets,
                bucket_ticks: 1024,
                max_count: 200,
                version: ver,
            };
            let preview = DensityPreview {
                density: view,
                track_colors: &colors,
            };
            ctx.run_ui(egui::RawInput::default(), |ui| {
                show(
                    ui,
                    rect,
                    2000.0,
                    &mut scroll_x,
                    &mut ppt,
                    10_000_000.0,
                    Some(&preview),
                    None,
                    &mut dirty,
                    yinhe_types::Orientation::Horizontal,
                );
            })
            .textures_delta
            .clear();
        }
        start.elapsed() / n as u32
    };
    run_frames(10, false); // 预热（字体/缓存初始化）
    println!(
        "steady-state (纹理复用): {:?}/frame",
        run_frames(100, false)
    );
    println!(
        "rebuild-every-frame (编辑中): {:?}/frame",
        run_frames(100, true)
    );
}

/// 纹理缓存失效矩阵：稳态复用不重建；数据版本（编辑/切显隐）、
/// 轨道配色（颜色重分配）、尺寸（窗口 resize）任一变化必须重建。
/// 通过 `textures_delta.set` 数量观察（该帧只调 show，无其他纹理产出）。
#[test]
fn density_texture_invalidation_matrix() {
    let ctx = egui::Context::default();
    let mut scroll_x = 0.0f32;
    let mut ppt = 1.0f32;
    let mut dirty = false;
    let n_buckets = 100;
    let n_keys = 128;
    let rows: Vec<u32> = (0..n_keys * n_buckets).map(|i| (i % 5) as u32).collect();
    let dominant: Vec<u32> = vec![0; n_keys * n_buckets];
    let colors_a: Vec<[f32; 4]> = vec![[1.0, 0.5, 0.2, 1.0]];
    let colors_b: Vec<[f32; 4]> = vec![[0.2, 0.5, 1.0, 1.0]];

    let mut run = |version: u64, rect_w: f32, colors: &[[f32; 4]]| -> usize {
        let view = NoteDensityView {
            rows: &rows,
            dominant: &dominant,
            n_keys,
            n_buckets,
            bucket_ticks: 10,
            max_count: 2,
            version,
        };
        let preview = DensityPreview {
            density: view,
            track_colors: colors,
        };
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(rect_w, 24.0));
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            show(
                ui,
                rect,
                rect_w,
                &mut scroll_x,
                &mut ppt,
                1000.0,
                Some(&preview),
                None,
                &mut dirty,
                yinhe_types::Orientation::Horizontal,
            );
        });
        let n = out.textures_delta.set.len();
        out.textures_delta.clear();
        n
    };
    // 首次建立纹理（顺带吞掉字体等一次性纹理帧）。
    let _ = run(1, 300.0, &colors_a);
    assert_eq!(run(1, 300.0, &colors_a), 0, "稳态不得重建纹理");
    assert!(
        run(2, 300.0, &colors_a) >= 1,
        "数据版本变化（编辑/切显隐）应重建"
    );
    assert_eq!(run(2, 300.0, &colors_a), 0, "继续稳态");
    assert!(
        run(2, 300.0, &colors_b) >= 1,
        "配色变化（颜色重分配）应重建"
    );
    assert!(
        run(2, 350.0, &colors_b) >= 1,
        "尺寸变化（窗口 resize）应重建"
    );
    assert_eq!(run(2, 350.0, &colors_b), 0, "稳态不得重建纹理");

    // HiDPI：逻辑尺寸不变、pixels_per_point 变化也必须重建（纹理按物理像素建）。
    ctx.set_pixels_per_point(2.0);
    assert!(
        run(2, 350.0, &colors_b) >= 1,
        "pixels_per_point 变化（HiDPI）应重建纹理"
    );
    assert_eq!(run(2, 350.0, &colors_b), 0, "HiDPI 后继续稳态");
}

/// 回归：桶宽大于一个像素列时，相邻桶之间不得出现整列空白。
/// 旧实现把每个桶只画到"跨越桶边界"的那一列，其余列留空，
/// 预览呈周期 = 桶宽 × 缩放的虚线（曲长/PPQ 不同则间距不同）。
#[test]
fn density_preview_has_no_periodic_gaps() {
    let n_buckets = 100usize;
    let cols = 200usize;
    let bucket_ticks = 1024u32;
    let total_ticks = n_buckets as u64 * bucket_ticks as u64; // 每桶跨 2 列
    let n_keys = 128;
    let mut rows = vec![0u32; n_keys * n_buckets];
    let mut dominant = vec![NO_TRACK; n_keys * n_buckets];
    for b in 0..n_buckets {
        // 每个桶都有音符：任何整列空白都只能是绘制映射的 bug。
        rows[60 * n_buckets + b] = 1;
        dominant[60 * n_buckets + b] = 0;
    }
    let view = NoteDensityView {
        rows: &rows,
        dominant: &dominant,
        n_keys,
        n_buckets,
        bucket_ticks,
        max_count: 1,
        version: 1,
    };
    let colors = vec![[1.0, 0.0, 0.0, 1.0]];
    let mut pixels = vec![egui::Color32::TRANSPARENT; cols];
    fill_density_pixels(
        &mut pixels,
        cols,
        1,
        false,
        cols as f32,
        1,
        cols as f64 / total_ticks as f64,
        &view,
        &colors,
    );
    let blank: Vec<usize> = pixels
        .iter()
        .enumerate()
        .filter_map(|(i, p)| (*p == egui::Color32::TRANSPARENT).then_some(i))
        .collect();
    assert!(blank.is_empty(), "桶间不应出现空白列：{blank:?}");
}

/// 密度预览：有数据时逐格绘制不 panic（含超出桶覆盖范围的尾部、
/// 主导轨缺失与配色表越界的回退路径）。
#[test]
fn density_preview_renders_without_panic() {
    let ctx = egui::Context::default();
    let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(300.0, 16.0));
    let mut scroll_x = 0.0f32;
    let mut ppt = 1.0f32;
    let mut dirty = false;
    let n_buckets = 100;
    let n_keys = 128;
    let rows: Vec<u32> = (0..n_keys * n_buckets)
        .map(|i| (i % 7) as u32 * 50)
        .collect();
    // 0..3 轮流当主导轨（3 越界 → 回退路径），部分桶无主导轨。
    let dominant: Vec<u32> = (0..n_keys * n_buckets)
        .map(|i| {
            if i % 11 == 0 {
                NO_TRACK
            } else {
                (i % 4) as u32
            }
        })
        .collect();
    let view = NoteDensityView {
        rows: &rows,
        dominant: &dominant,
        n_keys,
        n_buckets,
        bucket_ticks: 10,
        max_count: 300,
        version: 1,
    };
    let colors: Vec<[f32; 4]> = vec![
        [1.0, 0.5, 0.2, 1.0],
        [0.2, 0.6, 1.0, 1.0],
        [0.5, 1.0, 0.4, 1.0],
    ];
    let preview = DensityPreview {
        density: view,
        track_colors: &colors,
    };
    ctx.run_ui(egui::RawInput::default(), |ui| {
        show(
            ui,
            rect,
            300.0,
            &mut scroll_x,
            &mut ppt,
            1000.0,
            Some(&preview),
            None,
            &mut dirty,
            yinhe_types::Orientation::Horizontal,
        );
    })
    .textures_delta
    .clear();
}

/// 跑一帧值空间滚动条（show_vertical_value），无返回值。
fn run_frame_value(
    ctx: &egui::Context,
    raw: egui::RawInput,
    rect: egui::Rect,
    value_scroll: &mut f32,
    value_zoom: &mut f32,
    dirty: &mut bool,
) {
    ctx.run_ui(raw, |ui| {
        show_vertical_value(
            ui,
            rect,
            200.0,
            value_scroll,
            value_zoom,
            127.0,
            1.0,
            8.0,
            dirty,
        );
    })
    .textures_delta
    .clear();
}

fn press_event(pos: egui::Pos2) -> egui::RawInput {
    let mut raw = egui::RawInput::default();
    raw.events.push(egui::Event::PointerMoved(pos));
    raw.events.push(egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::default(),
    });
    raw
}

fn drag_event(pos: egui::Pos2) -> egui::RawInput {
    let mut raw = egui::RawInput::default();
    raw.events.push(egui::Event::PointerMoved(pos));
    raw
}

fn has_cursor_position(out: &egui::FullOutput) -> bool {
    out.viewport_output.values().any(|v| {
        v.commands
            .iter()
            .any(|c| matches!(c, egui::ViewportCommand::CursorPosition(_)))
    })
}

fn raw_with_screen(pos: egui::Pos2, pressed: Option<bool>) -> egui::RawInput {
    let mut raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::pos2(0.0, 0.0),
            egui::vec2(400.0, 300.0),
        )),
        ..Default::default()
    };
    raw.events.push(egui::Event::PointerMoved(pos));
    if let Some(pressed) = pressed {
        raw.events.push(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        });
    }
    raw
}

/// 指针靠近窗口边缘时拖动滚动条：瞬移到窗口中心；下一帧的反向 delta
/// 必须被丢弃，之后仍可继续拖动。松手后指针回到按下位置。
#[test]
fn scrollbar_drag_warps_away_from_window_edge() {
    let ctx = egui::Context::default();
    // 滚动条贴近窗口底部：y=278，距 300 的底边只有 22px。
    let rect = egui::Rect::from_min_size(egui::pos2(0.0, 270.0), egui::vec2(300.0, 16.0));
    let mut scroll_x = 0.0f32;
    let mut ppt = 1.0f32;
    let mut dirty = false;
    fn run(
        ctx: &egui::Context,
        raw: egui::RawInput,
        rect: egui::Rect,
        scroll_x: &mut f32,
        ppt: &mut f32,
        dirty: &mut bool,
    ) -> egui::FullOutput {
        let mut out = ctx.run_ui(raw, |ui| {
            show(
                ui,
                rect,
                300.0,
                scroll_x,
                ppt,
                1000.0,
                None,
                None,
                dirty,
                yinhe_types::Orientation::Horizontal,
            );
        });
        out.textures_delta.clear();
        out
    }
    let _ = run(
        &ctx,
        raw_with_screen(egui::pos2(45.0, 278.0), None),
        rect,
        &mut scroll_x,
        &mut ppt,
        &mut dirty,
    );
    let _ = run(
        &ctx,
        raw_with_screen(egui::pos2(45.0, 278.0), Some(true)),
        rect,
        &mut scroll_x,
        &mut ppt,
        &mut dirty,
    );

    // 向右拖到窗口右缘；y 仍在底边附近，但横向滚动条只检查 X 轴，
    // 不应因为贴底而吞掉横向位移。
    let out = run(
        &ctx,
        raw_with_screen(egui::pos2(370.0, 278.0), None),
        rect,
        &mut scroll_x,
        &mut ppt,
        &mut dirty,
    );
    let after_drag = scroll_x;
    assert!(
        after_drag > 0.0,
        "拖动本身仍应平移，实际 scroll_x={after_drag}"
    );
    assert!(
        has_cursor_position(&out),
        "指针靠近窗口边缘时应瞬移到窗口中心"
    );

    // 模拟瞬移后的 PointerMoved：指针已跳到中心，此时绝不能把反向 delta
    // 当成拖动，scroll_x 必须保持不变。
    let out = run(
        &ctx,
        raw_with_screen(egui::pos2(200.0, 150.0), None),
        rect,
        &mut scroll_x,
        &mut ppt,
        &mut dirty,
    );
    assert_eq!(scroll_x, after_drag, "瞬移产生的反向 delta 必须被丢弃");
    assert!(!has_cursor_position(&out), "中心位置不应再次瞬移");

    // 从中心继续拖（反向 10px）→ 正常平移。上一步已把 scroll 拖到上限，
    // 只有反向移动才能观察到变化。
    let _ = run(
        &ctx,
        raw_with_screen(egui::pos2(190.0, 150.0), None),
        rect,
        &mut scroll_x,
        &mut ppt,
        &mut dirty,
    );
    assert!(scroll_x < after_drag, "瞬移后应能继续拖动");

    // 松手：指针放回按下位置 (45,278)。
    let out = run(
        &ctx,
        raw_with_screen(egui::pos2(210.0, 150.0), Some(false)),
        rect,
        &mut scroll_x,
        &mut ppt,
        &mut dirty,
    );
    assert!(has_cursor_position(&out), "松手后应把可见指针放回按下位置");
}

/// 背景拖拽（thumb 之外的 band 区域）→ 不返回 dy、不平移。
/// 回归：拖动自动化锚点时鼠标靠近滚动条 band，绝不能和滚动条一起拖动、
/// 绝不能在没按到移动组件（thumb）时触发缩放。
/// 配置：total=1000 tick，view_width=300，ppt=1 → thumb 占 [0, 90]，
/// x=200 是背景区。
/// egui 的 hit test 基于上一帧注册的 widgets：先 hover 注册一帧再 press。
#[test]
fn background_drag_does_not_zoom() {
    let ctx = egui::Context::default();
    let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(300.0, 16.0));
    let mut scroll_x = 0.0f32;
    let mut ppt = 1.0f32;
    let mut dirty = false;

    let start = egui::pos2(200.0, 8.0);
    let end = egui::pos2(200.0, 28.0);
    // 帧1：hover 注册 widget（hit test 在下一帧生效）
    let _ = run_frame(
        &ctx,
        drag_event(start),
        rect,
        &mut scroll_x,
        &mut ppt,
        &mut dirty,
    );
    // 帧2：press
    let _ = run_frame(
        &ctx,
        press_event(start),
        rect,
        &mut scroll_x,
        &mut ppt,
        &mut dirty,
    );
    // 帧3：drag（本帧垂直移动 20px）→ 背景拖拽不返回 dy
    let dy = run_frame(
        &ctx,
        drag_event(end),
        rect,
        &mut scroll_x,
        &mut ppt,
        &mut dirty,
    );
    assert_eq!(dy, 0.0, "背景拖拽不应返回 dy（不得触发缩放），实际 {dy}");
    assert_eq!(scroll_x, 0.0, "背景拖拽不应平移");
}

/// 水平滚动条：鼠标在 band 外按下拖动（interact_radius 范围内）→ 不应平移/缩放。
/// 回归：拖动自动化锚点时鼠标靠近水平滚动条 band，egui 的 interact_radius
/// 会让 band 附近的指针命中滚动条 widget，导致误触发滚动条操作。
#[test]
fn horizontal_scrollbar_ignores_drag_outside_band() {
    let ctx = egui::Context::default();
    let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(300.0, 16.0));
    let mut scroll_x = 0.0f32;
    let mut ppt = 1.0f32;
    let mut dirty = false;

    // 鼠标在 band 外（y=-3，距 band 上边缘 3px < interact_radius=5），x 在 thumb 中间
    let start = egui::pos2(45.0, -3.0);
    let end = egui::pos2(95.0, -3.0);
    // 帧1：hover 注册 widget
    let _ = run_frame(
        &ctx,
        drag_event(start),
        rect,
        &mut scroll_x,
        &mut ppt,
        &mut dirty,
    );
    // 帧2：press（band 外）
    let _ = run_frame(
        &ctx,
        press_event(start),
        rect,
        &mut scroll_x,
        &mut ppt,
        &mut dirty,
    );
    // 帧3：drag（水平移动 50px）→ 不应平移、不应缩放
    let dy = run_frame(
        &ctx,
        drag_event(end),
        rect,
        &mut scroll_x,
        &mut ppt,
        &mut dirty,
    );
    assert_eq!(
        scroll_x, 0.0,
        "band 外拖动不应平移 scroll_x，实际 {scroll_x}"
    );
    assert_eq!(ppt, 1.0, "band 外拖动不应缩放 ppt，实际 {ppt}");
    assert_eq!(dy, 0.0, "band 外拖动不应返回 dy，实际 {dy}");
}

/// thumb 中间拖拽 = 平移，返回 0（不触发对面轴缩放）。
#[test]
fn thumb_drag_pans_and_returns_zero() {
    let ctx = egui::Context::default();
    let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(300.0, 16.0));
    let mut scroll_x = 0.0f32;
    let mut ppt = 1.0f32;
    let mut dirty = false;

    // thumb 中间 (45, 8)，拖到 (95, 8)：平移 50px = 50/0.3 tick。
    let start = egui::pos2(45.0, 8.0);
    let end = egui::pos2(95.0, 8.0);
    // 帧1：hover 注册
    let _ = run_frame(
        &ctx,
        drag_event(start),
        rect,
        &mut scroll_x,
        &mut ppt,
        &mut dirty,
    );
    // 帧2：press
    let _ = run_frame(
        &ctx,
        press_event(start),
        rect,
        &mut scroll_x,
        &mut ppt,
        &mut dirty,
    );
    // 帧3：drag（本帧移动 50px）→ thumb 平移，返回 0
    let dx = run_frame(
        &ctx,
        drag_event(end),
        rect,
        &mut scroll_x,
        &mut ppt,
        &mut dirty,
    );
    assert_eq!(dx, 0.0, "thumb 拖拽（平移）不应返回背景 dx");
    assert!(scroll_x > 0.0, "thumb 拖拽应平移 scroll_x");
}

/// thumb 上垂直拖动 → 返回 dy（缩放），不产生平移。
#[test]
fn thumb_vertical_drag_returns_dy() {
    let ctx = egui::Context::default();
    let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(300.0, 16.0));
    let mut scroll_x = 0.0f32;
    let mut ppt = 1.0f32;
    let mut dirty = false;

    let start = egui::pos2(45.0, 8.0);
    let end = egui::pos2(45.0, 28.0);
    // 帧1：hover 注册
    let _ = run_frame(
        &ctx,
        drag_event(start),
        rect,
        &mut scroll_x,
        &mut ppt,
        &mut dirty,
    );
    // 帧2：press
    let _ = run_frame(
        &ctx,
        press_event(start),
        rect,
        &mut scroll_x,
        &mut ppt,
        &mut dirty,
    );
    // 帧3：drag（本帧垂直移动 20px）→ 返回 dy，x 未动 → 不平移
    let dy = run_frame(
        &ctx,
        drag_event(end),
        rect,
        &mut scroll_x,
        &mut ppt,
        &mut dirty,
    );
    assert!(dy > 0.0, "thumb 上垂直拖应返回非零 dy，实际 {dy}");
    assert_eq!(scroll_x, 0.0, "纯垂直拖不得平移");
}

/// 鼠标在滚动条 band 外按下拖动（interact_radius 范围内）→ 不应触发平移/缩放。
/// 回归：拖动自动化锚点时鼠标靠近滚动条 band，egui 的 interact_radius 会让
/// band 附近的指针命中滚动条 widget，导致误触发滚动条操作。
#[test]
fn value_scrollbar_ignores_drag_outside_band() {
    let ctx = egui::Context::default();
    let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(24.0, 200.0));
    let mut value_scroll = 0.0f32;
    let mut value_zoom = 2.0f32; // visible_range=63.5，max_scroll>0，middle 拖动可平移
    let mut dirty = false;

    // 鼠标在 band 外（x=-3，距 band 左边缘 3px < interact_radius=5），y 在 thumb 中间
    let start = egui::pos2(-3.0, 100.0);
    let end = egui::pos2(-3.0, 130.0);
    // 帧1：hover 注册 widget
    run_frame_value(
        &ctx,
        drag_event(start),
        rect,
        &mut value_scroll,
        &mut value_zoom,
        &mut dirty,
    );
    // 帧2：press
    run_frame_value(
        &ctx,
        press_event(start),
        rect,
        &mut value_scroll,
        &mut value_zoom,
        &mut dirty,
    );
    // 帧3：drag（垂直移动 30px）→ 不应平移 value_scroll
    run_frame_value(
        &ctx,
        drag_event(end),
        rect,
        &mut value_scroll,
        &mut value_zoom,
        &mut dirty,
    );
    assert_eq!(
        value_scroll, 0.0,
        "band 外拖动不应平移 value_scroll，实际 {value_scroll}"
    );
    assert_eq!(
        value_zoom, 2.0,
        "band 外拖动不应缩放 value_zoom，实际 {value_zoom}"
    );
}

/// 鼠标在 band 外按下、拖动过程中划过 band → 仍不应触发滚动条操作。
/// 回归：拖动自动化锚点时鼠标按下在 band 外，但拖动路径经过 band。
#[test]
fn value_scrollbar_ignores_drag_crossing_band() {
    let ctx = egui::Context::default();
    let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(24.0, 200.0));
    let mut value_scroll = 0.0f32;
    let mut value_zoom = 2.0f32;
    let mut dirty = false;

    // 按下在 band 外（x=-3），拖动到 band 内（x=12）
    let start = egui::pos2(-3.0, 100.0);
    let end = egui::pos2(12.0, 130.0);
    // 帧1：hover 注册 widget
    run_frame_value(
        &ctx,
        drag_event(start),
        rect,
        &mut value_scroll,
        &mut value_zoom,
        &mut dirty,
    );
    // 帧2：press（band 外）
    run_frame_value(
        &ctx,
        press_event(start),
        rect,
        &mut value_scroll,
        &mut value_zoom,
        &mut dirty,
    );
    // 帧3：drag（进入 band 内）→ 仍不应触发
    run_frame_value(
        &ctx,
        drag_event(end),
        rect,
        &mut value_scroll,
        &mut value_zoom,
        &mut dirty,
    );
    assert_eq!(
        value_scroll, 0.0,
        "band 外按下后划过 band 不应平移，实际 {value_scroll}"
    );
    assert_eq!(
        value_zoom, 2.0,
        "band 外按下后划过 band 不应缩放，实际 {value_zoom}"
    );
}

/// 跑一帧像素空间垂直滚动条（show_vertical），返回背景/边缘拖拽返回值。
fn run_frame_vertical(
    ctx: &egui::Context,
    raw: egui::RawInput,
    rect: egui::Rect,
    scroll_y: &mut f32,
    cell_size: &mut f32,
    dirty: &mut bool,
) -> f32 {
    let mut out = 0.0f32;
    ctx.run_ui(raw, |ui| {
        out = show_vertical(
            ui,
            rect,
            100.0,
            scroll_y,
            cell_size,
            200.0,
            0.5,
            8.0,
            None,
            dirty,
            yinhe_types::Orientation::Horizontal,
        );
    })
    .textures_delta
    .clear();
    out
}

/// 背景拖拽（thumb 之外的 band 区域）→ 不返回 dx、不平移（与水平滚动条一致）。
/// 配置：total=200×2=400px，view=100px，thumb 占 [0, 25]，y=60 是背景区。
#[test]
fn vertical_background_drag_does_not_zoom() {
    let ctx = egui::Context::default();
    let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(16.0, 100.0));
    let mut scroll_y = 0.0f32;
    let mut cell_size = 2.0f32;
    let mut dirty = false;

    let start = egui::pos2(8.0, 60.0);
    let end = egui::pos2(28.0, 60.0);
    // 帧1：hover 注册 widget
    let _ = run_frame_vertical(
        &ctx,
        drag_event(start),
        rect,
        &mut scroll_y,
        &mut cell_size,
        &mut dirty,
    );
    // 帧2：press
    let _ = run_frame_vertical(
        &ctx,
        press_event(start),
        rect,
        &mut scroll_y,
        &mut cell_size,
        &mut dirty,
    );
    // 帧3：drag（水平移动 20px）→ 背景拖拽不返回 dx
    let dx = run_frame_vertical(
        &ctx,
        drag_event(end),
        rect,
        &mut scroll_y,
        &mut cell_size,
        &mut dirty,
    );
    assert_eq!(dx, 0.0, "背景拖拽不应返回 dx（不得触发缩放），实际 {dx}");
    assert_eq!(scroll_y, 0.0, "背景拖拽不应平移 scroll_y");
    assert_eq!(cell_size, 2.0, "背景拖拽不应缩放 cell_size");
}

/// 像素空间垂直滚动条：thumb 中间拖动 → 沿主轴平移 scroll_y，不改变 cell_size。
/// 配置：total=200×2=400px，view=100px → scale=0.25，下拖 30px = 平移 120px。
#[test]
fn vertical_thumb_drag_pans_scroll_y() {
    let ctx = egui::Context::default();
    let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(16.0, 100.0));
    let mut scroll_y = 0.0f32;
    let mut cell_size = 2.0f32;
    let mut dirty = false;

    // thumb 占 [0,25]，middle 区 [4,21]；按下 (8,12) 下拖到 (8,42)。
    let start = egui::pos2(8.0, 12.0);
    let end = egui::pos2(8.0, 42.0);
    let _ = run_frame_vertical(
        &ctx,
        drag_event(start),
        rect,
        &mut scroll_y,
        &mut cell_size,
        &mut dirty,
    );
    let _ = run_frame_vertical(
        &ctx,
        press_event(start),
        rect,
        &mut scroll_y,
        &mut cell_size,
        &mut dirty,
    );
    let dx = run_frame_vertical(
        &ctx,
        drag_event(end),
        rect,
        &mut scroll_y,
        &mut cell_size,
        &mut dirty,
    );

    assert_eq!(dx, 0.0, "纯主轴拖动不应返回副轴 dx，实际 {dx}");
    assert!(
        (scroll_y - 120.0).abs() < 0.5,
        "thumb 下拖 30px 应平移 scroll_y≈120，实际 {scroll_y}"
    );
    assert_eq!(cell_size, 2.0, "平移不应改变 cell_size");
}

/// 与 `run_frame_vertical` 相同，但可指定 cell 离散档位（测试吸附）。
fn run_frame_vertical_stepped(
    ctx: &egui::Context,
    raw: egui::RawInput,
    rect: egui::Rect,
    scroll_y: &mut f32,
    cell_size: &mut f32,
    cell_step: Option<f32>,
    dirty: &mut bool,
) -> f32 {
    let mut out = 0.0f32;
    ctx.run_ui(raw, |ui| {
        out = show_vertical(
            ui,
            rect,
            100.0,
            scroll_y,
            cell_size,
            200.0,
            0.5,
            8.0,
            cell_step,
            dirty,
            yinhe_types::Orientation::Horizontal,
        );
    })
    .textures_delta
    .clear();
    out
}

/// 像素空间垂直滚动条：拖终点边缩放 cell_size，并吸附到离散档位。
/// total=200×2=400px，view=100px，k=50；拖边使 thumb 长 47px → 原始 cell≈1.064，
/// 0.5 档位吸附到 1.0（锚定 thumb 起点 0，不产生滚动）。
#[test]
fn vertical_end_edge_drag_snaps_cell_size() {
    let ctx = egui::Context::default();
    let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(16.0, 100.0));
    let mut scroll_y = 0.0f32;
    let mut cell_size = 2.0f32;
    let mut dirty = false;
    let step = Some(0.5f32);

    // thumb 占 [0,25]，终点边 [21,25]；按下 (8,23) 下拖到 (8,45)。
    let start = egui::pos2(8.0, 23.0);
    let end = egui::pos2(8.0, 45.0);
    let _ = run_frame_vertical_stepped(
        &ctx,
        drag_event(start),
        rect,
        &mut scroll_y,
        &mut cell_size,
        step,
        &mut dirty,
    );
    let _ = run_frame_vertical_stepped(
        &ctx,
        press_event(start),
        rect,
        &mut scroll_y,
        &mut cell_size,
        step,
        &mut dirty,
    );
    let _ = run_frame_vertical_stepped(
        &ctx,
        drag_event(end),
        rect,
        &mut scroll_y,
        &mut cell_size,
        step,
        &mut dirty,
    );

    assert_eq!(
        cell_size, 1.0,
        "终点边下拖应缩放并吸附到 0.5 档位，实际 {cell_size}"
    );
    assert_eq!(
        scroll_y, 0.0,
        "锚定 thumb 起点应保持 scroll_y=0，实际 {scroll_y}"
    );
}

/// 值空间垂直滚动条：thumb 中间拖动 → 平移 value_scroll，不改变 value_zoom。
/// total=127，zoom=2 → visible=63.5，thumb 占 [100,200]；上拖 30px 使 scroll 增大。
#[test]
fn value_thumb_drag_scrolls_without_zoom() {
    let ctx = egui::Context::default();
    let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(24.0, 200.0));
    let mut value_scroll = 0.0f32;
    let mut value_zoom = 2.0f32;
    let mut dirty = false;

    // thumb [100,200]，middle [104,196]；按下 (12,150) 上拖到 (12,120)。
    let start = egui::pos2(12.0, 150.0);
    let end = egui::pos2(12.0, 120.0);
    run_frame_value(
        &ctx,
        drag_event(start),
        rect,
        &mut value_scroll,
        &mut value_zoom,
        &mut dirty,
    );
    run_frame_value(
        &ctx,
        press_event(start),
        rect,
        &mut value_scroll,
        &mut value_zoom,
        &mut dirty,
    );
    run_frame_value(
        &ctx,
        drag_event(end),
        rect,
        &mut value_scroll,
        &mut value_zoom,
        &mut dirty,
    );

    assert!(
        (value_scroll - 19.05).abs() < 0.1,
        "上拖 30px 应平移 value_scroll≈19.05（30/1.5748），实际 {value_scroll}"
    );
    assert_eq!(value_zoom, 2.0, "平移不应缩放 value_zoom");
}

/// 值空间垂直滚动条：拖顶边 → 缩放 value_zoom，锚定底边值。
/// 顶边上拖 22px：visible 63.5→77.47，zoom 2→1.639，底边值固定 0。
#[test]
fn value_top_edge_zoom_anchors_bottom() {
    let ctx = egui::Context::default();
    let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(24.0, 200.0));
    let mut value_scroll = 0.0f32;
    let mut value_zoom = 2.0f32;
    let mut dirty = false;

    // 顶边 [100,104]；按下 (12,102) 上拖到 (12,80)。
    let start = egui::pos2(12.0, 102.0);
    let end = egui::pos2(12.0, 80.0);
    run_frame_value(
        &ctx,
        drag_event(start),
        rect,
        &mut value_scroll,
        &mut value_zoom,
        &mut dirty,
    );
    run_frame_value(
        &ctx,
        press_event(start),
        rect,
        &mut value_scroll,
        &mut value_zoom,
        &mut dirty,
    );
    run_frame_value(
        &ctx,
        drag_event(end),
        rect,
        &mut value_scroll,
        &mut value_zoom,
        &mut dirty,
    );

    assert!(
        (value_zoom - 1.6393).abs() < 0.01,
        "顶边上拖应缩小 zoom 到 ≈1.639，实际 {value_zoom}"
    );
    assert_eq!(value_scroll, 0.0, "锚定底边时 value_scroll 应保持 0");
}

/// 垂直滚动条：鼠标在 band 外按下拖动（interact_radius 范围内）→ 不应平移/缩放。
/// 回归：拖动自动化锚点时鼠标靠近垂直滚动条 band。
#[test]
fn vertical_scrollbar_ignores_drag_outside_band() {
    let ctx = egui::Context::default();
    let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(16.0, 100.0));
    let mut scroll_y = 0.0f32;
    let mut cell_size = 2.0f32;
    let mut dirty = false;

    // 鼠标在 band 外（x=-3，距 band 左边缘 3px < interact_radius=5），y 在 thumb 中间
    let start = egui::pos2(-3.0, 12.0);
    let end = egui::pos2(-3.0, 42.0);
    // 帧1：hover 注册 widget
    let _ = run_frame_vertical(
        &ctx,
        drag_event(start),
        rect,
        &mut scroll_y,
        &mut cell_size,
        &mut dirty,
    );
    // 帧2：press（band 外）
    let _ = run_frame_vertical(
        &ctx,
        press_event(start),
        rect,
        &mut scroll_y,
        &mut cell_size,
        &mut dirty,
    );
    // 帧3：drag（垂直移动 30px）→ 不应平移、不应缩放
    let dx = run_frame_vertical(
        &ctx,
        drag_event(end),
        rect,
        &mut scroll_y,
        &mut cell_size,
        &mut dirty,
    );
    assert_eq!(
        scroll_y, 0.0,
        "band 外拖动不应平移 scroll_y，实际 {scroll_y}"
    );
    assert_eq!(
        cell_size, 2.0,
        "band 外拖动不应缩放 cell_size，实际 {cell_size}"
    );
    assert_eq!(dx, 0.0, "band 外拖动不应返回 dx，实际 {dx}");
}
