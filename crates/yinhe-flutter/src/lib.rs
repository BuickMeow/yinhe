//! Flutter(Android) 纹理桥 + 编辑器渲染宿主。
//!
//! 链路：Kotlin `SurfaceProducer` → JNI `nativeSetSurface` → `ANativeWindow`
//! → 渲染线程（wgpu / `yinhe-wgpu` 钢琴卷帘）→ present → Flutter `Texture` 合成。
//!
//! - 场景：`scene.rs`（文档快照 + 演示内容）；触摸：`input.rs`；渲染：`render.rs`
//! - 触摸坐标：dart:ffi 传画布逻辑像素；画布逻辑尺寸：`nativeSetViewport` 传入
//!   （surface buffer 为物理像素，两者分别用于 view 与交换链）

mod audio;
mod input;
mod render;
mod scene;
mod theme_colors;

use std::ffi::{CStr, CString, c_void};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, Sender};

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
pub(crate) struct OwnedWindow(pub(crate) *mut ANativeWindow);

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
    /// Flutter 帧驱动：渲染一帧，完成后通过 ack 回信。
    RenderNow(std::sync::mpsc::SyncSender<()>),
}

static CMD_TX: OnceLock<Sender<Cmd>> = OnceLock::new();

// ── 画布逻辑尺寸（Flutter 控件逻辑像素）──

static LOGICAL_W_BITS: AtomicU32 = AtomicU32::new(0);
static LOGICAL_H_BITS: AtomicU32 = AtomicU32::new(0);

/// 画布逻辑尺寸（视图/手势坐标系；未收到 `nativeSetViewport` 前为 0）。
pub(crate) fn logical_size() -> (f32, f32) {
    (
        f32::from_bits(LOGICAL_W_BITS.load(Ordering::Relaxed)),
        f32::from_bits(LOGICAL_H_BITS.load(Ordering::Relaxed)),
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
    });
}

// ── 设备资源 ──

/// 设备级 GPU 资源（跨 surface 存活）。
pub(crate) struct Gpu {
    pub(crate) instance: wgpu::Instance,
    pub(crate) adapter: wgpu::Adapter,
    pub(crate) device: wgpu::Device,
    pub(crate) queue: wgpu::Queue,
}

fn create_gpu() -> Option<Gpu> {
    log::info!("初始化 wgpu…");
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
    // 与桌面/旧安卓壳一致的 limits：GPU cull 需要 16 个 storage buffer/阶段
    // （默认只有 8，pipeline 会静默创建失败 → 音符不渲染）。
    let base_limits = if adapter.get_info().backend == wgpu::Backend::Gl {
        wgpu::Limits::downlevel_webgl2_defaults()
    } else {
        wgpu::Limits::default()
    };
    let (device, queue) =
        match pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("yinhe-flutter"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits {
                max_texture_dimension_2d: 8192,
                max_storage_buffers_per_shader_stage: 16,
                ..base_limits
            },
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
    Some(Gpu {
        instance,
        adapter,
        device,
        queue,
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
    let mut gpu: Option<Gpu> = None;
    let mut target: Option<render::Target> = None;

    // 帧驱动：由 Flutter 每帧请求渲染并同步等待，纹理与浮层同帧产出。
    while let Ok(first) = rx.recv() {
        // 1) 处理积压命令（取最终态）；渲染请求记下来统一应答。
        let mut pending: Vec<std::sync::mpsc::SyncSender<()>> = Vec::new();
        let mut cmd = Some(first);
        while let Some(c) = cmd {
            match c {
                Cmd::RenderNow(ack) => pending.push(ack),
                other => apply_cmd(&mut gpu, &mut target, other),
            }
            cmd = rx.try_recv().ok();
        }

        // 2) 有请求则渲染一帧（含 present），随后应答（Flutter 侧解除等待）。
        if !pending.is_empty() {
            if let (Some(g), Some(t)) = (&gpu, &mut target) {
                render::render_frame(g, t);
            }
            for ack in pending {
                let _ = ack.send(());
            }
        }
    }
    log::info!("渲染线程退出");
}

fn apply_cmd(gpu: &mut Option<Gpu>, target: &mut Option<render::Target>, cmd: Cmd) {
    match cmd {
        Cmd::SetSurface(window) => {
            // 先释放旧 target（surface 先于窗口销毁）。
            *target = None;
            input::reset();
            if gpu.is_none() {
                *gpu = create_gpu();
            }
            if let Some(g) = gpu {
                *target = render::create_target(g, window);
            }
        }
        Cmd::ClearSurface => {
            *target = None;
            input::reset();
        }
        Cmd::RenderNow(_) => {
            // 渲染请求由渲染循环统一处理与应答（这里不应到达）。
        }
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

/// `MainActivity.nativeSetViewport(w, h)`：画布逻辑尺寸（Flutter 逻辑像素）。
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_jieneng_yinhe_1flutter_MainActivity_nativeSetViewport(
    _env: *mut jni::sys::JNIEnv,
    _this: jni::sys::jobject,
    width: jni::sys::jfloat,
    height: jni::sys::jfloat,
) {
    init_logging();
    set_logical_size(width, height);
}

/// `MainActivity.onFilePicked(path)`：SAF 选择结果（已复制到私有目录）。
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_jieneng_yinhe_1flutter_MainActivity_onFilePicked<'local>(
    mut env: jni::EnvUnowned<'local>,
    _this: jni::objects::JObject<'local>,
    path: jni::objects::JString<'local>,
) {
    init_logging();
    match read_jstring(&mut env, &path) {
        Some(p) if !p.is_empty() => {
            log::info!("收到 MIDI 文件: {p}");
            scene::open_midi(p);
        }
        Some(_) => {}
        None => log::error!("onFilePicked: 读取路径失败"),
    }
}

/// `MainActivity.onSoundFontPicked(path)`：音色库（SF2/SF3）已复制到私有目录。
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_jieneng_yinhe_1flutter_MainActivity_onSoundFontPicked<'local>(
    mut env: jni::EnvUnowned<'local>,
    _this: jni::objects::JObject<'local>,
    path: jni::objects::JString<'local>,
) {
    init_logging();
    match read_jstring(&mut env, &path) {
        Some(p) if !p.is_empty() => {
            log::info!("收到音色库: {p}");
            audio::set_soundfont(p.clone());
            scene::save_soundfont_setting(&p);
        }
        Some(_) => {}
        None => log::error!("onSoundFontPicked: 读取路径失败"),
    }
}

/// 从 JNI 字符串读取 Rust String（失败返回 None）。
fn read_jstring(env: &mut jni::EnvUnowned<'_>, s: &jni::objects::JString<'_>) -> Option<String> {
    use jni::Outcome;
    let outcome =
        env.with_env(|env| -> jni::errors::Result<String> { Ok(s.mutf8_chars(env)?.to_string()) });
    match outcome.into_outcome() {
        Outcome::Ok(text) => Some(text),
        Outcome::Err(e) => {
            log::error!("JNI 字符串读取失败: {e}");
            None
        }
        Outcome::Panic(p) => {
            log::error!("JNI 字符串读取 panic: {p:?}");
            None
        }
    }
}

/// `MainActivity.nativeInitContext(context)`：初始化 `ndk-context`。
///
/// cpal 的 Android 后端（AAudio）需要 JavaVM + `android.content.Context`；
/// 传入 `applicationContext`（Application 单例由系统持有，无需额外全局引用）。
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_jieneng_yinhe_1flutter_MainActivity_nativeInitContext<'local>(
    mut env: jni::EnvUnowned<'local>,
    _this: jni::objects::JObject<'local>,
    context: jni::objects::JObject<'local>,
    files_dir: jni::objects::JString<'local>,
) {
    init_logging();
    use jni::Outcome;
    // 私有目录：最近文件持久化用。
    if let Some(dir) = read_jstring(&mut env, &files_dir)
        && !dir.is_empty()
    {
        scene::set_files_dir(dir);
    }
    let outcome = env.with_env(|env| -> jni::errors::Result<usize> {
        Ok(env.get_java_vm()?.get_raw() as usize)
    });
    match outcome.into_outcome() {
        Outcome::Ok(vm_ptr) => {
            let ctx_ptr = context.as_raw() as usize;
            static INIT: std::sync::Once = std::sync::Once::new();
            INIT.call_once(|| {
                // 安全性：vm/context 由 JVM 提供；applicationContext 在进程
                // 生命周期内常驻，不会被子 GC 回收。
                unsafe {
                    ndk_context::initialize_android_context(
                        vm_ptr as *mut std::ffi::c_void,
                        ctx_ptr as *mut std::ffi::c_void,
                    );
                }
                log::info!("ndk-context 已初始化");
            });
        }
        Outcome::Err(e) => log::error!("ndk-context 初始化失败: {e}"),
        Outcome::Panic(p) => log::error!("ndk-context 初始化 panic: {p:?}"),
    }
}

// ── dart:ffi 入口（Dart → Rust）──

/// 触摸事件：`kind` 0=down 1=move 2=up，`x`/`y` 为画布逻辑像素。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_touch(kind: i32, x: f32, y: f32, pointer: i32) {
    input::on_touch(kind, x, y, pointer);
}

/// 载入内置演示文档（阶段 B 临时入口；SAF 打开文件接入后移除）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_demo_load() {
    init_logging();
    scene::load_demo();
}

/// 切换视图：0=AR 走带，1=PR 卷帘。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_set_view_mode(mode: i32) {
    if let Ok(mut s) = scene::scene().lock() {
        s.view_mode = if mode == 0 {
            scene::ViewMode::Arrangement
        } else {
            scene::ViewMode::Pianoroll
        };
    }
}

/// 走带控制：0=播放，1=暂停，2=停止。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_transport(cmd: i32) {
    match cmd {
        0 => audio::play(),
        1 => audio::pause(),
        2 => audio::stop(),
        _ => {}
    }
}

/// 是否正在播放（0/1）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_is_playing() -> i32 {
    i32::from(audio::is_playing())
}

/// 当前播放位置（秒，UI 时间显示用）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_playback_seconds() -> f64 {
    audio::playback_seconds()
}

/// 播放光标在当前视图下的 x（逻辑像素；-1 = 无文档）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_playhead_x() -> f32 {
    scene::playhead_x()
}

/// 播放位置（tick，时间码显示用）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_playhead_tick() -> f64 {
    audio::playback_tick()
}

/// AR 长按排序：把 `from` 轨移动到 `to` 位置，成功返回 1。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_move_track(from: i32, to: i32) -> i32 {
    if from < 0 || to < 0 {
        return 0;
    }
    scene::move_track(from as usize, to as usize)
}

/// 视图快照（Dart 文本浮层用）：写入 12 个 f32，成功返回 1。
#[allow(clippy::not_unsafe_ptr_arg_deref)]
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_view_snapshot(out: *mut f32) -> i32 {
    if out.is_null() {
        return 0;
    }
    let snap = scene::view_snapshot();
    unsafe {
        std::ptr::copy_nonoverlapping(snap.as_ptr(), out, snap.len());
    }
    1
}

/// 最近文件 JSON（堆分配，调用方用 `yinhe_free_string` 释放）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_recent_json() -> *mut std::ffi::c_char {
    to_c_string(scene::recent_json())
}

/// 轨道面板 JSON（堆分配，调用方用 `yinhe_free_string` 释放）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_tracks_json() -> *mut std::ffi::c_char {
    to_c_string(scene::tracks_json())
}

/// PR 自动化栏数据（仅 PB/CC/RPN）JSON（堆分配）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_automation_json(track: i32) -> *mut std::ffi::c_char {
    to_c_string(scene::automation_json(track.max(0) as usize))
}

/// 力度条数据 JSON（堆分配）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_velocity_json(track: i32) -> *mut std::ffi::c_char {
    to_c_string(scene::velocity_json(track.max(0) as usize))
}

/// 编辑自动化锚点：track / lane_index(1-based) / tick / value(0..1)。成功 1。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_set_automation_value(
    track: i32,
    lane_index: i32,
    tick: u32,
    value: f32,
) -> i32 {
    if track < 0 || lane_index <= 0 {
        return 0;
    }
    scene::set_automation_value(track as usize, lane_index as usize, tick, value)
}

/// 当前音色库路径（未选择则空串，堆分配）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_soundfont_path() -> *mut std::ffi::c_char {
    to_c_string(audio::soundfont_path())
}

/// 歌曲标题（空串 = 未设置，堆分配）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_song_title() -> *mut std::ffi::c_char {
    to_c_string(scene::song_title())
}

/// 设置“显示轨道”过滤（-1 = 全部轨道）。成功 1。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_set_display_track(track: i32) -> i32 {
    scene::set_display_track(track)
}

/// 撤销一步；成功 1。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_undo() -> i32 {
    scene::undo()
}

/// 重做一步；成功 1。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_redo() -> i32 {
    scene::redo()
}

/// Flutter 帧驱动：请求渲染一帧并等待完成（纹理与浮层同帧产出）。成功 1。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_render_frame() -> i32 {
    let (ack_tx, ack_rx) = std::sync::mpsc::sync_channel::<()>(1);
    if cmd_tx().send(Cmd::RenderNow(ack_tx)).is_err() {
        return 0;
    }
    if ack_rx.recv().is_err() {
        return 0;
    }
    1
}

/// 释放本库返回的堆字符串。
#[allow(clippy::not_unsafe_ptr_arg_deref)]
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_free_string(ptr: *mut std::ffi::c_char) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        drop(CString::from_raw(ptr));
    }
}

/// 打开指定路径的 MIDI（最近文件列表点击用）。
#[allow(clippy::not_unsafe_ptr_arg_deref)]
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_open_path(path: *const std::ffi::c_char) {
    if path.is_null() {
        return;
    }
    let path = unsafe { CStr::from_ptr(path) };
    match path.to_str() {
        Ok(p) => scene::open_midi(p.to_string()),
        Err(e) => log::error!("打开路径无效: {e}"),
    }
}

fn to_c_string(s: String) -> *mut std::ffi::c_char {
    match CString::new(s) {
        Ok(c) => c.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}
