//! yinhe 的 xtask：把跨平台打包、验证等重复流程收编为可复用命令。
//!
//! 用法：
//!   cargo xtask bundle-macos              # 打包 macOS .app + .dmg
//!   cargo xtask package <macos|windows|linux>  # 统一三平台打包
//!   cargo xtask verify [crate..]          # fmt --check + clippy + test + release
//!
//! 依赖仅限标准库：xtask 自身编译要快，不应拉入任何第三方 crate。

mod bundle_macos;
mod cmd;
mod package;
mod verify;

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str);

    match cmd {
        Some("bundle-macos") => bundle_macos::run(),
        Some("package") => match args.get(1).map(String::as_str) {
            Some(target) => package::run(target),
            None => {
                eprintln!("用法: cargo xtask package <macos|windows|linux>");
                ExitCode::FAILURE
            }
        },
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
         cargo xtask bundle-macos                     打包 macOS .app 并生成 .dmg\n  \
         cargo xtask package <macos|windows|linux>    统一三平台打包\n  \
         cargo xtask verify [crate..]                 fmt --check + clippy + test + release\n\n\
         未指定 crate 时 verify 默认检查 default-members（crates/yinhe-egui）。"
    );
}
