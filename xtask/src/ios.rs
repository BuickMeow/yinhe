//! iOS 构建：Rust 静态库 → Flutter iOS App（模拟器 / 真机）。
//!
//! 流程（iOS 壳必须按此顺序，否则链接不到引擎）：
//! 1. `cargo rustc --release --lib --crate-type staticlib`（只出 .a，不出不可用的 dylib）；
//! 2. 复制 `libyinhe_flutter.a` 到 `flutter/ios/yinhe_engine/`（Runner 的
//!    xcconfig 用 `-force_load` 链进来；模拟器与真机是两份不同的 .a）；
//! 3. `flutter build ios`（模拟器走 `--simulator`）。
//!
//! 用法：
//!   cargo xtask ios                 # 模拟器 debug 包
//!   cargo xtask ios --release       # 模拟器 release 包
//!   cargo xtask ios --device        # 真机包（需要签名）
//!   cargo xtask ios --install       # 构建后装到已启动的模拟器并启动

use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::cmd;

/// 应用 Bundle ID（与 `ios/Runner.xcodeproj` 一致）。
const BUNDLE_ID: &str = "com.jieneng.yinhe";

pub fn run(args: &[String]) -> ExitCode {
    let mut release = false;
    let mut device = false;
    let mut install = false;
    for arg in args {
        match arg.as_str() {
            "--release" => release = true,
            "--device" => device = true,
            "--install" => install = true,
            "--help" | "-h" => {
                println!(
                    "用法: cargo xtask ios [--release] [--device] [--install]\n  \
                     --release  构建 release 包（默认 debug）\n  \
                     --device   构建真机包（默认模拟器）\n  \
                     --install  构建后通过 simctl 装到已启动的模拟器并启动"
                );
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("未知参数: {other}（--help 看用法）");
                return ExitCode::FAILURE;
            }
        }
    }
    match build(release, device, install) {
        Ok(()) => ExitCode::SUCCESS,
        Err(code) => code,
    }
}

fn build(release: bool, device: bool, install: bool) -> Result<(), ExitCode> {
    let root = workspace_root();
    let engine_dir = root.join("flutter/ios/yinhe_engine");
    std::fs::create_dir_all(&engine_dir).map_err(|e| {
        eprintln!("创建 {} 失败: {e}", engine_dir.display());
        ExitCode::FAILURE
    })?;

    // 1) Rust 引擎（release 与 Dart 的 buildMode 无关：一律 release，性能优先）。
    let target = if device {
        "aarch64-apple-ios"
    } else {
        "aarch64-apple-ios-sim"
    };
    let mut c = cmd::cargo();
    // 只编静态库：本 crate 同时声明了 cdylib（Android/鸿蒙用），但 iOS 不允许
    // app 内带 dylib，真机链接会失败，用 --crate-type 覆盖掉。
    c.args([
        "rustc",
        "--release",
        "--target",
        target,
        "-p",
        "yinhe-flutter",
        "--lib",
        "--crate-type",
        "staticlib",
    ]);
    // 与 Runner 的 IPHONEOS_DEPLOYMENT_TARGET 一致，避免 "built for newer iOS" 告警。
    c.env("IPHONEOS_DEPLOYMENT_TARGET", "15.0");
    c.current_dir(&root);
    cmd::status(&mut c)?;

    // 2) 拷静态库（模拟器与真机同名不同内容，靠文件名区分）。
    let name = if device {
        "libyinhe_flutter.a"
    } else {
        "libyinhe_flutter_sim.a"
    };
    let built = root.join(format!("target/{target}/release/libyinhe_flutter.a"));
    let dest = engine_dir.join(name);
    std::fs::copy(&built, &dest).map_err(|e| {
        eprintln!("复制 {} 失败: {e}", built.display());
        ExitCode::FAILURE
    })?;
    println!("静态库: {}", dest.display());

    // 3) 导出符号清单：release 的 dead_strip 会删掉没有任何原生代码引用的
    //    FFI 函数，而 Dart 侧靠 DynamicLibrary.process() 用 dlsym 查它们，
    //    所以必须显式列进 -exported_symbols_list（既是导出表也是保活根）。
    write_exports(&built, &engine_dir.join("exports.txt"))?;

    // 4) Flutter 壳。
    let mut f = cmd::cmd("flutter");
    f.current_dir(root.join("flutter"));
    f.arg("build").arg("ios");
    if device {
        // 真机包默认不签名：签名交给用户在 Xcode 里配一次即可。
        f.arg("--no-codesign");
    } else {
        f.arg("--simulator");
    }
    f.arg(if release { "--release" } else { "--debug" });
    cmd::status(&mut f)?;

    if install {
        if device {
            eprintln!("真机安装请用 `flutter run --release` 或 Xcode（需要签名配置）");
            return Ok(());
        }
        let app = root.join("flutter/build/ios/iphonesimulator/Runner.app");
        let mut i = cmd::cmd("xcrun");
        i.args(["simctl", "install", "booted"]).arg(&app);
        cmd::status(&mut i)?;
        let mut l = cmd::cmd("xcrun");
        l.args(["simctl", "launch", "booted", BUNDLE_ID]);
        cmd::status(&mut l)?;
    }
    Ok(())
}

/// 从静态库抽出 `_yinhe_*` 文本符号，写成 linker 的导出清单（含前导下划线）。
fn write_exports(archive: &Path, dest: &Path) -> Result<(), ExitCode> {
    let out = std::process::Command::new("nm")
        .arg("-gU")
        .arg(archive)
        .output()
        .map_err(|e| {
            eprintln!("执行 nm 失败: {e}");
            ExitCode::FAILURE
        })?;
    // nm 遇到 LLVM 23 的 bitcode 属性会打 error 并以非零码退出，但标准输出里的
    // 符号表仍然完整，所以这里只看有没有解析出符号。
    let mut symbols: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
            let mut it = line.split_whitespace();
            let _addr = it.next()?;
            let kind = it.next()?;
            let name = it.next()?;
            (kind == "T" && name.starts_with("_yinhe_")).then(|| name.to_string())
        })
        .collect();
    symbols.sort_unstable();
    symbols.dedup();
    if symbols.is_empty() {
        eprintln!("静态库里没有任何 _yinhe_* 符号，导出清单为空");
        return Err(ExitCode::FAILURE);
    }
    let text = symbols.join("\n") + "\n";
    std::fs::write(dest, text).map_err(|e| {
        eprintln!("写 {} 失败: {e}", dest.display());
        ExitCode::FAILURE
    })?;
    println!("导出符号: {} 个 -> {}", symbols.len(), dest.display());
    Ok(())
}

fn workspace_root() -> PathBuf {
    // xtask 的 CARGO_MANIFEST_DIR 是 <root>/xtask。
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask 必须位于 workspace 子目录")
        .to_path_buf()
}
