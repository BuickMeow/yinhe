use eframe::egui;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

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
    shrink_to_fit_on_next_size: bool,
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

        // Register a device-lost callback so we can skip GPU operations if the
        // device is lost (e.g. after a failed texture creation during resize).
        let device_lost = Arc::new(AtomicBool::new(false));
        {
            let flag = Arc::clone(&device_lost);
            device.set_device_lost_callback(move |reason, msg| {
                tracing::error!("wgpu device lost: {reason:?} — {msg}");
                flag.store(true, Ordering::Relaxed);
            });
        }

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
            shrink_to_fit_on_next_size: false,
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

        let device_lost = Arc::new(AtomicBool::new(false));
        {
            let flag = Arc::clone(&device_lost);
            device.set_device_lost_callback(move |reason, msg| {
                tracing::error!("wgpu device lost: {reason:?} — {msg}");
                flag.store(true, Ordering::Relaxed);
            });
        }

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
            shrink_to_fit_on_next_size: false,
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
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
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
    pub fn ensure_size(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }

        let should_shrink =
            self.shrink_to_fit_on_next_size && (self.width != width || self.height != height);
        let should_grow = width > self.width || height > self.height;

        if should_shrink || should_grow {
            self.recreate_target(width, height);
        }

        self.shrink_to_fit_on_next_size = false;
    }

    /// Grow the offscreen texture to the requested capacity, but never shrink.
    pub fn reserve_size(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }

        if width > self.width || height > self.height {
            self.recreate_target(width, height);
        }
    }

    /// Shrink to the next requested logical size after a temporary oversize allocation.
    pub fn request_shrink_to_fit(&mut self) {
        self.shrink_to_fit_on_next_size = true;
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
            self.device().as_hal::<wgpu::hal::api::Metal>().map(|hal_device| {
                use objc2_metal::MTLDevice;
                hal_device.raw_device().currentAllocatedSize() as u64
            })
        }
    }

    pub fn preview_view(&self) -> &wgpu::TextureView {
        &self.view
    }

    /// Render to the offscreen texture (if needed) and paint it into egui.
    ///
    /// `content_changed`: caller signals whether CPU-side data (instances,
    /// uniforms, viewport) has changed since the last frame. If false AND the
    /// texture was already rendered (not freshly created), GPU work is skipped
    /// entirely — only `painter.image()` is called to display the existing
    /// texture. This avoids accumulating GPU command buffers when nothing moved.
    ///
    /// Returns early if the GPU device has been lost.
    pub fn paint(
        &mut self,
        renderer: &yinhe_wgpu::PianorollRenderer,
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
            width as f32 / self.width as f32,
            height as f32 / self.height as f32,
        );

        let do_render = self.needs_render || content_changed;

        if do_render {
            let t0 = std::time::Instant::now();
            let mut encoder = self
                .device()
                .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some(label) });
            let t1 = std::time::Instant::now();
            renderer.draw(&mut encoder, self.preview_view(), width, height);
            let t2 = std::time::Instant::now();
            self.queue().submit(std::iter::once(encoder.finish()));
            let t3 = std::time::Instant::now();
            self.needs_render = false;

            // Trace GPU pipeline phases when any phase is suspiciously slow
            // (any phase taking >300us is worth a line).
            let enc_us = t1.saturating_duration_since(t0).as_secs_f64() * 1e6;
            let draw_us = t2.saturating_duration_since(t1).as_secs_f64() * 1e6;
            let sub_us = t3.saturating_duration_since(t2).as_secs_f64() * 1e6;
            if enc_us > 300.0 || draw_us > 300.0 || sub_us > 300.0 {
                eprintln!(
                    "[gpu_trace] {}: encoder={:.0}us draw={:.0}us submit={:.0}us",
                    label, enc_us, draw_us, sub_us,
                );
            }
        }

        let texture_id = self.preview_texture_id();
        painter.image(
            texture_id,
            rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), uv_max),
            egui::Color32::WHITE,
        );
    }
}
