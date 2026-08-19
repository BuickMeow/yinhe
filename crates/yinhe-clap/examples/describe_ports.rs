use yinhe_clap::{ClapPluginInstance, HostInfo};

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: describe_ports <path>");
    let infos = yinhe_clap::scan::scan_path(std::path::Path::new(&path)).expect("scan");
    for info in &infos {
        let host = HostInfo::new("yinhe-test", "yinhe", "", "0.1").expect("host info");
        let mut inst = ClapPluginInstance::load(info, &host).expect("load");
        println!(
            "== {} ({}) features={:?}",
            info.name, info.id, info.features
        );
        for line in inst.debug_dump_ports() {
            println!("  {line}");
        }
    }
}
