//! surface 级渲染资源与每帧渲染（钢琴卷帘接入 `yinhe-wgpu`）。
//!
//! 渲染线程每帧：
//! 1. 取场景快照（锁内先 `clamp_scroll` 收口视口）；
//! 2. `build_render_job` 构建 uniforms / 轨道色 / 选区并上传（内部去重）；
//! 3. 视口/尺寸/可见性/档位变化时才重建音符实例层（其余帧只上传缓存层）；
//! 4. `InstanceRenderer::draw` + present（Fifo 自带节流）。

use std::collections::HashSet;
use std::ptr::NonNull;
use std::time::{Duration, Instant};

use wgpu::rwh::{AndroidDisplayHandle, AndroidNdkWindowHandle, RawDisplayHandle, RawWindowHandle};

use yinhe_core::Selection;
use yinhe_wgpu::InstanceRenderer;
use yinhe_wgpu::pianoroll::{
    SUMMARY_BLOCK_TICKS, build_notes, build_render_job, build_summary_notes, select_summary_level,
};
use yinhe_wgpu::vertex::NoteInstance;

use crate::{Gpu, OwnedWindow};

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
    _window: OwnedWindow,
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
        _window: window,
    })
}

/// 渲染一帧（由渲染线程循环调用；内部自会跳过无效帧）。
pub fn render_frame(gpu: &Gpu, t: &mut Target) {
    let (w, h) = crate::logical_size();
    if w < 1.0 || h < 1.0 {
        return;
    }

    // ── 场景快照（锁内 clamp 视口 + 取只读数据）──
    let snapshot = {
        let Ok(mut s) = crate::scene::scene().lock() else {
            return;
        };
        let total_ticks = s.total_ticks;
        s.view.clamp_scroll(w, h, total_ticks);
        (
            s.model.clone(),
            s.view.clone(),
            s.track_colors.clone(),
            s.track_visible.clone(),
        )
    };
    let (model, view, track_colors, track_visible) = snapshot;

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
        // ── uniforms / 轨道色 / 选区（内部按值去重）──
        let job = build_render_job(
            w as u32,
            h as u32,
            &view,
            &Selection::default(),
            &track_colors,
            0.0,
            true,
        );
        t.renderer.upload_uniforms(job.uniforms);
        t.renderer.upload_track_colors(&job.track_colors);
        t.renderer.upload_selection(&job.selection);

        // ── 音符实例层：视口/尺寸/可见性/档位变化时才重建 ──
        let lod = select_summary_level(view.base.pixels_per_tick);
        let mut key = view.render_hash();
        key ^= (w.to_bits() as u64) << 17;
        key ^= (h.to_bits() as u64) << 29;
        key ^= (lod.map_or(0u64, |l| l as u64 + 1)) << 41;
        for (i, vis) in track_visible.iter().enumerate() {
            if *vis {
                key ^= 0x9E37_79B9_7F4A_7C15u64.wrapping_mul(i as u64 + 1);
            }
        }
        if key != t.notes_cache_key {
            t.notes_buf.clear();
            let hidden = HashSet::new();
            match lod {
                Some(level) => build_summary_notes(
                    &mut t.notes_buf,
                    w,
                    h,
                    model.as_ref(),
                    &view,
                    &hidden,
                    &track_visible,
                    SUMMARY_BLOCK_TICKS[level],
                ),
                None => {
                    let track_sel = HashSet::new();
                    build_notes(
                        &mut t.notes_buf,
                        w,
                        h,
                        model.as_ref(),
                        &view,
                        &hidden,
                        &track_visible,
                        &track_sel,
                        None,
                    );
                }
            }
            t.notes_cache_key = key;
        }
        t.renderer.ensure_layers(1);
        let notes_buf = &t.notes_buf;
        t.renderer
            .upload_note_layer(0, key, |out| out.extend_from_slice(notes_buf));

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
                        r: 0.0,
                        g: 0.0,
                        b: 0.0,
                        a: 0.0,
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
