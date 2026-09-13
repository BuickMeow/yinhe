use eframe::egui;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

/// 全局 device-lost 标志：同一 device 只注册一次回调，所有 `RenderContext`
/// 共享同一个 `Arc<AtomicBool>`。
///
/// `wgpu::Device::set_device_lost_callback` 是替换式 API，若每个 RenderContext
/// 各注册一次，后创建者（如自动化面板）会覆盖先注册的回调，导致主视口再也
/// 检测不到设备丢失。这里用进程级 `OnceLock` 保证只注册一次、全员可见。
static DEVICE_LOST: OnceLock<Arc<AtomicBool>> = OnceLock::new();

/// 查询全局 device-lost 标志（尚未注册回调时恒为 false）。
pub(crate) fn device_lost_global() -> bool {
    DEVICE_LOST.get().is_some_and(|f| f.load(Ordering::Relaxed))
}

/// 取得（首次调用时创建并注册回调）共享的 device-lost 标志。
fn shared_device_lost(device: &wgpu::Device) -> Arc<AtomicBool> {
    DEVICE_LOST
        .get_or_init(|| {
            let flag = Arc::new(AtomicBool::new(false));
            let f = Arc::clone(&flag);
            device.set_device_lost_callback(move |reason, msg| {
                tracing::error!("wgpu device lost: {reason:?} — {msg}");
                f.store(true, Ordering::Relaxed);
            });
            flag
        })
        .clone()
}

/// Calculate the byte size of a 2D texture with the given format and dimensions.
fn texture_byte_size(format: wgpu::TextureFormat, width: u32, height: u32, samples: u32) -> u64 {
    let bpp = match format {
        wgpu::TextureFormat::R8Unorm
        | wgpu::TextureFormat::R8Snorm
        | wgpu::TextureFormat::R8Uint
        | wgpu::TextureFormat::R8Sint => 1,
        wgpu::TextureFormat::R16Uint
        | wgpu::TextureFormat::R16Sint
        | wgpu::TextureFormat::R16Float
        | wgpu::TextureFormat::Rg8Unorm
        | wgpu::TextureFormat::Rg8Snorm
        | wgpu::TextureFormat::Rg8Uint
        | wgpu::TextureFormat::Rg8Sint => 2,
        wgpu::TextureFormat::R32Uint
        | wgpu::TextureFormat::R32Sint
        | wgpu::TextureFormat::R32Float
        | wgpu::TextureFormat::Rg16Uint
        | wgpu::TextureFormat::Rg16Sint
        | wgpu::TextureFormat::Rg16Float
        | wgpu::TextureFormat::Rgba8Unorm
        | wgpu::TextureFormat::Rgba8UnormSrgb
        | wgpu::TextureFormat::Rgba8Snorm
        | wgpu::TextureFormat::Rgba8Uint
        | wgpu::TextureFormat::Rgba8Sint
        | wgpu::TextureFormat::Bgra8Unorm
        | wgpu::TextureFormat::Bgra8UnormSrgb => 4,
        wgpu::TextureFormat::Rg32Uint
        | wgpu::TextureFormat::Rg32Sint
        | wgpu::TextureFormat::Rg32Float
        | wgpu::TextureFormat::Rgba16Uint
        | wgpu::TextureFormat::Rgba16Sint
        | wgpu::TextureFormat::Rgba16Float => 8,
        wgpu::TextureFormat::Rgba32Uint
        | wgpu::TextureFormat::Rgba32Sint
        | wgpu::TextureFormat::Rgba32Float => 16,
        _ => 4, // conservative fallback
    };
    (width as u64)
        .saturating_mul(height as u64)
        .saturating_mul(bpp)
        .saturating_mul(samples.max(1) as u64)
}

/// Manages the wgpu device/queue shared with eframe, and an offscreen render target.
pub struct RenderContext {
    wgpu_state: Arc<eframe::egui_wgpu::RenderState>,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    texture_id: egui::TextureId,
    width: u32,
    height: u32,
    /// True when the offscreen texture has been recreated and needs a full
    /// GPU render pass before it can be displayed. Set by `recreate_target`
    /// and cleared by `paint()` when it performs a render.
    needs_render: bool,
    /// Set to true by the device-lost callback. When true, GPU operations are
    /// skipped to avoid blocking the main thread on a lost device.
    device_lost: Arc<AtomicBool>,
    /// Tracked GPU byte size for `texture` so it can be subtracted on recreate.
    texture_size_bytes: u64,
}

impl RenderContext {
    pub fn new(cc: &eframe::CreationContext<'_>, width: u32, height: u32) -> Self {
        let wgpu_state: Arc<eframe::egui_wgpu::RenderState> = cc
            .wgpu_render_state
            .clone()
            .expect("wgpu backend required")
            .into();
        let device = &wgpu_state.device;
        let format = wgpu_state.target_format;

        // wgpu 默认把未捕获错误当 fatal 直接 panic，而这个 panic 发生在
        // winit 的 macOS 事件回调（extern "C" 边界）里无法 unwind，整个进程
        // 会 abort。睡眠唤醒后 Metal device 失效时，eframe 内部
        // Surface::configure 的 "wait for GPU idle" 失败就会走这条路。
        // 注册自定义 handler 把错误降级为日志：configure 返回 () 不中断执行，
        // 下一帧 SurfaceError::Lost/Outdated 会由 eframe 自动 reconfigure 恢复。
        device.on_uncaptured_error(Arc::new(|err| {
            tracing::error!("wgpu uncaptured error: {err}");
        }));

        // 共享的 device-lost 回调（同一 device 只注册一次，见 `DEVICE_LOST`）。
        let device_lost = shared_device_lost(device);

        let (texture, view, texture_id, texture_size_bytes) = Self::create_target(
            device,
            &mut wgpu_state.renderer.write(),
            format,
            width,
            height,
        );

        Self {
            wgpu_state,
            texture,
            view,
            texture_id,
            width,
            height,
            needs_render: true, // fresh texture, needs first render
            device_lost,
            texture_size_bytes,
        }
    }

    /// Create a `RenderContext` from an existing `Arc<RenderState>`.
    ///
    /// This is used for automation panels where the device/queue/format are
    /// derived from an existing `RenderContext` rather than from a
    /// `CreationContext`.
    pub fn from_render_state(
        wgpu_state: Arc<eframe::egui_wgpu::RenderState>,
        width: u32,
        height: u32,
    ) -> Self {
        let device = &wgpu_state.device;
        let format = wgpu_state.target_format;

        // 共享的 device-lost 回调（同一 device 只注册一次，见 `DEVICE_LOST`）。
        let device_lost = shared_device_lost(device);

        let (texture, view, texture_id, texture_size_bytes) = Self::create_target(
            device,
            &mut wgpu_state.renderer.write(),
            format,
            width,
            height,
        );

        Self {
            wgpu_state,
            texture,
            view,
            texture_id,
            width,
            height,
            needs_render: true,
            device_lost,
            texture_size_bytes,
        }
    }

    /// Access the shared wgpu `RenderState` (for creating additional
    /// `RenderContext`s, e.g. for automation panels).
    pub fn wgpu_state(&self) -> &Arc<eframe::egui_wgpu::RenderState> {
        &self.wgpu_state
    }

    fn create_target(
        device: &wgpu::Device,
        egui_renderer: &mut eframe::egui_wgpu::Renderer,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> (wgpu::Texture, wgpu::TextureView, egui::TextureId, u64) {
        yinhe_memtrace::with_tag(yinhe_memtrace::AllocTag::Gpu, || {
            // GPU 纹理尺寸硬上限（如 8192）：超限 create_texture 校验失败会 panic。
            // 视口纹理超限时降级为上限尺寸（内容完整显示，仅分辨率降低）。
            let max_dim = device.limits().max_texture_dimension_2d;
            let width = width.min(max_dim).max(1);
            let height = height.min(max_dim).max(1);
            // Provide a linear (non-srgb) view format for backends that require it
            // when TEXTURE_BINDING is used with an sRGB format (e.g. Metal, Vulkan).
            // Without this, creating a shader resource view can fail and cause
            // device loss — especially on window maximize when the texture is large.
            let linear_format = match format {
                wgpu::TextureFormat::Bgra8UnormSrgb => Some(wgpu::TextureFormat::Bgra8Unorm),
                wgpu::TextureFormat::Rgba8UnormSrgb => Some(wgpu::TextureFormat::Rgba8Unorm),
                _ => None,
            };
            let view_formats: &[wgpu::TextureFormat] = if let Some(lf) = &linear_format {
                std::slice::from_ref(lf)
            } else {
                &[]
            };

            let size_bytes = texture_byte_size(format, width, height, 1);
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("pianoroll_preview"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats,
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let texture_id =
                egui_renderer.register_native_texture(device, &view, wgpu::FilterMode::Nearest);
            yinhe_memtrace::add_gpu_resource(size_bytes);
            (texture, view, texture_id, size_bytes)
        })
    }

    fn recreate_target(&mut self, width: u32, height: u32) {
        yinhe_memtrace::with_tag(yinhe_memtrace::AllocTag::Gpu, || {
            // Skip if the device was already lost — creating a new texture on a
            // lost device can panic or hang.
            if self.device_lost.load(Ordering::Relaxed) {
                return;
            }

            let format = self.wgpu_state.target_format;
            let device = &self.wgpu_state.device;
            let mut egui_renderer = self.wgpu_state.renderer.write();
            egui_renderer.free_texture(&self.texture_id);

            // The old texture is about to be dropped; subtract its tracked size
            // before the new one is created.
            yinhe_memtrace::sub_gpu_resource(self.texture_size_bytes);

            let (texture, view, texture_id, texture_size_bytes) =
                Self::create_target(device, &mut egui_renderer, format, width, height);
            self.texture = texture;
            self.view = view;
            self.texture_id = texture_id;
            self.texture_size_bytes = texture_size_bytes;
            self.width = width;
            self.height = height;
            self.needs_render = true; // fresh texture, must re-render
        });
    }

    /// Resize the offscreen texture if needed.
    ///
    /// 尺寸双向跟随：GUI 缩放（zoom_factor）或窗口 resize 改变 ppp 后，
    /// 视口像素尺寸可能变大也可能变小，必须双向重建。旧实现用
    /// `shrink_to_fit_on_next_size` 延迟缩小，但该 flag 从未被置位，
    /// 导致纹理只增不减——GUI 缩放调小后 uv_max < 1，内容被放大显示。
    pub fn ensure_size(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }

        // 与 create_target 相同的设备上限裁剪：否则超限尺寸会每帧触发 recreate。
        let max_dim = self.wgpu_state.device.limits().max_texture_dimension_2d;
        let width = width.min(max_dim).max(1);
        let height = height.min(max_dim).max(1);

        if width != self.width || height != self.height {
            self.recreate_target(width, height);
        }
    }

    /// 当前纹理的实际像素尺寸（可能因 GPU 尺寸上限被裁剪）。
    /// 渲染线程的目标视口必须用实际尺寸，否则视口超出纹理会裁剪内容。
    pub fn actual_size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn preview_texture_id(&self) -> egui::TextureId {
        self.texture_id
    }

    pub fn device(&self) -> &wgpu::Device {
        &self.wgpu_state.device
    }

    pub fn queue(&self) -> &wgpu::Queue {
        &self.wgpu_state.queue
    }

    pub fn target_format(&self) -> wgpu::TextureFormat {
        self.wgpu_state.target_format
    }

    /// Query the underlying Metal driver's current allocated size (macOS only).
    #[cfg(target_os = "macos")]
    pub fn metal_allocated_size(&self) -> Option<u64> {
        unsafe {
            self.device()
                .as_hal::<wgpu::hal::api::Metal>()
                .map(|hal_device| {
                    use objc2_metal::MTLDevice;
                    hal_device.raw_device().currentAllocatedSize() as u64
                })
        }
    }

    pub fn preview_view(&self) -> &wgpu::TextureView {
        &self.view
    }

    /// Render to the offscreen texture and paint it into egui.
    ///
    /// When `render_thread` is `Some`, GPU rendering is done asynchronously
    /// by the render thread and `paint()` only displays the latest texture.
    /// When `render_thread` is `None`, rendering happens synchronously on the
    /// calling thread (legacy mode).
    ///
    /// `content_changed`: caller signals whether CPU-side data (instances,
    /// uniforms, viewport) has changed since the last frame. If false AND the
    /// texture was already rendered (not freshly created), GPU work is skipped
    /// entirely — only `painter.image()` is called to display the existing
    /// texture. This avoids accumulating GPU command buffers when nothing moved.
    ///
    /// Returns early if the GPU device has been lost.
    #[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
    pub fn paint(
        &mut self,
        renderer: &mut yinhe_wgpu::InstanceRenderer,
        width: u32,
        height: u32,
        label: &str,
        painter: &egui::Painter,
        rect: egui::Rect,
        content_changed: bool,
    ) {
        // Guard against operations on a lost device.
        if self.device_lost.load(Ordering::Relaxed) {
            tracing::warn!("wgpu device lost — skipping paint '{}'", label);
            return;
        }

        let uv_max = egui::pos2(
            (width as f32 / self.width as f32).min(1.0),
            (height as f32 / self.height as f32).min(1.0),
        );

        let do_render = self.needs_render || content_changed;

        if do_render {
            let mut encoder = self
                .device()
                .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some(label) });
            // 视口用纹理实际尺寸（可能小于期望尺寸），避免渲染目标外裁剪
            renderer.draw(&mut encoder, self.preview_view(), self.width, self.height);
            self.queue().submit(std::iter::once(encoder.finish()));
            self.needs_render = false;
        }

        let texture_id = self.preview_texture_id();
        painter.image(
            texture_id,
            rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), uv_max),
            egui::Color32::WHITE,
        );
    }

    /// Paint the offscreen texture into egui **without** doing any GPU work.
    ///
    /// Used when a `RenderThreadHandle` is performing the GPU rendering
    /// asynchronously.  This method only displays the latest texture content.
    pub fn paint_texture_only(
        &self,
        width: u32,
        height: u32,
        painter: &egui::Painter,
        rect: egui::Rect,
    ) {
        let uv_max = egui::pos2(
            (width as f32 / self.width as f32).min(1.0),
            (height as f32 / self.height as f32).min(1.0),
        );

        let texture_id = self.preview_texture_id();
        painter.image(
            texture_id,
            rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), uv_max),
            egui::Color32::WHITE,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texture_byte_size_rgba8() {
        let size = texture_byte_size(wgpu::TextureFormat::Rgba8Unorm, 100, 100, 1);
        assert_eq!(size, 100 * 100 * 4);
    }

    #[test]
    fn texture_byte_size_r8() {
        let size = texture_byte_size(wgpu::TextureFormat::R8Unorm, 200, 100, 1);
        assert_eq!(size, 200 * 100);
    }

    #[test]
    fn texture_byte_size_rgba32() {
        let size = texture_byte_size(wgpu::TextureFormat::Rgba32Float, 50, 50, 1);
        assert_eq!(size, 50 * 50 * 16);
    }

    #[test]
    fn texture_byte_size_msaa4x() {
        let size = texture_byte_size(wgpu::TextureFormat::Rgba8Unorm, 100, 100, 4);
        assert_eq!(size, 100 * 100 * 4 * 4);
    }

    #[test]
    fn texture_byte_size_1x1() {
        let size = texture_byte_size(wgpu::TextureFormat::Rgba8Unorm, 1, 1, 1);
        assert_eq!(size, 4);
    }

    #[test]
    fn texture_byte_size_rg16() {
        let size = texture_byte_size(wgpu::TextureFormat::Rg16Float, 100, 100, 1);
        assert_eq!(size, 100 * 100 * 4);
    }
}
