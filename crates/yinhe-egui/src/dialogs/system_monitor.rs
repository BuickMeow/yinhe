use std::time::Instant;

use sysinfo::{CpuRefreshKind, Pid, ProcessesToUpdate, System};

pub(crate) struct SystemMonitor {
    sysinfo: System,
    self_pid: Option<Pid>,
    last_refresh: Instant,
    pub cpu_usage: f32,
    pub mem_mb: f64,
}

impl SystemMonitor {
    pub fn new() -> Self {
        let mut sysinfo = System::new();
        sysinfo.refresh_cpu_list(CpuRefreshKind::everything());
        Self {
            sysinfo,
            self_pid: sysinfo::get_current_pid().ok(),
            last_refresh: Instant::now(),
            cpu_usage: 0.0,
            mem_mb: 0.0,
        }
    }

    pub fn refresh_if_needed(&mut self) {
        if self.last_refresh.elapsed().as_secs_f64()
            >= crate::theme::SYS_REFRESH_INTERVAL_SECS
        {
            if let Some(pid) = self.self_pid {
                let _ = self
                    .sysinfo
                    .refresh_processes(ProcessesToUpdate::Some(&[pid]), false);
                if let Some(p) = self.sysinfo.process(pid) {
                    let num_cpus = self.sysinfo.cpus().len().max(1) as f32;
                    self.cpu_usage = p.cpu_usage() / num_cpus;
                    self.mem_mb = p.memory() as f64 / 1_048_576.0;
                }
            }
            self.last_refresh = Instant::now();
        }
    }
}
