//! Windows VST3 模块加载（LoadLibrary）。
//!
//! VST3 规范：`InitDll` 在 `GetPluginFactory` 前调用（若导出），
//! `ExitDll` 在卸载前调用（若导出）。两者都是可选符号。

use std::path::Path;

use libloading::{Library, Symbol};
use vst3::Steinberg::IPluginFactory;

use super::{LoadError, resolve_binary};

type InitDllFunc = unsafe extern "C" fn() -> bool;
type ExitDllFunc = unsafe extern "C" fn() -> bool;
type GetPluginFactoryFunc = unsafe extern "C" fn() -> *mut IPluginFactory;

/// Windows 加载的 VST3 模块；drop 时 ExitDll + FreeLibrary。
pub(super) struct WindowsModule {
    #[allow(dead_code)] // 持有到 drop，保证 factory 指针有效
    library: Library,
    exit_dll: Option<ExitDllFunc>,
}

impl WindowsModule {
    pub(super) fn load(bundle: &Path) -> Result<(Self, *mut IPluginFactory), LoadError> {
        let binary = resolve_binary(bundle)?;
        unsafe {
            let library = Library::new(&binary).map_err(|e| {
                LoadError::Failed(format!("LoadLibrary {} 失败: {e}", binary.display()))
            })?;
            // InitDll（可选）：拿到函数指针后立即调用。
            let init_dll: Option<InitDllFunc> = library
                .get(b"InitDll\0")
                .map(|s: Symbol<InitDllFunc>| *s)
                .ok();
            if let Some(init) = init_dll {
                let _ = init();
            }
            let exit_dll: Option<ExitDllFunc> = library
                .get(b"ExitDll\0")
                .map(|s: Symbol<ExitDllFunc>| *s)
                .ok();
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
            Ok((Self { library, exit_dll }, factory))
        }
    }
}

impl Drop for WindowsModule {
    fn drop(&mut self) {
        if let Some(exit) = self.exit_dll.take() {
            unsafe {
                let _ = exit();
            }
        }
        // library 随后由字段 drop 自动 FreeLibrary。
    }
}
