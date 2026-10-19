//! `cargo xtask emulator`：启动并连接 Android 模拟器。
//!
//! - 已有 `emulator-*` 设备时直接复用；
//! - 否则按项目约定参数启动 `yinhe-dev`（可在 PATH / ANDROID_HOME 中找 SDK）；
//! - 等待 `sys.boot_completed=1`，打印可直接使用的 `ANDROID_SERIAL`；
//! - `--install` / `--install-debug` 安装并打开最新 APK。

use std::env;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

/// 项目专用 AVD（与日常测试一致）。
const DEFAULT_AVD: &str = "yinhe-dev";
const DEFAULT_TIMEOUT_SECS: u64 = 240;
const PACKAGE: &str = "com.jieneng.yinhe";
const ACTIVITY: &str = "com.jieneng.yinhe/.MainActivity";
const RELEASE_APK: &str = "flutter/build/app/outputs/flutter-apk/app-release.apk";
const DEBUG_APK: &str = "flutter/build/app/outputs/flutter-apk/app-debug.apk";

#[derive(Clone, Copy, PartialEq, Eq)]
enum ApkKind {
    Release,
    Debug,
}

struct Options {
    avd: String,
    timeout: Duration,
    install: Option<ApkKind>,
    launch: bool,
    cold: bool,
}

pub fn run(args: &[String]) -> ExitCode {
    let options = match parse_args(args) {
        Ok(Some(options)) => options,
        Ok(None) => {
            print_help();
            return ExitCode::SUCCESS;
        }
        Err(message) => {
            eprintln!("参数错误: {message}\n");
            print_help();
            return ExitCode::FAILURE;
        }
    };
    match connect(&options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("\n模拟器启动失败: {message}");
            ExitCode::FAILURE
        }
    }
}

fn parse_args(args: &[String]) -> Result<Option<Options>, String> {
    let mut options = Options {
        avd: DEFAULT_AVD.to_string(),
        timeout: Duration::from_secs(DEFAULT_TIMEOUT_SECS),
        install: None,
        launch: false,
        cold: false,
    };
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        match arg {
            "--help" | "-h" => return Ok(None),
            "--avd" => {
                i += 1;
                options.avd = args.get(i).ok_or("--avd 缺少名称")?.clone();
            }
            "--timeout" => {
                i += 1;
                let secs: u64 = args
                    .get(i)
                    .ok_or("--timeout 缺少秒数")?
                    .parse()
                    .map_err(|_| "--timeout 必须是整数秒")?;
                options.timeout = Duration::from_secs(secs.max(30));
            }
            "--install" | "--release" => options.install = Some(ApkKind::Release),
            "--install-debug" | "--debug" => options.install = Some(ApkKind::Debug),
            "--launch" => options.launch = true,
            "--cold" => options.cold = true,
            other => return Err(format!("未知参数: {other}")),
        }
        i += 1;
    }
    Ok(Some(options))
}

fn print_help() {
    println!(
        "启动并连接 Android 模拟器\n\n\
         用法:\n  \
         cargo xtask emulator [选项]\n\n\
         选项:\n  \
         --avd NAME           AVD 名称（默认 {DEFAULT_AVD}）\n  \
         --timeout SECS       等待开机超时秒数（默认 {DEFAULT_TIMEOUT_SECS}）\n  \
         --install            安装并打开最新 release APK\n  \
         --install-debug      安装并打开最新 debug APK\n  \
         --launch             只启动已安装的 App（不安装）\n  \
         --cold               冷启动（不加载/不保存快照）\n\n\
         已连接的模拟器会被复用；启动日志在系统临时目录 yinhe-emulator.log。"
    );
}

fn connect(options: &Options) -> Result<(), String> {
    let adb = find_tool("adb").ok_or_else(|| {
        "找不到 adb。请安装 Android SDK platform-tools，或设置 ANDROID_HOME。".to_string()
    })?;
    // 单次调用失败不影响后续（adb server 可能已在运行）。
    let _ = Command::new(&adb).arg("start-server").status();

    let running = emulator_devices(&adb);
    let (serial, log) = if let Some(serial) = running
        .iter()
        .find(|(_, state)| state == "device")
        .map(|(serial, _)| serial.clone())
    {
        println!("复用已连接的模拟器: {serial}");
        (serial, None)
    } else {
        let emulator = find_tool("emulator").ok_or_else(|| {
            "找不到 emulator。请安装 Android SDK emulator，或设置 ANDROID_HOME。".to_string()
        })?;
        let avds = list_avds(&emulator);
        if !avds.iter().any(|avd| avd == &options.avd) {
            return Err(format!(
                "AVD `{}` 不存在。可用 AVD: {}",
                options.avd,
                if avds.is_empty() {
                    "（无）".to_string()
                } else {
                    avds.join(", ")
                }
            ));
        }
        let log = env::temp_dir().join("yinhe-emulator.log");
        if running.is_empty() {
            spawn_emulator(&emulator, options, &log)?;
            println!("已启动 AVD `{}`（日志: {}）", options.avd, log.display());
        } else {
            println!("模拟器进程已存在，等待连接...");
        }
        let serial = wait_for_device(&adb, options.timeout, Some(&log))?;
        (serial, Some(log))
    };

    println!(
        "等待 {serial} 开机完成（最多 {}s）...",
        options.timeout.as_secs()
    );
    wait_for_boot(&adb, &serial, options.timeout, log.as_deref())?;
    println!("模拟器就绪: {serial}");
    println!("  export ANDROID_SERIAL={serial}");

    if let Some(kind) = options.install {
        let apk = match kind {
            ApkKind::Release => RELEASE_APK,
            ApkKind::Debug => DEBUG_APK,
        };
        install_and_launch(&adb, &serial, &workspace_root().join(apk), kind)?;
    } else if options.launch {
        launch(&adb, &serial)?;
    }
    Ok(())
}

/// 查找 adb / emulator：优先 ANDROID_HOME，再按平台默认目录，最后 PATH。
fn find_tool(name: &str) -> Option<PathBuf> {
    let exe = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    if let Some(sdk) = sdk_root() {
        let candidate = if name == "adb" {
            sdk.join("platform-tools").join(&exe)
        } else {
            sdk.join("emulator").join(&exe)
        };
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    env::var_os("PATH").and_then(|paths| {
        env::split_paths(&paths)
            .map(|dir| dir.join(&exe))
            .find(|candidate| candidate.is_file())
    })
}

pub(crate) fn sdk_root() -> Option<PathBuf> {
    for key in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        if let Some(value) = env::var_os(key) {
            let path = PathBuf::from(value);
            if path.is_dir() {
                return Some(path);
            }
        }
    }
    let home = env::var_os("HOME").map(PathBuf::from);
    let local = env::var_os("LOCALAPPDATA").map(PathBuf::from);
    let candidates = if cfg!(target_os = "macos") {
        vec![home.map(|home| home.join("Library/Android/sdk"))]
    } else if cfg!(windows) {
        vec![local.map(|local| local.join("Android/Sdk"))]
    } else {
        vec![
            home.clone().map(|home| home.join("Android/Sdk")),
            home.map(|home| home.join("android-sdk")),
        ]
    };
    candidates.into_iter().flatten().find(|path| path.is_dir())
}

fn list_avds(emulator: &Path) -> Vec<String> {
    let Ok(output) = Command::new(emulator).arg("-list-avds").output() else {
        return Vec::new();
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

fn spawn_emulator(emulator: &Path, options: &Options, log: &Path) -> Result<(), String> {
    // 追加写：若已有模拟器正在启动（尚未出现在 adb），避免截断它的日志。
    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
        .map_err(|e| format!("无法创建模拟器日志 {}: {e}", log.display()))?;
    let stderr = log_file
        .try_clone()
        .map_err(|e| format!("无法复制日志句柄: {e}"))?;

    let mut args = vec![
        "-avd",
        &options.avd,
        "-gpu",
        "host",
        "-feature",
        "GLESDynamicVersion",
        "-no-boot-anim",
        "-memory",
        "4096",
        "-cores",
        "4",
    ];
    if options.cold {
        args.extend(["-no-snapshot-load", "-no-snapshot-save"]);
    }

    // Unix 用 nohup 脱离终端（关闭终端后模拟器仍存活）；Windows 子进程
    // 默认不随父进程退出，直接 spawn。
    let mut command = if cfg!(windows) {
        let mut command = Command::new(emulator);
        command.args(&args);
        command
    } else {
        let mut command = Command::new("nohup");
        command.arg(emulator).args(&args);
        command
    };
    command
        .stdin(Stdio::null())
        .stdout(Stdio::from(log_file))
        .stderr(Stdio::from(stderr));
    command
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("无法启动模拟器: {e}"))
}

/// 当前 `emulator-*` 设备（含 offline）。
fn emulator_devices(adb: &Path) -> Vec<(String, String)> {
    let Ok(output) = Command::new(adb).arg("devices").output() else {
        return Vec::new();
    };
    parse_devices(&String::from_utf8_lossy(&output.stdout))
        .into_iter()
        .filter(|(serial, _)| serial.starts_with("emulator-"))
        .collect()
}

fn parse_devices(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with("List of devices") || line.starts_with('*') {
                return None;
            }
            let (serial, rest) = line.split_once('\t').or_else(|| line.split_once(' '))?;
            let state = rest.split_whitespace().next()?;
            Some((serial.trim().to_string(), state.to_string()))
        })
        .collect()
}

fn wait_for_device(adb: &Path, timeout: Duration, log: Option<&Path>) -> Result<String, String> {
    let start = Instant::now();
    let mut last_report = 0u64;
    loop {
        if let Some((serial, _)) = emulator_devices(adb)
            .into_iter()
            .find(|(_, state)| state == "device")
        {
            return Ok(serial);
        }
        let elapsed = start.elapsed().as_secs();
        if elapsed > timeout.as_secs() {
            print_log_tail(log);
            return Err(format!(
                "{}s 内未连接到模拟器（日志: {}）",
                timeout.as_secs(),
                log.map(|p| p.display().to_string())
                    .unwrap_or_else(|| "无".to_string())
            ));
        }
        if elapsed >= last_report + 10 {
            last_report = elapsed;
            println!("  等待设备连接... {elapsed}s");
        }
        sleep(Duration::from_secs(1));
    }
}

fn wait_for_boot(
    adb: &Path,
    serial: &str,
    timeout: Duration,
    log: Option<&Path>,
) -> Result<(), String> {
    let start = Instant::now();
    let mut last_report = 0u64;
    loop {
        if adb_output(adb, serial, &["shell", "getprop", "sys.boot_completed"]).as_deref()
            == Some("1")
        {
            return Ok(());
        }
        let elapsed = start.elapsed().as_secs();
        if elapsed > timeout.as_secs() {
            print_log_tail(log);
            return Err(format!("{}s 内未完成开机", timeout.as_secs()));
        }
        if elapsed >= last_report + 10 {
            last_report = elapsed;
            println!("  等待开机... {elapsed}s");
        }
        sleep(Duration::from_secs(1));
    }
}

fn adb_output(adb: &Path, serial: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(adb)
        .args(["-s", serial])
        .args(args)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn install_and_launch(adb: &Path, serial: &str, apk: &Path, kind: ApkKind) -> Result<(), String> {
    if !apk.is_file() {
        let (flag, build) = match kind {
            ApkKind::Release => ("--release", "release"),
            ApkKind::Debug => ("--debug", "debug"),
        };
        return Err(format!(
            "找不到 APK: {}\n先构建：\n  cargo ndk -t arm64-v8a -P 35 -o flutter/android/app/src/main/jniLibs build {flag} -p yinhe-flutter\n  (cd flutter && flutter build apk {flag})\n（当前为 {build} 包）",
            apk.display()
        ));
    }
    println!("安装 {} ...", apk.display());
    let status = Command::new(adb)
        .args(["-s", serial, "install", "-r"])
        .arg(apk)
        .status()
        .map_err(|e| format!("无法执行 adb install: {e}"))?;
    if !status.success() {
        return Err("adb install 失败".to_string());
    }
    launch(adb, serial)
}

fn launch(adb: &Path, serial: &str) -> Result<(), String> {
    let _ = Command::new(adb)
        .args(["-s", serial, "shell", "am", "force-stop", PACKAGE])
        .status();
    let status = Command::new(adb)
        .args(["-s", serial, "shell", "am", "start", "-n", ACTIVITY])
        .status()
        .map_err(|e| format!("无法启动 App: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("App 启动失败".to_string())
    }
}

/// 启动失败时打印模拟器日志末尾，便于判断锁冲突/GPU 错误。
fn print_log_tail(log: Option<&Path>) {
    let Some(log) = log else { return };
    let Ok(mut file) = File::open(log) else {
        return;
    };
    let len = file.metadata().map(|meta| meta.len()).unwrap_or(0);
    let offset = len.saturating_sub(8192);
    if file.seek(SeekFrom::Start(offset)).is_err() {
        return;
    }
    let mut tail = String::new();
    if file.read_to_string(&mut tail).is_ok() && !tail.is_empty() {
        println!("\n--- 模拟器日志末尾 ({}) ---\n{tail}", log.display());
    }
}

/// 仓库根目录（xtask 的上一级），保证从子目录运行也能找到 APK。
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::parse_devices;

    #[test]
    fn parses_adb_devices_output() {
        let text =
            "List of devices attached\nemulator-5554\tdevice\n10.0.2.2:5555\tunauthorized\n\n";
        assert_eq!(
            parse_devices(text),
            vec![
                ("emulator-5554".to_string(), "device".to_string()),
                ("10.0.2.2:5555".to_string(), "unauthorized".to_string()),
            ]
        );
    }

    #[test]
    fn parses_offline_device() {
        let text = "List of devices attached\nemulator-5554\toffline\n";
        assert_eq!(
            parse_devices(text),
            vec![("emulator-5554".to_string(), "offline".to_string())]
        );
    }
}
