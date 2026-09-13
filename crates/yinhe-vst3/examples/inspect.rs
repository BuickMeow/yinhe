//! 手动验证：创建 VST3 实例并枚举/读写参数。
//!
//! 用法：`cargo run -p yinhe-vst3 --example inspect -- <bundle路径> <class_id>`

use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let Some(bundle) = args.get(1) else {
        eprintln!("用法: inspect <bundle.vst3> <class_id>");
        std::process::exit(2);
    };
    let Some(class_id) = args.get(2) else {
        eprintln!("用法: inspect <bundle.vst3> <class_id>");
        std::process::exit(2);
    };

    let instance = match yinhe_vst3::Vst3PluginInstance::load(Path::new(bundle), class_id) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("创建实例失败: {e}");
            std::process::exit(1);
        }
    };
    println!("实例创建成功: {} 个参数", instance.params().len());
    for p in instance.params().iter().take(12) {
        let v = instance.get_param_normalized(p.id);
        let text = instance
            .format_param(p.id, v)
            .unwrap_or_else(|| format!("{v:.3}"));
        println!(
            "  #{} {} = {v:.4} [{text}] (step={}, flags={:#x}, units={})",
            p.id, p.title, p.step_count, p.flags, p.units
        );
    }

    // 参数写读往返：找一个非只读参数（kIsReadOnly = 1<<1）。
    if let Some(p) = instance.params().iter().find(|p| p.flags & (1 << 1) == 0) {
        let before = instance.get_param_normalized(p.id);
        let target = if before > 0.5 { 0.25 } else { 0.75 };
        instance.set_param_normalized(p.id, target);
        let after = instance.get_param_normalized(p.id);
        println!(
            "参数往返: #{} {} {before:.3} -> 写 {target:.3} -> 读 {after:.3}",
            p.id, p.title
        );
    }
    // 注意：状态保存/恢复（save_state/load_state）需在音频激活后调用——
    // 部分插件（实测 Serum 2）在 setupProcessing/setActive 之前 getState 会段错误。
    println!("完成");
}
