//! 子进程执行的小工具：统一继承 stdio、失败即返回错误。

use std::process::{Command, ExitCode};

/// 运行命令并继承 stdio；非零退出码返回 `Err(ExitCode::FAILURE)`。
pub fn status(cmd: &mut Command) -> Result<(), ExitCode> {
    let prog = prog_name(cmd);
    println!("$ {}", format_cmd(cmd));
    match cmd.status() {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => {
            eprintln!("命令失败（退出码 {:?}）: {prog}", s.code());
            Err(ExitCode::FAILURE)
        }
        Err(e) => {
            eprintln!("无法执行命令 {prog}: {e}");
            Err(ExitCode::FAILURE)
        }
    }
}

fn prog_name(cmd: &Command) -> String {
    cmd.get_program().to_string_lossy().into_owned()
}

/// 可读的命令行：程序名取 basename，参数用引号包裹含空格的。
fn format_cmd(cmd: &Command) -> String {
    let prog = std::path::Path::new(&cmd.get_program())
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| prog_name(cmd));
    let mut out = prog;
    for a in cmd.get_args() {
        let a = a.to_string_lossy();
        if a.contains(' ') {
            out.push_str(&format!(" {a:?}"));
        } else {
            out.push(' ');
            out.push_str(&a);
        }
    }
    out
}

/// 构造 `cargo` 命令。
pub fn cargo() -> Command {
    Command::new(env!("CARGO"))
}

/// 构造普通命令。
pub fn cmd(program: &str) -> Command {
    Command::new(program)
}
