//! VST3 插件目录扫描与元数据解析。
//!
//! 只读文件系统 + `moduleinfo.json`，**不加载任何二进制**（加载在后续模块实现）。
//! 无 `moduleinfo.json` 的旧插件产出一个 `needs_factory` 占位条目：当前不可加载，
//! 待 factory 枚举实现后补全。
//!
//! 返回粒度：一个 `.vst3` bundle 可能声明多个类（多个插件），
//! [`ScanOutcome::Loaded`] 携带该 bundle 的全部插件。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::factory;
use crate::loader::LoadedModule;
use crate::moduleinfo::read_moduleinfo;

/// 一个可加载的 VST3 插件类（UI 插件浏览器的一行）。
#[derive(Clone, Debug, PartialEq)]
pub struct PluginInfo {
    /// `.vst3` bundle 路径（加载时用）。
    pub path: PathBuf,
    /// 类 ID（32 位十六进制）；`needs_factory` 占位为空串。
    pub class_id: String,
    pub name: String,
    pub vendor: String,
    pub version: String,
    pub is_instrument: bool,
    pub is_effect: bool,
    /// 无 `moduleinfo.json` 的旧插件：需要加载 factory 枚举类信息（暂不可加载）。
    pub needs_factory: bool,
}

/// 单个 `.vst3` bundle 的扫描结果。
pub enum ScanOutcome {
    /// 扫描成功：该 bundle 的全部插件类（可能为空——见 `Failed`）。
    Loaded(Vec<PluginInfo>),
    Failed {
        path: PathBuf,
        error: String,
    },
}

/// 三平台默认 VST3 目录（与 SDK 安装约定一致）。
pub fn default_plugin_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    #[cfg(target_os = "macos")]
    {
        dirs.push(PathBuf::from("/Library/Audio/Plug-Ins/VST3"));
        if let Some(home) = dirs::home_dir() {
            dirs.push(home.join("Library/Audio/Plug-Ins/VST3"));
        }
    }
    #[cfg(target_os = "windows")]
    {
        dirs.push(PathBuf::from(r"C:\Program Files\Common Files\VST3"));
        dirs.push(PathBuf::from(r"C:\Program Files (x86)\Common Files\VST3"));
    }
    #[cfg(target_os = "linux")]
    {
        dirs.push(PathBuf::from("/usr/lib/vst3"));
        dirs.push(PathBuf::from("/usr/local/lib/vst3"));
        if let Some(home) = dirs::home_dir() {
            dirs.push(home.join(".vst3"));
        }
    }
    dirs
}

/// 扫描目录集合：递归查找 `.vst3`（不深入 bundle 内部），逐个解析元数据。
pub fn scan_dirs(dirs: &[PathBuf]) -> Vec<ScanOutcome> {
    let mut bundles = Vec::new();
    let mut visited = HashSet::new();
    for dir in dirs {
        if dir.is_dir() {
            collect_bundles(dir, &mut bundles, &mut visited);
        }
    }
    bundles.sort();
    bundles.dedup();
    bundles.iter().map(|b| scan_bundle(b)).collect()
}

/// 递归收集 `.vst3` bundle；`visited`（canonical 路径）防御符号链接循环。
fn collect_bundles(dir: &Path, out: &mut Vec<PathBuf>, visited: &mut HashSet<PathBuf>) {
    let Ok(real) = dir.canonicalize() else {
        return;
    };
    if !visited.insert(real) {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "vst3") {
            // 是 bundle（目录）或单文件（Windows DLL）：收集，不再深入。
            out.push(path);
        } else if path.is_dir() {
            collect_bundles(&path, out, visited);
        }
    }
}

/// 解析单个 bundle 的元数据。
fn scan_bundle(path: &Path) -> ScanOutcome {
    match read_moduleinfo(path) {
        Ok(Some(info)) => {
            let plugins: Vec<PluginInfo> = info
                .classes
                .iter()
                .map(|c| PluginInfo {
                    path: path.to_path_buf(),
                    class_id: c.class_id.clone(),
                    name: c.name.clone(),
                    vendor: c.vendor.clone(),
                    version: c.version.clone(),
                    is_instrument: c.is_instrument,
                    is_effect: c.is_effect,
                    needs_factory: false,
                })
                .collect();
            if plugins.is_empty() {
                ScanOutcome::Failed {
                    path: path.to_path_buf(),
                    error: "moduleinfo 中未找到音频处理器类".into(),
                }
            } else {
                ScanOutcome::Loaded(plugins)
            }
        }
        // 无 moduleinfo：旧插件，加载 factory 枚举类信息。
        Ok(None) => scan_with_factory(path),
        Err(e) => ScanOutcome::Failed {
            path: path.to_path_buf(),
            error: e.to_string(),
        },
    }
}

/// 无 moduleinfo 的插件：加载 factory 枚举。
///
/// 注意：这会**进程内加载第三方二进制**（与 CLAP 扫描同一风险模型：损坏/恶意
/// 插件可能杀死宿主进程）。后续阶段将改为子进程隔离扫描。
fn scan_with_factory(path: &Path) -> ScanOutcome {
    let module = match LoadedModule::load(path) {
        Ok(module) => module,
        Err(e) => {
            return ScanOutcome::Failed {
                path: path.to_path_buf(),
                error: format!("加载模块失败: {e}"),
            };
        }
    };
    // SAFETY: factory 指针由模块拥有且生命周期覆盖本函数；ComRef 借用不增减引用。
    let Some(factory) = (unsafe { vst3::ComRef::from_raw(module.factory_ptr()) }) else {
        return ScanOutcome::Failed {
            path: path.to_path_buf(),
            error: "factory 指针为空".into(),
        };
    };
    let info = factory::factory_info(&factory);
    let classes = factory::enumerate_classes(&factory);
    if classes.is_empty() {
        return ScanOutcome::Failed {
            path: path.to_path_buf(),
            error: "factory 未导出音频处理器类".into(),
        };
    }
    ScanOutcome::Loaded(
        classes
            .into_iter()
            .map(|c| PluginInfo {
                path: path.to_path_buf(),
                class_id: c.class_id,
                name: c.name,
                vendor: if c.vendor.is_empty() {
                    info.vendor.clone()
                } else {
                    c.vendor
                },
                version: c.version,
                is_instrument: c.is_instrument,
                is_effect: c.is_effect,
                needs_factory: false,
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODULEINFO: &str = r#"{
        "Name": "DemoBundle",
        "Version": "1.0",
        "Factory Info": { "Vendor": "DemoVendor" },
        "Classes": [
            {
                "CID": "ABCDEF0123456789ABCDEF0123456789",
                "Category": "Audio Module Class",
                "Name": "Demo Synth",
                "Vendor": "DemoVendor",
                "Version": "1.0",
                "Sub Categories": ["Instrument", "Synth"]
            },
            {
                "CID": "11111111111111111111111111111111",
                "Category": "Audio Module Class",
                "Name": "Demo FX",
                "Vendor": "DemoVendor",
                "Version": "1.0",
                "Sub Categories": ["Fx"]
            }
        ]
    }"#;

    fn write_bundle(root: &Path, name: &str, moduleinfo: Option<&str>) -> PathBuf {
        let bundle = root.join(format!("{name}.vst3"));
        let resources = bundle.join("Contents/Resources");
        std::fs::create_dir_all(&resources).expect("mkdir");
        if let Some(json) = moduleinfo {
            std::fs::write(resources.join("moduleinfo.json"), json).expect("write");
        }
        bundle
    }

    #[test]
    fn scans_bundle_with_multiple_classes() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_bundle(dir.path(), "Demo", Some(MODULEINFO));
        let outcomes = scan_dirs(&[dir.path().to_path_buf()]);
        assert_eq!(outcomes.len(), 1);
        let ScanOutcome::Loaded(plugins) = &outcomes[0] else {
            panic!("expected loaded");
        };
        assert_eq!(plugins.len(), 2);
        assert!(plugins[0].is_instrument || plugins[1].is_instrument);
        assert!(plugins.iter().all(|p| !p.needs_factory));
    }

    #[test]
    fn bundle_without_moduleinfo_fails_without_binary() {
        // 假 bundle 内没有真实二进制：factory 加载失败 → Failed。
        // （真实无 moduleinfo 插件会走 factory 枚举，见 loader/factory 模块。）
        let dir = tempfile::tempdir().expect("tempdir");
        write_bundle(dir.path(), "OldPlugin", None);
        let outcomes = scan_dirs(&[dir.path().to_path_buf()]);
        assert_eq!(outcomes.len(), 1);
        assert!(matches!(outcomes[0], ScanOutcome::Failed { .. }));
    }

    #[test]
    fn invalid_moduleinfo_is_failure() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_bundle(dir.path(), "Broken", Some("{ not json"));
        let outcomes = scan_dirs(&[dir.path().to_path_buf()]);
        assert_eq!(outcomes.len(), 1);
        assert!(matches!(outcomes[0], ScanOutcome::Failed { .. }));
    }

    #[test]
    fn scans_nested_directories() {
        let dir = tempfile::tempdir().expect("tempdir");
        let vendor = dir.path().join("VendorX");
        std::fs::create_dir_all(&vendor).expect("mkdir");
        write_bundle(&vendor, "Nested", Some(MODULEINFO));
        let outcomes = scan_dirs(&[dir.path().to_path_buf()]);
        assert_eq!(outcomes.len(), 1);
        assert!(matches!(outcomes[0], ScanOutcome::Loaded(_)));
    }

    #[test]
    fn missing_dir_is_skipped() {
        let outcomes = scan_dirs(&[PathBuf::from("/nonexistent/vst3/dir/xyz")]);
        assert!(outcomes.is_empty());
    }
}
