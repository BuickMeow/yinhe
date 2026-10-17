//! Flutter(Android) 纹理桥：wgpu 直接渲染进 Flutter `Texture` 的画布。
//!
//! 阶段 A（本文件当前形态）——最小链路验证：
//! - Kotlin `SurfaceProducer` 回调 → JNI `nativeSetSurface` → `ANativeWindow`
//! - 渲染线程：`create_surface_unsafe` → 清屏 + 旋转方块 → `present`（Fifo 限速）
//! - 触摸：dart:ffi → [`yinhe_touch`] → 方块跟手 + 点击换色
//!
//! 阶段 B 会把演示渲染替换为 `yinhe-wgpu` 的钢琴卷帘渲染。

use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use wgpu::rwh::{AndroidDisplayHandle, AndroidNdkWindowHandle, RawDisplayHandle, RawWindowHandle};
use wgpu::util::DeviceExt;

// ── ANativeWindow 最小 C 接口 ──
// 不用 `ndk` crate：只需要这四个函数，直接声明可避免 jni/ndk 的版本纠缠。

type ANativeWindow = c_void;

#[link(name = "android")]
unsafe extern "C" {
    fn ANativeWindow_fromSurface(
        env: *mut jni::sys::JNIEnv,
        surface: jni::sys::jobject,
    ) -> *mut ANativeWindow;
    fn ANativeWindow_release(window: *mut ANativeWindow);
    fn ANativeWindow_getWidth(window: *mut ANativeWindow) -> i32;
    fn ANativeWindow_getHeight(window: *mut ANativeWindow) -> i32;
}

/// 持有引用的 `ANativeWindow`（`ANativeWindow_fromSurface` 已 +1 引用计数）。
struct OwnedWindow(*mut ANativeWindow);

// ANativeWindow 引用计数是线程安全的，跨线程移交所有权合法。
unsafe impl Send for OwnedWindow {}

impl OwnedWindow {
    fn width(&self) -> u32 {
        (unsafe { ANativeWindow_getWidth(self.0) }).max(1) as u32
    }

    fn height(&self) -> u32 {
        (unsafe { ANativeWindow_getHeight(self.0) }).max(1) as u32
    }
}

impl Drop for OwnedWindow {
    fn drop(&mut self) {
        unsafe { ANativeWindow_release(self.0) }
    }
}

// ── 渲染线程命令 ──

/// 从 Kotlin 侧发往渲染线程的命令。
enum Cmd {
    /// 新的生产端 surface（旧 surface 被替换）。
    SetSurface(OwnedWindow),
    /// surface 销毁（`onSurfaceCleanup`）。
    ClearSurface,
}

static CMD_TX: OnceLock<Sender<Cmd>> = OnceLock::new();

/// 触摸输入（dart:ffi 写入，渲染线程读取；原子共享，无需锁）。
struct InputState {
    /// 指尖 NDC 坐标（-1..1），以 `f32` 位模式存储。
    x: AtomicU32,
    y: AtomicU32,
    /// 是否按住。
    down: AtomicBool,
    /// 累计按下次数（用于换色）。
    taps: AtomicU32,
}

static INPUT: InputState = InputState {
    x: AtomicU32::new(0), // 位模式 0 == 0.0f32
    y: AtomicU32::new(0),
    down: AtomicBool::new(false),
    taps: AtomicU32::new(0),
};

/// 初始化 Android 日志（幂等；host 构建下为空操作）。
fn init_logging() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        #[cfg(target_os = "android")]
        android_logger::init_once(
            android_logger::Config::default()
                .with_max_level(log::LevelFilter::Info)
                .with_tag("yinhe_flutter"),
        );
    });
}

/// 获取（惰性创建）渲染线程的命令发送端。
fn cmd_tx() -> &'static Sender<Cmd> {
    CMD_TX.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel();
        let spawn = std::thread::Builder::new()
            .name("yinhe-flutter-render".into())
            .stack_size(8 * 1024 * 1024)
            .spawn(move || render_thread_main(rx));
        if let Err(e) = spawn {
            // 发送端仍然返回：后续 send 会因接收端丢失而失败，由调用方记录。
            log::error!("渲染线程创建失败: {e}");
        }
        tx
    })
}

// ── 演示渲染 ──

/// 演示用 uniform（WGSL uniform 要求 16 字节对齐，总 48 字节）。
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    /// `[cos, sin, 0, 0]` 旋转分量。
    rot: [f32; 4],
    color: [f32; 4],
    /// `[x, y, 0, 0]` 位置偏移（NDC）。
    offset: [f32; 4],
}

const SHADER: &str = r"
struct Uniforms {
    rot: vec4<f32>,
    color: vec4<f32>,
    offset: vec4<f32>,
};
@group(0) @binding(0) var<uniform> u: Uniforms;

@vertex
fn vs_main(@location(0) pos: vec2<f32>) -> @builtin(position) vec4<f32> {
    let c = u.rot.x;
    let s = u.rot.y;
    let r = vec2<f32>(pos.x * c - pos.y * s, pos.x * s + pos.y * c);
    return vec4<f32>(r + u.offset.xy, 0.0, 1.0);
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    return u.color;
}
";

/// 方块顶点（NDC，两个三角形）。
const QUAD: [[f32; 2]; 6] = [
    [-0.3, -0.3],
    [0.3, -0.3],
    [0.3, 0.3],
    [-0.3, -0.3],
    [0.3, 0.3],
    [-0.3, 0.3],
];

/// 点击换色的调色板。
const PALETTE: [[f32; 4]; 4] = [
    [0.35, 0.75, 1.0, 1.0],
    [1.0, 0.55, 0.25, 1.0],
    [0.55, 0.9, 0.45, 1.0],
    [0.85, 0.5, 1.0, 1.0],
];

/// 设备级 GPU 资源（跨 surface 存活）。
struct Gpu {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    uniform_buf: wgpu::Buffer,
    vertex_buf: wgpu::Buffer,
}

/// surface 级资源（随 `SetSurface`/`ClearSurface` 创建与销毁）。
struct Target {
    /// 注意 drop 顺序：`surface` 必须在 `_window` 之前销毁
    /// （ANativeWindow 需存活到 surface 释放之后；Rust 按声明顺序 drop）。
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    created: Instant,
    /// FPS 统计窗口。
    fps_start: Instant,
    frames: u32,
    /// 仅为保活持有（尺寸在创建时已读取）。
    _window: OwnedWindow,
}

fn render_thread_main(rx: Receiver<Cmd>) {
    log::info!("渲染线程启动");
    let mut gpu: Option<Gpu> = None;
    let mut target: Option<Target> = None;

    loop {
        // 1) 处理所有积压命令（渲染期间可能进度条式到达多条，取最终态）。
        let mut disconnected = false;
        loop {
            match rx.try_recv() {
                Ok(cmd) => apply_cmd(&mut gpu, &mut target, cmd),
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }
        if disconnected {
            break;
        }

        // 2) 有目标则渲染一帧（Fifo present 自带节流）；
        //    无目标则阻塞等待命令，避免空转。
        match (&gpu, &mut target) {
            (Some(g), Some(t)) => render_frame(g, t),
            _ => match rx.recv() {
                Ok(cmd) => apply_cmd(&mut gpu, &mut target, cmd),
                Err(_) => break,
            },
        }
    }
    log::info!("渲染线程退出");
}

fn apply_cmd(gpu: &mut Option<Gpu>, target: &mut Option<Target>, cmd: Cmd) {
    match cmd {
        Cmd::SetSurface(window) => {
            // 先释放旧 target（surface 先于窗口销毁）。
            *target = None;
            if gpu.is_none() {
                *gpu = create_gpu();
            }
            if let Some(g) = gpu {
                *target = create_target(g, window);
            }
        }
        Cmd::ClearSurface => *target = None,
    }
}

fn create_gpu() -> Option<Gpu> {
    log::info!("初始化 wgpu（Vulkan/GLES）…");
    // 默认实例在 Android 上即 VULKAN|GL 后端（按平台编译裁剪）。
    let instance = wgpu::Instance::default();
    let adapter = match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    })) {
        Ok(a) => a,
        Err(e) => {
            log::error!("请求适配器失败: {e}");
            return None;
        }
    };
    log::info!("适配器: {:?}", adapter.get_info());
    let (device, queue) =
        match pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("yinhe-flutter"),
            ..Default::default()
        })) {
            Ok(dq) => dq,
            Err(e) => {
                log::error!("创建设备失败: {e}");
                return None;
            }
        };

    let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("demo_uniforms"),
        size: std::mem::size_of::<Uniforms>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let vertex_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("demo_vertices"),
        contents: bytemuck::cast_slice(&QUAD),
        usage: wgpu::BufferUsages::VERTEX,
    });

    Some(Gpu {
        instance,
        adapter,
        device,
        queue,
        uniform_buf,
        vertex_buf,
    })
}

/// 演示管线：自动布局（bind group layout 由 shader 推导）。
fn create_demo_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("demo_shader"),
        source: wgpu::ShaderSource::Wgsl(SHADER.into()),
    });
    const ATTRS: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![0 => Float32x2];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("demo_pipeline"),
        layout: None,
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: 8,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &ATTRS,
            })],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

fn create_target(gpu: &Gpu, window: OwnedWindow) -> Option<Target> {
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

    let pipeline = create_demo_pipeline(&gpu.device, format);
    let bgl = pipeline.get_bind_group_layout(0);
    let bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("demo_bind_group"),
        layout: &bgl,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: gpu.uniform_buf.as_entire_binding(),
        }],
    });

    log::info!(
        "surface 就绪: {}x{} {format:?}",
        config.width,
        config.height
    );
    Some(Target {
        surface,
        config,
        pipeline,
        bind_group,
        created: Instant::now(),
        fps_start: Instant::now(),
        frames: 0,
        _window: window,
    })
}

fn render_frame(gpu: &Gpu, t: &mut Target) {
    // ── 每帧 uniform：旋转 + 触摸位置/颜色 ──
    let angle = t.created.elapsed().as_secs_f32() * 1.2;
    let down = INPUT.down.load(Ordering::Relaxed);
    let offset = if down {
        [
            f32::from_bits(INPUT.x.load(Ordering::Relaxed)).clamp(-0.85, 0.85),
            f32::from_bits(INPUT.y.load(Ordering::Relaxed)).clamp(-0.85, 0.85),
            0.0,
            0.0,
        ]
    } else {
        [0.0; 4]
    };
    let taps = INPUT.taps.load(Ordering::Relaxed) as usize;
    let uniforms = Uniforms {
        rot: [angle.cos(), angle.sin(), 0.0, 0.0],
        color: PALETTE[taps % PALETTE.len()],
        offset,
    };
    gpu.queue
        .write_buffer(&gpu.uniform_buf, 0, bytemuck::bytes_of(&uniforms));

    // ── 绘制 ──
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
    let view = frame
        .texture
        .create_view(&wgpu::TextureViewDescriptor::default());
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("demo_frame"),
        });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("demo_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.06,
                        g: 0.06,
                        b: 0.09,
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
        pass.set_pipeline(&t.pipeline);
        pass.set_bind_group(0, &t.bind_group, &[]);
        pass.set_vertex_buffer(0, gpu.vertex_buf.slice(..));
        pass.draw(0..6, 0..1);
    }
    gpu.queue.submit([encoder.finish()]);
    gpu.queue.present(frame);

    // ── FPS 统计（每秒一条，验证链路吞吐）──
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

// ── JNI 入口（Kotlin → Rust）──

/// `MainActivity.nativeSetSurface(surface)`：替换生产端 surface。
// JVM 保证 env/surface 句柄有效；JNI 导出函数无法用 Rust 安全类型表达入参。
#[allow(clippy::not_unsafe_ptr_arg_deref)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_jieneng_yinhe_1flutter_MainActivity_nativeSetSurface(
    env: *mut jni::sys::JNIEnv,
    _this: jni::sys::jobject,
    surface: jni::sys::jobject,
) {
    init_logging();
    if surface.is_null() {
        log::warn!("nativeSetSurface 收到空 surface");
        return;
    }
    // 安全性：本函数只由 JVM 调用，env/surface 句柄有效；空指针已检查。
    let raw = unsafe { ANativeWindow_fromSurface(env, surface) };
    if raw.is_null() {
        log::error!("ANativeWindow_fromSurface 返回空指针");
        return;
    }
    // 发送失败时 OwnedWindow 随错误值 drop，引用计数正确释放。
    if cmd_tx().send(Cmd::SetSurface(OwnedWindow(raw))).is_err() {
        log::error!("渲染线程不可用，丢弃 surface");
    }
}

/// `MainActivity.nativeClearSurface()`：surface 销毁。
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_jieneng_yinhe_1flutter_MainActivity_nativeClearSurface(
    _env: *mut jni::sys::JNIEnv,
    _this: jni::sys::jobject,
) {
    init_logging();
    if cmd_tx().send(Cmd::ClearSurface).is_err() {
        log::error!("渲染线程不可用，忽略 ClearSurface");
    }
}

// ── dart:ffi 入口（Dart → Rust）──

/// 触摸事件：`kind` 0=down 1=move 2=up，`x`/`y` 为 NDC（-1..1）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_touch(kind: i32, x: f32, y: f32, _pointer: i32) {
    match kind {
        0 => {
            INPUT.x.store(x.to_bits(), Ordering::Relaxed);
            INPUT.y.store(y.to_bits(), Ordering::Relaxed);
            INPUT.down.store(true, Ordering::Relaxed);
            INPUT.taps.fetch_add(1, Ordering::Relaxed);
        }
        1 => {
            if INPUT.down.load(Ordering::Relaxed) {
                INPUT.x.store(x.to_bits(), Ordering::Relaxed);
                INPUT.y.store(y.to_bits(), Ordering::Relaxed);
            }
        }
        _ => INPUT.down.store(false, Ordering::Relaxed),
    }
}
