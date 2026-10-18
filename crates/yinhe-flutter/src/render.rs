//! surface 级渲染资源与每帧渲染（PR 钢琴卷帘 / AR 工程走带）。
//!
//! 渲染线程每帧：
//! 1. 取场景快照（锁内先按视图 `clamp` 视口）；
//! 2. 按视图构建 uniforms / 轨道色 / 音符实例层（缓存键含视图标签，避免串层）；
//! 3. `InstanceRenderer::draw` + present（Fifo 自带节流）。

use std::collections::HashSet;
use std::ptr::NonNull;
use std::time::{Duration, Instant};

use wgpu::rwh::{AndroidDisplayHandle, AndroidNdkWindowHandle, RawDisplayHandle, RawWindowHandle};

use yinhe_core::{Selection, YinModel};
use yinhe_types::{ArrangementView, PianoRollView};
use yinhe_wgpu::InstanceRenderer;
use yinhe_wgpu::pianoroll::{
    SUMMARY_BLOCK_TICKS, build_notes, build_render_job, build_summary_notes, select_summary_level,
};
use yinhe_wgpu::vertex::{DrawInstance, NoteInstance};

use crate::Gpu;
use crate::OwnedWindow;
use crate::scene::ViewMode;
use crate::theme_colors as tc;

/// 音符层缓存键里的视图标签（防止 PR/AR 切换时缓存串层）。
const VIEW_TAG_PIANOROLL: u64 = 1;
const VIEW_TAG_ARRANGEMENT: u64 = 2;

/// surface 级资源（随 `SetSurface`/`ClearSurface` 创建与销毁）。
pub struct Target {
    /// 注意 drop 顺序：`surface` 必须在 `_window` 之前销毁
    /// （ANativeWindow 需存活到 surface 释放之后；Rust 按声明顺序 drop）。
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    renderer: InstanceRenderer,
    /// 音符层缓存键：与上次相同则跳过实例重建。
    notes_cache_key: u64,
    notes_buf: Vec<NoteInstance>,
    fps_start: Instant,
    frames: u32,
    /// 仅为保活持有（尺寸在创建时已读取）。
    window: OwnedWindow,
}

impl Target {
    /// 生产端窗口指针（用于忽略重复 SetSurface）。
    pub fn window_ptr(&self) -> usize {
        self.window.0 as usize
    }
}

/// 用生产端 `ANativeWindow` 建立 wgpu surface 与渲染器。
pub fn create_target(gpu: &Gpu, window: OwnedWindow) -> Option<Target> {
    let handle = AndroidNdkWindowHandle::new(NonNull::new(window.0)?);
    let surface = match unsafe {
        // 安全性：`window`（ANativeWindow 引用）由本结构持有并保证存活到
        // surface 销毁之后；DisplayHandle 是静态单例语义。
        gpu.instance
            .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: Some(RawDisplayHandle::Android(AndroidDisplayHandle::new())),
                raw_window_handle: RawWindowHandle::AndroidNdk(handle),
            })
    } {
        Ok(s) => s,
        Err(e) => {
            log::error!("创建 surface 失败: {e}");
            return None;
        }
    };

    let caps = surface.get_capabilities(&gpu.adapter);
    let Some(format) = caps
        .formats
        .iter()
        .copied()
        .find(|f| f.is_srgb())
        .or_else(|| caps.formats.first().copied())
    else {
        log::error!("surface 无可用格式");
        return None;
    };
    let config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format,
        color_space: wgpu::SurfaceColorSpace::Auto,
        width: window.width(),
        height: window.height(),
        present_mode: wgpu::PresentMode::Fifo,
        desired_maximum_frame_latency: 2,
        alpha_mode: caps
            .alpha_modes
            .first()
            .copied()
            .unwrap_or(wgpu::CompositeAlphaMode::Auto),
        view_formats: vec![],
    };
    surface.configure(&gpu.device, &config);

    let renderer = InstanceRenderer::new(gpu.device.clone(), gpu.queue.clone(), format);
    log::info!(
        "surface 就绪: {}x{} {format:?}",
        config.width,
        config.height
    );
    Some(Target {
        surface,
        config,
        renderer,
        notes_cache_key: 0,
        notes_buf: Vec::new(),
        fps_start: Instant::now(),
        frames: 0,
        window,
    })
}

/// 渲染一帧（由渲染线程循环调用；内部自会跳过无效帧）。
pub fn render_frame(gpu: &Gpu, t: &mut Target) {
    let (w, h) = crate::logical_size();
    if w < 1.0 || h < 1.0 {
        return;
    }
    // resize：只重配置交换链（对同一 ANativeWindow 重建 surface 会触发
    // VK_ERROR_NATIVE_WINDOW_IN_USE_KHR）。
    let pw = crate::pixel_size().0.max(1);
    let ph = crate::pixel_size().1.max(1);
    if t.config.width != pw || t.config.height != ph {
        t.config.width = pw;
        t.config.height = ph;
        t.surface.configure(&gpu.device, &t.config);
        log::info!("交换链重配置: {pw}x{ph}");
    }

    // ── 场景快照（锁内按视图 clamp + 取只读数据）──
    let snapshot = {
        let Ok(mut s) = crate::scene::scene().lock() else {
            return;
        };
        let total_ticks = s.total_ticks;
        let num_tracks = s.num_tracks;
        match s.view_mode {
            ViewMode::Pianoroll => s.pr_view.clamp_scroll(w, h, total_ticks),
            ViewMode::Arrangement => s.arr_view.clamp_scroll(w, h, total_ticks, num_tracks),
        }
        (
            s.view_mode,
            s.model.clone(),
            s.pr_view.clone(),
            s.arr_view.clone(),
            s.track_colors.clone(),
            s.track_visible.clone(),
            s.num_tracks,
            s.ppq,
            s.display_track,
            s.document
                .as_ref()
                .map(|d| d.edit.selected.clone())
                .unwrap_or_default(),
            s.document
                .as_ref()
                .map(|d| d.edit.track_selected.clone())
                .unwrap_or_default(),
            s.document.as_ref().map(|d| d.data.revision).unwrap_or(0),
        )
    };
    let (
        view_mode,
        model,
        pr_view,
        arr_view,
        track_colors,
        track_visible,
        num_tracks,
        ppq,
        display_track,
        selection,
        track_selected,
        revision,
    ) = snapshot;

    // ── 取帧纹理 ──
    let frame = match t.surface.get_current_texture() {
        wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
        wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
            t.surface.configure(&gpu.device, &t.config);
            return;
        }
        wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => return,
        wgpu::CurrentSurfaceTexture::Validation => {
            log::error!("获取帧纹理时发生校验错误");
            return;
        }
    };
    let target_view = frame
        .texture
        .create_view(&wgpu::TextureViewDescriptor::default());
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("yinhe_flutter_frame"),
        });

    if let Some(model) = &model {
        match view_mode {
            ViewMode::Pianoroll => upload_pianoroll(
                t,
                model,
                &pr_view,
                &track_colors,
                &track_visible,
                display_track,
                &selection,
                &track_selected,
                revision,
                ppq,
                w,
                h,
            ),
            ViewMode::Arrangement => upload_arrangement(
                t,
                model,
                &arr_view,
                &track_colors,
                &track_visible,
                display_track,
                num_tracks,
                revision,
                ppq,
                w,
                h,
            ),
        }
        t.renderer
            .draw(&mut encoder, &target_view, t.config.width, t.config.height);
    } else {
        // 无文档：透明清屏（画布底色由 Flutter 侧绘制）。
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("yinhe_flutter_clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target_view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 240.0 / 255.0,
                        g: 240.0 / 255.0,
                        b: 244.0 / 255.0,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    }

    gpu.queue.submit([encoder.finish()]);
    gpu.queue.present(frame);

    // ── FPS 统计（每秒一条）──
    t.frames += 1;
    if t.fps_start.elapsed() >= Duration::from_secs(1) {
        log::info!(
            "画布 FPS: {}（{}x{}）",
            t.frames,
            t.config.width,
            t.config.height
        );
        t.frames = 0;
        t.fps_start = Instant::now();
    }
}

/// “显示轨道”过滤后的可见掩码（None = 原样）。
fn effective_visible(track_visible: &[bool], display_track: Option<usize>) -> Vec<bool> {
    match display_track {
        None => track_visible.to_vec(),
        Some(t) => (0..track_visible.len())
            .map(|i| i == t && track_visible[i])
            .collect(),
    }
}

/// PR 视图：uniforms + 音符层（含 LOD 摘要）上传。
#[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
fn upload_pianoroll(
    t: &mut Target,
    model: &YinModel,
    view: &PianoRollView,
    track_colors: &[[f32; 4]],
    track_visible: &[bool],
    display_track: Option<usize>,
    selection: &Selection,
    track_selected: &HashSet<u16>,
    revision: u64,
    ppq: u32,
    w: f32,
    h: f32,
) {
    let visible = effective_visible(track_visible, display_track);
    upload_pianoroll_decor(t, view, ppq, w, h);
    let job = build_render_job(w as u32, h as u32, view, selection, track_colors, 0.0, true);
    t.renderer.upload_uniforms(job.uniforms);
    t.renderer.upload_track_colors(&job.track_colors);
    t.renderer.upload_selection(&job.selection);

    let lod = select_summary_level(view.base.pixels_per_tick);
    let mut key = view.render_hash();
    key ^= (w.to_bits() as u64) << 17;
    key ^= (h.to_bits() as u64) << 29;
    key ^= (lod.map_or(0u64, |l| l as u64 + 1)) << 41;
    key ^= selection.state_hash().rotate_left(7);
    // 文档修订号 + 音符数：编辑后（视图未变）也必须重建音符层。
    key ^= revision.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    key ^= model.note_count.rotate_left(11);
    key ^= model.note_count.rotate_left(11);
    for (i, vis) in visible.iter().enumerate() {
        if *vis {
            key ^= 0x9E37_79B9_7F4A_7C15u64.wrapping_mul(i as u64 + 1);
        }
    }
    for t_sel in track_selected {
        key ^= 0x2545_F491_4F6C_DD1Du64.wrapping_mul(u64::from(*t_sel) + 1);
    }
    let key = yinhe_wgpu::layer_cache_key(&[key, VIEW_TAG_PIANOROLL]);
    if key != t.notes_cache_key {
        t.notes_buf.clear();
        let hidden = HashSet::new();
        match lod {
            Some(level) => build_summary_notes(
                &mut t.notes_buf,
                w,
                h,
                model,
                view,
                &hidden,
                &visible,
                SUMMARY_BLOCK_TICKS[level],
            ),
            None => {
                build_notes(
                    &mut t.notes_buf,
                    w,
                    h,
                    model,
                    view,
                    &hidden,
                    &visible,
                    track_selected,
                    Some(selection),
                );
            }
        }
        log::info!(
            "PR 音符层重建: {} 实例 rev={} sel={} vis0={:?}",
            t.notes_buf.len(),
            revision,
            selection.len(),
            visible.first()
        );
        t.notes_cache_key = key;
    }
    t.renderer.ensure_layers(2);
    let notes_buf = &t.notes_buf;
    t.renderer
        .upload_note_layer(1, key, |out| out.extend_from_slice(notes_buf));
}

/// AR 视图：uniforms + 轨道布局（offsets/行高）+ 音符层上传。
#[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
fn upload_arrangement(
    t: &mut Target,
    model: &YinModel,
    view: &ArrangementView,
    track_colors: &[[f32; 4]],
    track_visible: &[bool],
    display_track: Option<usize>,
    num_tracks: usize,
    revision: u64,
    ppq: u32,
    w: f32,
    h: f32,
) {
    let num_tracks = num_tracks.min(yinhe_wgpu::MAX_TRACKS);
    if num_tracks == 0 {
        return;
    }
    let visible = effective_visible(track_visible, display_track);
    upload_arrangement_decor(t, view, track_colors, num_tracks, ppq, w, h);
    let lane_h = view.lane_height();
    let offsets: Vec<f32> = (0..num_tracks).map(|i| i as f32 * lane_h).collect();
    // ★ shader 用 row_heights 计算行高；不传会退化成 1px 高的音符。
    let heights: Vec<f32> = vec![lane_h; num_tracks];
    t.renderer.upload_track_offsets(&offsets);
    t.renderer.upload_row_heights(&heights);
    t.renderer.upload_track_colors(track_colors);

    let uniforms = yinhe_wgpu::Uniforms {
        width: w,
        height: h,
        scroll_x: view.base.scroll_x,
        scroll_y: view.base.scroll_y,
        pixels_per_tick: view.base.pixels_per_tick,
        key_height: 0.0, // AR 未使用
        keyboard_width: view.base.left_panel_width,
        mode: 2, // AR notes
        min_border_width: 0.0,
        track_count: num_tracks as u32,
        sel_rect_count: 0,
        note_outline: 0,
        lane_height: lane_h,
        value_zoom: 0.0,
        value_scroll: 0.0,
        orientation: 0,
        ..Default::default()
    };
    t.renderer.upload_uniforms(uniforms);
    // 空选区：防止从 PR 切过来时残留高亮。
    t.renderer
        .upload_selection(&yinhe_wgpu::vertex::SelectionUniform {
            rects: [[0; 4]; yinhe_wgpu::vertex::MAX_SEL_RECTS * 2],
        });

    let mut key = view.render_hash();
    key ^= (w.to_bits() as u64) << 17;
    key ^= (h.to_bits() as u64) << 29;
    key ^= revision.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    key ^= model.note_count.rotate_left(11);
    for (i, vis) in visible.iter().enumerate() {
        if *vis {
            key ^= 0x9E37_79B9_7F4A_7C15u64.wrapping_mul(i as u64 + 1);
        }
    }
    let key = yinhe_wgpu::layer_cache_key(&[key, VIEW_TAG_ARRANGEMENT]);
    if key != t.notes_cache_key {
        t.notes_buf.clear();
        let hidden = HashSet::new();
        // demo 阶段构建全部轨道（可见性裁剪留给后续优化）。
        yinhe_wgpu::build_arr_notes(
            &mut t.notes_buf,
            w,
            model,
            view,
            (0, num_tracks),
            &visible,
            &hidden,
        );
        t.notes_cache_key = key;
    }
    t.renderer.ensure_layers(2);
    let notes_buf = &t.notes_buf;
    t.renderer
        .upload_note_layer(1, key, |out| out.extend_from_slice(notes_buf));
}

// ── 装饰层（wgpu decor，画在音符下层，像素级对齐桌面视觉）──

/// PR 装饰：键盘列 + 时间轴网格。
fn upload_pianoroll_decor(t: &mut Target, view: &PianoRollView, ppq: u32, w: f32, h: f32) {
    let kb_w = view.keyboard_width();
    let kh = view.key_height;
    let mut items: Vec<DrawInstance> = Vec::with_capacity(560);
    // 底层背景（surface 清屏透明，统一在 decor 里铺浅色底）。
    items.push(DrawInstance::solid_rect(0.0, 0.0, w, h, tc::BG));
    if kh > 0.0 {
        for key in 0..=127u8 {
            let y = view.key_to_y(key);
            if y + kh < 0.0 || y > h {
                continue;
            }
            let black = yinhe_types::is_black_key(key);
            let color = if black { tc::KEY_BLACK } else { tc::KEY_WHITE };
            items.push(DrawInstance::solid_rect(0.0, y, kb_w, kh, color));
            // 内容区：黑键行整行铺深色条（与桌面 `bg.rs` 的 stripe_bg 一致）。
            if black {
                items.push(DrawInstance::solid_rect(
                    kb_w,
                    y,
                    w - kb_w,
                    kh,
                    tc::STRIPE_BG,
                ));
            }
        }
        // 八度线：每个 C 的键行顶部 1px（与桌面 `paint_octave_lines` 一致）。
        for key in (0..=127u8).step_by(12) {
            let y = view.key_to_y(key);
            if y + kh < 0.0 || y > h {
                continue;
            }
            items.push(DrawInstance::solid_rect(
                kb_w,
                y,
                w - kb_w,
                1.0,
                tc::LINE_FG,
            ));
        }
        // 键盘右缘分隔线。
        items.push(DrawInstance::solid_rect(
            kb_w - 1.0,
            0.0,
            1.0,
            h,
            tc::TEXT_PRIMARY,
        ));
    }
    push_time_grid(&mut items, &view.base, ppq, w, h, kb_w, true);
    // 缓存键：视图/尺寸/ppq 不变时跳过重建（播放静止时省 CPU）。
    let mut key = view.render_hash();
    key ^= (w.to_bits() as u64) << 17;
    key ^= (h.to_bits() as u64) << 29;
    key ^= u64::from(ppq).rotate_left(5);
    let key = yinhe_wgpu::layer_cache_key(&[key, VIEW_TAG_PIANOROLL ^ 0xD1CE]);
    t.renderer
        .upload_decor_layer(0, key, |out| out.extend_from_slice(&items));
}

/// AR 装饰：轨道行背景/色条 + 时间轴网格。
fn upload_arrangement_decor(
    t: &mut Target,
    view: &ArrangementView,
    track_colors: &[[f32; 4]],
    num_tracks: usize,
    ppq: u32,
    w: f32,
    h: f32,
) {
    let panel = view.base.left_panel_width;
    let lane_h = view.lane_height();
    let mut items: Vec<DrawInstance> = Vec::with_capacity(256);
    // 底层背景（surface 清屏透明，统一在 decor 里铺浅色底）。
    items.push(DrawInstance::solid_rect(0.0, 0.0, w, h, tc::BG));
    if lane_h > 0.0 {
        let (first, last) = yinhe_types::ArrangementView::visible_track_range_static(
            view.base.scroll_y,
            h,
            lane_h,
            num_tracks,
        );
        for idx in first..last {
            let y = yinhe_types::ArrangementView::lane_y_static(idx, view.base.scroll_y, lane_h);
            // 隔行深浅：偶数行整行（面板 + 内容区）铺条纹底（与桌面一致）。
            if idx % 2 == 0 {
                items.push(DrawInstance::solid_rect(0.0, y, w, lane_h, tc::STRIPE_BG));
            }
            // 色带由 Flutter 绘制（sRGB 直绘保持饱和；与桌面 egui badge 一致）。
        }
        items.push(DrawInstance::solid_rect(
            panel - 1.0,
            0.0,
            1.0,
            h,
            tc::TEXT_PRIMARY,
        ));
    }
    push_time_grid(&mut items, &view.base, ppq, w, h, panel, false);
    // 缓存键：视图/尺寸/ppq/轨道色不变时跳过重建。
    let mut key = view.render_hash();
    key ^= (w.to_bits() as u64) << 17;
    key ^= (h.to_bits() as u64) << 29;
    key ^= u64::from(ppq).rotate_left(5);
    for c in track_colors.iter().take(64) {
        for v in c {
            key ^= u64::from(v.to_bits()).rotate_left(9);
        }
    }
    let key = yinhe_wgpu::layer_cache_key(&[key, VIEW_TAG_ARRANGEMENT ^ 0xD1CE]);
    t.renderer
        .upload_decor_layer(0, key, |out| out.extend_from_slice(&items));
}

/// 时间轴网格线（与桌面 `paint_grid_lines` 同规则；默认 4/4 定拍）。
///
/// 级别：合并小节线 / 小节线 / 拍线 / 十六分线 / tick 线，
/// 由 ppu 与 `GRID_MIN_SPACING` 决定（与桌面一致）。
#[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
fn push_time_grid(
    items: &mut Vec<DrawInstance>,
    base: &yinhe_types::TimelineViewBase,
    ppq: u32,
    w: f32,
    h: f32,
    start_x: f32,
    with_sub_tick: bool,
) {
    let ppu = base.pixels_per_tick;
    if ppu <= 0.001 {
        return;
    }
    let tpb = ppq.max(1);
    let measure = tpb * 4;
    let measure_px = measure as f32 * ppu;
    let mut divisor = 1u32;
    while (measure_px * divisor as f32) < tc::GRID_MIN_SPACING && divisor < 16 {
        divisor *= 2;
    }
    let merged = divisor > 1;
    let show_beat = !merged;
    let show_sub = with_sub_tick && show_beat && (tpb as f32 * ppu) >= tc::GRID_MIN_SPACING;
    let show_tick = with_sub_tick && show_sub && ppu >= 10.0;
    let sub_ticks = (tpb / 4).max(1);
    let step: u32 = if show_tick {
        1
    } else if show_sub {
        sub_ticks
    } else if show_beat {
        tpb
    } else {
        (measure * divisor / 2).max(1)
    };
    let (tick_start, tick_end) = base.visible_tick_range(w);
    let mut tick = ((tick_start.max(0.0) as u32) / step) * step;
    while (tick as f64) <= tick_end {
        let x = base.tick_to_x(tick as f64);
        if x >= start_x {
            let color = if tick.is_multiple_of(measure) {
                tc::GRID_MEASURE
            } else if tick.is_multiple_of(tpb) {
                tc::GRID_BEAT
            } else if tick.is_multiple_of(sub_ticks) {
                tc::GRID_SUB
            } else {
                tc::GRID_TICK
            };
            items.push(DrawInstance::solid_rect(x, 0.0, 1.0, h, color));
        }
        tick = tick.saturating_add(step.max(1));
    }
}
