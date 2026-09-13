//! Linux VST3 模块加载（dlopen）。
//!
//! VST3 在 Linux 上没有 Init/Exit 导出约定，直接 dlopen bundle 内二进制
//! 并取 `GetPluginFactory`。

use std::path::Path;

use libloading::{Library, Symbol};
use vst3::Steinberg::IPluginFactory;

use super::{LoadError, resolve_binary};

type GetPluginFactoryFunc = unsafe extern "C" fn() -> *mut IPluginFactory;

/// Linux 加载的 VST3 模块；drop 时 dlclose。
pub(super) struct LinuxModule {
    #[allow(dead_code)] // 持有到 drop，保证 factory 指针有效
    library: Library,
}

impl LinuxModule {
    pub(super) fn load(bundle: &Path) -> Result<(Self, *mut IPluginFactory), LoadError> {
        let binary = resolve_binary(bundle)?;
        unsafe {
            let library = Library::new(&binary)
                .map_err(|e| LoadError::Failed(format!("dlopen {} 失败: {e}", binary.display())))?;
            let factory = {
                let symbol: Symbol<GetPluginFactoryFunc> = library
                    .get(b"GetPluginFactory\0")
                    .map_err(|_| LoadError::MissingFactoryExport)?;
                let get_factory = *symbol;
                get_factory()
            };
            if factory.is_null() {
                return Err(LoadError::Failed("GetPluginFactory 返回空".into()));
            }
            Ok((Self { library }, factory))
        }
    }
}
