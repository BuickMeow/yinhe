//! yinhe-android：银河 MIDI 编辑器的安卓端 UI（触屏优先，完全重写）。
//!
//! 结构：
//! - [`app`]：YinheApp 结构 + 音频/MIDI 生命周期
//! - [`pages`]：页面 UI（菜单/AR/PR/走带），按页面解耦
//! - [`ar_view`] / [`pr_view`]：GPU 音符视图（wgpu 渲染 + 触摸交互）
//! - [`insets`]：挖孔/刘海安全区桥（Kotlin → JNI）
//! - [`file_picker`]：本地打开 MIDI（SAF 文件选择器桥）
//!
//! 安卓入口是 `android_main`（winit android-activity 约定），桌面端入口
//! 见 `src/bin/desktop.rs`，两者共用同一个 [`YinheApp`]。

use eframe::egui;
use yinhe_core::YinModel;

mod app;
mod ar_view;
mod file_picker;
mod ime;
mod insets;
mod pages;
mod pr_view;
mod ui_common;

pub(crate) use app::{Page, YinheApp};
pub(crate) use ui_common::track_colors_for;

/// 音频验证用的音色库路径（adb push 到 app 私有目录）。
pub(crate) const TEST_SF_PATH: &str = "/data/data/com.jieneng.yinhe/files/generaluser.sf2";
/// 测试 MIDI：小曲（链路验证）与大曲（性能测试）。
pub(crate) const TEST_MIDI_PATH: &str = "/data/data/com.jieneng.yinhe/files/test.mid";
pub(crate) const BIG_MIDI_PATH: &str = "/data/data/com.jieneng.yinhe/files/big.mid";

/// 加载项目自带字体（与桌面端一致的 Pretendard 主字体 + MiSans 中文回退）。
fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "Pretendard".to_owned(),
        egui::FontData::from_static(include_bytes!("../../../assets/Pretendard-Medium.otf")).into(),
    );
    fonts.font_data.insert(
        "MiSans".to_owned(),
        egui::FontData::from_static(include_bytes!("../../../assets/MiSans-Medium.otf")).into(),
    );
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        let list = fonts.families.entry(family).or_default();
        list.insert(0, "Pretendard".to_owned());
        list.insert(1, "MiSans".to_owned());
    }
    ctx.set_fonts(fonts);
    // Material Icons（走带/图标按钮用）；y_offset_factor=0 保证字形居中（同桌面端）。
    let mut font_insert = egui_material_icons::font_insert();
    font_insert.data.tweak.y_offset_factor = 0.0;
    ctx.add_font(font_insert);
}

impl eframe::App for YinheApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // 输入法文本注入 / egui 光标回推（工程设置等 TextEdit 输入用）。
        ime::pump_into(&ctx);
        // 安卓上无触摸事件时 egui 不重绘（桌面有鼠标移动持续触发）——
        // 请求周期重绘让计时/状态文字持续刷新。
        ctx.request_repaint_after(std::time::Duration::from_millis(100));

        // 挖孔/刘海安全区：物理 px → 逻辑点（insets 变化后至多 100ms 内生效）。
        let ppp = ctx.pixels_per_point().max(0.25);
        let px = insets::safe_insets_px();
        self.safe_insets = [
            px[0] as f32 / ppp,
            px[1] as f32 / ppp,
            px[2] as f32 / ppp,
            px[3] as f32 / ppp,
        ];

        // ── 统一主题色：checkbox 对勾等 fg_stroke 用主文字色，避免与 btn_bg 同系不可见（与桌面端 main_loop 一致）
        {
            let theme = yinhe_theme::egui_colors::derive_theme(yinhe_theme::base::BaseColors::DARK);
            let mut visuals = egui::Visuals::dark();
            visuals.window_fill = theme.app_bg;
            visuals.panel_fill = theme.app_bg;
            visuals.selection.bg_fill = theme.selected_bg;
            visuals.selection.stroke = egui::Stroke::new(1.5, theme.accent_active);
            visuals.text_cursor.stroke = egui::Stroke::new(2.0, theme.accent_active);
            visuals.text_cursor.preview = false;
            visuals.extreme_bg_color = theme.control_bg;
            let btn = theme.btn_bg;
            let line = theme.line_fg;
            visuals.widgets.inactive.bg_fill = btn;
            visuals.widgets.inactive.weak_bg_fill = theme.app_bg;
            visuals.widgets.hovered.bg_fill = theme.hovered(btn);
            visuals.widgets.hovered.weak_bg_fill = theme.hovered(theme.app_bg);
            visuals.widgets.active.bg_fill = theme.pressed(btn);
            visuals.widgets.active.weak_bg_fill = theme.pressed(theme.app_bg);
            visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.5, theme.text_primary);
            visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, line);
            visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.5, theme.text_primary);
            visuals.widgets.hovered.bg_stroke =
                egui::Stroke::new(1.0, theme.accent_active.gamma_multiply(0.85));
            visuals.widgets.active.fg_stroke = egui::Stroke::new(1.5, theme.text_primary);
            visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0, theme.accent_active);
            visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.5, theme.text_disabled);
            visuals.widgets.noninteractive.weak_bg_fill = theme.app_bg;
            visuals.override_text_color = Some(theme.text_primary);
            ctx.set_visuals(visuals);
        }

        match self.page {
            Page::Menu => self.ui_menu(ui),
            Page::Ar => self.ui_ar(ui),
            Page::Pr => self.ui_pr(ui),
        }
    }
}

/// 由秒反查 tick：tempo_map 只提供 tick_to_seconds（随 tempo 分段单调递增），
/// 二分 40 次足够收敛到亚 tick 精度，音频播放位置反查用。
pub(crate) fn seconds_to_tick(model: &YinModel, seconds: f64) -> f64 {
    let total = model.tempo_map.tick_length.max(1) as f64;
    let mut lo = 0.0;
    let mut hi = total;
    for _ in 0..40 {
        let mid = (lo + hi) * 0.5;
        if model.tempo_map.tick_to_seconds(mid as u64) < seconds {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    (lo + hi) * 0.5
}

/// 创建 eframe 运行入口（安卓与桌面共用）。
pub fn run(options: eframe::NativeOptions) -> Result<(), eframe::Error> {
    eframe::run_native(
        "Yinhe",
        options,
        Box::new(|cc| Ok(Box::new(YinheApp::new(cc)))),
    )
}

/// 安卓入口（winit android-activity 约定，由 GameActivity 加载 cdylib 后调用）。
#[cfg(target_os = "android")]
// android-activity 官方约定的入口签名；AndroidApp 是 JNI 指针的透明包装。
#[allow(improper_ctypes_definitions)]
#[unsafe(no_mangle)]
pub extern "C" fn android_main(app: winit::platform::android::activity::AndroidApp) {
    // 本地打开文件（SAF）桥需要 AndroidApp 引用。
    file_picker::init(app.clone());
    android_logger::init_once(
        android_logger::Config::default()
            .with_tag("yinhe")
            .with_max_level(log::LevelFilter::Debug),
    );
    // tracing → log 桥：yinhe-wgpu 内部用 tracing（cull 状态等），安卓 stderr 不可见。
    // 默认 LogTracer 只转发 Info 以上，cull 的 debug 日志全被吞——显式提到 Debug。
    tracing_log::LogTracer::builder()
        .with_max_level(log::LevelFilter::Debug)
        .init()
        .ok();
    // Rust panic 在安卓上直接 abort 且消息不可见（不进 logcat），必须显式 hook。
    std::panic::set_hook(Box::new(|info| {
        log::error!("PANIC: {info}");
        let bt = std::backtrace::Backtrace::force_capture();
        log::error!("BACKTRACE:\n{bt}");
    }));
    log::info!("yinhe-android starting");
    let options = eframe::NativeOptions {
        android_app: Some(app),
        renderer: eframe::Renderer::Wgpu,
        // 与桌面端一致的 wgpu 配置：GPU cull 需要 13 个 storage buffer
        //（默认 limits 只有 8，pipeline 会静默创建失败 → 音符不渲染）。
        wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
            wgpu_setup: {
                use eframe::egui_wgpu::wgpu;
                let mut setup = eframe::egui_wgpu::WgpuSetupCreateNew::without_display_handle();
                setup.device_descriptor = std::sync::Arc::new(|adapter| {
                    let base_limits = if adapter.get_info().backend == wgpu::Backend::Gl {
                        wgpu::Limits::downlevel_webgl2_defaults()
                    } else {
                        wgpu::Limits::default()
                    };
                    wgpu::DeviceDescriptor {
                        label: Some("egui wgpu device"),
                        // cull 已改为 CPU 读回 args + 直接 draw_indexed
                        // （Adreno indirect draw 失效），不再需要
                        // INDIRECT_FIRST_INSTANCE feature。
                        required_features: wgpu::Features::empty(),
                        required_limits: wgpu::Limits {
                            max_texture_dimension_2d: 8192,
                            max_storage_buffers_per_shader_stage: 16,
                            ..base_limits
                        },
                        ..Default::default()
                    }
                });
                eframe::egui_wgpu::WgpuSetup::CreateNew(setup)
            },
            ..Default::default()
        },
        ..Default::default()
    };
    let _ = run(options);
}
