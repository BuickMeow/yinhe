//! 复现 2：激活后把处理器 move 到另一线程处理（模拟渲染线程）。
//! 用法：cargo run -p yinhe-clap --example load_plugin_threaded -- /path/to/x.clap

use yinhe_clap::{ClapPluginInstance, HostInfo};

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: load_plugin_threaded <path>");
    let infos = yinhe_clap::scan::scan_path(std::path::Path::new(&path)).expect("scan");
    let info = infos.first().expect("no plugins");
    eprintln!("[1] loading {}", info.id);
    let host = HostInfo::new("yinhe-test", "yinhe", "", "0.1").expect("host info");
    let mut inst = ClapPluginInstance::load(info, &host).expect("load");
    eprintln!("[2] activating");
    let mut proc = inst.activate(48000.0, 512).expect("activate");
    eprintln!("[3] moving processor to worker thread, processing there");
    let handle = std::thread::Builder::new()
        .name("fake-render".into())
        .spawn(move || {
            let mut left = vec![0.0f32; 512];
            let mut right = vec![0.0f32; 512];
            for i in 0..100 {
                proc.process_effect(&mut left, &mut right, &[], None)
                    .expect("process");
                if i % 20 == 0 {
                    eprintln!("  block {i} ok");
                }
            }
            proc
        })
        .expect("spawn");
    let proc = handle.join().expect("join");
    eprintln!("[4] back on main thread, deactivate");
    inst.deactivate(proc);
    eprintln!("[5] done");
}
