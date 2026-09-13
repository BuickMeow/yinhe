//! VST3 模块加载：从 `.vst3` bundle 拿到 `IPluginFactory`。
//!
//! 平台差异（VST3 规范）：
//! - macOS：必须走 CFBundle 流程（`CFBundleLoadExecutable` → `bundleEntry` →
//!   `GetPluginFactory` → drop 时 `bundleExit`），`bundleEntry`/`bundleExit` 是必需导出；
//! - Windows：`LoadLibrary` → `InitDll`（可选）→ `GetPluginFactory`，
//!   drop 时 `ExitDll` → `FreeLibrary`；
//! - Linux：`dlopen` → `GetPluginFactory`。
//!
//! `LoadedModule` 持有模块句柄与 factory 裸指针（factory 由模块拥有，
//! 生命周期同模块；宿主不增减其引用计数）。

use std::path::Path;

use vst3::Steinberg::IPluginFactory;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

/// 模块加载失败。
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),
    #[error("加载失败: {0}")]
    Failed(String),
    #[error("模块未导出 GetPluginFactory")]
    MissingFactoryExport,
}

/// 已加载的 VST3 模块。drop 时卸载模块（平台相关的清理流程）。
pub struct LoadedModule {
    /// 平台模块句柄：只为在 `LoadedModule` 存活期间保持动态库加载，不直接读取。
    #[cfg(target_os = "macos")]
    _inner: macos::MacOsModule,
    #[cfg(target_os = "windows")]
    _inner: windows::WindowsModule,
    #[cfg(target_os = "linux")]
    _inner: linux::LinuxModule,
    factory: *mut IPluginFactory,
}

impl LoadedModule {
    /// 加载 bundle 并调用 `GetPluginFactory`。
    pub fn load(bundle: &Path) -> Result<Self, LoadError> {
        #[cfg(target_os = "macos")]
        let (inner, factory) = macos::MacOsModule::load(bundle)?;
        #[cfg(target_os = "windows")]
        let (inner, factory) = windows::WindowsModule::load(bundle)?;
        #[cfg(target_os = "linux")]
        let (inner, factory) = linux::LinuxModule::load(bundle)?;
        Ok(Self {
            _inner: inner,
            factory,
        })
    }

    /// factory 裸指针；有效期同本模块。
    pub fn factory_ptr(&self) -> *mut IPluginFactory {
        self.factory
    }
}

/// 解析 bundle 内的动态库二进制路径（Windows/Linux 用；macOS 走 CFBundle 不需要）。
///
/// bundle 目录布局：`Contents/<arch-dir>/<binary>`；单文件布局（旧 Windows）直接用原路径。
#[cfg(any(target_os = "windows", target_os = "linux"))]
pub(crate) fn resolve_binary(bundle: &Path) -> Result<std::path::PathBuf, LoadError> {
    if bundle.is_file() {
        return Ok(bundle.to_path_buf());
    }
    // 各平台可能的架构目录（同一 bundle 通常只有一个存在）。
    const ARCH_DIRS: &[&str] = &[
        "x86_64-win",
        "x86-win",
        "arm64-win",
        "x86_64-linux",
        "aarch64-linux",
        "arm64-linux",
        "i386-linux",
    ];
    for dir in ARCH_DIRS {
        let candidate = bundle.join("Contents").join(dir);
        let Ok(entries) = std::fs::read_dir(&candidate) else {
            continue;
        };
        // 目录内通常是单个二进制文件（可能与 bundle 同名）。
        if let Some(file) = entries.flatten().map(|e| e.path()).find(|p| p.is_file()) {
            return Ok(file);
        }
    }
    Err(LoadError::Failed(format!(
        "未找到 bundle 内的动态库: {}",
        bundle.display()
    )))
}
