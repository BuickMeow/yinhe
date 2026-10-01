//! 跨 crate 验证组合：fmt --check + clippy + test + release 构建。
//!
//! 用法：
//!   cargo xtask verify                       # 默认 default-members（crates/yinhe-egui）
//!   cargo xtask verify yinhe-egui yinhe-editor-core
//!
//! 与 AGENTS.md 的验证约定一致：默认只验证本次修改涉及的 crate 及其下游，
//! 全量测试留给 CI 或大改动前。这里不传 crate 时用 default-members，
//! 需要全量可显式传 `--workspace`。

use std::process::ExitCode;

use crate::cmd::{cargo, status};

pub fn run(crates: &[String]) -> ExitCode {
    match run_inner(crates) {
        Ok(()) => {
            println!("验证通过");
            ExitCode::SUCCESS
        }
        Err(code) => code,
    }
}

fn run_inner(crates: &[String]) -> Result<(), ExitCode> {
    let mut package_args: Vec<String> = Vec::new();
    for name in crates {
        package_args.push("-p".to_string());
        package_args.push(name.clone());
    }

    // fmt --check：不修改文件，只检查格式（与 CI 一致）。
    status(cargo().args(["fmt", "--check"]).args(&package_args))?;

    // clippy：不追加 -D warnings，避免既有警告（可能来自无关 crate）中断验证；
    // 警告仍会打印出来，按 AGENTS.md 需报告并消除。
    status(cargo().args(["clippy"]).args(&package_args))?;

    // test。
    status(cargo().args(["test"]).args(&package_args))?;

    // release 构建：方便会话结束后直接测试（AGENTS.md 第五节）。
    status(cargo().args(["build", "--release"]).args(&package_args))?;

    Ok(())
}
