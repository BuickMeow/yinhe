//! 音符 shader 的像素级回归：力度填充高度必须真的改变画出来的像素，
//! 而不只是 shader 能编译。无 GPU 环境自动跳过。

use crate::InstanceRenderer;
use crate::cull::tests::headless_device;
use crate::vertex::{MAX_SEL_RECTS, NoteInstance, SelectionUniform, Uniforms};
use wgpu::*;

fn read_rgba(
    device: &Device,
    queue: &Queue,
    texture: &Texture,
    width: u32,
    height: u32,
) -> Vec<[u8; 4]> {
    let bytes_per_row = width * 4;
    let aligned_row = bytes_per_row.div_ceil(256) * 256;
    let buffer = device.create_buffer(&BufferDescriptor {
        label: Some("velocity_fill_readback"),
        size: (aligned_row * height) as u64,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut enc = device.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: Origin3d::ZERO,
            aspect: TextureAspect::All,
        },
        TexelCopyBufferInfo {
            buffer: &buffer,
            layout: TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(aligned_row),
                rows_per_image: Some(height),
            },
        },
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([enc.finish()]);

    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let done2 = done.clone();
    buffer.slice(..).map_async(MapMode::Read, move |_| {
        done2.store(true, std::sync::atomic::Ordering::SeqCst);
    });
    device
        .poll(PollType::wait_indefinitely())
        .expect("poll failed");
    assert!(done.load(std::sync::atomic::Ordering::SeqCst));

    let mapped = buffer.slice(..).get_mapped_range().expect("readback map");
    let mut out = Vec::with_capacity((width * height) as usize);
    for row in 0..height {
        let start = row as usize * aligned_row as usize;
        let row_data = &mapped[start..start + bytes_per_row as usize];
        for px in row_data.as_chunks::<4>().0 {
            out.push(*px);
        }
    }
    drop(mapped);
    buffer.unmap();
    out
}

/// 力度 64：音符上半部分是暗色未填充体，下半部分是轨道色填充；
/// 力度 127：整个内部都是轨道色。
#[test]
fn velocity_fill_height_changes_pixels() {
    let Some((device, queue)) = headless_device() else {
        return;
    };
    let format = TextureFormat::Rgba8UnormSrgb;
    let (width, height) = (200u32, 200u32);
    let mut renderer = InstanceRenderer::new(device.clone(), queue.clone(), format);

    let target = device.create_texture(&TextureDescriptor {
        label: Some("velocity_fill_target"),
        size: Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format,
        usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let target_view = target.create_view(&Default::default());

    // key 60 行落在 y∈[100,120)，key 62 行落在 y∈[60,80)。
    let uniforms = Uniforms {
        width: width as f32,
        height: height as f32,
        scroll_x: 0.0,
        scroll_y: 1240.0,
        pixels_per_tick: 2.0,
        key_height: 20.0,
        keyboard_width: 0.0,
        mode: 1,
        note_outline: 1,
        track_count: 1,
        ..Default::default()
    };
    let track_color = [0.2, 0.7, 1.0, 1.0];
    renderer.upload_uniforms(uniforms);
    renderer.upload_track_colors(&[track_color]);
    renderer.upload_selection(&SelectionUniform {
        rects: [[0; 4]; MAX_SEL_RECTS * 2],
    });
    renderer.ensure_layers(1);
    renderer.upload_note_layer(0, 0, |out| {
        out.push(NoteInstance {
            start_tick: 0,
            end_tick: 100,
            packed: NoteInstance::pack(60, 0, 64),
        });
        out.push(NoteInstance {
            start_tick: 0,
            end_tick: 100,
            packed: NoteInstance::pack(62, 0, 127),
        });
    });

    let mut enc = device.create_command_encoder(&Default::default());
    renderer.draw(&mut enc, &target_view, width, height);
    queue.submit([enc.finish()]);

    let pixels = read_rgba(&device, &queue, &target, width, height);
    let at = |x: u32, y: u32| pixels[(y * width + x) as usize];

    let mid_top = at(100, 105);
    let mid_bottom = at(100, 115);
    let full_top = at(100, 65);

    assert!(
        mid_bottom[2] > 100,
        "力度 64 的下半部分应画出来，实际 {mid_bottom:?}"
    );
    assert!(
        mid_bottom[0] > mid_top[0] + 10 && mid_bottom[2] > mid_top[2] + 40,
        "下半部分应明显亮于上半部分：top={mid_top:?} bottom={mid_bottom:?}"
    );
    assert!(
        full_top[2] > mid_top[2] + 40,
        "力度 127 的顶部也应是填充色：mid_top={mid_top:?} full_top={full_top:?}"
    );
}
