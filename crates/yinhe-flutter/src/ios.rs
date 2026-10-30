//! iOS 外部纹理（CVPixelBuffer / IOSurface）呈现端。
//!
//! iOS 没有 `ANativeWindow` 那种由系统缓冲的生产端 surface，Flutter 的纹理是
//! 由宿主（`YinheTexture`，见 `ios/Runner/YinheTexture.swift`）持有的一组
//! `CVPixelBuffer`。链路：
//!
//! 1. 渲染线程每帧向宿主要一个「可写槽位」（`acquire`），拿到它的 `MTLTexture*`；
//! 2. 用 `wgpu-hal` 把该纹理包成 wgpu 纹理（零拷贝，不经过 CPU）；
//! 3. 画完后通知宿主 `submit`：宿主把它标成最新帧并 `textureFrameAvailable`，
//!    Flutter 下一次合成时 `copyPixelBuffer` 取走。
//!
//! 槽位选择由宿主负责（它知道 Flutter 当前正持有哪一块），Rust 只按 `(指针, 尺寸)`
//! 缓存包装结果——指针或尺寸变了就重建。

use std::ffi::c_void;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_metal::{MTLTexture, MTLTextureType};

/// 取一个可写槽位：返回 `MTLTexture*`，槽位下标写入 `slot`。失败返回 null。
pub type AcquireFn = unsafe extern "C" fn(kind: i32, w: u32, h: u32, slot: *mut i32) -> *mut c_void;

/// 提交一帧：宿主标记最新槽位并通知 Flutter 有新帧。
pub type SubmitFn = unsafe extern "C" fn(kind: i32, slot: i32);

static HOST: OnceLock<(AcquireFn, SubmitFn)> = OnceLock::new();

/// wgpu 使用的 MTLDevice 裸指针（宿主建 `CVMetalTextureCache` 必须用同一个设备）。
static METAL_DEVICE: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn set_metal_device(ptr: *mut c_void) {
    METAL_DEVICE.store(ptr as usize, Ordering::Release);
}

/// iOS 日志直接写 stderr（Xcode 控制台可见，无需 os_log 桥）。
pub(crate) fn init_logging() {
    struct Logger;

    impl log::Log for Logger {
        fn enabled(&self, _: &log::Metadata<'_>) -> bool {
            true
        }

        fn log(&self, record: &log::Record<'_>) {
            eprintln!("[yinhe {}] {}", record.level(), record.args());
        }

        fn flush(&self) {}
    }

    let _ = log::set_logger(&Logger);
    log::set_max_level(log::LevelFilter::Info);
}

/// 宿主登记回调（AppDelegate / YinhePlugin 在启动时调用一次）。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_ios_set_host(acquire: AcquireFn, submit: SubmitFn) {
    let _ = HOST.set((acquire, submit));
}

/// 宿主查询 wgpu 的 MTLDevice；渲染线程还没建好设备时返回 null。
#[unsafe(no_mangle)]
pub extern "C" fn yinhe_ios_metal_device() -> *mut c_void {
    METAL_DEVICE.load(Ordering::Acquire) as *mut c_void
}

/// 一个已包装的槽位（外部纹理 + 其渲染视图）。
struct Slot {
    /// 宿主的 `MTLTexture*`：变了就说明池被重建，需要重新包装。
    ptr: usize,
    /// 纹理尺寸：尺寸变化同样要重建。
    size: (u32, u32),
    view: wgpu::TextureView,
}

/// 一个外部纹理呈现端（主画布 / AM 面板各一个，靠 `kind` 区分）。
pub(crate) struct IosPresenter {
    kind: i32,
    /// 当前画布物理尺寸（由 `setSize` 上报）。
    size: (u32, u32),
    slots: [Option<Slot>; 3],
}

impl IosPresenter {
    pub(crate) fn new(kind: i32) -> Self {
        Self {
            kind,
            size: (1, 1),
            slots: [None, None, None],
        }
    }

    /// 画布物理尺寸（像素）；宿主下一次 acquire 时按它建池。
    pub(crate) fn size(&self) -> (u32, u32) {
        self.size
    }

    pub(crate) fn set_size(&mut self, w: u32, h: u32) {
        self.size = (w.max(1), h.max(1));
    }

    /// 向宿主要一帧并返回（槽位, 渲染视图）；宿主的池按需重建。
    pub(crate) fn acquire(&mut self, gpu: &crate::Gpu) -> Option<(i32, wgpu::TextureView)> {
        let (w, h) = self.size;
        let (acquire, _) = HOST.get()?;
        let mut slot = -1i32;
        // SAFETY: 回调由宿主提供，参数只是尺寸/出参，指针生命周期由宿主保证到 submit。
        let ptr = unsafe { acquire(self.kind, w, h, &mut slot) };
        if ptr.is_null() || !(0..self.slots.len() as i32).contains(&slot) {
            return None;
        }
        let i = slot as usize;
        let reuse = matches!(&self.slots[i], Some(s) if s.ptr == ptr as usize && s.size == (w, h));
        if !reuse {
            self.slots[i] = Some(Slot {
                ptr: ptr as usize,
                size: (w, h),
                view: wrap_texture(gpu, ptr, w, h)?,
            });
        }
        self.slots[i].as_ref().map(|s| (slot, s.view.clone()))
    }

    /// 通知宿主这一槽位已经画完。
    pub(crate) fn submit(&self, slot: i32) {
        if let Some((_, submit)) = HOST.get() {
            // SAFETY: 回调由宿主提供，参数只是下标。
            unsafe { submit(self.kind, slot) };
        }
    }
}

/// 把宿主的 `MTLTexture*` 包成 wgpu 纹理的渲染视图（sRGB 视图，与 Android 的
/// srgb surface 格式一致，颜色不会偏亮/偏暗）。
fn wrap_texture(gpu: &crate::Gpu, ptr: *mut c_void, w: u32, h: u32) -> Option<wgpu::TextureView> {
    // SAFETY: 指针是宿主 CVPixelBuffer 对应的 MTLTexture，且宿主保证它至少活到
    // 下一次 acquire 该槽位；这里 +1 引用后由 wgpu 纹理持有。
    let raw: Retained<ProtocolObject<dyn MTLTexture>> =
        unsafe { Retained::retain(ptr.cast::<ProtocolObject<dyn MTLTexture>>()) }?;
    let hal_texture = unsafe {
        wgpu::hal::metal::Device::texture_from_raw(
            raw,
            wgpu::TextureFormat::Bgra8Unorm,
            MTLTextureType::Type2D,
            1,
            1,
            wgpu::hal::CopyExtent {
                width: w,
                height: h,
                depth: 1,
            },
            None,
        )
    };
    // SAFETY: `hal_texture` 来自上面同一个设备包装的 MTLTexture，尺寸/格式与 desc 一致。
    let texture = unsafe {
        gpu.device.create_texture_from_hal::<wgpu::hal::api::Metal>(
            hal_texture,
            &wgpu::TextureDescriptor {
                label: Some("yinhe_ios_texture"),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Bgra8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[wgpu::TextureFormat::Bgra8UnormSrgb],
            },
            wgpu::wgt::TextureUses::COLOR_TARGET,
        )
    };
    Some(texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("yinhe_ios_texture_view"),
        format: Some(wgpu::TextureFormat::Bgra8UnormSrgb),
        ..Default::default()
    }))
}
