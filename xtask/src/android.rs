//! Android 构建：Rust 核心库 → Flutter APK（release / debug）。
//!
//! 固定流程（Android 壳的构建必须按此顺序，否则运行时会崩）：
//! 1. `cargo ndk` 构建 `libyinhe_flutter.so` 并复制到 jniLibs；
//! 2. 复制 NDK 的 `libc++_shared.so` 到同一目录（压缩包支持 unarc-rs 带 C++，
//!    `libyinhe_flutter.so` 的 DT_NEEDED 指向它，缺了会 dlopen 失败）；
//! 3. `flutter build apk`。
//!
//! 用法：
//!   cargo xtask android            # release APK
//!   cargo xtask android --debug    # debug APK
//!   cargo xtask android --install  # 构建后 adb install + 启动

use std::env;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use crate::cmd;

const APK_RELEASE: &str = "flutter/build/app/outputs/flutter-apk/app-release.apk";
const APK_DEBUG: &str = "flutter/build/app/outputs/flutter-apk/app-debug.apk";

pub fn run(args: &[String]) -> ExitCode {
    let mut debug = false;
    let mut install = false;
    for arg in args {
        match arg.as_str() {
            "--debug" => debug = true,
            "--install" => install = true,
            "--help" | "-h" => {
                println!(
                    "用法: cargo xtask android [--debug] [--install]\n  \
                     --debug    构建 debug APK（默认 release）\n  \
                     --install  构建后安装到已连接设备/模拟器并启动"
                );
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("未知参数: {other}（--help 看用法）");
                return ExitCode::FAILURE;
            }
        }
    }
    match build(debug, install) {
        Ok(()) => ExitCode::SUCCESS,
        Err(code) => code,
    }
}

fn build(debug: bool, install: bool) -> Result<(), ExitCode> {
    let root = workspace_root();
    let jni = root.join("flutter/android/app/src/main/jniLibs/arm64-v8a");

    // 1) Rust 核心库（cargo-ndk）。
    let mut c = cmd::cargo();
    c.args([
        "ndk",
        "-t",
        "arm64-v8a",
        "-P",
        "35",
        "-o",
        jni.to_str()
            .unwrap_or("flutter/android/app/src/main/jniLibs"),
        "build",
    ]);
    if !debug {
        c.arg("--release");
    }
    c.args(["-p", "yinhe-flutter"]);
    c.current_dir(&root);
    cmd::status(&mut c)?;

    // 2) C++ 运行时（必需，见模块注释）。
    let ndk = find_ndk().ok_or_else(|| {
        eprintln!("找不到 Android NDK（设置 ANDROID_NDK_HOME 或安装到 SDK 的 ndk/ 下）");
        ExitCode::FAILURE
    })?;
    let host = if cfg!(target_os = "macos") {
        "darwin-x86_64"
    } else if cfg!(windows) {
        "windows-x86_64"
    } else {
        "linux-x86_64"
    };
    let cxx = ndk
        .join("toolchains/llvm/prebuilt")
        .join(host)
        .join("sysroot/usr/lib/aarch64-linux-android/libc++_shared.so");
    if !cxx.is_file() {
        eprintln!("找不到 {}", cxx.display());
        return Err(ExitCode::FAILURE);
    }
    std::fs::create_dir_all(&jni).map_err(|e| {
        eprintln!("创建 jniLibs 失败: {e}");
        ExitCode::FAILURE
    })?;
    std::fs::copy(&cxx, jni.join("libc++_shared.so")).map_err(|e| {
        eprintln!("复制 libc++_shared.so 失败: {e}");
        ExitCode::FAILURE
    })?;

    // 3) Flutter APK。
    let mut f = cmd::cmd("flutter");
    f.args(["build", "apk", if debug { "--debug" } else { "--release" }]);
    f.current_dir(root.join("flutter"));
    cmd::status(&mut f)?;

    // 4) 可选：安装并启动。
    if install {
        let adb = find_tool("adb").ok_or_else(|| {
            eprintln!("找不到 adb（设置 ANDROID_HOME 或加入 PATH）");
            ExitCode::FAILURE
        })?;
        let apk = root.join(if debug { APK_DEBUG } else { APK_RELEASE });
        let mut i = Command::new(&adb);
        i.args(["install", "-r"]).arg(&apk);
        cmd::status(&mut i)?;
        let mut s = Command::new(&adb);
        s.args([
            "shell",
            "am",
            "start",
            "-n",
            "com.jieneng.yinhe/.MainActivity",
        ]);
        cmd::status(&mut s)?;
    }
    Ok(())
}

/// NDK 目录：优先 ANDROID_NDK_HOME，其次 SDK 下 ndk/ 的最新版本。
fn find_ndk() -> Option<PathBuf> {
    if let Some(p) = env::var_os("ANDROID_NDK_HOME") {
        let path = PathBuf::from(p);
        if path.is_dir() {
            return Some(path);
        }
    }
    let sdk = crate::emulator::sdk_root()?;
    let ndk = sdk.join("ndk");
    let mut versions: Vec<PathBuf> = std::fs::read_dir(&ndk)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir())
        .collect();
    versions.sort();
    versions.pop()
}

fn find_tool(name: &str) -> Option<PathBuf> {
    let exe = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    if let Some(sdk) = crate::emulator::sdk_root() {
        let candidate = sdk.join("platform-tools").join(&exe);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    env::var_os("PATH").and_then(|paths| {
        env::split_paths(&paths)
            .map(|dir| dir.join(&exe))
            .find(|c| c.is_file())
    })
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}
