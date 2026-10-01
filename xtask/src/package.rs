//! 统一三平台打包。
//!
//! - macos：委托 `cargo xtask bundle-macos`（.app + .dmg）
//! - windows：`cargo build --release` 后压缩为 zip
//! - linux：`cargo build --release` 后打包为 tar.gz

use std::process::ExitCode;

use crate::cmd::{cargo, cmd, status};

pub fn run(target: &str) -> ExitCode {
    match target {
        "macos" => crate::bundle_macos::run(),
        "windows" => windows(),
        "linux" => linux(),
        other => {
            eprintln!("未知目标平台: {other}（可选：macos|windows|linux）");
            ExitCode::FAILURE
        }
    }
}

fn windows() -> ExitCode {
    let outcome = (|| -> Result<(), ExitCode> {
        status(cargo().args(["build", "--release", "-p", "yinhe-egui"]))?;
        status(
            cmd("powershell")
                .args(["-NoProfile", "-Command"])
                .arg("Compress-Archive -Force -Path target/release/yinhe-egui.exe -DestinationPath Yinhe-windows.zip"),
        )
    })();
    finish(outcome, "Yinhe-windows.zip")
}

fn linux() -> ExitCode {
    let outcome = (|| -> Result<(), ExitCode> {
        status(cargo().args(["build", "--release", "-p", "yinhe-egui"]))?;
        status(
            cmd("tar")
                .args(["-czf", "Yinhe-linux.tar.gz", "-C", "target/release"])
                .arg("yinhe-egui"),
        )
    })();
    finish(outcome, "Yinhe-linux.tar.gz")
}

fn finish(outcome: Result<(), ExitCode>, artifact: &str) -> ExitCode {
    match outcome {
        Ok(()) => {
            println!("打包完成: {artifact}");
            ExitCode::SUCCESS
        }
        Err(code) => code,
    }
}
