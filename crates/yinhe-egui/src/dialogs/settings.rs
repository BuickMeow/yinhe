use deunicode::deunicode_char;
use eframe::egui;
use pinyin::ToPinyin;
use rust_i18n::t;

use yinhe_theme::base::{BaseColors, Rgba};

use crate::audio_settings::AudioSettings;

// ── 设置分类（左侧导航，顺序即 settings_tab 索引） ──

const CATEGORY_KEYS: [&str; 7] = [
    "settings.cat.theme",
    "settings.cat.language",
    "settings.cat.audio",
    "settings.cat.render",
    "settings.cat.midi",
    "settings.cat.display",
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
        cat: 4,
        zh: "MIDI 导入编码",
        en: "MIDI import encoding",
        ja: "MIDI インポートエンコーディング",
        ko: "MIDI 가져오기 인코딩",
    },
    SettingItem {
        cat: 5,
        zh: "界面缩放",
        en: "UI scale",
        ja: "UI スケール",
        ko: "UI 배율",
    },
    SettingItem {
        cat: 6,
        zh: "刷新设备列表",
        en: "Refresh devices",
        ja: "デバイス更新",
        ko: "장치 새로고침",
    },
    SettingItem {
        cat: 6,
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
            0 => show_theme_tab(ui, settings),
            1 => show_language_tab(ui, settings),
            2 => show_audio_tab(ui, settings),
            3 => show_render_tab(ui, settings),
            4 => show_midi_tab(ui, settings),
            5 => show_display_tab(ui, settings, main_ctx),
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
            crate::theme::text_hint(),
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

fn show_theme_tab(ui: &mut egui::Ui, settings: &mut AudioSettings) -> bool {
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
            if ui.checkbox(&mut settings.note_outline, "").changed() {
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
            if ui.checkbox(&mut settings.use_gpu_cull, "").changed() {
                changed = true;
            }
            ui.end_row();
        });
    changed
}

fn show_midi_tab(ui: &mut egui::Ui, settings: &mut AudioSettings) -> bool {
    let mut changed = false;
    ui.heading(t!("settings.midi_import.heading").as_ref());
    ui.add_space(8.0);
    egui::Grid::new("midi_import_grid")
        .num_columns(2)
        .spacing([12.0, 8.0])
        .show(ui, |ui| {
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

fn show_display_tab(
    ui: &mut egui::Ui,
    settings: &mut AudioSettings,
    main_ctx: &egui::Context,
) -> bool {
    let mut changed = false;
    ui.heading(t!("settings.display.heading").as_ref());
    ui.add_space(8.0);

    egui::Grid::new("display_settings_grid")
        .num_columns(2)
        .spacing([12.0, 8.0])
        .show(ui, |ui| {
            ui.label(t!("settings.display.ui_scale").as_ref());
            ui.horizontal(|ui| {
                let mut scale = settings.ui_scale;
                if ui
                    .add(egui::Slider::new(&mut scale, 0.75..=2.0).step_by(0.05))
                    .changed()
                {
                    settings.ui_scale = scale;
                    main_ctx.set_zoom_factor(scale);
                    changed = true;
                }
                if ui
                    .button(t!("settings.display.reset_scale").as_ref())
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

fn show_general_tab(ui: &mut egui::Ui, settings: &mut AudioSettings) -> bool {
    let mut changed = false;
    ui.heading(t!("settings.general.heading").as_ref());
    ui.add_space(8.0);

    if ui.button(t!("settings.refresh_devices").as_ref()).clicked() {
        let devices = crate::audio_settings::list_output_devices();
        let (default_rate, rates) = crate::audio_settings::discover_sample_rates();
        settings.refresh_devices(devices, rates, default_rate);
    }

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

    ui.horizontal(|ui| {
        // ── 左侧：搜索框 + 分类导航（窄） ──
        ui.vertical(|ui| {
            ui.set_width(132.0);

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

            for (i, key) in CATEGORY_KEYS.iter().enumerate() {
                let selected = settings.settings_tab == i;
                if ui.selectable_label(selected, t!(*key).as_ref()).clicked() {
                    settings.settings_tab = i;
                }
            }
        });

        ui.add_space(4.0);
        ui.separator();
        ui.add_space(8.0);

        // ── 右侧：搜索结果（搜索中）或当前分类内容（宽） ──
        ui.vertical(|ui| {
            ui.set_width(ui.available_width());
            changed |= show_search_results(ui, settings, main_ctx);
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
                                eframe::egui::ScrollArea::vertical()
                                    .auto_shrink([false; 2])
                                    .show(ui, |ui| {
                                        let changed = show_content(ui, s, &main_ctx);
                                        if changed {
                                            s.save();
                                        }
                                    });
                            });
                    });
                if close {
                    vctx.send_viewport_cmd(eframe::egui::ViewportCommand::Visible(false));
                    s.show_settings = false;
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
}
