//! 鸿蒙（OpenHarmony）构建：Rust 引擎 → Flutter-OH → HAP。
//!
//! 流程（与 Android 类似，但工具链是 OpenHarmony SDK + hvigor）：
//! 1. `cargo build --target aarch64-unknown-linux-ohos -p yinhe-flutter`；
//! 2. 把 `.so`、NDK 的 `libc++_shared.so` 拷到 `flutter/ohos/entry/libs/arm64-v8a`；
//! 3. `hvigorw assembleHap`（内部会跑 `flutter assemble` 编 Dart）。
//!
//! 用法：
//!   cargo xtask ohos             # debug HAP（未签名）
//!   cargo xtask ohos --release   # release HAP
//!
//! 环境变量：
//!   OHOS_BASE_SDK_HOME  OpenHarmony SDK 根目录（下含 `<version>/native`）
//!   FLUTTER_OH_HOME     Flutter-OH 仓库根目录（bin/flutter 所在）

use std::env;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use crate::cmd;

const TARGET: &str = "aarch64-unknown-linux-ohos";
const ABI: &str = "arm64-v8a";

pub fn run(args: &[String]) -> ExitCode {
    let mut release = false;
    let mut install = false;
    for arg in args {
        match arg.as_str() {
            "--release" => release = true,
            "--install" => install = true,
            "--help" | "-h" => {
                println!(
                    "用法: cargo xtask ohos [--release] [--install]\n  \
                     --release  构建 release HAP（默认 debug）\n  \
                     --install  构建后通过 hdc 安装并启动（模拟器/真机）"
                );
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("未知参数: {other}（--help 看用法）");
                return ExitCode::FAILURE;
            }
        }
    }
    match build(release, install) {
        Ok(()) => ExitCode::SUCCESS,
        Err(code) => code,
    }
}

fn build(release: bool, install: bool) -> Result<(), ExitCode> {
    let root = workspace_root();
    let ohos_dir = root.join("flutter/ohos");
    let libs = ohos_dir.join("entry/libs").join(ABI);

    let sdk = find_ohos_sdk().ok_or_else(|| {
        eprintln!("找不到 OpenHarmony SDK：设置 OHOS_BASE_SDK_HOME（下含 <version>/native）");
        ExitCode::FAILURE
    })?;
    let flutter_oh = env::var_os("FLUTTER_OH_HOME")
        .map(PathBuf::from)
        .filter(|p| p.join("bin/flutter").is_file())
        .or_else(|| dirs_candidate("harmony/flutter-oh"))
        .ok_or_else(|| {
            eprintln!("找不到 Flutter-OH：设置 FLUTTER_OH_HOME（含 bin/flutter）");
            ExitCode::FAILURE
        })?;
    let native = find_native_dir(&sdk).ok_or_else(|| {
        eprintln!("SDK 下没有 native 目录: {}", sdk.display());
        ExitCode::FAILURE
    })?;

    // 1) Rust 引擎（release 与 Dart 的 buildMode 无关：一律用 release，性能优先）。
    let clang = native.join("llvm/bin/aarch64-unknown-linux-ohos-clang");
    let clangxx = native.join("llvm/bin/aarch64-unknown-linux-ohos-clang++");
    let ar = native.join("llvm/bin/llvm-ar");
    let sysroot = native.join("sysroot");
    let lib_dir = sysroot.join("usr/lib/aarch64-linux-ohos");
    let cxx_shared = native.join("llvm/lib/aarch64-linux-ohos/libc++_shared.so");

    // 部分 C++ 依赖（unrar）硬编码 -lstdc++：指向鸿蒙的 libc++。
    let shim = root.join("target/ohos-link");
    std::fs::create_dir_all(&shim).map_err(|e| {
        eprintln!("创建 {} 失败: {e}", shim.display());
        ExitCode::FAILURE
    })?;
    let stdcpp = shim.join("libstdc++.so");
    let _ = std::fs::remove_file(&stdcpp);
    std::os::unix::fs::symlink(&cxx_shared, &stdcpp).map_err(|e| {
        eprintln!("创建 libstdc++.so 软链失败: {e}");
        ExitCode::FAILURE
    })?;

    let mut c = cmd::cargo();
    c.args(["build", "--target", TARGET, "-p", "yinhe-flutter"]);
    if release {
        c.arg("--release");
    }
    c.current_dir(&root)
        .env(
            format!(
                "CARGO_TARGET_{}_LINKER",
                TARGET.to_uppercase().replace('-', "_")
            ),
            &clang,
        )
        .env(
            format!(
                "CARGO_TARGET_{}_RUSTFLAGS",
                TARGET.to_uppercase().replace('-', "_")
            ),
            format!(
                "-C link-arg=--sysroot={} -C link-arg=-L{} -C link-arg=-L{}",
                sysroot.display(),
                lib_dir.display(),
                shim.display()
            ),
        )
        .env("CC_aarch64_unknown_linux_ohos", &clang)
        .env("CXX_aarch64_unknown_linux_ohos", &clangxx)
        .env("AR_aarch64_unknown_linux_ohos", &ar)
        .env(
            "CFLAGS_aarch64_unknown_linux_ohos",
            format!("--sysroot={}", sysroot.display()),
        )
        .env(
            "CXXFLAGS_aarch64_unknown_linux_ohos",
            format!("--sysroot={}", sysroot.display()),
        );
    cmd::status(&mut c)?;

    // 2) 拷进 HAP 的 libs（gitignore；每次覆盖）。
    std::fs::create_dir_all(&libs).map_err(|e| {
        eprintln!("创建 {} 失败: {e}", libs.display());
        ExitCode::FAILURE
    })?;
    let built = root.join(format!("target/{TARGET}/release/libyinhe_flutter.so"));
    for (from, name) in [
        (&built, "libyinhe_flutter.so"),
        (&cxx_shared, "libc++_shared.so"),
    ] {
        let to = libs.join(name);
        std::fs::copy(from, &to).map_err(|e| {
            eprintln!("复制 {} 失败: {e}", from.display());
            ExitCode::FAILURE
        })?;
    }

    // 3) hvigor：需要 flutter-hvigor-plugin（由 flutter_tools 提供）。
    ensure_hvigor_plugin(&ohos_dir, &flutter_oh)?;

    let hvigorw = which("hvigorw").ok_or_else(|| {
        eprintln!("PATH 里没有 hvigorw（DevEco 或 ~/harmony/bin/hvigorw 包装脚本）");
        ExitCode::FAILURE
    })?;

    let path = env::var_os("PATH").unwrap_or_default();
    let mut new_path = Vec::new();
    new_path.push(flutter_oh.join("bin"));
    new_path.push(native.join("toolchains"));
    new_path.extend(env::split_paths(&path));
    let joined = env::join_paths(new_path).unwrap_or(path.clone());

    let mut h = Command::new(&hvigorw);
    h.args([
        "assembleHap",
        "-p",
        "product=default",
        "-p",
        if release {
            "buildMode=release"
        } else {
            "buildMode=debug"
        },
        "--no-daemon",
    ])
    .current_dir(&ohos_dir)
    .env("PATH", joined)
    .env("OHOS_BASE_SDK_HOME", &sdk)
    .env("OHOS_SDK_HOME", &sdk)
    .env("PUB_HOSTED_URL", "https://pub.flutter-io.cn")
    .env("FLUTTER_STORAGE_BASE_URL", "https://storage.flutter-io.cn");
    cmd::status(&mut h)?;

    let hap = ohos_dir
        .join("entry/build/default/outputs/default")
        .join(if release {
            "entry-default-signed.hap"
        } else {
            "entry-default-unsigned.hap"
        });
    println!("\n产物: {}", hap.display());
    if install {
        install_hap(&sdk, &hap)?;
    }
    Ok(())
}

/// 通过 hdc 安装并启动（模拟器/真机；真机需要已配置签名）。
fn install_hap(sdk: &Path, hap: &Path) -> Result<(), ExitCode> {
    let hdc = find_native_dir(sdk)
        .map(|n| n.join("toolchains/hdc"))
        .filter(|p| p.is_file())
        .or_else(|| which("hdc"))
        .ok_or_else(|| {
            eprintln!("找不到 hdc（SDK toolchains 或 PATH）");
            ExitCode::FAILURE
        })?;
    let mut devices = Command::new(&hdc);
    devices.arg("list").arg("targets");
    let out = devices.output().map_err(|e| {
        eprintln!("执行 hdc list targets 失败: {e}");
        ExitCode::FAILURE
    })?;
    let list = String::from_utf8_lossy(&out.stdout);
    if list.trim().is_empty() || list.contains("[Empty]") {
        eprintln!("没有连接中的设备/模拟器：先在 DevEco 里启动模拟器");
        return Err(ExitCode::FAILURE);
    }
    let mut install = Command::new(&hdc);
    install.arg("install").arg("-r").arg(hap);
    cmd::status(&mut install)?;
    let mut start = Command::new(&hdc);
    start.args([
        "shell",
        "aa",
        "start",
        "-b",
        "com.jieneng.yinhe",
        "-a",
        "EntryAbility",
    ]);
    cmd::status(&mut start)
}

fn ensure_hvigor_plugin(ohos_dir: &Path, flutter_oh: &Path) -> Result<(), ExitCode> {
    let installed = ohos_dir.join("node_modules/flutter-hvigor-plugin");
    if installed.is_dir() {
        return Ok(());
    }
    let plugin = flutter_oh.join("packages/flutter_tools/hvigor");
    if !plugin.is_dir() {
        eprintln!("找不到 flutter-hvigor-plugin 源目录: {}", plugin.display());
        return Err(ExitCode::FAILURE);
    }
    let package_json = format!(
        "{{\"dependencies\": {{\"flutter-hvigor-plugin\": \"file:{}\"}}}}\n",
        plugin.display()
    );
    std::fs::write(ohos_dir.join("package.json"), package_json).map_err(|e| {
        eprintln!("写 package.json 失败: {e}");
        ExitCode::FAILURE
    })?;
    let mut npm = cmd::cmd("npm");
    npm.arg("install").current_dir(ohos_dir);
    cmd::status(&mut npm)
}

fn which(name: &str) -> Option<PathBuf> {
    env::var_os("PATH").and_then(|paths| {
        env::split_paths(&paths).find_map(|dir| {
            let p = dir.join(name);
            p.is_file().then_some(p)
        })
    })
}

fn dirs_candidate(rel: &str) -> Option<PathBuf> {
    env::var_os("HOME").map(|home| PathBuf::from(home).join(rel))
}

fn find_native_dir(sdk: &Path) -> Option<PathBuf> {
    let mut versions: Vec<PathBuf> = std::fs::read_dir(sdk)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.join("native").is_dir())
        .collect();
    versions.sort();
    versions.pop().map(|p| p.join("native"))
}

fn find_ohos_sdk() -> Option<PathBuf> {
    if let Some(p) = env::var_os("OHOS_BASE_SDK_HOME") {
        let path = PathBuf::from(p);
        if find_native_dir(&path).is_some() {
            return Some(path);
        }
    }
    dirs_candidate("harmony/sdk26").filter(|p| find_native_dir(p).is_some())
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask 应在 <repo>/xtask 下")
        .to_path_buf()
}
