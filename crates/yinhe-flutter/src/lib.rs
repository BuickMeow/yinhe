//! Flutter(Android/OpenHarmony) 纹理桥 + 编辑器渲染宿主。
//!
//! 链路（Android）：Kotlin `SurfaceProducer` → JNI `nativeSetSurface` → `ANativeWindow`
//! 链路（OHOS）：ArkTS 外接纹理 → NAPI → `yinhe_ohos_set_surface` → `OHNativeWindow`
//! → 渲染线程（wgpu / `yinhe-wgpu` 钢琴卷帘）→ present → Flutter `Texture` 合成。

// 本 crate 只在 Android / OpenHarmony 上产出 cdylib；桌面构建仅用于 workspace 检查，
// 平台专属入口（JNI / OHOS C ABI）的辅助函数在桌面上必然"未使用"。
#![cfg_attr(not(any(target_os = "android", target_env = "ohos")), allow(dead_code))]
//!
//! - 场景：`scene.rs`（文档快照 + 演示内容）；惯性：`inertia.rs`；渲染：`render.rs`
//! - 手势坐标：Flutter 传画布逻辑像素；画布逻辑尺寸：`nativeSetViewport` 传入
//!   （surface buffer 为物理像素，两者分别用于 view 与交换链）

mod android;
mod audio;
mod autosave;
mod dart_ffi;
mod doc_job;
mod edit;
mod inertia;
#[cfg(target_os = "ios")]
mod ios;
#[cfg(target_os = "ios")]
mod ios_ffi;
mod library;
mod midi_export;
mod midi_import;
mod midi_names;
#[cfg(target_env = "ohos")]
mod ohos_ffi;
mod render;
mod scene;
mod selection_materializer;
mod soundfont_archive;
mod theme_colors;
mod velocity_cache;

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Mutex, OnceLock};

// ── 原生窗口句柄 ──
// Android：`ANativeWindow`（SurfaceProducer 的 Surface 经 JNI 转换）；
// OpenHarmony：`OHNativeWindow`（XComponent 的 OnSurfaceCreated 回调直接给 C 指针）。
// 只需要尺寸 / 释放两件事，不用 `ndk` crate，避免版本纠缠。

#[cfg(target_os = "android")]
mod native_window {
    use std::ffi::c_void;

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
    pub(crate) struct OwnedWindow(pub(crate) *mut ANativeWindow);

    // ANativeWindow 引用计数是线程安全的，跨线程移交所有权合法。
    unsafe impl Send for OwnedWindow {}

    impl OwnedWindow {
        pub(crate) fn width(&self) -> u32 {
            (unsafe { ANativeWindow_getWidth(self.0) }).max(1) as u32
        }

        pub(crate) fn height(&self) -> u32 {
            (unsafe { ANativeWindow_getHeight(self.0) }).max(1) as u32
        }
    }

    impl Drop for OwnedWindow {
        fn drop(&mut self) {
            unsafe { ANativeWindow_release(self.0) }
        }
    }

    /// Surface → `OwnedWindow`（JNI 侧入口用）。
    pub(crate) fn from_surface(
        env: *mut jni::sys::JNIEnv,
        surface: jni::sys::jobject,
    ) -> *mut c_void {
        unsafe { ANativeWindow_fromSurface(env, surface) }
    }
}

#[cfg(not(target_os = "android"))]
mod native_window {
    use std::ffi::c_void;

    /// XComponent 回调用 `OH_NativeXComponent_GetNativeWindow` 拿到的窗口指针。
    pub(crate) struct OwnedWindow(pub(crate) *mut c_void);

    // OHNativeWindow 的所有权由应用持有，跨线程移交合法。
    unsafe impl Send for OwnedWindow {}

    impl OwnedWindow {
        // 尺寸由 ArkTS 侧 XComponent 布局决定，原生侧不查询（与 Android 一致走
        // nativeSetSize 上报），这里给 1 只用于 surface 首次创建前的兜底。
        pub(crate) fn width(&self) -> u32 {
            1
        }

        pub(crate) fn height(&self) -> u32 {
            1
        }
    }

    impl Drop for OwnedWindow {
        fn drop(&mut self) {
            // OHNativeWindow 的生命周期由 XComponent 管理，这里不释放。
        }
    }
}

pub(crate) use native_window::OwnedWindow;

// ── 渲染线程命令 ──

/// 从 Kotlin 侧发往渲染线程的命令。
enum Cmd {
    /// 新的生产端 surface（旧 surface 被替换）。
    SetSurface(OwnedWindow),
    /// surface 销毁（`onSurfaceCleanup`）。
    ClearSurface,
    /// AM 面板独立 surface。
    SetAutomationSurface(OwnedWindow),
    ClearAutomationSurface,
    /// iOS：登记一个外部纹理呈现端（0 = 主画布，1 = AM 面板）。
    #[cfg(target_os = "ios")]
    EnsureExternal(i32),
    /// Flutter 帧驱动：渲染一帧，present 完成后通过 `done` 回信。
    RenderNow(std::sync::mpsc::SyncSender<()>),
}

static CMD_TX: OnceLock<Sender<Cmd>> = OnceLock::new();

// ── 画布逻辑尺寸（Flutter 控件逻辑像素）──

static LOGICAL_W_BITS: AtomicU32 = AtomicU32::new(0);
static LOGICAL_H_BITS: AtomicU32 = AtomicU32::new(0);
static PIXEL_W: AtomicU32 = AtomicU32::new(0);
static PIXEL_H: AtomicU32 = AtomicU32::new(0);
static AM_PIXEL_W: AtomicU32 = AtomicU32::new(0);
static AM_PIXEL_H: AtomicU32 = AtomicU32::new(0);

/// 画布逻辑尺寸（视图/手势坐标系；未收到 `nativeSetViewport` 前为 0）。
pub(crate) fn logical_size() -> (f32, f32) {
    (
        f32::from_bits(LOGICAL_W_BITS.load(Ordering::Relaxed)),
        f32::from_bits(LOGICAL_H_BITS.load(Ordering::Relaxed)),
    )
}

/// surface 像素尺寸（Kotlin `nativeSetSize` 上报）。
pub(crate) fn pixel_size() -> (u32, u32) {
    (
        PIXEL_W.load(Ordering::Relaxed),
        PIXEL_H.load(Ordering::Relaxed),
    )
}

/// AM 独立 surface 的像素尺寸（Kotlin `nativeSetAutomationSize` 上报）。
pub(crate) fn automation_pixel_size() -> (u32, u32) {
    (
        AM_PIXEL_W.load(Ordering::Relaxed),
        AM_PIXEL_H.load(Ordering::Relaxed),
    )
}

fn set_logical_size(w: f32, h: f32) {
    LOGICAL_W_BITS.store(w.max(1.0).to_bits(), Ordering::Relaxed);
    LOGICAL_H_BITS.store(h.max(1.0).to_bits(), Ordering::Relaxed);
}

// ── 日志 ──

/// 初始化 Android 日志（幂等；host 构建下为空操作）。
fn init_logging() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        // panic 输出到 logcat（Android 上 stderr 默认不可见，缺失时会"静默死亡"）。
        std::panic::set_hook(Box::new(|info| {
            log::error!("panic: {info}");
            let bt = std::backtrace::Backtrace::force_capture();
            log::error!("BACKTRACE:\n{bt}");
        }));
        #[cfg(target_os = "android")]
        android_logger::init_once(
            android_logger::Config::default()
                .with_max_level(log::LevelFilter::Info)
                .with_tag("yinhe_flutter"),
        );
        #[cfg(target_env = "ohos")]
        crate::hilog::init();
        #[cfg(target_os = "ios")]
        crate::ios::init_logging();
    });
}

// ── 鸿蒙 hilog ──

/// 最小 `log` → hilog 适配（`OH_LOG_Print`）。
#[cfg(target_env = "ohos")]
mod hilog {
    use std::ffi::{CString, c_char, c_int, c_uint};

    #[link(name = "hilog_ndk.z")]
    unsafe extern "C" {
        fn OH_LOG_Print(
            log_type: c_int,
            level: c_int,
            domain: c_uint,
            tag: *const c_char,
            fmt: *const c_char,
            ...
        ) -> c_int;
    }

    /// `LOG_APP`（应用日志域）。
    const LOG_APP: c_int = 0;
    const LOG_INFO: c_int = 4;
    const LOG_ERROR: c_int = 6;

    fn emit(level: c_int, msg: &str) {
        // hilog 的格式化串需要 `%{public}s`；消息本身先转 C 字符串，避免格式注入。
        let Ok(msg) = CString::new(msg) else {
            return;
        };
        let tag = c"yinhe";
        let fmt = c"%{public}s";
        // SAFETY: 两个 C 字符串在调用期间有效；OH_LOG_Print 是可变参数 C 函数。
        unsafe {
            OH_LOG_Print(LOG_APP, level, 0, tag.as_ptr(), fmt.as_ptr(), msg.as_ptr());
        }
    }

    struct Logger;

    impl log::Log for Logger {
        fn enabled(&self, _: &log::Metadata<'_>) -> bool {
            true
        }

        fn log(&self, record: &log::Record<'_>) {
            let level = match record.level() {
                log::Level::Error => LOG_ERROR,
                _ => LOG_INFO,
            };
            emit(level, &format!("{}: {}", record.target(), record.args()));
        }

        fn flush(&self) {}
    }

    pub(crate) fn init() {
        let _ = log::set_logger(&Logger);
        log::set_max_level(log::LevelFilter::Info);
    }
}

// ── 设备资源 ──

/// 设备级 GPU 资源（跨 surface 存活）。
pub(crate) struct Gpu {
    pub(crate) instance: wgpu::Instance,
    pub(crate) adapter: wgpu::Adapter,
    pub(crate) device: wgpu::Device,
    pub(crate) queue: wgpu::Queue,
    /// 设备能力（iOS 模拟器不支持间接绘制，渲染要退回 CPU 回读）。
    pub(crate) caps: yinhe_wgpu::GpuCaps,
}

fn create_gpu() -> Option<Gpu> {
    log::info!("初始化 wgpu…");
    // 默认实例在 Android 上即 VULKAN|GL 后端（按平台编译裁剪）。
    // OHOS：wgpu-hal 只有 GLES 后端实现了 `RawWindowHandle::OhosNdk`，Vulkan 还没有
    // `VK_OHOS_surface`；必须显式限定后端，否则会挑到 Vulkan adapter、建 surface 失败。
    #[cfg(target_env = "ohos")]
    let instance = {
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
        desc.backends = wgpu::Backends::GL;
        wgpu::Instance::new(desc)
    };
    // Apple（iOS）：只有 Metal 一个后端，显式限定避免多余的后端探测。
    #[cfg(all(not(target_env = "ohos"), target_vendor = "apple"))]
    let instance = {
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
        desc.backends = wgpu::Backends::METAL;
        wgpu::Instance::new(desc)
    };
    #[cfg(all(not(target_env = "ohos"), not(target_vendor = "apple")))]
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
    // 直接申请适配器自身支持的 limits：GLES 3.0（鸿蒙模拟器）的
    // max_inter_stage_shader_variables 只有 7，硬套桌面/WebGL2 的下限会直接
    // 创建设备失败。拿不到 16 个 storage buffer 时 GPU cull 建不起来，
    // 渲染会自动退回 `draw_legacy` 的 CPU 路径。
    // 间接绘制能力：iOS 模拟器等软件 GPU 没有 INDIRECT_EXECUTION。
    let caps = yinhe_wgpu::GpuCaps {
        indirect_execution: adapter
            .get_downlevel_capabilities()
            .flags
            .contains(wgpu::DownlevelFlags::INDIRECT_EXECUTION),
    };
    let required_limits = adapter.limits();
    let (device, queue) =
        match pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("yinhe-flutter"),
            required_features: wgpu::Features::empty(),
            required_limits,
            ..Default::default()
        })) {
            Ok(dq) => dq,
            Err(e) => {
                log::error!("创建设备失败: {e}");
                return None;
            }
        };
    // wgpu 校验错误转日志：安卓 stderr 不可见，必须显式打到 logcat
    // （也用于暴露 create_buffer 等静默失败的真实原因）。
    device.on_uncaptured_error(std::sync::Arc::new(|err| {
        log::error!("wgpu uncaptured error: {err}");
    }));
    // 设备丢失是"后续全部操作 invalid"的典型前因，必须可见。
    device.set_device_lost_callback(|reason, msg| {
        log::error!("wgpu device lost: {reason:?} msg={msg}");
    });
    // iOS：宿主的 CVPixelBuffer 要建 `CVMetalTextureCache`，必须与 wgpu 用同一
    // 个 MTLDevice，这里把裸指针交给它。
    #[cfg(target_os = "ios")]
    unsafe {
        use objc2::rc::Retained;
        if let Some(hal) = device.as_hal::<wgpu::hal::api::Metal>() {
            let ptr = Retained::as_ptr(hal.raw_device()) as *mut std::ffi::c_void;
            ios::set_metal_device(ptr);
        }
    }
    Some(Gpu {
        instance,
        adapter,
        device,
        queue,
        caps,
    })
}

// ── 渲染线程 ──

/// 获取（惰性创建）渲染线程的命令发送端。
fn cmd_tx() -> &'static Sender<Cmd> {
    CMD_TX.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel();
        let spawn = std::thread::Builder::new()
            .name("yinhe-flutter-render".into())
            .stack_size(32 * 1024 * 1024)
            .spawn(move || render_thread_main(rx));
        if let Err(e) = spawn {
            // 发送端仍然返回：后续 send 会因接收端丢失而失败，由调用方记录。
            log::error!("渲染线程创建失败: {e}");
        }
        tx
    })
}

fn render_thread_main(rx: Receiver<Cmd>) {
    log::info!("渲染线程启动");
    // 主题由持久化的 ui_theme 决定。必须同步给 `yinhe-wgpu` 的共享 GPU
    // 主题；否则 `build_render_job` 会读 `GpuTheme::default()` 的 DARK，
    // 导致亮色主题下选中/ghost 音符按暗色方向渲染。
    let dark = scene::ui_theme() == "dark";
    yinhe_theme::set_current_gpu_theme(yinhe_theme::GpuTheme::from_base(if dark {
        yinhe_theme::base::BaseColors::DARK
    } else {
        yinhe_theme::base::BaseColors::LIGHT
    }));
    let mut gpu: Option<Gpu> = None;
    let mut target: Option<render::Target> = None;
    let mut am_target: Option<render::AmTarget> = None;

    // 帧驱动：由 Flutter 每帧请求渲染并同步等待，纹理与浮层同帧产出。
    while let Ok(first) = rx.recv() {
        // 1) 处理积压命令（取最终态）；合并本轮的渲染请求。
        let mut need_render: Option<std::sync::mpsc::SyncSender<()>> = None;
        let mut cmd = Some(first);
        while let Some(c) = cmd {
            match c {
                Cmd::RenderNow(done) => need_render = Some(done),
                other => apply_cmd(&mut gpu, &mut target, &mut am_target, other),
            }
            cmd = rx.try_recv().ok();
        }

        // 2) 有请求则渲染一帧（含 present）；完成后回信，UI 线程才继续 paint。
        if let Some(done) = need_render {
            if let (Some(g), Some(t)) = (&gpu, &mut target) {
                render::render_frame(g, t);
            }
            let summary = target.as_ref().and_then(|t| t.velocity_summary());
            if let (Some(g), Some(am)) = (&gpu, &mut am_target) {
                render::render_automation_frame(g, am, summary);
            }
            let _ = done.send(());
        }
    }
    log::info!("渲染线程退出");
}

fn apply_cmd(
    gpu: &mut Option<Gpu>,
    target: &mut Option<render::Target>,
    am_target: &mut Option<render::AmTarget>,
    cmd: Cmd,
) {
    match cmd {
        Cmd::SetSurface(window) => {
            // 同一 ANativeWindow 重复设置：忽略（对同一窗口再建 Vulkan surface
            // 会触发 VK_ERROR_NATIVE_WINDOW_IN_USE_KHR 使渲染线程 panic）。
            if let Some(t) = target.as_ref()
                && t.window_ptr() == Some(window.0 as usize)
            {
                log::info!("忽略重复 surface（同一窗口）");
                return;
            }
            // 先释放旧 target（surface 先于窗口销毁）。
            *target = None;
            inertia::stop();
            crate::edit::cancel();
            if gpu.is_none() {
                *gpu = create_gpu();
            }
            if let Some(g) = gpu {
                *target = render::create_target(g, window);
            }
        }
        Cmd::ClearSurface => {
            *target = None;
            inertia::stop();
            crate::edit::cancel();
        }
        Cmd::SetAutomationSurface(window) => {
            if let Some(t) = am_target.as_ref()
                && t.window_ptr() == Some(window.0 as usize)
            {
                log::info!("忽略重复 AM surface（同一窗口）");
                return;
            }
            *am_target = None;
            if gpu.is_none() {
                *gpu = create_gpu();
            }
            if let Some(g) = gpu.as_ref() {
                *am_target = render::create_automation_target(g, window);
            }
        }
        Cmd::ClearAutomationSurface => {
            *am_target = None;
        }
        #[cfg(target_os = "ios")]
        Cmd::EnsureExternal(kind) => {
            if gpu.is_none() {
                *gpu = create_gpu();
            }
            let Some(g) = gpu.as_ref() else {
                return;
            };
            if kind == 0 {
                if target.is_none() {
                    *target = render::create_external_target(g, 0);
                }
            } else if am_target.is_none() {
                *am_target = render::create_external_automation_target(g, 1);
            }
        }
        Cmd::RenderNow(_) => {
            // 渲染请求由渲染循环统一处理（这里不应到达）。
        }
    }
}
