//! 插件目录扫描。
//!
//! 只加载 entry、读取 factory 元数据，不实例化处理组件。
//!
//! 安全性说明：`PluginEntry::load` 会执行插件动态库的初始化代码，
//! 恶意/损坏插件理论上可以让进程崩溃。第一期在进程内扫描（崩溃即整体退出，
//! 用户重扫时把该路径加入黑名单跳过）；后续阶段把 `scan_path` 包一层
//! 子进程（同一 exe 加 --scan-plugin 参数）即可，本模块 API 不需要变。

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use clack_host::entry::PluginEntry;

use crate::describe::PluginInfo;
use crate::error::PluginError;

/// 各平台默认 CLAP 搜索目录（存在的才返回）。
pub fn default_plugin_dirs() -> Vec<PathBuf> {
    let candidates: Vec<PathBuf> = {
        #[cfg(target_os = "windows")]
        {
            vec![PathBuf::from(r"C:\Program Files\Common Files\CLAP")]
        }
        #[cfg(target_os = "macos")]
        {
            let mut v = vec![PathBuf::from("/Library/Audio/Plug-Ins/CLAP")];
            if let Some(home) = std::env::var_os("HOME") {
                v.push(PathBuf::from(home).join("Library/Audio/Plug-Ins/CLAP"));
            }
            v
        }
        #[cfg(target_os = "linux")]
        {
            let mut v = vec![PathBuf::from("/usr/lib/clap")];
            if let Some(home) = std::env::var_os("HOME") {
                v.push(PathBuf::from(home).join(".clap"));
            }
            v
        }
    };
    candidates.into_iter().filter(|p| p.is_dir()).collect()
}

/// 扫描单个 .clap 包（文件或 bundle 目录），返回其中全部插件的描述。
pub fn scan_path(path: &Path) -> Result<Vec<PluginInfo>, PluginError> {
    // SAFETY: 加载外部动态库本身是 unsafe 的（见模块文档安全性说明）。
    let entry = unsafe { PluginEntry::load(path.as_os_str())? };
    let factory = entry.get_plugin_factory().ok_or_else(|| {
        PluginError::PluginIdNotFound(format!("{}: 无 plugin factory", path.display()))
    })?;

    let mut infos = Vec::new();
    for descriptor in factory.plugin_descriptors() {
        let Some(id) = descriptor.id() else { continue };
        let Some(name) = descriptor.name() else {
            continue;
        };
        infos.push(PluginInfo {
            path: path.to_path_buf(),
            id: id.to_string_lossy().into_owned(),
            name: name.to_string_lossy().into_owned(),
            vendor: descriptor
                .vendor()
                .map(|v| v.to_string_lossy().into_owned()),
            version: descriptor
                .version()
                .map(|v| v.to_string_lossy().into_owned()),
            features: descriptor
                .features()
                .map(|f| f.to_string_lossy().into_owned())
                .collect(),
        });
    }
    Ok(infos)
}

/// 扫描结果条目：成功的插件列表或失败原因（供黑名单记录）。
#[derive(Debug)]
pub enum ScanOutcome {
    Loaded(Vec<PluginInfo>),
    Failed { path: PathBuf, error: PluginError },
}

/// 扫描一个目录（含一层子目录，macOS bundle 是目录）。
/// 单个包失败不影响其他包。
pub fn scan_dir(dir: &Path) -> Vec<ScanOutcome> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut outcomes = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension() != Some(OsStr::new("clap")) {
            continue;
        }
        let outcome = match scan_path(&path) {
            Ok(infos) => ScanOutcome::Loaded(infos),
            Err(error) => ScanOutcome::Failed { path, error },
        };
        outcomes.push(outcome);
    }
    outcomes
}

/// 扫描全部给定目录并汇总。
pub fn scan_dirs(dirs: &[PathBuf]) -> Vec<ScanOutcome> {
    dirs.iter().flat_map(|d| scan_dir(d)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_missing_dir_returns_empty() {
        assert!(scan_dir(Path::new("/nonexistent-yinhe-dir")).is_empty());
    }

    #[test]
    fn scan_non_plugin_files_skipped() {
        let dir = std::env::temp_dir().join("yinhe-clap-scan-test");
        std::fs::create_dir_all(&dir).expect("create temp dir");
        std::fs::write(dir.join("not-a-plugin.txt"), b"x").expect("write");
        assert!(scan_dir(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
