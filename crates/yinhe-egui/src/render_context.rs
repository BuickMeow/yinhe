use eframe::egui;
use std::sync::Arc;

/// Manages the wgpu device/queue shared with eframe, and an offscreen render target.
pub struct RenderContext {
    wgpu_state: Arc<eframe::egui_wgpu::RenderState>,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    texture_id: egui::TextureId,
    width: u32,
    height: u32,
}

impl RenderContext {
    pub fn new(cc: &eframe::CreationContext<'_>, width: u32, height: u32) -> Self {
        let wgpu_state = cc
            .wgpu_render_state
            .clone()
            .expect("wgpu backend required");
        let device = &wgpu_state.device;
        let format = wgpu_state.target_format;

        let (texture, view, texture_id) = Self::create_target(device, &mut wgpu_state.renderer.write(), format, width, height);

        Self {
            wgpu_state: wgpu_state.into(),
            texture,
            view,
            texture_id,
            width,
            height,
        }
    }

    fn create_target(
        device: &wgpu::Device,
        egui_renderer: &mut eframe::egui_wgpu::Renderer,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> (wgpu::Texture, wgpu::TextureView, egui::TextureId) {
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
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let texture_id = egui_renderer.register_native_texture(device, &view, wgpu::FilterMode::Nearest);
        (texture, view, texture_id)
    }

    /// Resize the offscreen texture if needed.
    pub fn ensure_size(&mut self, width: u32, height: u32) {
        if self.width == width && self.height == height {
            return;
        }
        let format = self.wgpu_state.target_format;
        let device = &self.wgpu_state.device;
        let mut egui_renderer = self.wgpu_state.renderer.write();
        egui_renderer.free_texture(&self.texture_id);
        let (texture, view, texture_id) = Self::create_target(device, &mut egui_renderer, format, width, height);
        self.texture = texture;
        self.view = view;
        self.texture_id = texture_id;
        self.width = width;
        self.height = height;
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

    pub fn preview_view(&self) -> &wgpu::TextureView {
        &self.view
    }

    /// Encode a draw call, submit to GPU, and paint the resulting texture into egui.
    pub fn render_and_display(
        &mut self,
        renderer: &yinhe_wgpu::PianorollRenderer,
        width: u32,
        height: u32,
        label: &str,
        painter: &egui::Painter,
        rect: egui::Rect,
        texture_id: egui::TextureId,
    ) {
        let mut encoder = self
            .device()
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some(label),
            });
        renderer.draw(&mut encoder, self.preview_view(), width, height);
        self.queue().submit(std::iter::once(encoder.finish()));

        painter.image(
            texture_id,
            rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
    }
}
