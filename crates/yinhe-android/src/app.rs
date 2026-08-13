//! 应用根：YinheApp 结构与音频/MIDI 生命周期。
//! 页面 UI（菜单/AR/PR/走带）在 `pages/` 各模块中，按页面解耦。

use std::sync::Arc;

use yinhe_audio::spawn::{AudioCommand, CpalAudioHandle};
use yinhe_core::YinModel;

use crate::ar_view::ArView;
use crate::pr_view::PrView;

/// 页面：菜单（启动页，选歌/设置）/ AR 工程走带（根）/ PR 钢琴卷帘。
#[derive(PartialEq)]
pub(crate) enum Page {
    Menu,
    Ar,
    Pr,
}

/// 编辑工具（初期只做选择 UI 与状态，实际编辑功能后续接入）。
/// 图标与桌面端 tools_panel 一致。
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Tool {
    Select,
    Pencil,
    Eraser,
}

impl Tool {
    pub(crate) const ALL: [Tool; 3] = [Tool::Select, Tool::Pencil, Tool::Eraser];

    pub(crate) fn icon(self) -> egui_material_icons::MaterialIcon {
        use egui_material_icons::icons::*;
        match self {
            Self::Select => ICON_SELECT,
            Self::Pencil => ICON_EDIT,
            Self::Eraser => ICON_INK_ERASER,
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Select => "选择",
            Self::Pencil => "铅笔",
            Self::Eraser => "橡皮",
        }
    }
}

/// 银河 MIDI 编辑器 App（安卓触屏版）。
/// 字段 pub(crate)：页面 UI 分散在 pages/ 模块，需访问共享状态。
pub(crate) struct YinheApp {
    /// 当前页面。
    pub(crate) page: Page,
    /// AR 工程走带（首页）：音轨面板 + GPU 音符视图。
    pub(crate) ar_view: ArView,
    /// PR 钢琴卷帘（GPU cull 渲染 + 触摸交互）。
    pub(crate) pr_view: PrView,
    /// 音频引擎：cpal(AAudio) + xsynth 全链路。
    pub(crate) audio: Option<CpalAudioHandle>,
    pub(crate) audio_status: String,
    /// 音色加载诊断：开始时刻 + 加载前的已加载计数。
    pub(crate) sf_load_start: Option<std::time::Instant>,
    pub(crate) sf_loaded_baseline: usize,
    /// MIDI 加载（复用 file_loading 模块，后台线程 + 进度）。
    pub(crate) midi_loader: Option<yinhe_editor_core::file_loading::FileLoader>,
    pub(crate) midi_load_start: Option<std::time::Instant>,
    /// 最近一次加载结果统计。
    pub(crate) midi_stats: String,
    /// 当前 MIDI 模型（加载完成后的唯一引用，音频引擎与 PR 视图共享同一份）。
    pub(crate) model: Option<Arc<YinModel>>,
    /// 播放光标（tick）：暂停/停止后保留，作为下次播放起点。
    pub(crate) cursor_tick: f64,
    /// 跟随播放：开启后滚动让光标始终位于内容区中央。
    pub(crate) follow_play: bool,
    /// 走带位置/时间显示：false = 时间 m:ss.mmm，true = 位置 小节.拍.tick。
    pub(crate) time_show_ticks: bool,
    /// 当前编辑工具（工具弹窗选择）。
    pub(crate) tool: Tool,
    /// 工具选择弹窗是否打开。
    pub(crate) tool_picker_open: bool,
    /// 安全区 insets（逻辑点）：[left, top, right, bottom]，每帧从 [`crate::insets`] 刷新。
    pub(crate) safe_insets: [f32; 4],
}

impl YinheApp {
    pub(crate) fn new(cc: &eframe::CreationContext<'_>) -> Self {
        crate::setup_fonts(&cc.egui_ctx);
        let mut app = Self {
            page: Page::Menu,
            ar_view: ArView::new(cc),
            pr_view: PrView::new(cc),
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
    pub(crate) fn init_audio(&mut self) {
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
    pub(crate) fn load_soundfont(&mut self) {
        let Some(audio) = &self.audio else {
            self.audio_status = "请先初始化音频".to_string();
            return;
        };
        if !std::path::Path::new(crate::TEST_SF_PATH).exists() {
            log::error!("audio: 音色库不存在 {}", crate::TEST_SF_PATH);
            self.audio_status = format!("音色库不存在: {}", crate::TEST_SF_PATH);
            return;
        }
        audio.handle.send(AudioCommand::LoadSoundFont {
            port: 0,
            paths: vec![crate::TEST_SF_PATH.to_string()],
        });
        self.sf_load_start = Some(std::time::Instant::now());
        self.sf_loaded_baseline = audio.handle.sf_loaded_count();
        self.audio_status = "音色加载中（大文件需几秒），稍后点播放...".to_string();
    }

    /// 每帧更新音色加载状态：轮询 sf_loaded_count 显示完成/耗时。
    pub(crate) fn poll_sf_load(&mut self) {
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
    pub(crate) fn start_midi_load(&mut self, path: &str) {
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

    /// 每帧轮询 MIDI 加载结果，完成后生成统计并进入 AR 页。
    pub(crate) fn poll_midi_load(&mut self) {
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
                // 加载完成 → 模型交给音频引擎（播放）与 AR/PR 视图（渲染）
                let model = Arc::new(model);
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
