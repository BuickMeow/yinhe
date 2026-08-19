//! 手动复现工具：加载指定 .clap → 激活 → 跑几块静音。
//! 用法：cargo run -p yinhe-clap --example load_plugin -- /path/to/x.clap [plugin-id]

use yinhe_clap::{ClapPluginInstance, HostInfo, PluginInfo};

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: load_plugin <path> [id]");
    eprintln!("[1] scanning {path}");
    let infos = yinhe_clap::scan::scan_path(std::path::Path::new(&path)).expect("scan");
    for (i, info) in infos.iter().enumerate() {
        eprintln!(
            "  [{}] id={} name={} features={:?}",
            i, info.id, info.name, info.features
        );
    }
    let want_id = std::env::args().nth(2);
    let info: &PluginInfo = match &want_id {
        Some(id) => infos.iter().find(|i| &i.id == id).expect("id not found"),
        None => infos.first().expect("no plugins in bundle"),
    };
    eprintln!("[2] loading instance: {}", info.id);
    let host = HostInfo::new("yinhe-test", "yinhe", "", "0.1").expect("host info");
    let mut inst = ClapPluginInstance::load(info, &host).expect("load");
    eprintln!("[3] activating 48000/512");
    let mut proc = inst.activate(48000.0, 512).expect("activate");
    eprintln!("[4] processing 10 blocks of silence");
    let mut left = vec![0.0f32; 512];
    let mut right = vec![0.0f32; 512];
    for i in 0..10 {
        proc.process_effect(&mut left, &mut right, &[], None)
            .expect("process");
        eprintln!("  block {i} ok");
    }
    eprintln!("[5] deactivate");
    inst.deactivate(proc);
    eprintln!("[6] done, dropping");
}
