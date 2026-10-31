//! surface 级渲染资源与每帧渲染（PR 钢琴卷帘 / AR 工程走带）。
//!
//! 渲染线程每帧：
//! 1. 取场景快照（锁内先按视图 `clamp` 视口）；
//! 2. 按视图构建 uniforms / 轨道色 / 音符实例层（缓存键含视图标签，避免串层）；
//! 3. `InstanceRenderer::draw` + present（Fifo 自带节流）。

use std::collections::HashSet;
#[cfg(any(target_os = "android", target_env = "ohos"))]
use std::ptr::NonNull;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[cfg(any(target_os = "android", target_env = "ohos"))]
use wgpu::rwh::RawDisplayHandle;
#[cfg(target_os = "android")]
use wgpu::rwh::{AndroidDisplayHandle, AndroidNdkWindowHandle, RawWindowHandle};
#[cfg(target_env = "ohos")]
use wgpu::rwh::{OhosDisplayHandle, OhosNdkWindowHandle, RawWindowHandle};

use yinhe_core::{Selection, TrackKind, YinModel};
use yinhe_types::{
    ArrangementView, AutomationLane, AutomationPanelView, AutomationTarget, PianoRollView,
    TimelineViewBase,
};
use yinhe_wgpu::pianoroll::build_render_job;
use yinhe_wgpu::vertex::NoteInstance;
use yinhe_wgpu::{AutomationGhost, InstanceRenderer};

use crate::Gpu;
use crate::OwnedWindow;
use crate::scene::ViewMode;
use crate::theme_colors as tc;

/// 音符层缓存键里的视图标签（防止 PR/AR 切换时缓存串层）。
const VIEW_TAG_PIANOROLL: u64 = 1;
const VIEW_TAG_ARRANGEMENT: u64 = 2;

/// 呈现端：原生窗口 surface（Android/OpenHarmony）或 Flutter 外部纹理（iOS）。
enum Sink {
    Surface {
        /// 注意 drop 顺序：`surface` 必须在 `window` 之前销毁
        /// （ANativeWindow 需存活到 surface 释放之后；Rust 按声明顺序 drop）。
        surface: wgpu::Surface<'static>,
        config: wgpu::SurfaceConfiguration,
        window: OwnedWindow,
    },
    /// iOS：CVPixelBuffer 的槽位由 Swift 宿主管理（见 `ios.rs`）。
    #[cfg(target_os = "ios")]
    External(crate::ios::IosPresenter),
}

/// 一帧的渲染视图，以及画完后如何交还给系统。
struct Frame {
    view: wgpu::TextureView,
    /// surface 帧：画完必须 present；外部纹理为 None。
    surface: Option<wgpu::SurfaceTexture>,
    /// 外部纹理槽位（iOS 专用）。
    #[cfg(target_os = "ios")]
    slot: i32,
}

/// 用生产端原生窗口建 wgpu surface。
///
/// Android：`ANativeWindow`（SurfaceProducer 的 Surface）；
/// OpenHarmony：`OHNativeWindow`（Flutter-OH 外接纹理的 native window），
/// wgpu 的 GLES 后端已实现 `Rwh::OhosNdk`，Vulkan 暂不支持该 surface。
fn create_raw_surface(gpu: &Gpu, window: &OwnedWindow) -> Option<wgpu::Surface<'static>> {
    // 桌面只为 `cargo clippy --workspace` 能编译（本 crate 只产出移动端库）。
    #[cfg(not(any(target_os = "android", target_env = "ohos")))]
    {
        let _ = (gpu, window);
        None
    }
    #[cfg(any(target_os = "android", target_env = "ohos"))]
    {
        let native = NonNull::new(window.0)?;
        #[cfg(target_os = "android")]
        let (display, handle) = (
            RawDisplayHandle::Android(AndroidDisplayHandle::new()),
            RawWindowHandle::AndroidNdk(AndroidNdkWindowHandle::new(native)),
        );
        #[cfg(target_env = "ohos")]
        let (display, handle) = (
            RawDisplayHandle::Ohos(OhosDisplayHandle::new()),
            RawWindowHandle::OhosNdk(OhosNdkWindowHandle::new(native)),
        );
        // SAFETY: `window` 由 Target 持有，保证存活到 surface 销毁之后。
        match unsafe {
            gpu.instance
                .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                    raw_display_handle: Some(display),
                    raw_window_handle: handle,
                })
        } {
            Ok(surface) => Some(surface),
            Err(e) => {
                log::error!("创建 surface 失败: {e}");
                None
            }
        }
    }
}

impl Sink {
    /// 用生产端原生窗口建 surface 呈现端。
    ///
    /// `prefer_alpha` = AM 面板：优先 premultiplied 混合模式（Flutter 按预乘
    /// alpha 合成外接纹理）。
    fn surface(gpu: &Gpu, window: OwnedWindow, prefer_alpha: bool) -> Option<Self> {
        let surface = create_raw_surface(gpu, &window)?;

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
        // Mailbox 不排队旧帧：新帧直接替换待呈现帧，降低 wgpu 层延迟；
        // 设备不支持时退回 Fifo，避免创建 surface 失败。
        let present_mode = if caps.present_modes.contains(&wgpu::PresentMode::Mailbox) {
            wgpu::PresentMode::Mailbox
        } else {
            log::warn!("设备不支持 Mailbox，回退 Fifo");
            wgpu::PresentMode::Fifo
        };
        let alpha_mode = if prefer_alpha {
            caps.alpha_modes
                .iter()
                .copied()
                .find(|m| {
                    matches!(
                        m,
                        wgpu::CompositeAlphaMode::PreMultiplied
                            | wgpu::CompositeAlphaMode::PostMultiplied
                            | wgpu::CompositeAlphaMode::Inherit
                    )
                })
                .or_else(|| caps.alpha_modes.first().copied())
                .unwrap_or(wgpu::CompositeAlphaMode::Auto)
        } else {
            caps.alpha_modes
                .first()
                .copied()
                .unwrap_or(wgpu::CompositeAlphaMode::Auto)
        };
        // OHOS 的 OwnedWindow 拿不到真实尺寸（XComponent 由 ArkTS 布局，尺寸走
        // nativeSetSize 上报）：用上报值建交换链，避免先建 1×1 再重配。
        #[cfg(target_env = "ohos")]
        let (init_w, init_h) = {
            let (w, h) = crate::pixel_size();
            (w.max(1), h.max(1))
        };
        #[cfg(not(target_env = "ohos"))]
        let (init_w, init_h) = (window.width(), window.height());
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width: init_w,
            height: init_h,
            present_mode,
            desired_maximum_frame_latency: 2,
            alpha_mode,
            view_formats: vec![],
        };
        surface.configure(&gpu.device, &config);
        log::info!(
            "surface 就绪: {}x{} {format:?} {present_mode:?}",
            config.width,
            config.height
        );
        Some(Self::Surface {
            surface,
            config,
            window,
        })
    }

    /// iOS：外部纹理呈现端（宏尺寸由 `setSize` 上报，槽位由宿主管）。
    #[cfg(target_os = "ios")]
    fn external(kind: i32) -> Self {
        log::info!("外部纹理呈现端就绪: kind={kind}");
        Self::External(crate::ios::IosPresenter::new(kind))
    }

    fn size(&self) -> (u32, u32) {
        match self {
            Self::Surface { config, .. } => (config.width, config.height),
            #[cfg(target_os = "ios")]
            Self::External(p) => p.size(),
        }
    }

    fn format(&self) -> wgpu::TextureFormat {
        match self {
            Self::Surface { config, .. } => config.format,
            // 视图按 sRGB 解释（与 Android 的 srgb surface 格式一致）。
            #[cfg(target_os = "ios")]
            Self::External(_) => wgpu::TextureFormat::Bgra8UnormSrgb,
        }
    }

    fn window_ptr(&self) -> Option<usize> {
        match self {
            Self::Surface { window, .. } => Some(window.0 as usize),
            #[cfg(target_os = "ios")]
            Self::External(_) => None,
        }
    }

    /// 重配交换链（外部纹理的尺寸由宿主池自行重建，这里只记录）。
    fn resize(&mut self, gpu: &Gpu, w: u32, h: u32) {
        match self {
            Self::Surface {
                surface, config, ..
            } => {
                config.width = w;
                config.height = h;
                surface.configure(&gpu.device, config);
                log::info!("交换链重配置: {w}x{h}");
            }
            #[cfg(target_os = "ios")]
            Self::External(p) => p.set_size(w, h),
        }
    }

    /// 取一帧的渲染视图。
    fn acquire(&mut self, gpu: &Gpu) -> Option<Frame> {
        match self {
            Self::Surface {
                surface, config, ..
            } => {
                let frame = match surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(f)
                    | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
                    wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                        surface.configure(&gpu.device, config);
                        return None;
                    }
                    wgpu::CurrentSurfaceTexture::Timeout
                    | wgpu::CurrentSurfaceTexture::Occluded => return None,
                    wgpu::CurrentSurfaceTexture::Validation => {
                        log::error!("获取帧纹理时发生校验错误");
                        return None;
                    }
                };
                let view = frame
                    .texture
                    .create_view(&wgpu::TextureViewDescriptor::default());
                Some(Frame {
                    view,
                    surface: Some(frame),
                    #[cfg(target_os = "ios")]
                    slot: -1,
                })
            }
            #[cfg(target_os = "ios")]
            Self::External(p) => {
                let (slot, view) = p.acquire(gpu)?;
                Some(Frame {
                    view,
                    surface: None,
                    slot,
                })
            }
        }
    }

    /// 交还这一帧：surface present / 通知宿主有新帧。
    fn present(&mut self, gpu: &Gpu, frame: Frame) {
        match self {
            Self::Surface { .. } => {
                if let Some(f) = frame.surface {
                    gpu.queue.present(f);
                }
            }
            #[cfg(target_os = "ios")]
            Self::External(p) => p.submit(frame.slot),
        }
    }
}

/// surface 级资源（随 `SetSurface`/`ClearSurface` 创建与销毁）。
pub struct Target {
    sink: Sink,
    renderer: InstanceRenderer,
    /// AR 音符层缓存键：与上次相同则跳过实例重建。
    notes_cache_key: u64,
    /// AR 音符实例缓冲。
    notes_buf: Vec<NoteInstance>,
    /// PR 可见窗口 + key 分桶缓存；滚动在 padding 内只改 uniform。
    pr_note_cache: yinhe_wgpu::NoteLayerCache,
    /// 上一次的文档身份，用于换文档时清掉力度条摘要。
    last_model_epoch: u64,
    fps_start: Instant,
    frames: u32,
}

impl Target {
    fn new(gpu: &Gpu, sink: Sink) -> Self {
        let format = sink.format();
        Self {
            sink,
            renderer: InstanceRenderer::new_with_caps(
                gpu.device.clone(),
                gpu.queue.clone(),
                format,
                gpu.caps,
            ),
            notes_cache_key: 0,
            notes_buf: Vec::new(),
            pr_note_cache: yinhe_wgpu::NoteLayerCache::new(VIEW_TAG_PIANOROLL),
            last_model_epoch: 0,
            fps_start: Instant::now(),
            frames: 0,
        }
    }

    /// 生产端窗口指针（用于忽略重复 SetSurface）。
    pub fn window_ptr(&self) -> Option<usize> {
        self.sink.window_ptr()
    }

    /// 供 AM renderer 共享的力度条 LOD 摘要。
    pub fn velocity_summary(&self) -> Option<Arc<yinhe_wgpu::VelocitySummary>> {
        self.renderer.velocity_summary()
    }
}

/// 用生产端原生窗口建立 wgpu surface 与渲染器。
pub fn create_target(gpu: &Gpu, window: OwnedWindow) -> Option<Target> {
    let sink = Sink::surface(gpu, window, false)?;
    Some(Target::new(gpu, sink))
}

/// iOS：外部纹理呈现端（主画布）。
#[cfg(target_os = "ios")]
pub fn create_external_target(gpu: &Gpu, kind: i32) -> Option<Target> {
    Some(Target::new(gpu, Sink::external(kind)))
}

/// 渲染一帧（由渲染线程循环调用；内部自会跳过无效帧）。
pub fn render_frame(gpu: &Gpu, t: &mut Target) {
    let (w, h) = crate::logical_size();
    if w < 1.0 || h < 1.0 {
        return;
    }
    // resize：只重配置交换链（对同一 ANativeWindow 重建 surface 会触发
    // VK_ERROR_NATIVE_WINDOW_IN_USE_KHR；iOS 则是宿主重建纹理池）。
    let pw = crate::pixel_size().0.max(1);
    let ph = crate::pixel_size().1.max(1);
    if t.sink.size() != (pw, ph) {
        t.sink.resize(gpu, pw, ph);
    }
    let (fw, fh) = t.sink.size();

    // 播放位置：先取一次（audio 内部会短暂锁 scene），再进场景锁，
    // 避免在持有场景锁时反向等待音频锁。同一 tick 同时用于跟随滚动和
    // 播放头，保证两者落在同一帧的坐标系里。
    let playing_tick = if crate::audio::is_playing() {
        Some(crate::audio::playback_tick().max(0.0))
    } else {
        None
    };
    let follow_tick = if crate::audio::follow_playhead() {
        playing_tick
    } else {
        None
    };

    // ── 场景快照（锁内按视图 clamp + 取只读数据）──
    let snapshot = {
        let Ok(mut s) = crate::scene::scene().lock() else {
            return;
        };
        match follow_tick {
            Some(tick) => crate::scene::apply_follow(&mut s, w, h, tick),
            None => {
                s.follow_paused = false;
                s.follow_out_since = None;
                s.follow_last_at = None;
                s.follow_anim_until = None;
            }
        }
        // 预览缓动每帧推进；选框视觉位置变化时通知 Flutter 重取 overlay。
        if s.tick_preview_anim() {
            s.overlay_rev = s.overlay_rev.wrapping_add(1);
        }
        let total_ticks = s.total_ticks;
        let num_tracks = s.num_tracks;
        // 首次进入 PR：手机默认 32 键/屏，而不是桌面式的 64 键。
        let first_pr_frame = s.pr_view.viewport_h == 0.0;
        match s.view_mode {
            ViewMode::Pianoroll => {
                s.pr_view.clamp_scroll(w, h, total_ticks);
                if first_pr_frame {
                    let cross = s.pr_view.cross_axis_len(w, h);
                    s.pr_view
                        .set_initial_cross_zoom(cross, crate::scene::INITIAL_VISIBLE_KEYS);
                }
            }
            ViewMode::Arrangement => {
                // +1 = AR 列表末尾的"添加音轨"占位行，允许滚动到它。
                s.arr_view.clamp_scroll(w, h, total_ticks, num_tracks + 1)
            }
        }
        // 播放头与刚 clamp 完的视口使用同一个 tick，Dart 读取时不再重算。
        let playhead_tick = playing_tick.unwrap_or(s.cursor_tick);
        s.last_playhead_x = if s.model.is_some() {
            match s.view_mode {
                ViewMode::Pianoroll => s.pr_view.tick_to_x(playhead_tick),
                ViewMode::Arrangement => s.arr_view.base.tick_to_x(playhead_tick),
            }
        } else {
            -1.0
        };
        (
            s.view_mode,
            s.model.clone(),
            s.pr_view.clone(),
            s.arr_view.clone(),
            s.track_colors.clone(),
            s.track_visible.clone(),
            s.pr_track_visible.clone(),
            s.num_tracks,
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
            s.model_epoch,
            s.preview.clone(),
            (s.ghost_dx, s.ghost_dy),
        )
    };
    let (
        view_mode,
        model,
        pr_view,
        arr_view,
        track_colors,
        track_visible,
        pr_track_visible,
        num_tracks,
        display_track,
        selection,
        track_selected,
        revision,
        model_epoch,
        preview,
        preview_offset,
    ) = snapshot;

    // 文档切换：旧摘要不能用于新文档，先清空再预热。
    if model_epoch != t.last_model_epoch {
        t.last_model_epoch = model_epoch;
        t.renderer.clear_velocity_summary();
    }

    // AM 力度条摘要：不等 AM 展开就后台预热；load 完成后第一帧即开始构建，
    // 首次打开 AM 时通常已经就绪，避免先走一帧昂贵的原始力度条。
    if let Some(model) = model.as_ref() {
        let visible = effective_visible(&pr_track_visible, display_track);
        let tv_hash = yinhe_wgpu::hash_bools(&visible);
        t.renderer
            .poll_velocity_summary(Some(model), &visible, revision ^ tv_hash.rotate_left(32));
    }

    // ── 取帧纹理 ──
    let Some(frame) = t.sink.acquire(gpu) else {
        return;
    };
    let target_view = frame.view.clone();
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
                &pr_track_visible,
                display_track,
                &selection,
                &track_selected,
                model_epoch,
                w,
                h,
                &preview,
                preview_offset,
            ),
            ViewMode::Arrangement => upload_arrangement(
                t,
                model,
                &arr_view,
                &track_colors,
                &track_visible,
                // AR 不参与 PR 的「显示轨道」过滤：始终按 AR 自身显隐绘制。
                None,
                num_tracks,
                revision,
                model_epoch,
                w,
                h,
                &preview,
                preview_offset,
            ),
        }
        t.renderer.draw(&mut encoder, &target_view, fw, fh);
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
    t.sink.present(gpu, frame);

    // ── FPS 统计（每秒一条）──
    t.frames += 1;
    if t.fps_start.elapsed() >= Duration::from_secs(1) {
        log::info!("画布 FPS: {}（{}x{}）", t.frames, fw, fh);
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

/// AR 折叠自动化 lane：只收集可见轨道行的非 conductor lane。
fn folded_automation_lanes<'a>(
    model: &'a YinModel,
    view: &ArrangementView,
    visible: &[bool],
    height: f32,
    auto_ghost: &'a [crate::scene::AutoGhostLane],
) -> Vec<yinhe_wgpu::ArrAutomationLane<'a>> {
    let num_tracks = model.tracks.len();
    if num_tracks == 0 {
        return Vec::new();
    }
    let (first, last) = ArrangementView::visible_track_range_static(
        view.base.scroll_y,
        height,
        view.lane_height(),
        num_tracks,
    );
    let lane_h = view.lane_height();
    let mut lanes = Vec::new();
    for i in first..last.min(num_tracks) {
        let track = &model.tracks[i];
        if !visible.get(i).copied().unwrap_or(true) {
            continue;
        }
        if track.kind == TrackKind::Master {
            // Conductor 行：只显示 Tempo 曲线，值域 = 当前最大 Tempo（满高）。
            let lane = &model.conductor.tempo;
            if !lane.events.is_empty() {
                lanes.push(yinhe_wgpu::ArrAutomationLane {
                    lane,
                    y_top: i as f32 * lane_h - view.base.scroll_y,
                    height: lane_h,
                    max_val: crate::scene::tempo_max_bpm(model),
                    highlight_ticks: &[],
                    skip_ticks: &[],
                    show_anchors: false,
                    translucent: true,
                });
            }
            continue;
        }
        for lane in &track.automation_lanes {
            let max_val = if lane.target == AutomationTarget::Tempo {
                lane.events
                    .iter()
                    .map(|e| e.value)
                    .fold(0.0_f32, f32::max)
                    .max(1.0)
            } else {
                1.0
            };
            // 拖动时隐藏源位置被选中的竖线，由 layer 4 的 ghost 画在目标位置。
            let skip_ticks = auto_ghost
                .iter()
                .find(|g| g.src_track == i as u16 && g.target == lane.target)
                .map(|g| g.src_ticks.as_slice())
                .unwrap_or(&[]);
            lanes.push(yinhe_wgpu::ArrAutomationLane {
                lane,
                y_top: i as f32 * lane_h - view.base.scroll_y,
                height: lane_h,
                max_val,
                highlight_ticks: &[],
                skip_ticks,
                show_anchors: false,
                translucent: true,
            });
        }
    }
    lanes
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
    model_epoch: u64,
    w: f32,
    h: f32,
    preview: &crate::scene::DragPreview,
    preview_offset: (f32, f32),
) {
    let visible = effective_visible(track_visible, display_track);
    let mut job = build_render_job(w as u32, h as u32, view, selection, track_colors, 0.0, true);
    job.uniforms.ghost_dx = preview_offset.0;
    job.uniforms.ghost_dy = preview_offset.1;
    t.renderer.upload_uniforms(job.uniforms);
    t.renderer.upload_track_colors(&job.track_colors);
    t.renderer.upload_selection(&job.selection);

    let changed = t.pr_note_cache.update(&yinhe_wgpu::NoteCacheParams {
        midi: model,
        view,
        width: w,
        height: h,
        track_visible: &visible,
        track_selected,
        hidden_notes: preview.hidden.as_ref(),
        hidden_hash: preview.hidden_hash,
        selection: Some(selection),
        note_revisions: &model.note_revisions,
        lod_enabled: true,
        source_tag: model_epoch,
    });
    let key = t.pr_note_cache.cache_key();
    let _ = changed;
    t.renderer.ensure_layers(3);
    let cache = &t.pr_note_cache;
    t.renderer.upload_note_layer(1, key, |out| {
        out.extend_from_slice(cache.instances());
    });

    // ghost 层：独立强制更新键（preview.revision 每次 ghost 变化自增）。
    // 结束时即使 ghost 为空也会上传一次空层，清掉上一帧预览。
    let ghost_key = yinhe_wgpu::layer_cache_key(&[
        VIEW_TAG_PIANOROLL ^ 0xA5A5_5A5A,
        preview.revision,
        u64::from(preview.selected),
        model_epoch,
    ])
    .max(1);
    let theme = yinhe_theme::current_gpu_theme();
    t.renderer.upload_note_layer(2, ghost_key, |out| {
        for &(start, end, key, track) in preview.ghost.iter() {
            yinhe_wgpu::build_ghost_note(out, start, end, key, track, preview.selected, &theme);
        }
    });

    // PR 不显示 AR 折叠自动化；上传空 layer 3 清掉 AR 切过来时的残留。
    let am_key = yinhe_wgpu::layer_cache_key(&[
        VIEW_TAG_PIANOROLL ^ 0x51_51_51_51,
        u64::from(w.to_bits()),
        u64::from(h.to_bits()),
    ]);
    yinhe_wgpu::prepare_folded_automation(
        &mut t.renderer,
        w,
        h,
        &view.base,
        &[],
        &visible,
        track_colors,
        tc::BG,
        tc::STRIPE_BG,
        am_key,
    );
    // 清掉 AR 拖动留下的自动化竖线 ghost（layer 4）。
    yinhe_wgpu::prepare_folded_automation_ghost(&mut t.renderer, w, &view.base, &[], &[], &[]);
}

/// AM 面板专用 wgpu 目标（独立呈现端 + renderer）。
pub struct AmTarget {
    sink: Sink,
    renderer: InstanceRenderer,
}

impl AmTarget {
    fn new(gpu: &Gpu, sink: Sink) -> Self {
        let format = sink.format();
        Self {
            sink,
            renderer: InstanceRenderer::new_with_caps(
                gpu.device.clone(),
                gpu.queue.clone(),
                format,
                gpu.caps,
            ),
        }
    }

    pub fn window_ptr(&self) -> Option<usize> {
        self.sink.window_ptr()
    }
}

/// 用 AM 面（独立 Flutter 纹理）的原生窗口建立独立 surface 与 renderer。
pub fn create_automation_target(gpu: &Gpu, window: OwnedWindow) -> Option<AmTarget> {
    let sink = Sink::surface(gpu, window, true)?;
    Some(AmTarget::new(gpu, sink))
}

/// iOS：AM 面板的外部纹理呈现端。
#[cfg(target_os = "ios")]
pub fn create_external_automation_target(gpu: &Gpu, kind: i32) -> Option<AmTarget> {
    Some(AmTarget::new(gpu, Sink::external(kind)))
}

/// 渲染一帧 AM 面板：力度条/自动化曲线直接画进独立纹理。
///
/// 主 renderer 的力度条摘要由调用方传入（只读共享），AM renderer 不重建。
pub fn render_automation_frame(
    gpu: &Gpu,
    t: &mut AmTarget,
    summary: Option<Arc<yinhe_wgpu::VelocitySummary>>,
) {
    let (pw, ph) = match crate::automation_pixel_size() {
        (w, h) if w > 0 && h > 0 => (w, h),
        _ => t.sink.size(),
    };
    if t.sink.size() != (pw, ph) {
        t.sink.resize(gpu, pw, ph);
    }
    let (fw, fh) = t.sink.size();

    let snapshot = {
        let Ok(s) = crate::scene::scene().lock() else {
            return;
        };
        let state = s.automation_panel.clone();
        if !state.visible || state.w < 1.0 || state.h < 1.0 {
            return;
        }
        let Some(model) = s.model.clone() else {
            return;
        };
        let visible = {
            let raw = s
                .document
                .as_ref()
                .map(|d| d.edit.pr_visible())
                .unwrap_or_default();
            effective_visible(&raw, s.display_track)
        };
        let track_colors = s.track_colors.clone();
        let track_selected = s
            .document
            .as_ref()
            .map(|d| d.edit.track_selected.clone())
            .unwrap_or_default();
        let revision = s.document.as_ref().map(|d| d.data.revision).unwrap_or(0);
        let pr_view = s.pr_view.clone();
        (
            state,
            model,
            visible,
            track_colors,
            track_selected,
            revision,
            pr_view,
        )
    };
    let (state, model, visible, track_colors, track_selected, revision, pr_view) = snapshot;

    let is_velocity = state.lane.is_none();
    let is_conductor = model
        .tracks
        .get(state.active_track)
        .is_some_and(|t| t.kind == TrackKind::Master);
    // Conductor 的 AM 只有 Tempo（不在轨 lane 里，直接取 conductor.tempo）。
    let tempo_lane = if is_conductor {
        Some(model.conductor.tempo.clone())
    } else {
        None
    };
    let lane = if is_conductor {
        if state.lane == Some(0) {
            tempo_lane.as_ref()
        } else {
            None
        }
    } else {
        state.lane.and_then(|idx| {
            model.tracks.get(state.active_track).and_then(|track| {
                track
                    .automation_lanes
                    .iter()
                    .filter(|l| crate::scene::lane_kind(&l.target).is_some())
                    .nth(idx)
            })
        })
    };
    let lanes: Vec<&AutomationLane> = lane.into_iter().collect();
    let value_max = if is_conductor {
        crate::scene::tempo_max_bpm(&model)
    } else {
        1.0
    };
    // Conductor 的拖动 ghost 也传归一化值，绘制前换算成 BPM。
    let ghost_moves: Vec<(u32, u32, f32)> = if is_conductor {
        state
            .ghost
            .iter()
            .map(|&(old, new, value)| (old, new, value * value_max))
            .collect()
    } else {
        state.ghost.clone()
    };
    let ghost = lanes.first().and_then(|lane| {
        if ghost_moves.is_empty() {
            return None;
        }
        let color = track_colors
            .get(state.active_track)
            .map(|c| [c[0], c[1], c[2]])
            .unwrap_or([0.5, 0.5, 0.5]);
        Some(AutomationGhost::Move {
            lane: yinhe_wgpu::build_lane_multi_move(lane, &ghost_moves),
            color,
        })
    });

    let view = AutomationPanelView {
        base: TimelineViewBase {
            pixels_per_tick: pr_view.base.pixels_per_tick,
            scroll_x: pr_view.base.scroll_x,
            scroll_y: 0.0,
            // 纹理从 combo 列右侧开始，局部 x=0 就是网格左缘。
            left_panel_width: 0.0,
            dirty: false,
            track_panel_row_height: 0.0,
            track_panel_scroll_y: 0.0,
            follow_target: None,
            follow_anim_start: 0.0,
            follow_anim_elapsed: 0.0,
        },
        panel_height: state.h,
        selected_target: lanes
            .first()
            .map(|l| l.target.clone())
            .unwrap_or(AutomationTarget::CC { controller: 7 }),
        show_velocity: is_velocity,
        ..Default::default()
    };

    let Some(frame) = t.sink.acquire(gpu) else {
        return;
    };
    let target_view = frame.view.clone();
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("yinhe_automation_frame"),
        });

    t.renderer.set_velocity_summary_shared(summary);

    if !is_velocity && lanes.is_empty() {
        t.renderer.upload_curve_layer(1, 0, |_| {});
        t.renderer.upload_curve_layer(2, 0, |_| {});
    } else {
        yinhe_wgpu::prepare_automation(
            &mut t.renderer,
            1,
            state.w as u32,
            state.h as u32,
            &view,
            &lanes,
            Some(model.as_ref()),
            &visible,
            &track_selected,
            &track_colors,
            1.0,
            state.show_anchors,
            if is_velocity { 127.0 } else { value_max },
            ghost,
            revision,
            &[],
        );
    }

    t.renderer.draw(&mut encoder, &target_view, fw, fh);
    gpu.queue.submit([encoder.finish()]);
    t.sink.present(gpu, frame);
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
    model_epoch: u64,
    w: f32,
    h: f32,
    preview: &crate::scene::DragPreview,
    preview_offset: (f32, f32),
) {
    let num_tracks = num_tracks.min(yinhe_wgpu::MAX_TRACKS);
    if num_tracks == 0 {
        return;
    }
    let visible = effective_visible(track_visible, display_track);
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
        ghost_dx: preview_offset.0,
        ghost_dy: preview_offset.1,
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
    key ^= model_epoch.wrapping_mul(0xD6E8_FEB8_6659_FD93);
    key ^= model.note_count.rotate_left(11);
    key ^= preview.hidden_hash.wrapping_mul(0x94D0_49BB_1331_11EB);
    for (i, vis) in visible.iter().enumerate() {
        if *vis {
            key ^= 0x9E37_79B9_7F4A_7C15u64.wrapping_mul(i as u64 + 1);
        }
    }
    let key = yinhe_wgpu::layer_cache_key(&[key, VIEW_TAG_ARRANGEMENT]);
    if key != t.notes_cache_key {
        t.notes_buf.clear();
        // 只构建当前可见轨道行：undo/Riff 后 AR 层重建不再扫全工程所有轨。
        let (trk_first, trk_last) = ArrangementView::visible_track_range_static(
            view.base.scroll_y,
            h,
            view.lane_height(),
            num_tracks,
        );
        yinhe_wgpu::build_arr_notes(
            &mut t.notes_buf,
            w,
            model,
            view,
            (trk_first, trk_last),
            &visible,
            preview.hidden.as_ref(),
        );
        t.notes_cache_key = key;
    }
    t.renderer.ensure_layers(3);
    let notes_buf = &t.notes_buf;
    t.renderer
        .upload_note_layer(1, key, |out| out.extend_from_slice(notes_buf));
    // AR ghost：与 PR 共用 DragPreview；无 ghost 时上传空层清掉上一帧。
    let ghost_key = yinhe_wgpu::layer_cache_key(&[
        VIEW_TAG_ARRANGEMENT ^ 0xA5A5_5A5A,
        preview.revision,
        model_epoch,
    ])
    .max(1);
    let mut ghost = preview.ghost.as_ref().clone();
    t.renderer.upload_note_layer(2, ghost_key, |out| {
        yinhe_wgpu::build_ghost_notes(
            out,
            ghost.as_mut_slice(),
            w,
            view,
            (0, num_tracks),
            &visible,
        );
        for inst in out.iter_mut() {
            inst.packed |= yinhe_wgpu::NoteInstance::GHOST_BIT;
        }
    });

    // 折叠自动化竖线：只给可见轨道、只生成可见 tick 的事件，画在 layer 3。
    let lanes = folded_automation_lanes(model, view, &visible, h, preview.auto_ghost.as_slice());
    let mut am_key = view.render_hash();
    am_key ^= (w.to_bits() as u64) << 17;
    am_key ^= (h.to_bits() as u64) << 29;
    am_key ^= revision.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    am_key ^= model_epoch.wrapping_mul(0xD6E8_FEB8_6659_FD93);
    yinhe_wgpu::prepare_folded_automation(
        &mut t.renderer,
        w,
        h,
        &view.base,
        &lanes,
        &visible,
        track_colors,
        tc::BG,
        tc::STRIPE_BG,
        yinhe_wgpu::layer_cache_key(&[am_key, VIEW_TAG_ARRANGEMENT]),
    );

    // 拖动 ghost：选中自动化锚点按拖动量移动后的竖线（layer 4）。
    let mut ghost_owners: Vec<AutomationLane> = Vec::with_capacity(preview.auto_ghost.len());
    for g in preview.auto_ghost.iter() {
        ghost_owners.push(AutomationLane {
            target: g.target.clone(),
            track: g.track,
            events: g.events.clone(),
        });
    }
    let mut ghost_lanes: Vec<yinhe_wgpu::ArrAutomationLane<'_>> =
        Vec::with_capacity(ghost_owners.len());
    for (g, lane) in preview.auto_ghost.iter().zip(&ghost_owners) {
        ghost_lanes.push(yinhe_wgpu::ArrAutomationLane {
            lane,
            y_top: f32::from(g.track) * lane_h - view.base.scroll_y,
            height: lane_h,
            max_val: g.max_val,
            highlight_ticks: &[],
            skip_ticks: &[],
            show_anchors: false,
            translucent: true,
        });
    }
    yinhe_wgpu::prepare_folded_automation_ghost(
        &mut t.renderer,
        w,
        &view.base,
        &ghost_lanes,
        &visible,
        track_colors,
    );
}

// ── 装饰层（wgpu decor，画在音符下层，像素级对齐桌面视觉）──
