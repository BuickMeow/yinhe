//! macOS VST3 模块加载（CFBundle + bundleEntry/bundleExit）。
//!
//! VST3 规范要求 macOS bundle 导出 `bundleEntry`/`bundleExit` 且宿主必须调用；
//! JUCE 等框架的初始化依赖这一流程。

use std::ffi::CString;
use std::path::Path;
use std::ptr;

use core_foundation::base::{Boolean, CFRelease, CFRetain, CFTypeRef, TCFType};
use core_foundation::bundle::{
    CFBundleCreate, CFBundleGetFunctionPointerForName, CFBundleLoadExecutable, CFBundleRef,
    CFBundleUnloadExecutable,
};
use core_foundation::string::CFString;
use core_foundation::url::CFURLCreateFromFileSystemRepresentation;
use vst3::Steinberg::IPluginFactory;

use super::LoadError;

type BundleEntryFunc = unsafe extern "C" fn(bundle: CFBundleRef) -> Boolean;
type BundleExitFunc = unsafe extern "C" fn() -> Boolean;
type GetPluginFactoryFunc = unsafe extern "C" fn() -> *mut IPluginFactory;

/// macOS 加载的 VST3 模块；drop 时 bundleExit + unload + release。
pub(super) struct MacOsModule {
    bundle: CFBundleRef,
    bundle_exit: Option<BundleExitFunc>,
}

impl MacOsModule {
    /// 按规范顺序加载并返回 `(模块, factory)`。
    pub(super) fn load(path: &Path) -> Result<(Self, *mut IPluginFactory), LoadError> {
        unsafe {
            let path_cstring = CString::new(path.to_string_lossy().as_bytes())
                .map_err(|e| LoadError::Failed(format!("非法路径: {e}")))?;
            let url = CFURLCreateFromFileSystemRepresentation(
                ptr::null_mut(),
                path_cstring.as_ptr() as *const u8,
                path_cstring.as_bytes().len() as isize,
                1, // isDirectory：VST3 bundle 是目录
            );
            if url.is_null() {
                return Err(LoadError::Failed("创建 bundle URL 失败".into()));
            }
            let bundle = CFBundleCreate(ptr::null_mut(), url);
            CFRelease(url as CFTypeRef);
            if bundle.is_null() {
                return Err(LoadError::Failed("创建 CFBundle 失败".into()));
            }

            if CFBundleLoadExecutable(bundle) == 0 {
                CFRelease(bundle as CFTypeRef);
                return Err(LoadError::Failed(
                    "加载 bundle 可执行文件失败（架构不匹配？）".into(),
                ));
            }

            // bundleEntry / bundleExit：VST3 规范要求的必需导出。
            let entry_name = CFString::new("bundleEntry");
            let entry_ptr =
                CFBundleGetFunctionPointerForName(bundle, entry_name.as_concrete_TypeRef());
            if entry_ptr.is_null() {
                unload_after_failure(bundle);
                return Err(LoadError::Failed("bundle 未导出 bundleEntry".into()));
            }
            let bundle_entry: BundleEntryFunc = std::mem::transmute(entry_ptr);

            let exit_name = CFString::new("bundleExit");
            let exit_ptr =
                CFBundleGetFunctionPointerForName(bundle, exit_name.as_concrete_TypeRef());
            if exit_ptr.is_null() {
                unload_after_failure(bundle);
                return Err(LoadError::Failed("bundle 未导出 bundleExit".into()));
            }
            let bundle_exit: BundleExitFunc = std::mem::transmute(exit_ptr);

            // bundleEntry 必须在 GetPluginFactory 之前调用。
            CFRetain(bundle as CFTypeRef);
            if bundle_entry(bundle) == 0 {
                CFRelease(bundle as CFTypeRef);
                unload_after_failure(bundle);
                return Err(LoadError::Failed("bundleEntry 返回失败".into()));
            }

            let factory_name = CFString::new("GetPluginFactory");
            let factory_sym =
                CFBundleGetFunctionPointerForName(bundle, factory_name.as_concrete_TypeRef());
            if factory_sym.is_null() {
                let _ = bundle_exit();
                CFRelease(bundle as CFTypeRef); // bundleEntry 的 retain
                unload_after_failure(bundle);
                return Err(LoadError::MissingFactoryExport);
            }
            let get_factory: GetPluginFactoryFunc = std::mem::transmute(factory_sym);
            let factory = get_factory();
            if factory.is_null() {
                let _ = bundle_exit();
                CFRelease(bundle as CFTypeRef);
                unload_after_failure(bundle);
                return Err(LoadError::Failed("GetPluginFactory 返回空".into()));
            }

            Ok((
                Self {
                    bundle,
                    bundle_exit: Some(bundle_exit),
                },
                factory,
            ))
        }
    }
}

/// 加载中途失败：unload + release（未调用过 bundleEntry，无需 bundleExit）。
unsafe fn unload_after_failure(bundle: CFBundleRef) {
    unsafe {
        CFBundleUnloadExecutable(bundle);
        CFRelease(bundle as CFTypeRef);
    }
}

impl Drop for MacOsModule {
    fn drop(&mut self) {
        unsafe {
            if let Some(exit) = self.bundle_exit.take() {
                let _ = exit();
                // 释放 bundleEntry 的 retain。
                CFRelease(self.bundle as CFTypeRef);
            }
            CFBundleUnloadExecutable(self.bundle);
            CFRelease(self.bundle as CFTypeRef);
        }
    }
}
