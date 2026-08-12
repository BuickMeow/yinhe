//! yinhe-android：银河 MIDI 编辑器的安卓端 UI（触屏优先，完全重写）。
//!
//! 阶段 0（当前）：跑通 eframe 安卓管道，验证三件事——
//! 1. 中文渲染（复用 assets 里的 MiSans/Pretendard 字体）
//! 2. 触摸事件链路（多点、捏合缩放、长按=右键）
//! 3. wgpu 在真机上的渲染
//!
//! 安卓入口是 `android_main`（winit android-activity 约定），桌面端入口
//! 见 `src/bin/desktop.rs`，两者共用同一个 [`YinheApp`]。

use eframe::egui;
use yinhe_audio::spawn::{AudioCommand, CpalAudioHandle};

/// 阶段 0.5 音频验证用的音色库路径（adb push 到 app 私有目录）。
const TEST_SF_PATH: &str = "/data/data/com.jieneng.yinhe/files/generaluser.sf2";

/// 阶段 0 的最小验证 App。
pub struct YinheApp {
    /// 点按画布上留下的触点（验证触摸位置）。
    taps: Vec<egui::Pos2>,
    /// 最近一次触摸手势摘要。
    last_gesture: String,
    /// 双击计数（验证双击事件）。
    double_click_count: u32,
    /// 长按检测状态：按下起点 + 按下时刻。
    press_state: Option<(egui::Pos2, f64)>,
    /// 音频验证：cpal(AAudio) + xsynth 全链路。
    audio: Option<CpalAudioHandle>,
    audio_status: String,
}

impl YinheApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        setup_fonts(&cc.egui_ctx);
        Self {
            taps: Vec::new(),
            last_gesture: String::new(),
            double_click_count: 0,
            press_state: None,
            audio: None,
            audio_status: "未初始化".to_string(),
        }
    }

    /// 初始化音频引擎：cpal(AAudio) + xsynth 渲染线程。
    fn init_audio(&mut self) {
        use yinhe_audio::channel_layout::ChannelLayout;
        let layout = ChannelLayout::from_mask(vec![true; 16]);
        match yinhe_audio::spawn_cpal_audio(48000, layout, cpal::BufferSize::Default, None) {
            Ok(handle) => {
                self.audio_status = format!("音频引擎已初始化 @ {}Hz", handle.sample_rate);
                self.audio = Some(handle);
            }
            Err(e) => {
                self.audio_status = format!("初始化失败: {e}");
            }
        }
    }

    /// 加载测试音色库（GeneralUser GS）。
    fn load_soundfont(&mut self) {
        let Some(audio) = &self.audio else {
            self.audio_status = "请先初始化音频".to_string();
            return;
        };
        if !std::path::Path::new(TEST_SF_PATH).exists() {
            self.audio_status = format!("音色库不存在: {TEST_SF_PATH}");
            return;
        }
        audio.handle.send(AudioCommand::LoadSoundFont {
            port: 0,
            paths: vec![TEST_SF_PATH.to_string()],
        });
        self.audio_status = "音色加载中（大文件需几秒），稍后点播放...".to_string();
    }

    /// 播放 C 大调和弦（持续音，直到点停止）。
    fn play_chord(&mut self) {
        let Some(audio) = &self.audio else {
            self.audio_status = "请先初始化音频".to_string();
            return;
        };
        use yinhe_audio::spawn::PreviewNoteParams;
        audio.handle.send(AudioCommand::PreviewNotes {
            notes: vec![
                PreviewNoteParams {
                    channel: 0,
                    key: 60,
                    velocity: 100,
                    target_tick: 0,
                    duration_ticks: 0,
                },
                PreviewNoteParams {
                    channel: 0,
                    key: 64,
                    velocity: 100,
                    target_tick: 0,
                    duration_ticks: 0,
                },
                PreviewNoteParams {
                    channel: 0,
                    key: 67,
                    velocity: 100,
                    target_tick: 0,
                    duration_ticks: 0,
                },
            ],
        });
        self.audio_status = "播放中（点停止结束）".to_string();
    }
}

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
}

impl eframe::App for YinheApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        // ── 触摸手势摘要 ──
        let mut gesture = String::new();
        if let Some(mt) = ctx.multi_touch() {
            gesture = format!(
                "触点={} 捏合缩放={:.2} 双指平移=({:.1}, {:.1})",
                mt.num_touches, mt.zoom_delta, mt.translation_delta.x, mt.translation_delta.y
            );
        }

        // ── 长按检测（egui 的 is_long_press 是 pub(crate)，这里手动计时）──
        let (press_origin, now) = ctx.input(|i| (i.pointer.press_origin(), i.time));
        let mut long_press = false;
        if let Some(origin) = press_origin {
            let entry = self.press_state.get_or_insert((origin, now));
            if (entry.0 - origin).length() > 12.0 {
                // 按下位置漂移超过阈值，视为拖拽而非长按
                *entry = (origin, now);
            }
            long_press = now - entry.1 > 0.8;
        } else {
            self.press_state = None;
        }

        egui::CentralPanel::default().show(ui, |ui| {
            let screen = ui.ctx().viewport_rect();
            ui.heading("银河 MIDI 编辑器 · Android 验证");
            ui.label("阶段 0：管道跑通 + 中文渲染 + 触摸事件");
            ui.separator();

            ui.horizontal(|ui| {
                ui.label(format!(
                    "屏幕: {:.0} x {:.0}",
                    screen.width(),
                    screen.height()
                ));
                ui.separator();
                ui.label(format!("长按: {}", if long_press { "是" } else { "否" }));
                ui.separator();
                ui.label(format!("双击次数: {}", self.double_click_count));
            });
            if !gesture.is_empty() {
                self.last_gesture.clone_from(&gesture);
            }
            if !self.last_gesture.is_empty() {
                ui.label(&self.last_gesture);
            }

            // ── 音频验证（cpal/AAudio + xsynth 全链路）──
            ui.separator();
            ui.label(egui::RichText::new("音频验证").strong());
            ui.horizontal_wrapped(|ui| {
                if ui.button("初始化音频").clicked() {
                    self.init_audio();
                }
                if ui.button("加载音色库").clicked() {
                    self.load_soundfont();
                }
                if ui.button("播放 C 和弦").clicked() {
                    self.play_chord();
                }
                if ui.button("停止").clicked()
                    && let Some(a) = &self.audio
                {
                    a.handle.send(AudioCommand::PreviewStop);
                    self.audio_status = "已停止".to_string();
                }
            });
            ui.label(
                egui::RichText::new(&self.audio_status)
                    .color(egui::Color32::from_rgb(120, 200, 120)),
            );
            ui.separator();

            // ── 触摸画布：占满剩余空间，点按画点、双击计数、拖拽跟手 ──
            let rect = ui.available_rect_before_wrap();
            let resp = ui.allocate_rect(rect, egui::Sense::click_and_drag());
            let rect = resp.rect;
            let painter = ui.painter();
            painter.rect_filled(rect, 8.0, egui::Color32::from_gray(28));
            painter.rect_stroke(
                rect,
                8.0,
                egui::Stroke::new(1.0, egui::Color32::from_gray(90)),
                egui::StrokeKind::Inside,
            );
            painter.text(
                rect.left_top() + egui::vec2(8.0, 6.0),
                egui::Align2::LEFT_TOP,
                format!(
                    "画布: {:.0} x {:.0}（点这里测试）",
                    rect.width(),
                    rect.height()
                ),
                egui::FontId::proportional(12.0),
                egui::Color32::from_gray(140),
            );

            if resp.double_clicked() {
                self.double_click_count += 1;
            }
            if let Some(pos) = resp.interact_pointer_pos()
                && rect.contains(pos)
            {
                self.taps.push(pos);
                if self.taps.len() > 200 {
                    self.taps.remove(0);
                }
            }
            for tap in &self.taps {
                painter.circle_filled(*tap, 6.0, egui::Color32::from_rgb(255, 140, 60));
            }
            if let Some(hover) = resp.hover_pos() {
                painter.circle_stroke(hover, 12.0, egui::Stroke::new(1.5, egui::Color32::GRAY));
            }
        });
    }
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
    android_logger::init_once(
        android_logger::Config::default()
            .with_tag("yinhe")
            .with_max_level(log::LevelFilter::Info),
    );
    log::info!("yinhe-android starting");
    let options = eframe::NativeOptions {
        android_app: Some(app),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    let _ = run(options);
}
