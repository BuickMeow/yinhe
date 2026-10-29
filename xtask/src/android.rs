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

/// 运行时最低 API（与 `flutter/android/app/build.gradle.kts` 的 `ndkApi` 一致）。
/// AAudio 自 API 26 才有，`libyinhe_flutter.so` 的 DT_NEEDED 直接指向它。
const NDK_API: &str = "26";

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
    let jni_libs = root.join("flutter/android/app/src/main/jniLibs");
    let jni = jni_libs.join("arm64-v8a");

    // 清掉上次构建的 .so：避免 cargo-ndk 旧输出（以及历史遗留的
    // jniLibs/arm64-v8a/arm64-v8a 嵌套目录）被 Gradle 合并进 APK。
    if jni.exists() {
        std::fs::remove_dir_all(&jni).map_err(|e| {
            eprintln!("清理 jniLibs 失败: {e}");
            ExitCode::FAILURE
        })?;
    }

    // 1) Rust 核心库（cargo-ndk）。
    // 注意 -o 必须传 jniLibs 根目录：cargo-ndk 会自己追加 ABI 子目录。
    // -P 26 是运行时最低 API（不是 NDK 版本）：桩库按 26 取，生成物最多
    // 只要求 LIBC_O(26) 这类符号。用 35 会引入 _Unwind_*@LIBC_R(30)，
    // 导致安卓 10 / 鸿蒙 4 在 dlopen 阶段直接失败。
    // （与 flutter/android/app/build.gradle.kts 的 ndkApi 保持一致。）
    ensure_sysroot_libs(NDK_API)?;
    let mut c = cmd::cargo();
    c.args([
        "ndk",
        "-t",
        "arm64-v8a",
        "-P",
        NDK_API,
        "-o",
        jni_libs
            .to_str()
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
    let host = ndk_host_dir();
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
    // 只打 arm64：Rust 侧也只构建了 arm64；universal APK 里的
    // armeabi-v7a/x86_64 Flutter 库没有对应 Rust 库，纯属死重量。
    f.args([
        "build",
        "apk",
        if debug { "--debug" } else { "--release" },
        "--target-platform",
        "android-arm64",
    ]);
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

/// NDK 的 `llvm/prebuilt/<host>` 目录名。
fn ndk_host_dir() -> &'static str {
    if cfg!(target_os = "macos") {
        "darwin-x86_64"
    } else if cfg!(windows) {
        "windows-x86_64"
    } else {
        "linux-x86_64"
    }
}

/// bionic 把 pthread 并进了 libc，NDK r29 的 sysroot 顶层没有 libpthread.so，
/// 而 rustc 的 android 目标固定传 `-lpthread`。补一个指向**目标 API** 的
/// libc.so 软链；不能指向 35/，否则生成物记录 `LIBC_R`(API 30) 符号版本，
/// 安卓 10 / 鸿蒙 4 在 dlopen 阶段就失败。
fn ensure_sysroot_libs(api: &str) -> Result<(), ExitCode> {
    let ndk = find_ndk().ok_or_else(|| {
        eprintln!("找不到 Android NDK（设置 ANDROID_NDK_HOME 或安装到 SDK 的 ndk/ 下）");
        ExitCode::FAILURE
    })?;
    let link = ndk
        .join("toolchains/llvm/prebuilt")
        .join(ndk_host_dir())
        .join("sysroot/usr/lib/aarch64-linux-android/libpthread.so");
    #[cfg(unix)]
    {
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(format!("{api}/libc.so"), &link).map_err(|e| {
            eprintln!("创建 libpthread.so 软链失败: {e}");
            ExitCode::FAILURE
        })?;
    }
    #[cfg(not(unix))]
    let _ = (&link, api);
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
