//! 手动验证工具：扫描默认 VST3 目录并打印元数据。
//!
//! 运行：`cargo run -p yinhe-vst3 --example scan`

use yinhe_vst3::scan::{ScanOutcome, default_plugin_dirs, scan_dirs};

fn main() {
    let dirs = default_plugin_dirs();
    println!("扫描目录:");
    for d in &dirs {
        println!("  {} (存在: {})", d.display(), d.is_dir());
    }
    let outcomes = scan_dirs(&dirs);
    let mut plugins = 0;
    let mut failed = 0;
    for outcome in &outcomes {
        match outcome {
            ScanOutcome::Loaded(infos) => {
                for info in infos {
                    plugins += 1;
                    let kind = if info.needs_factory {
                        "旧插件(需factory)"
                    } else if info.is_instrument && info.is_effect {
                        "乐器+效果"
                    } else if info.is_instrument {
                        "乐器"
                    } else {
                        "效果器"
                    };
                    println!(
                        "[{kind}] {} / {} — {} ({})",
                        info.name,
                        info.vendor,
                        info.class_id,
                        if info.version.is_empty() {
                            "?"
                        } else {
                            &info.version
                        }
                    );
                }
            }
            ScanOutcome::Failed { path, error } => {
                failed += 1;
                println!("[失败] {}: {error}", path.display());
            }
        }
    }
    println!("\n共 {plugins} 个插件，{failed} 个包失败");
}
