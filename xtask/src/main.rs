//! yinhe 的 xtask：把跨平台打包、验证等重复流程收编为可复用命令。
//!
//! 用法：
//!   cargo xtask bundle-macos              # 打包 macOS .app + .dmg
//!   cargo xtask verify [crate..]          # fmt --check + clippy + test + release
//!
//! 依赖仅限标准库：xtask 自身编译要快，不应拉入任何第三方 crate。
//!
//! Windows/Linux 的打包各只有一行原生命令（Compress-Archive / tar），
//! 逻辑简单、又无法在 macOS 本地验证，故保留在 CI 的 YAML 里，不进 xtask。

mod bundle_macos;
mod cmd;
mod verify;

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str);

    match cmd {
        Some("bundle-macos") => bundle_macos::run(),
        Some("verify") => verify::run(&args[1..]),
        Some("help") | Some("-h") | Some("--help") | None => {
            print_help();
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("未知命令: {other}\n");
            print_help();
            ExitCode::FAILURE
        }
    }
}

fn print_help() {
    println!(
        "yinhe xtask\n\n\
         用法:\n  \
         cargo xtask bundle-macos       打包 macOS .app 并生成 .dmg\n  \
         cargo xtask verify [crate..]   fmt --check + clippy + test + release\n\n\
         未指定 crate 时 verify 默认检查 default-members（crates/yinhe-egui）。"
    );
}
