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
use yinhe_core::YinModel;

mod pr_view;

/// 阶段 0.5 音频验证用的音色库路径（adb push 到 app 私有目录）。
const TEST_SF_PATH: &str = "/data/data/com.jieneng.yinhe/files/generaluser.sf2";
/// 阶段 1 测试 MIDI：小曲（链路验证）与大曲（性能测试）。
const TEST_MIDI_PATH: &str = "/data/data/com.jieneng.yinhe/files/test.mid";
const BIG_MIDI_PATH: &str = "/data/data/com.jieneng.yinhe/files/big.mid";

/// 页面：验证页 / PR 钢琴卷帘。
#[derive(PartialEq)]
enum Page {
    Verify,
    Pr,
}

/// 编辑工具（初期只做选择 UI 与状态，实际编辑功能后续接入）。
/// 图标与桌面端 tools_panel 一致。
#[derive(Clone, Copy, PartialEq)]
enum Tool {
    Select,
    Pencil,
    Eraser,
}

impl Tool {
    const ALL: [Tool; 3] = [Tool::Select, Tool::Pencil, Tool::Eraser];

    fn icon(self) -> egui_material_icons::MaterialIcon {
        use egui_material_icons::icons::*;
        match self {
            Self::Select => ICON_SELECT,
            Self::Pencil => ICON_EDIT,
            Self::Eraser => ICON_INK_ERASER,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Select => "选择",
            Self::Pencil => "铅笔",
            Self::Eraser => "橡皮",
        }
    }
}

/// 阶段 0 的最小验证 App。
pub struct YinheApp {
    /// 当前页面。
    page: Page,
    /// PR 钢琴卷帘（GPU cull 渲染 + 触摸交互）。
    pr_view: pr_view::PrView,
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
    /// 音色加载诊断：开始时刻 + 加载前的已加载计数。
    sf_load_start: Option<std::time::Instant>,
    sf_loaded_baseline: usize,
    /// 阶段 1：MIDI 加载（复用 file_loading 模块，后台线程 + 进度）。
    midi_loader: Option<yinhe_editor_core::file_loading::FileLoader>,
    midi_load_start: Option<std::time::Instant>,
    /// 最近一次加载结果统计。
    midi_stats: String,
    /// 当前 MIDI 模型（加载完成后的唯一引用，音频引擎与 PR 视图共享同一份）。
    model: Option<std::sync::Arc<YinModel>>,
    /// 播放光标（tick）：暂停/停止后保留，作为下次播放起点。
    cursor_tick: f64,
    /// 跟随播放：开启后滚动让光标始终位于内容区中央。
    follow_play: bool,
    /// 走带位置/时间显示：false = 时间 m:ss.mmm，true = 位置 小节.拍.tick。
    time_show_ticks: bool,
    /// 当前编辑工具（工具弹窗选择）。
    tool: Tool,
    /// 工具选择弹窗是否打开。
    tool_picker_open: bool,
}

impl YinheApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        setup_fonts(&cc.egui_ctx);
        let mut app = Self {
            page: Page::Verify,
            pr_view: pr_view::PrView::new(cc),
            taps: Vec::new(),
            last_gesture: String::new(),
            double_click_count: 0,
            press_state: None,
            audio: None,
            audio_status: "未初始化".to_string(),
            sf_load_start: None,
            sf_loaded_baseline: 0,
            midi_loader: None,
            midi_load_start: None,
            midi_stats: String::new(),
            model: None,
            cursor_tick: 0.0,
            follow_play: false,
            time_show_ticks: false,
            tool: Tool::Select,
            tool_picker_open: false,
        };
        // 调试阶段：启动即自动加载小曲 + 初始化音频/音色库，免去手动点击按钮。
        app.start_midi_load(TEST_MIDI_PATH);
        app.init_audio();
        app.load_soundfont();
        app
    }

    /// 初始化音频引擎：cpal(AAudio) + xsynth 渲染线程。
    fn init_audio(&mut self) {
        use yinhe_audio::channel_layout::ChannelLayout;
        let layout = ChannelLayout::from_mask(vec![true; 16]);
        // 固定 2048 帧缓冲：AAudio 动态调优在 MIUI 上收敛慢，固定值更稳；
        // 蓝牙 A2DP 路径需要更大缓冲（编码/传输延迟高，1024 会周期性欠载
        // → 平均断续），cpal 的 Fixed(n) 实际 capacity 是 2n，2048≈85ms。
        match yinhe_audio::spawn_cpal_audio(48000, layout, cpal::BufferSize::Fixed(2048), None) {
            Ok(handle) => {
                log::info!(
                    "audio: 引擎初始化成功 @ {}Hz (sample_rate={})",
                    handle.sample_rate,
                    handle.sample_rate
                );
                self.audio_status = format!("音频引擎已初始化 @ {}Hz", handle.sample_rate);
                self.audio = Some(handle);
            }
            Err(e) => {
                log::error!("audio: 引擎初始化失败: {e}");
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
            log::error!("audio: 音色库不存在 {TEST_SF_PATH}");
            self.audio_status = format!("音色库不存在: {TEST_SF_PATH}");
            return;
        }
        audio.handle.send(AudioCommand::LoadSoundFont {
            port: 0,
            paths: vec![TEST_SF_PATH.to_string()],
        });
        self.sf_load_start = Some(std::time::Instant::now());
        self.sf_loaded_baseline = audio.handle.sf_loaded_count();
        self.audio_status = "音色加载中（大文件需几秒），稍后点播放...".to_string();
    }

    /// 每帧更新音色加载状态：轮询 sf_loaded_count 显示完成/耗时。
    fn poll_sf_load(&mut self) {
        let Some(start) = self.sf_load_start else {
            return;
        };
        let Some(audio) = &self.audio else {
            self.sf_load_start = None;
            return;
        };
        let elapsed = start.elapsed().as_secs_f32();
        let loaded = audio.handle.sf_loaded_count();
        if loaded > self.sf_loaded_baseline {
            self.audio_status = format!("音色加载完成，耗时 {elapsed:.1} 秒！点播放试试");
            self.sf_load_start = None;
        } else {
            self.audio_status = format!("音色加载中... 已等待 {elapsed:.0} 秒");
        }
    }

    /// 加载指定 MIDI（file_loading 后台线程 + 进度上报）。
    fn start_midi_load(&mut self, path: &str) {
        use yinhe_editor_core::file_loading::{FileLoader, LoadStageLabels};
        let loader = self.midi_loader.get_or_insert_with(|| {
            FileLoader::new(
                yinhe_editor_core::progress::new_shared(),
                LoadStageLabels {
                    yin: String::new(),
                    archive: String::new(),
                    yin_decompress: String::new(),
                    yin_rebuild: String::new(),
                    yin_resort: String::new(),
                },
            )
        });
        if loader.is_loading() {
            self.midi_stats = "已在加载中，请等待".to_string();
            return;
        }
        loader.load_path(path.to_string(), yinhe_mid2::MidiImportEncoding::Utf8);
        self.midi_load_start = Some(std::time::Instant::now());
        self.midi_stats = format!("加载中: {path}");
    }

    /// 每帧轮询 MIDI 加载结果，完成后生成统计。
    fn poll_midi_load(&mut self) {
        let Some(loader) = &mut self.midi_loader else {
            return;
        };
        if !loader.is_loading() {
            return;
        }
        // 进度条：stage 0 的 progress/detail。
        if let Ok(p) = loader.load_progress().lock()
            && let Some(stage) = p.stages.first()
            && stage.status != yinhe_editor_core::progress::StageStatus::Done
        {
            let pct = stage.progress * 100.0;
            self.midi_stats = format!("解析中 {pct:.0}% ({})", stage.detail);
        }
        use yinhe_editor_core::file_loading::LoadResult;
        match loader.poll_loading() {
            LoadResult::ModelLoaded { path, model } => {
                let elapsed = self
                    .midi_load_start
                    .take()
                    .map(|t| t.elapsed().as_secs_f32())
                    .unwrap_or(0.0);
                let notes: u64 = model.track_note_count.iter().sum();
                let seconds = model.tempo_map.duration_seconds();
                let tracks = model.tracks.len();
                let minutes = seconds / 60.0;
                self.midi_stats = format!(
                    "加载完成！{} 音符 | {tracks} 轨 | 时长 {minutes:.1} 分钟\n耗时 {elapsed:.1} 秒",
                    notes,
                );
                // 加载完成 → 模型交给音频引擎（播放）与 PR 卷帘视图（渲染）
                let model = std::sync::Arc::new(model);
                if let Some(audio) = &self.audio {
                    audio.handle.send(AudioCommand::LoadModel {
                        model: model.clone(),
                    });
                    log::info!(
                        "audio: LoadModel 已发送 ({} 音符)",
                        model.track_note_count.iter().sum::<u64>()
                    );
                    self.audio_status = "模型已加载到音频引擎，可播放".to_string();
                } else {
                    self.audio_status = "音频未初始化，无法播放".to_string();
                }
                self.model = Some(model.clone());
                self.pr_view.set_model(model);
                self.page = Page::Pr;
                let _ = path;
            }
            LoadResult::ModelFromYin { .. }
            | LoadResult::ArchivePickerNeeded { .. }
            | LoadResult::PasswordNeeded { .. }
            | LoadResult::ArchiveError(_)
            | LoadResult::NotReady => {}
        }
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
    // Material Icons（走带/图标按钮用）；y_offset_factor=0 保证字形居中（同桌面端）。
    let mut font_insert = egui_material_icons::font_insert();
    font_insert.data.tweak.y_offset_factor = 0.0;
    ctx.add_font(font_insert);
}

impl eframe::App for YinheApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // 安卓上无触摸事件时 egui 不重绘（桌面有鼠标移动持续触发）——
        // 请求周期重绘让计时/状态文字持续刷新。
        ctx.request_repaint_after(std::time::Duration::from_millis(100));

        match self.page {
            Page::Verify => self.ui_verify(ui),
            Page::Pr => self.ui_pr(ui),
        }
    }
}

impl YinheApp {
    /// PR 钢琴卷帘页：顶部工具条（返回 + 走带控制 + 工具）+ 视图。
    fn ui_pr(&mut self, ui: &mut egui::Ui) {
        self.update_transport();
        // 工具条保留少量内边距；内容区零边框，PR 视图铺满（不留黑缝）。
        egui::Panel::top("pr_toolbar")
            .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(8, 6)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let icon_text = |icon: egui_material_icons::MaterialIcon| {
                        egui::RichText::new(icon.codepoint)
                            .family(icon.font_family())
                            .size(18.0)
                    };
                    use egui_material_icons::icons::ICON_ARROW_BACK;
                    if ui
                        .button(icon_text(ICON_ARROW_BACK))
                        .on_hover_text("返回")
                        .clicked()
                    {
                        self.page = Page::Verify;
                    }
                    ui.label(egui::RichText::new("钢琴卷帘").strong());
                    self.transport_ui(ui);
                });
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                self.pr_view.ui(ui);
            });
        // 工具选择弹窗：屏幕中央、横向排列（选择/铅笔/橡皮）。
        if self.tool_picker_open {
            egui::Window::new("工具")
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .collapsible(false)
                .resizable(false)
                .show(ui.ctx(), |ui| {
                    ui.horizontal(|ui| {
                        for t in Tool::ALL {
                            let icon = t.icon();
                            let text =
                                egui::RichText::new(format!("{}\n{}", icon.codepoint, t.name()))
                                    .family(icon.font_family())
                                    .size(26.0)
                                    .text_style(egui::TextStyle::Body);
                            if ui.selectable_label(self.tool == t, text).clicked() {
                                self.tool = t;
                                self.tool_picker_open = false;
                            }
                        }
                    });
                });
        }
    }

    /// 走带控制条：播放/暂停、停止、跟随播放开关、BPM、位置/时间显示。
    /// 位置 = 小节.拍.tick、时间 = m:ss.mmm（桌面端 timecode 同款格式），
    /// 点击位置/时间在两者间切换。
    fn transport_ui(&mut self, ui: &mut egui::Ui) {
        if self.model.is_none() {
            ui.label("模型未加载");
            return;
        }
        let Some(audio) = &self.audio else {
            ui.label("音频未初始化");
            return;
        };
        let playing = audio.handle.is_playing();
        use egui_material_icons::icons::{ICON_PAUSE, ICON_PLAY_ARROW, ICON_STOP};
        let play_icon = if playing { ICON_PAUSE } else { ICON_PLAY_ARROW };
        let icon_text = |icon: egui_material_icons::MaterialIcon| {
            egui::RichText::new(icon.codepoint)
                .family(icon.font_family())
                .size(18.0)
        };
        if ui
            .button(icon_text(play_icon))
            .on_hover_text("播放/暂停")
            .clicked()
        {
            if playing {
                audio.handle.send(AudioCommand::Pause);
            } else {
                let from_sample = (self
                    .model
                    .as_ref()
                    .map(|m| m.tempo_map.tick_to_seconds(self.cursor_tick as u64))
                    .unwrap_or(0.0)
                    * audio.sample_rate as f64) as u64;
                audio.handle.send(AudioCommand::Play { from_sample });
            }
        }
        if ui
            .button(icon_text(ICON_STOP))
            .on_hover_text("停止")
            .clicked()
        {
            audio.handle.send(AudioCommand::Stop);
            self.cursor_tick = 0.0;
            self.pr_view.set_cursor(Some(0.0));
        }
        // 跟随播放：图标按钮，选中高亮。
        use egui_material_icons::icons::ICON_CENTER_FOCUS_STRONG;
        if ui
            .add(egui::Button::new(icon_text(ICON_CENTER_FOCUS_STRONG)).selected(self.follow_play))
            .on_hover_text(if self.follow_play {
                "跟随播放：开"
            } else {
                "跟随播放：关"
            })
            .clicked()
        {
            self.follow_play = !self.follow_play;
        }
        // 工具按钮（显示当前工具图标，点击弹出居中工具选择窗）。
        // 位置：跟随之后、BPM 之前。
        if ui
            .button(icon_text(self.tool.icon()))
            .on_hover_text(format!("工具：{}", self.tool.name()))
            .clicked()
        {
            self.tool_picker_open = !self.tool_picker_open;
        }

        let Some(model) = &self.model else {
            return;
        };
        let tm = &model.tempo_map;
        // BPM：当前光标处的速度（tempo 分段变化时随位置更新）。
        let cur_sec = tm.tick_to_seconds(self.cursor_tick as u64);
        ui.label(format!(
            "{} BPM",
            yinhe_types::time_format::format_bpm(tm.bpm_at_time(cur_sec))
        ));
        // 位置（小节.拍.tick）与时间（m:ss.mmm）：点击切换显示。
        let (def_num, def_den) = tm.time_sig_default;
        let pos_str = yinhe_types::time_format::format_tick_bar_beat_with_time_sig(
            self.cursor_tick,
            model.meta.ppq,
            &tm.time_sig_events,
            def_num,
            def_den,
        );
        let time_str = yinhe_types::time_format::format_time(cur_sec);
        let time_resp = ui
            .add(
                egui::Label::new(if self.time_show_ticks {
                    pos_str
                } else {
                    time_str
                })
                .sense(egui::Sense::click()),
            )
            .on_hover_text(if self.time_show_ticks {
                "时间 (秒)".to_string()
            } else {
                "位置 (小节.拍.tick)".to_string()
            });
        if time_resp.clicked() {
            self.time_show_ticks = !self.time_show_ticks;
        }
    }

    /// 每帧从音频引擎同步播放位置：换算 tick 更新播放光标，跟随模式时滚动视口。
    fn update_transport(&mut self) {
        let Some(audio) = &self.audio else {
            return;
        };
        if !audio.handle.is_playing() {
            return;
        }
        let Some(model) = &self.model else {
            return;
        };
        let seconds = audio.handle.sample_position() as f64 / audio.sample_rate as f64;
        let tick = seconds_to_tick(model, seconds);
        self.cursor_tick = tick;
        self.pr_view.set_cursor(Some(tick));
        if self.follow_play {
            self.pr_view.follow_cursor();
        }
    }

    /// 验证页：触摸/音频/MIDI 加载验证。
    fn ui_verify(&mut self, ui: &mut egui::Ui) {
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
            self.poll_sf_load();
            ui.label(
                egui::RichText::new(&self.audio_status)
                    .color(egui::Color32::from_rgb(120, 200, 120)),
            );
            ui.separator();

            // ── 阶段 1：MIDI 加载验证（file_loading 模块）──
            ui.label(egui::RichText::new("MIDI 加载验证").strong());
            ui.horizontal_wrapped(|ui| {
                if ui.button("加载小曲").clicked() {
                    self.start_midi_load(TEST_MIDI_PATH);
                }
                if ui.button("加载大曲").clicked() {
                    self.start_midi_load(BIG_MIDI_PATH);
                }
            });
            self.poll_midi_load();
            ui.label(
                egui::RichText::new(&self.midi_stats).color(egui::Color32::from_rgb(140, 200, 255)),
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

/// 由秒反查 tick：tempo_map 只提供 tick_to_seconds（随 tempo 分段单调递增），
/// 二分 40 次足够收敛到亚 tick 精度，音频播放位置反查用。
fn seconds_to_tick(model: &YinModel, seconds: f64) -> f64 {
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
        // 安卓上窗口状态持久化会走 storage_dir → home_dir → getpwuid_r，
        // 静态链接的 bionic libc 的 getpwuid 路径在部分设备（小米）上崩溃
        //（oem_id_from_name → sscanf → strtod 空指针）。关闭持久化绕开。
        persist_window: false,
        ..Default::default()
    };
    let _ = run(options);
}
