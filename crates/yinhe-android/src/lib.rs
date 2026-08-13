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

mod ar_view;
mod file_picker;
mod insets;
mod pr_view;

/// 阶段 0.5 音频验证用的音色库路径（adb push 到 app 私有目录）。
const TEST_SF_PATH: &str = "/data/data/com.jieneng.yinhe/files/generaluser.sf2";
/// 阶段 1 测试 MIDI：小曲（链路验证）与大曲（性能测试）。
const TEST_MIDI_PATH: &str = "/data/data/com.jieneng.yinhe/files/test.mid";
const BIG_MIDI_PATH: &str = "/data/data/com.jieneng.yinhe/files/big.mid";

/// 页面：菜单（启动页，选歌/设置）/ AR 工程走带（根）/ PR 钢琴卷帘。
#[derive(PartialEq)]
enum Page {
    Menu,
    Ar,
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

/// 银河 MIDI 编辑器 App（安卓触屏版）。
pub struct YinheApp {
    /// 当前页面。
    page: Page,
    /// AR 工程走带（首页）：音轨面板 + GPU 音符视图。
    ar_view: ar_view::ArView,
    /// PR 钢琴卷帘（GPU cull 渲染 + 触摸交互）。
    pr_view: pr_view::PrView,
    /// 音频引擎：cpal(AAudio) + xsynth 全链路。
    audio: Option<CpalAudioHandle>,
    audio_status: String,
    /// 音色加载诊断：开始时刻 + 加载前的已加载计数。
    sf_load_start: Option<std::time::Instant>,
    sf_loaded_baseline: usize,
    /// MIDI 加载（复用 file_loading 模块，后台线程 + 进度）。
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
    /// 安全区 insets（逻辑点）：[left, top, right, bottom]，每帧从 [`insets`] 刷新。
    safe_insets: [f32; 4],
}

impl YinheApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        setup_fonts(&cc.egui_ctx);
        let mut app = Self {
            page: Page::Menu,
            ar_view: ar_view::ArView::new(cc),
            pr_view: pr_view::PrView::new(cc),
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
            safe_insets: [0.0; 4],
        };
        // 启动先进菜单（选歌/设置），不再自动加载测试 MIDI；音频/音色库提前初始化。
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
                self.pr_view.set_model(model.clone());
                self.ar_view.set_model(model);
                self.page = Page::Ar;
                let _ = path;
            }
            LoadResult::ModelFromYin { .. }
            | LoadResult::ArchivePickerNeeded { .. }
            | LoadResult::PasswordNeeded { .. }
            | LoadResult::ArchiveError(_)
            | LoadResult::NotReady => {}
        }
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

/// 图标按钮文字（Material Icons 字形，走带/工具条用）。
fn icon_text(icon: egui_material_icons::MaterialIcon) -> egui::RichText {
    egui::RichText::new(icon.codepoint)
        .family(icon.font_family())
        .size(18.0)
}

/// 顶栏：默认面板背景色 + 挖孔安全区避让 + 对称内边距（按钮垂直居中）。
/// 三个页面（菜单/AR/PR）共用，保证视觉一致。
fn show_toolbar(
    ui: &mut egui::Ui,
    id: &'static str,
    safe: [f32; 4],
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    let [sl, st, sr, _] = safe;
    egui::Panel::top(id)
        .frame(egui::Frame::NONE.fill(ui.visuals().panel_fill))
        .show(ui, |ui| {
            let avail = ui.available_rect_before_wrap();
            // frame margin 是 i8（放不下大 inset），手动缩进：上下对称 8px。
            let inner = egui::Rect::from_min_max(
                avail.min + egui::vec2(sl + 8.0, st + 8.0),
                avail.max - egui::vec2(sr + 8.0, 8.0),
            );
            if inner.width() <= 0.0 || inner.height() <= 0.0 {
                return;
            }
            ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
                ui.horizontal_centered(|ui| {
                    add_contents(ui);
                });
            });
        });
}

impl eframe::App for YinheApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
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

        match self.page {
            Page::Menu => self.ui_menu(ui),
            Page::Ar => self.ui_ar(ui),
            Page::Pr => self.ui_pr(ui),
        }
    }
}

impl YinheApp {
    /// PR 钢琴卷帘页：顶部工具条（返回 + 走带控制 + 工具）+ 视图。
    fn ui_pr(&mut self, ui: &mut egui::Ui) {
        self.update_transport();
        // 顶栏：默认背景色 + 挖孔安全区避让 + 对称内边距。
        show_toolbar(ui, "pr_toolbar", self.safe_insets, |ui| {
            use egui_material_icons::icons::ICON_ARROW_BACK;
            if ui
                .button(icon_text(ICON_ARROW_BACK))
                .on_hover_text("返回")
                .clicked()
            {
                self.page = Page::Ar;
            }
            ui.label(egui::RichText::new("钢琴卷帘").strong());
            self.transport_ui(ui);
        });
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                // 页面背景（含挖孔区域）铺默认面板背景色。
                ui.painter().rect_filled(
                    ui.available_rect_before_wrap(),
                    0.0,
                    ui.visuals().panel_fill,
                );
                self.pr_view.ui(ui, self.safe_insets);
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
            ui.label("未加载工程");
            return;
        }
        let Some(audio) = &self.audio else {
            ui.label("音频未初始化");
            return;
        };
        let playing = audio.handle.is_playing();
        use egui_material_icons::icons::{ICON_PAUSE, ICON_PLAY_ARROW, ICON_STOP};
        let play_icon = if playing { ICON_PAUSE } else { ICON_PLAY_ARROW };
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
            self.ar_view.set_cursor(Some(0.0));
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
        self.ar_view.set_cursor(Some(tick));
        if self.follow_play {
            self.pr_view.follow_cursor();
            self.ar_view.follow_cursor();
        }
    }

    /// AR 首页：顶栏（工程名 + 走带 + 打开 + 设置）+ 音轨面板 + GPU 音符视图。
    fn ui_ar(&mut self, ui: &mut egui::Ui) {
        self.update_transport();
        // 每帧轮询后台加载结果（模型加载完成后留在 AR 页展示）。
        self.poll_midi_load();
        // 顶栏：内边距 + 挖孔安全区避让（同 ui_pr）。
        // 顶栏：默认背景色 + 挖孔安全区避让 + 对称内边距。
        show_toolbar(ui, "ar_toolbar", self.safe_insets, |ui| {
            // 最左侧：主页按钮（进入菜单，文件夹/设置入口合并于此）。
            use egui_material_icons::icons::ICON_HOME;
            if ui
                .button(icon_text(ICON_HOME))
                .on_hover_text("主页")
                .clicked()
            {
                self.page = Page::Menu;
            }
            // 工程名。
            let title = self
                .model
                .as_ref()
                .map(|m| m.meta.name.clone())
                .unwrap_or_else(|| "未命名工程".to_string());
            ui.label(egui::RichText::new(title).strong());
            ui.separator();
            self.transport_ui(ui);
        });
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                // 页面背景（含挖孔区域）铺默认面板背景色。
                ui.painter().rect_filled(
                    ui.available_rect_before_wrap(),
                    0.0,
                    ui.visuals().panel_fill,
                );
                let events = self.ar_view.ui(ui, self.safe_insets);
                for ev in events {
                    match ev {
                        ar_view::ArEvent::EnterPr(track) => {
                            log::info!("AR: 点击轨道 {track}，进入钢琴卷帘");
                            self.page = Page::Pr;
                        }
                        ar_view::ArEvent::SkipTracks(skip) => {
                            if let Some(a) = &self.audio {
                                a.handle.send(AudioCommand::SkipTracks { skip });
                            }
                        }
                    }
                }
            });
    }

    /// 菜单页（启动页）：左上角返回 AR；左侧歌曲卡片 + 本地打开；右侧设置。
    fn ui_menu(&mut self, ui: &mut egui::Ui) {
        // 每帧轮询：选歌后加载完成自动进入 AR；音色/文件选择结果在此消费。
        self.poll_midi_load();
        self.poll_sf_load();
        if let Some(path) = file_picker::take_picked_path() {
            self.start_midi_load(&path);
        }
        // 顶栏：返回（回 AR）+ 标题，默认背景色 + 挖孔避让 + 对称内边距。
        show_toolbar(ui, "menu_toolbar", self.safe_insets, |ui| {
            use egui_material_icons::icons::ICON_ARROW_BACK;
            if ui
                .button(icon_text(ICON_ARROW_BACK))
                .on_hover_text("返回工程")
                .clicked()
            {
                self.page = Page::Ar;
            }
            ui.label(egui::RichText::new("菜单").strong());
        });
        // 左右分栏：左侧选歌，右侧设置。
        let [sl, st, sr, sb] = self.safe_insets;
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                let avail = ui.available_rect_before_wrap();
                // 页面背景（含挖孔区域）铺默认面板背景色。
                ui.painter()
                    .rect_filled(avail, 0.0, ui.visuals().panel_fill);
                let inner = egui::Rect::from_min_max(
                    avail.min + egui::vec2(sl, st),
                    avail.max - egui::vec2(sr, sb),
                );
                if inner.width() <= 0.0 || inner.height() <= 0.0 {
                    return;
                }
                let left_w = (inner.width() * 0.55).clamp(280.0, 520.0);
                let left_rect = egui::Rect::from_min_max(
                    inner.min,
                    egui::pos2(inner.min.x + left_w, inner.max.y),
                );
                let right_rect = egui::Rect::from_min_max(
                    egui::pos2(inner.min.x + left_w + 12.0, inner.min.y),
                    inner.max,
                );
                ui.scope_builder(egui::UiBuilder::new().max_rect(left_rect), |ui| {
                    self.menu_songs_ui(ui);
                });
                ui.scope_builder(egui::UiBuilder::new().max_rect(right_rect), |ui| {
                    self.menu_settings_ui(ui);
                });
            });
    }

    /// 菜单左侧：歌曲卡片（测试曲目）+ 本地打开（SAF 文件选择器）。
    fn menu_songs_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("选择歌曲");
        ui.add_space(10.0);
        let card_w = ui.available_width();
        let cards = [
            ("小曲", "test.mid（链路验证）", TEST_MIDI_PATH),
            ("大曲", "big.mid（性能测试）", BIG_MIDI_PATH),
        ];
        for (title, desc, path) in cards {
            if ui
                .add_sized(
                    [card_w, 64.0],
                    egui::Button::new(egui::RichText::new(title).size(18.0).strong()),
                )
                .on_hover_text(desc)
                .clicked()
            {
                self.start_midi_load(path);
            }
            ui.label(
                egui::RichText::new(desc)
                    .small()
                    .color(ui.visuals().weak_text_color()),
            );
            ui.add_space(8.0);
        }
        // 本地打开：SAF 系统文件选择器（MainActivity 桥）。
        if ui
            .add_sized(
                [card_w, 56.0],
                egui::Button::new(egui::RichText::new("本地打开 MIDI").size(16.0)),
            )
            .clicked()
        {
            file_picker::open_file_picker();
        }
        ui.add_space(10.0);
        // 加载进度/结果。
        if !self.midi_stats.is_empty() {
            ui.label(
                egui::RichText::new(&self.midi_stats).color(egui::Color32::from_rgb(140, 200, 255)),
            );
        }
    }

    /// 菜单右侧：设置（音色库 + 音频状态）。
    fn menu_settings_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("设置");
        ui.add_space(10.0);
        ui.label(egui::RichText::new("音色库").strong());
        ui.label(&self.audio_status);
        if self.sf_load_start.is_some() {
            ui.label("音色加载中...");
        }
        if ui.button("重新加载音色库").clicked() {
            self.load_soundfont();
        }
        ui.separator();
        ui.label(egui::RichText::new("音频").strong());
        let sr = self.audio.as_ref().map(|a| a.sample_rate).unwrap_or(0);
        ui.label(format!("采样率: {sr} Hz"));
        let playing = self
            .audio
            .as_ref()
            .map(|a| a.handle.is_playing())
            .unwrap_or(false);
        ui.label(format!(
            "播放状态: {}",
            if playing { "播放中" } else { "停止" }
        ));
        if self.audio.is_none() && ui.button("初始化音频").clicked() {
            self.init_audio();
        }
    }
}

/// 轨道颜色：TRACK_PALETTE 循环分配（与桌面端 track_panel/AR 一致）。
/// PR 与 AR 共用，保证同一工程两个视图的轨道色相同。
pub(crate) fn track_colors_for(model: &YinModel) -> Vec<[f32; 4]> {
    yinhe_theme::palette::TRACK_PALETTE
        .iter()
        .cycle()
        .take(model.tracks.len())
        .map(|&[r, g, b]| [r, g, b, 1.0])
        .collect()
}

/// 加载项目自带字体（与桌面端一致的 Pretendard 主字体 + MiSans 中文回退）。
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
        // 安卓上窗口状态持久化会走 storage_dir → home_dir → getpwuid_r，
        // 静态链接的 bionic libc 的 getpwuid 路径在部分设备（小米）上崩溃
        //（oem_id_from_name → sscanf → strtod 空指针）。关闭持久化绕开。
        persist_window: false,
        ..Default::default()
    };
    let _ = run(options);
}
