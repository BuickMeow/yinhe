use deunicode::deunicode_char;
use eframe::egui;
use pinyin::ToPinyin;
use rust_i18n::t;

use yinhe_editor_core::shortcuts::{self, KeyCombo};
use yinhe_theme::base::{BaseColors, Rgba};

use crate::audio_settings::AudioSettings;

// ── 设置分类（左侧导航，顺序即 settings_tab 索引） ──

const CATEGORY_KEYS: [&str; 6] = [
    "settings.cat.theme",
    "settings.cat.language",
    "settings.cat.audio",
    "settings.cat.render",
    "settings.cat.shortcuts",
    "settings.cat.general",
];

/// 设置项注册表（供搜索）：各语言名称均可直接检索。
/// 中文额外支持拼音（由 `to_search_keys` 运行时折叠，无需手写转写）。
struct SettingItem {
    cat: usize,
    zh: &'static str,
    en: &'static str,
    ja: &'static str,
    ko: &'static str,
}

const SETTING_ITEMS: &[SettingItem] = &[
    SettingItem {
        cat: 0,
        zh: "主题预设",
        en: "Theme preset",
        ja: "テーマプリセット",
        ko: "테마 프리셋",
    },
    SettingItem {
        cat: 0,
        zh: "背景",
        en: "Background color",
        ja: "背景色",
        ko: "배경색",
    },
    SettingItem {
        cat: 0,
        zh: "主文字",
        en: "Text color",
        ja: "テキスト色",
        ko: "텍스트 색",
    },
    SettingItem {
        cat: 0,
        zh: "强调色",
        en: "Accent color",
        ja: "アクセント色",
        ko: "강조색",
    },
    SettingItem {
        cat: 0,
        zh: "危险色",
        en: "Danger color",
        ja: "危険色",
        ko: "위험 색",
    },
    SettingItem {
        cat: 0,
        zh: "警告色",
        en: "Warning color",
        ja: "警告色",
        ko: "경고 색",
    },
    SettingItem {
        cat: 1,
        zh: "语言",
        en: "Language",
        ja: "言語",
        ko: "언어",
    },
    SettingItem {
        cat: 2,
        zh: "输出设备",
        en: "Output device",
        ja: "出力デバイス",
        ko: "출력 장치",
    },
    SettingItem {
        cat: 2,
        zh: "采样率",
        en: "Sample rate",
        ja: "サンプルレート",
        ko: "샘플 레이트",
    },
    SettingItem {
        cat: 2,
        zh: "缓冲区大小",
        en: "Buffer size",
        ja: "バッファサイズ",
        ko: "버퍼 크기",
    },
    SettingItem {
        cat: 2,
        zh: "合成器层数",
        en: "Synth layers",
        ja: "シンセレイヤー",
        ko: "신스 레이어",
    },
    SettingItem {
        cat: 2,
        zh: "合成引擎",
        en: "Synth engine",
        ja: "シンセエンジン",
        ko: "신스 엔진",
    },
    SettingItem {
        cat: 3,
        zh: "滚动模式",
        en: "Scroll mode",
        ja: "スクロールモード",
        ko: "스크롤 모드",
    },
    SettingItem {
        cat: 3,
        zh: "自动化密度",
        en: "Automation density",
        ja: "オートメーション密度",
        ko: "자동화 밀도",
    },
    SettingItem {
        cat: 3,
        zh: "音符描边",
        en: "Note outline",
        ja: "ノート枠線",
        ko: "노트 외곽선",
    },
    SettingItem {
        cat: 3,
        zh: "最小边框宽度",
        en: "Min border width",
        ja: "最小枠線幅",
        ko: "최소 테두리 폭",
    },
    SettingItem {
        cat: 3,
        zh: "GPU 裁剪",
        en: "GPU culling",
        ja: "GPU カリング",
        ko: "GPU 컬링",
    },
    SettingItem {
        cat: 1,
        zh: "MIDI 导入编码",
        en: "MIDI import encoding",
        ja: "MIDI インポートエンコーディング",
        ko: "MIDI 가져오기 인코딩",
    },
    SettingItem {
        cat: 0,
        zh: "界面缩放",
        en: "UI scale",
        ja: "UI スケール",
        ko: "UI 배율",
    },
    SettingItem {
        cat: 2,
        zh: "刷新设备列表",
        en: "Refresh devices",
        ja: "デバイス更新",
        ko: "장치 새로고침",
    },
    SettingItem {
        cat: 4,
        zh: "快捷键",
        en: "Shortcuts",
        ja: "ショートカット",
        ko: "단축키",
    },
    SettingItem {
        cat: 4,
        zh: "恢复默认快捷键",
        en: "Reset shortcuts",
        ja: "ショートカット初期化",
        ko: "단축키 초기화",
    },
    SettingItem {
        cat: 5,
        zh: "恢复出厂设置",
        en: "Factory reset",
        ja: "工場出荷時リセット",
        ko: "공장 초기화",
    },
];

/// 归一化：小写并去掉所有空白（拼音/罗马音的带空格输入也能匹配）。
fn norm(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect()
}

/// 把任意语言文本折叠成检索键：`(全拼/原文, 汉字拼音首字母缩写)`。
/// - 汉字逐字转拼音（常见读音；ü 折叠成 v，兼容键盘 lv 输入），
///   并收集首字母供缩写搜索（如 zt→主题）；
/// - 其余字符经 deunicode 折叠（é→e、假名→罗马音等，为未来多语言做准备）。
fn to_search_keys(s: &str) -> (String, String) {
    let mut full = String::new();
    let mut initials = String::new();
    for c in s.chars() {
        if let Some(py) = c.to_pinyin() {
            full.push_str(&py.plain().replace('ü', "v"));
            initials.push_str(py.first_letter());
        } else if let Some(lat) = deunicode_char(c) {
            full.push_str(lat);
        } else if !c.is_whitespace() {
            full.push(c);
        }
    }
    (norm(&full), initials)
}

/// 搜索词是否匹配设置项：任意语言名称原文、中文拼音或其首字母缩写。
fn item_matches(item: &SettingItem, query: &str) -> bool {
    let q = norm(query);
    let qf = to_search_keys(query).0;
    [item.zh, item.en, item.ja, item.ko].iter().any(|name| {
        let (full, initials) = to_search_keys(name);
        full.contains(&qf) || (!initials.is_empty() && initials.contains(&q))
    })
}

/// 右侧内容：有搜索词时显示跨分类搜索结果，否则显示当前分类。
fn show_search_results(
    ui: &mut egui::Ui,
    settings: &mut AudioSettings,
    main_ctx: &egui::Context,
) -> bool {
    let query = settings.settings_search.trim().to_string();
    if query.is_empty() {
        return match settings.settings_tab {
            0 => show_theme_tab(ui, settings, main_ctx),
            1 => show_language_tab(ui, settings),
            2 => show_audio_tab(ui, settings),
            3 => show_render_tab(ui, settings),
            4 => show_shortcuts_tab(ui, settings),
            _ => show_general_tab(ui, settings),
        };
    }

    ui.heading(t!("settings.search_results").as_ref());
    ui.add_space(6.0);
    let mut matched = 0usize;
    for item in SETTING_ITEMS {
        if !item_matches(item, &query) {
            continue;
        }
        matched += 1;
        let cat = item.cat;
        let cat_name = t!(CATEGORY_KEYS[cat]).to_string();
        if ui
            .selectable_label(false, format!("{}  ·  {}", cat_name, item.zh))
            .clicked()
        {
            settings.settings_tab = cat;
            settings.settings_search.clear();
        }
    }
    if matched == 0 {
        ui.colored_label(
            crate::theme::text_disabled(),
            t!("settings.search_none").as_ref(),
        );
    }
    false
}

/// 编辑一个标准色（调色板弹窗：RGBA/HSV 数值可切换 ↔ 主题 Rgba）。
fn edit_std_color(ui: &mut egui::Ui, label: &str, rgba: &mut Rgba) -> bool {
    ui.label(label);
    let mut c = rgba.to_color32();
    let changed = crate::widgets::color_picker::color_edit_button(ui, &mut c).changed();
    if changed {
        *rgba = Rgba::from_color32(c);
    }
    changed
}

// ── 各分类内容 ──

fn show_theme_tab(
    ui: &mut egui::Ui,
    settings: &mut AudioSettings,
    main_ctx: &egui::Context,
) -> bool {
    let mut changed = false;

    ui.heading(t!("settings.theme.heading").as_ref());
    ui.add_space(8.0);

    egui::Grid::new("theme_preset_grid")
        .num_columns(2)
        .spacing([12.0, 8.0])
        .show(ui, |ui| {
            ui.label(t!("settings.theme.preset").as_ref());
            // 预设列表来自 BaseColors::PRESETS，i18n key 为 settings.theme.<name>
            //（name 中的 '-' 换成 '_'），末尾追加"自定义"。
            let preset_names: Vec<(String, String)> = BaseColors::PRESETS
                .iter()
                .map(|(n, _)| {
                    let key = format!("settings.theme.{}", n.replace('-', "_"));
                    (n.to_string(), t!(key.as_str()).to_string())
                })
                .chain(std::iter::once((
                    "custom".to_string(),
                    t!("settings.theme.custom").to_string(),
                )))
                .collect();
            let current_preset = preset_names
                .iter()
                .find(|(n, _)| *n == settings.theme_preset)
                .map(|(_, l)| l.clone())
                .unwrap_or_else(|| t!("settings.theme.custom").to_string());
            egui::ComboBox::from_id_salt("theme_preset")
                .selected_text(current_preset)
                .show_ui(ui, |ui| {
                    for (name, label) in preset_names {
                        let selected = settings.theme_preset == name;
                        if ui.selectable_label(selected, label).clicked() {
                            settings.theme_preset = name.clone();
                            if let Some(base) = BaseColors::preset_by_name(&name) {
                                settings.theme_base = base;
                            }
                            crate::theme::set_theme(settings.theme_base);
                            changed = true;
                        }
                    }
                });
            ui.end_row();
        });

    ui.add_space(4.0);
    let mut base = settings.theme_base;
    let mut base_changed = false;
    egui::Grid::new("theme_colors_grid")
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            base_changed |= edit_std_color(ui, t!("settings.theme.bg").as_ref(), &mut base.bg);
            ui.end_row();
            base_changed |= edit_std_color(ui, t!("settings.theme.text").as_ref(), &mut base.text);
            ui.end_row();
            base_changed |=
                edit_std_color(ui, t!("settings.theme.accent").as_ref(), &mut base.accent);
            ui.end_row();
            base_changed |=
                edit_std_color(ui, t!("settings.theme.danger").as_ref(), &mut base.danger);
            ui.end_row();
            base_changed |=
                edit_std_color(ui, t!("settings.theme.warning").as_ref(), &mut base.warning);
            ui.end_row();
        });
    if base_changed {
        settings.theme_base = base;
        settings.theme_preset = "custom".to_string();
        crate::theme::set_theme(base);
        changed = true;
    }

    // 界面缩放：拖动中不缩放（缩放会让滑条自身位置来回跑），松手才应用
    egui::Grid::new("theme_scale_grid")
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            ui.label(t!("settings.theme.ui_scale").as_ref());
            ui.horizontal(|ui| {
                let mut scale = settings.ui_scale;
                let resp = ui.add(
                    egui::Slider::new(&mut scale, 0.75..=2.0)
                        .step_by(0.05)
                        .show_value(true),
                );
                if resp.changed() {
                    settings.ui_scale = scale;
                    changed = true;
                }
                if resp.drag_stopped() {
                    main_ctx.set_zoom_factor(settings.ui_scale);
                }
                if ui
                    .button(t!("settings.theme.reset_scale").as_ref())
                    .clicked()
                {
                    settings.ui_scale = 1.0;
                    main_ctx.set_zoom_factor(1.0);
                    changed = true;
                }
            });
            ui.end_row();
        });
    changed
}

fn show_language_tab(ui: &mut egui::Ui, settings: &mut AudioSettings) -> bool {
    let mut changed = false;
    ui.heading(t!("settings.language").as_ref());
    ui.add_space(8.0);
    egui::Grid::new("language_grid")
        .num_columns(2)
        .spacing([12.0, 8.0])
        .show(ui, |ui| {
            ui.label(t!("settings.language").as_ref());
            let locales = [
                ("zh-CN", "简体中文"),
                ("zh-HK", "繁體中文（香港）"),
                ("zh-TW", "繁體中文（台灣）"),
                ("bo-CN", "བོད་སྐད།"),
                ("en-US", "English"),
                ("de-DE", "Deutsch"),
                ("es-ES", "Español"),
                ("fr-FR", "Français"),
                ("it-IT", "Italiano"),
                ("pl-PL", "Polski"),
                ("pt-PT", "Português (Portugal)"),
                ("pt-BR", "Português (Brasil)"),
                ("ru-RU", "Русский"),
                ("tr-TR", "Türkçe"),
                ("uk-UA", "Українська"),
                ("fil-PH", "Filipino"),
                ("ja-JP", "日本語"),
                ("ko-KR", "한국어"),
                ("hi-IN", "हिन्दी"),
                ("id-ID", "Bahasa Indonesia"),
                ("ms-MY", "Bahasa Melayu"),
                ("th-TH", "ไทย"),
                ("vi-VN", "Tiếng Việt"),
                ("lo-LA", "ລາວ"),
                ("my-MM", "မြန်မာ"),
                ("km-KH", "ខ្មែរ"),
            ];
            let current = locales
                .iter()
                .find(|(code, _)| *code == settings.locale)
                .map(|(_, name)| *name)
                .unwrap_or("简体中文");
            egui::ComboBox::from_id_salt("locale_select")
                .selected_text(current)
                .show_ui(ui, |ui| {
                    for (code, name) in locales {
                        let selected = settings.locale == code;
                        if ui.selectable_label(selected, name).clicked() {
                            settings.locale = code.to_string();
                            rust_i18n::set_locale(code);
                            changed = true;
                        }
                    }
                });
            ui.end_row();

            // 音轨名编码（MIDI 文件内文本的语言编码，归类到语言设置）
            ui.label(t!("settings.midi_import.encoding").as_ref());
            egui::ComboBox::from_id_salt("midi_import_encoding")
                .selected_text(settings.midi_import_encoding.label())
                .show_ui(ui, |ui| {
                    for &enc in yinhe_mid2::MidiImportEncoding::ALL {
                        let selected = settings.midi_import_encoding == enc;
                        if ui.selectable_label(selected, enc.label()).clicked() {
                            settings.midi_import_encoding = enc;
                            changed = true;
                        }
                    }
                });
            ui.end_row();
        });
    changed
}

fn show_audio_tab(ui: &mut egui::Ui, settings: &mut AudioSettings) -> bool {
    let mut changed = false;
    ui.heading(t!("settings.audio.heading").as_ref());
    ui.add_space(8.0);

    egui::Grid::new("audio_settings_grid")
        .num_columns(2)
        .spacing([12.0, 8.0])
        .show(ui, |ui| {
            ui.label(t!("settings.audio.output_device").as_ref());
            let default_device = t!("settings.audio.default_device").to_string();
            let current_device = settings
                .output_device_name
                .as_deref()
                .unwrap_or(default_device.as_str());
            egui::ComboBox::from_id_salt("output_device")
                .selected_text(current_device)
                .show_ui(ui, |ui| {
                    for device_name in settings.available_devices().to_vec() {
                        let selected = settings.output_device_name.as_ref() == Some(&device_name);
                        if ui.selectable_label(selected, &device_name).clicked() {
                            settings.output_device_name = Some(device_name);
                            changed = true;
                        }
                    }
                    let is_default = settings.output_device_name.is_none();
                    if ui
                        .selectable_label(is_default, t!("settings.audio.default_device").as_ref())
                        .clicked()
                    {
                        settings.output_device_name = None;
                        changed = true;
                    }
                });
            ui.end_row();

            ui.label(t!("settings.audio.sample_rate").as_ref());
            let sr_label = format!("{} Hz", settings.sample_rate);
            egui::ComboBox::from_id_salt("sample_rate")
                .selected_text(&sr_label)
                .show_ui(ui, |ui| {
                    for sr in settings.available_sample_rates().to_vec() {
                        let selected = settings.sample_rate == sr;
                        if ui
                            .selectable_label(selected, format!("{} Hz", sr))
                            .clicked()
                        {
                            settings.sample_rate = sr;
                            changed = true;
                        }
                    }
                });
            ui.end_row();

            ui.label(t!("settings.audio.buffer_size").as_ref());
            let buf_sizes: &[(u32, String)] = &[
                (0, t!("settings.audio.buffer.default").to_string()),
                (128, t!("settings.audio.buffer.frames", n = 128).to_string()),
                (256, t!("settings.audio.buffer.frames", n = 256).to_string()),
                (512, t!("settings.audio.buffer.frames", n = 512).to_string()),
                (
                    1024,
                    t!("settings.audio.buffer.frames", n = 1024).to_string(),
                ),
                (
                    2048,
                    t!("settings.audio.buffer.frames", n = 2048).to_string(),
                ),
                (
                    4096,
                    t!("settings.audio.buffer.frames", n = 4096).to_string(),
                ),
            ];
            let custom_buf = t!("settings.audio.buffer.custom").to_string();
            let buf_label = buf_sizes
                .iter()
                .find(|(v, _)| *v == settings.buffer_size)
                .map(|(_, l)| l.as_str())
                .unwrap_or(custom_buf.as_str());
            egui::ComboBox::from_id_salt("buffer_size")
                .selected_text(buf_label)
                .show_ui(ui, |ui| {
                    for &(val, ref label) in buf_sizes {
                        let selected = settings.buffer_size == val;
                        if ui.selectable_label(selected, label).clicked() {
                            settings.buffer_size = val;
                            changed = true;
                        }
                    }
                });
            ui.end_row();

            ui.label(t!("settings.audio.xsynth_layers").as_ref());
            let mut layers = settings.xsynth_layers as usize;
            if ui
                .add(
                    crate::widgets::numeric_input::decimal_drag_value(&mut layers)
                        .range(0..=128)
                        .speed(1.0),
                )
                .changed()
            {
                settings.xsynth_layers = layers as u32;
                changed = true;
            }
            let layer_label = if settings.xsynth_layers == 0 {
                t!("common.unlimited").to_string()
            } else {
                String::new()
            };
            if !layer_label.is_empty() {
                ui.label(layer_label);
            }
            ui.end_row();

            ui.label(t!("settings.audio.synth_engine").as_ref());
            let engine_names = [
                t!("settings.audio.engine_cpu").to_string(),
                t!("settings.audio.engine_gpu").to_string(),
            ];
            let current_engine = if settings.use_gpu_synth { 1 } else { 0 };
            egui::ComboBox::from_id_salt("synth_engine")
                .selected_text(engine_names[current_engine].clone())
                .show_ui(ui, |ui| {
                    for (i, name) in engine_names.iter().enumerate() {
                        let selected = (i == 1) == settings.use_gpu_synth;
                        if ui.selectable_label(selected, name).clicked() {
                            settings.use_gpu_synth = i == 1;
                            changed = true;
                        }
                    }
                });
            ui.end_row();
        });

    // 设备列表变更（热插拔等）后手动刷新
    if ui.button(t!("settings.refresh_devices").as_ref()).clicked() {
        let devices = crate::audio_settings::list_output_devices();
        let (default_rate, rates) = crate::audio_settings::discover_sample_rates();
        settings.refresh_devices(devices, rates, default_rate);
        changed = true;
    }
    changed
}

fn show_render_tab(ui: &mut egui::Ui, settings: &mut AudioSettings) -> bool {
    let mut changed = false;
    ui.heading(t!("settings.render.heading").as_ref());
    ui.add_space(8.0);

    egui::Grid::new("render_settings_grid")
        .num_columns(2)
        .spacing([12.0, 8.0])
        .show(ui, |ui| {
            ui.label(t!("settings.render.scroll_mode").as_ref());
            let mode_names = [
                t!("settings.render.scroll.raw").to_string(),
                t!("settings.render.scroll.integer").to_string(),
                t!("settings.render.scroll.subpixel").to_string(),
            ];
            let current = settings.scroll_mode as usize;
            egui::ComboBox::from_id_salt("scroll_mode")
                .selected_text(mode_names[current].clone())
                .show_ui(ui, |ui| {
                    for (i, name) in mode_names.iter().enumerate() {
                        let selected = settings.scroll_mode == i as u32;
                        if ui.selectable_label(selected, name).clicked() {
                            settings.scroll_mode = i as u32;
                            changed = true;
                        }
                    }
                });
            ui.end_row();

            ui.label(t!("settings.render.automation_density").as_ref());
            let mut density = settings.automation_event_density as i32;
            let drag = ui.add(
                crate::widgets::numeric_input::decimal_drag_value(&mut density)
                    .range(1..=480)
                    .speed(0.2)
                    .suffix(" tick"),
            );
            if drag.changed() {
                settings.automation_event_density = density.max(1) as u32;
                changed = true;
            }
            ui.end_row();

            ui.label(t!("settings.render.note_outline").as_ref());
            if crate::widgets::checkbox::checkbox(ui, &mut settings.note_outline, "").changed() {
                changed = true;
            }
            ui.end_row();

            ui.label(t!("settings.render.min_border_width").as_ref());
            let mut bw = settings.min_border_width;
            if ui
                .add(egui::Slider::new(&mut bw, 0.0..=5.0).step_by(0.5))
                .changed()
            {
                settings.min_border_width = bw;
                changed = true;
            }
            ui.end_row();

            ui.label(t!("settings.render.content_opacity").as_ref());
            let mut op = settings.content_opacity;
            if ui.add(egui::Slider::new(&mut op, 0.0..=1.0)).changed() {
                settings.content_opacity = op;
                changed = true;
            }
            ui.end_row();

            ui.label(t!("settings.render.gpu_cull").as_ref());
            if crate::widgets::checkbox::checkbox(ui, &mut settings.use_gpu_cull, "").changed() {
                changed = true;
            }
            ui.end_row();
        });
    changed
}

/// 快捷键配置页：列出全部可配置动作。
/// 每个动作可绑定多个快捷键：第一个快捷键与动作名同行，其余各占一行；
/// 点击快捷键按钮重新录制，点击 + 追加一个，点击 × 移除。
/// 录制期间 `settings.shortcut_recording` 置位，让全局快捷键让位给录制器。
fn show_shortcuts_tab(ui: &mut egui::Ui, settings: &mut AudioSettings) -> bool {
    let mut changed = false;

    let rec_id = egui::Id::new("kb_recording_action");
    // 录制目标：(动作 id, 快捷键索引)；索引 == 列表长度表示追加新快捷键
    let recording: Option<(String, usize)> = ui.data(|d| d.get_temp(rec_id));

    // ── 录制：捕获本帧按键（Esc 取消）──
    if let Some((action_id, idx)) = &recording {
        let mut captured: Option<Option<KeyCombo>> = None; // None = 取消
        let events = ui.input(|i| i.events.clone());
        for ev in &events {
            if let egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } = ev
            {
                if *key == egui::Key::Escape {
                    captured = Some(None);
                    break;
                }
                if crate::shortcuts::is_recordable_key(*key) {
                    captured = Some(Some(KeyCombo {
                        command: modifiers.command || modifiers.ctrl,
                        shift: modifiers.shift,
                        alt: modifiers.alt,
                        key: crate::shortcuts::key_to_str(*key),
                    }));
                    break;
                }
            }
        }
        if let Some(combo) = captured {
            if let Some(combo) = combo {
                let mut combos = settings.keybindings.get(action_id);
                if *idx < combos.len() {
                    combos[*idx] = combo; // 替换现有快捷键
                } else {
                    combos.push(combo); // 追加新快捷键
                }
                settings.keybindings.set(action_id, combos);
                changed = true;
            }
            ui.data_mut(|d| d.remove::<(String, usize)>(rec_id));
            settings.shortcut_recording = false;
        }
    }

    // ── 标题 + 提示 ──
    ui.heading(t!("settings.shortcuts.title").as_ref());
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new(t!("settings.shortcuts.hint").as_ref())
            .color(crate::theme::text_secondary()),
    );
    ui.add_space(6.0);

    // ── 分组列表 ──
    let groups: [(&str, &[&str]); 3] = [
        (
            "settings.shortcuts.group_file",
            &[
                shortcuts::ACTION_NEW_PROJECT,
                shortcuts::ACTION_OPEN,
                shortcuts::ACTION_SAVE,
                shortcuts::ACTION_SAVE_AS,
                shortcuts::ACTION_CLOSE_DOCUMENT,
                shortcuts::ACTION_EXPORT_AUDIO,
                shortcuts::ACTION_EXPORT_MIDI,
                shortcuts::ACTION_SETTINGS,
                shortcuts::ACTION_EXIT,
            ],
        ),
        (
            "settings.shortcuts.group_edit",
            &[
                shortcuts::ACTION_UNDO,
                shortcuts::ACTION_REDO,
                shortcuts::ACTION_CUT,
                shortcuts::ACTION_COPY,
                shortcuts::ACTION_PASTE,
                shortcuts::ACTION_SELECT_ALL,
                shortcuts::ACTION_DUPLICATE,
                shortcuts::ACTION_DELETE,
                shortcuts::ACTION_TRANSPOSE_UP,
                shortcuts::ACTION_TRANSPOSE_DOWN,
            ],
        ),
        (
            "settings.shortcuts.group_play",
            &[shortcuts::ACTION_TOGGLE_PLAY, shortcuts::ACTION_STOP],
        ),
    ];

    for (group_key, ids) in groups {
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new(t!(group_key).as_ref())
                .strong()
                .color(crate::theme::text_secondary()),
        );
        for &id in ids {
            let combos = settings.keybindings.get(id);
            let add_idx = combos.len();
            // 是否正在为该动作追加新快捷键（录制中）
            let is_adding = recording
                .as_ref()
                .is_some_and(|(a, idx)| a.as_str() == id && *idx == add_idx);

            // 第一行：动作名 + 第一个快捷键 + ×（+ 追加按钮）
            // 动作名标签用 Extend 撑满 150 宽，保证与后续缩进行精确对齐
            ui.horizontal(|ui| {
                let label_key = crate::shortcuts::action_label_key(id);
                ui.add_sized(
                    [150.0, 24.0],
                    egui::Label::new(egui::RichText::new(t!(label_key).as_ref()))
                        .selectable(false)
                        .wrap_mode(egui::TextWrapMode::Extend),
                );

                if let Some(first) = combos.first() {
                    changed |= shortcut_combo_ui(ui, rec_id, id, 0, first, &recording, settings);
                }

                // 追加新快捷键
                let add_btn = egui::Button::new(if is_adding {
                    egui::RichText::new(t!("settings.shortcuts.recording").as_ref())
                } else {
                    egui::RichText::new("+").strong()
                })
                .min_size(egui::vec2(28.0, 24.0));
                if ui
                    .add(add_btn)
                    .on_hover_text(t!("settings.shortcuts.add").as_ref())
                    .clicked()
                    && !is_adding
                {
                    ui.data_mut(|d| d.insert_temp(rec_id, (id.to_string(), add_idx)));
                    settings.shortcut_recording = true;
                    ui.ctx().request_repaint();
                }
            });

            // 其余快捷键各占一行（缩进对齐）。
            // 用 allocate_exact_size 而非 add_space：add_space 推进后不再加
            // item_spacing，会导致比第一行动作名标签（add_sized）少 8px 而错位。
            for (i, combo) in combos.iter().enumerate().skip(1) {
                ui.horizontal(|ui| {
                    ui.allocate_exact_size(egui::vec2(150.0, 24.0), egui::Sense::hover());
                    changed |= shortcut_combo_ui(ui, rec_id, id, i, combo, &recording, settings);
                });
            }

            // 追加录制中：立即在下一行显示录制占位框（按完键后变为新快捷键）
            if is_adding {
                ui.horizontal(|ui| {
                    ui.allocate_exact_size(egui::vec2(150.0, 24.0), egui::Sense::hover());
                    let place_btn = egui::Button::new(t!("settings.shortcuts.recording").as_ref())
                        .min_size(egui::vec2(140.0, 24.0));
                    ui.add(place_btn);
                });
            }
        }
    }

    // ── 底部：恢复默认 ──
    ui.add_space(12.0);
    ui.separator();
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button(t!("settings.shortcuts.reset").as_ref()).clicked() {
                settings.keybindings.reset_to_defaults();
                // 同时取消进行中的录制，避免残留状态干扰
                ui.data_mut(|d| d.remove::<(String, usize)>(rec_id));
                settings.shortcut_recording = false;
                changed = true;
            }
        });
    });

    changed
}

/// 单个快捷键行：录制按钮（点击重新录制）+ × 移除。
/// 同一快捷键允许被多个动作绑定，不做冲突限制。
/// 返回是否修改了设置。
fn shortcut_combo_ui(
    ui: &mut egui::Ui,
    rec_id: egui::Id,
    action_id: &str,
    idx: usize,
    combo: &KeyCombo,
    recording: &Option<(String, usize)>,
    settings: &mut AudioSettings,
) -> bool {
    let mut changed = false;

    let is_recording = recording
        .as_ref()
        .is_some_and(|(a, i)| a.as_str() == action_id && *i == idx);
    let btn_text = if is_recording {
        t!("settings.shortcuts.recording").to_string()
    } else {
        crate::shortcuts::display_combo(combo)
    };
    let kb_btn = egui::Button::new(btn_text).min_size(egui::vec2(140.0, 24.0));
    if ui.add(kb_btn).clicked() && !is_recording {
        ui.data_mut(|d| d.insert_temp(rec_id, (action_id.to_string(), idx)));
        settings.shortcut_recording = true;
        ui.ctx().request_repaint();
    }

    if !is_recording
        && ui
            .add(
                egui::Button::new(egui::RichText::new("×").strong())
                    .min_size(egui::vec2(28.0, 24.0)),
            )
            .on_hover_text(t!("settings.shortcuts.clear").as_ref())
            .clicked()
    {
        settings.keybindings.remove(action_id, combo);
        changed = true;
    }

    changed
}

fn show_general_tab(ui: &mut egui::Ui, settings: &mut AudioSettings) -> bool {
    let mut changed = false;
    ui.heading(t!("settings.general.heading").as_ref());
    ui.add_space(8.0);

    ui.add_space(16.0);
    ui.separator();
    ui.add_space(8.0);

    ui.horizontal(|ui| {
        if ui
            .button(
                egui::RichText::new(t!("settings.factory_reset").as_ref())
                    .color(crate::theme::danger_text()),
            )
            .clicked()
        {
            let default_settings = AudioSettings::default();
            // Preserve runtime fields (device lists, etc.)
            let devices = std::mem::take(&mut settings.available_devices);
            let rates = std::mem::take(&mut settings.available_sample_rates);
            *settings = default_settings;
            settings.available_devices = devices;
            settings.available_sample_rates = rates;
            changed = true;
        }
    });
    changed
}

/// Show the settings dialog content inside an existing Ui.
/// Returns `true` if settings were changed.
pub fn show_content(
    ui: &mut egui::Ui,
    settings: &mut AudioSettings,
    main_ctx: &egui::Context,
) -> bool {
    let mut changed = false;

    // 左右侧栏等高，以整个窗口高度为基准
    let full_height = ui.available_height();
    ui.horizontal(|ui| {
        // ── 左侧：搜索框 + 分类导航（窄，独立滚动，撑满窗口高度） ──
        ui.vertical(|ui| {
            ui.set_width(132.0);
            ui.set_height(full_height);
            // 显式 id_salt：与右侧滚动区区分，避免两个 ScrollArea 的 id 冲突导致滚动串扰
            egui::ScrollArea::vertical()
                .id_salt("settings_left_scroll")
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    // 搜索框（多语言检索设置项）
                    ui.add(
                        egui::TextEdit::singleline(&mut settings.settings_search)
                            .hint_text(t!("settings.search_hint").as_ref())
                            .id_salt("settings_search")
                            .desired_width(132.0),
                    );
                    if !settings.settings_search.is_empty()
                        && ui.button(t!("settings.search_clear").as_ref()).clicked()
                    {
                        settings.settings_search.clear();
                    }
                    ui.add_space(6.0);

                    // 分类导航：与菜单项同款（铺满整行 + 无边框，选中项高亮）
                    for (i, key) in CATEGORY_KEYS.iter().enumerate() {
                        let selected = settings.settings_tab == i;
                        if ui
                            .add(crate::widgets::menu::menu_item_button(
                                ui,
                                selected,
                                t!(*key),
                            ))
                            .clicked()
                        {
                            settings.settings_tab = i;
                        }
                    }
                });
        });

        ui.add_space(4.0);
        ui.separator();
        ui.add_space(8.0);

        // ── 右侧：搜索结果（搜索中）或当前分类内容（宽，独立滚动，撑满窗口高度） ──
        ui.vertical(|ui| {
            ui.set_width(ui.available_width());
            ui.set_height(full_height);
            egui::ScrollArea::vertical()
                .id_salt("settings_right_scroll")
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    changed |= show_search_results(ui, settings, main_ctx);
                });
        });
    });

    changed
}

pub(crate) fn show_viewport(
    ctx: &eframe::egui::Context,
    settings: &mut AudioSettings,
    audio: &Option<yinhe_audio::CpalAudioHandle>,
) -> bool {
    let viewport_id = eframe::egui::ViewportId::from_hash_of("settings_dialog");
    if !settings.show_settings {
        return false;
    }

    let prev_xsynth_layers = settings.xsynth_layers;
    let settings_rc = std::rc::Rc::new(std::cell::RefCell::new(Some(std::mem::take(settings))));
    let ctx_clone = ctx.clone();
    let main_ctx = ctx.clone(); // 闭包内使用（zoom_factor 设在主窗口 ctx）
    let settings_cb = settings_rc.clone();

    ctx_clone.show_viewport_immediate(
        viewport_id,
        crate::chrome::dialog::viewport_builder(
            t!("settings.title").as_ref(),
            [760.0, 620.0],
            true,
        ),
        move |vctx, _class| {
            let mut slot = settings_cb.borrow_mut().take();
            if let Some(ref mut s) = slot {
                let mut close = false;
                if vctx.input(|i| i.viewport().close_requested()) {
                    close = true;
                }
                eframe::egui::CentralPanel::default()
                    .frame(eframe::egui::Frame {
                        fill: crate::theme::app_bg(),
                        ..Default::default()
                    })
                    .show(vctx, |ui| {
                        crate::chrome::dialog::title_bar(
                            ui,
                            t!("settings.title").as_ref(),
                            &mut close,
                        );
                        eframe::egui::Frame::new()
                            .inner_margin(eframe::egui::Margin {
                                left: 12,
                                right: 12,
                                top: 0,
                                bottom: 12,
                            })
                            .show(ui, |ui| {
                                let changed = show_content(ui, s, &main_ctx);
                                if changed {
                                    s.save();
                                }
                            });
                    });
                if close {
                    vctx.send_viewport_cmd(eframe::egui::ViewportCommand::Visible(false));
                    s.show_settings = false;
                    s.shortcut_recording = false;
                }
            }
            *settings_cb.borrow_mut() = slot;
        },
    );

    if let Some(s) = std::rc::Rc::into_inner(settings_rc).and_then(|rc| rc.into_inner()) {
        *settings = s;
        if settings.xsynth_layers != prev_xsynth_layers
            && let Some(audio) = audio
        {
            let count = if settings.xsynth_layers == 0 {
                None
            } else {
                Some(settings.xsynth_layers as usize)
            };
            audio
                .handle
                .send(yinhe_audio::AudioCommand::SetLayerCount { count });
        }
        !settings.show_settings
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(zh: &str) -> &'static SettingItem {
        SETTING_ITEMS
            .iter()
            .find(|i| i.zh == zh)
            .expect("settings item exists")
    }

    #[test]
    fn search_matches_original_names_of_all_languages() {
        assert!(item_matches(item("主题预设"), "主题"));
        assert!(item_matches(item("主题预设"), "Theme"));
        assert!(item_matches(item("主题预设"), "プリセット"));
        assert!(item_matches(item("主题预设"), "프리셋"));
        assert!(item_matches(item("背景"), "배경"));
    }

    #[test]
    fn search_matches_pinyin_and_initials() {
        assert!(item_matches(item("主题预设"), "zhuti"));
        assert!(item_matches(item("主题预设"), "zhu ti"));
        assert!(item_matches(item("主题预设"), "zt"));
        assert!(item_matches(item("缓冲区大小"), "huanchongqu"));
        assert!(!item_matches(item("主题预设"), "xyz"));
    }

    #[test]
    fn search_is_case_insensitive() {
        assert!(item_matches(item("采样率"), "SAMPLE"));
        assert!(item_matches(item("采样率"), "caiyanglv"));
    }

    /// 回归测试：快捷键第一行（动作名 150 宽标签）与后续缩进行（add_space 150）
    /// 的快捷键按钮必须水平对齐。此前 add_sized 按标签文本宽度推进导致错位。
    #[test]
    fn shortcut_rows_align() {
        let mut first_x = 0.0f32;
        let mut second_x = 0.0f32;
        let ctx = egui::Context::default();
        ctx.set_fonts(egui::FontDefinitions::empty()); // 免加载字体，节省测试时间
        let output = ctx.run_ui(Default::default(), |ui| {
            ui.horizontal(|ui| {
                ui.add_sized(
                    [150.0, 24.0],
                    egui::Label::new(egui::RichText::new("保存"))
                        .selectable(false)
                        .wrap_mode(egui::TextWrapMode::Extend),
                );
                let resp = ui.add(egui::Button::new("⌘S").min_size(egui::vec2(140.0, 24.0)));
                first_x = resp.rect.min.x;
            });
            ui.horizontal(|ui| {
                ui.allocate_exact_size(egui::vec2(150.0, 24.0), egui::Sense::hover());
                let resp = ui.add(egui::Button::new("⌘S").min_size(egui::vec2(140.0, 24.0)));
                second_x = resp.rect.min.x;
            });
        });
        output.drop_without_applying_deltas();
        assert!(
            (first_x - second_x).abs() < 0.5,
            "快捷键两行未对齐：第一行 x={first_x}，第二行 x={second_x}"
        );
    }
}
