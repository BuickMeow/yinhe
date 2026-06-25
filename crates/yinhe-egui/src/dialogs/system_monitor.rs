use std::time::Instant;

use eframe::egui;
use sysinfo::{CpuRefreshKind, Pid, ProcessesToUpdate, System};

use crate::app::App;
use yinhe_memtrace::{AllocTag, Snapshot};

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

impl App {
    pub(crate) fn refresh_system_stats(&mut self) {
        self.sys_monitor.refresh_if_needed();
    }

    pub(crate) fn show_memory_breakdown(&mut self, ui: &mut egui::Ui) {
        if !self.show_mem_breakdown {
            return;
        }
        let snapshot = Snapshot::capture();
        let mem_mb = self.sys_monitor.mem_mb;
        egui::Window::new("内存占用详情")
            .id(egui::Id::new("memory_breakdown_window"))
            .default_size(crate::theme::MEM_POPUP_SIZE)
            .collapsible(false)
            .resizable(false)
            .show(ui.ctx(), |ui| {
                ui.label(format!("系统统计总内存: {:.1} MB", mem_mb));
                ui.label(format!("分配器追踪内存: {:.1} MB", snapshot.total_mb()));
                ui.label(format!("wgpu 显式 GPU 资源: {:.1} MB", snapshot.gpu_mb()));

                #[cfg(target_os = "macos")]
                {
                    let metal_size = self
                        .render_ctx
                        .metal_allocated_size()
                        .unwrap_or(0)
                        .saturating_add(self.arr_render_ctx.metal_allocated_size().unwrap_or(0));
                    ui.label(format!(
                        "Metal 驱动真实显存: {:.1} MB",
                        metal_size as f64 / 1_048_576.0
                    ));
                }

                ui.separator();

                ui.heading("按子系统分类");
                egui::Grid::new("mem_breakdown_grid")
                    .num_columns(2)
                    .spacing([12.0, 8.0])
                    .show(ui, |ui| {
                        for tag in AllocTag::ALL {
                            if tag == AllocTag::Unknown && snapshot.get(tag) <= 0 {
                                continue;
                            }
                            ui.label(tag.name());
                            ui.label(format!("{:.1} MB", snapshot.mb(tag)));
                            ui.end_row();
                        }
                    });

                ui.separator();
                ui.small(
                    "注：GPU 资源计数反映应用显式创建的 wgpu Texture/Buffer 大小；\
                     驱动层额外开销（swapchain、depth、pipeline cache 等）\
                     不纳入此项统计。",
                );

                if ui.button("关闭").clicked() {
                    self.show_mem_breakdown = false;
                }
            });
    }
}
