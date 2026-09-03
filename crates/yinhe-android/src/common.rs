//! 安卓端 PR/AR 视图共享逻辑（GPU 离屏纹理等）

use eframe::egui;

/// 创建离屏渲染目标（wgpu 纹理 + 视图 + egui 纹理句柄）
///
/// PR/AR 视图之前各自拷贝了一份此函数（仅 label 不同），现抽为公共函数
/// 消除重复：`label` 为调试标签（如 "pianoroll_preview" / "yinhe-ar-offscreen"）
pub fn create_target(
    device: &wgpu::Device,
    egui_renderer: &mut eframe::egui_wgpu::Renderer,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
    label: &str,
) -> (wgpu::Texture, wgpu::TextureView, egui::TextureId) {
    let max_dim = device.limits().max_texture_dimension_2d;
    let width = width.min(max_dim).max(1);
    let height = height.min(max_dim).max(1);
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
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
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
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats,
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let texture_id =
        egui_renderer.register_native_texture(device, &view, wgpu::FilterMode::Nearest);
    (texture, view, texture_id)
}
