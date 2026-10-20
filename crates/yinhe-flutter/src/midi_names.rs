//! 平台无关的 MIDI 静态名称/上限表（Flutter 通过 FFI 一次性读取）。
//!
//! 数据源：`yinhe_types::gm`、`yinhe_types::automation`、`yinhe_types::NOTE_NAMES`。
//! 这些数据与平台无关，移动端不再各自维护一份。

use serde_json::{Map, Value, json};

/// MIDI 名称表 JSON：
/// `{"gm":{"melodic":[128],"drums":[9]},"cc":{n:name},"rpn":{n:name},
///   "notes":[12],"limits":{...}}`
pub fn midi_names_json() -> String {
    let mut cc = Map::new();
    for n in 0..=127u8 {
        let name = yinhe_types::automation::cc_name(n);
        if !name.is_empty() {
            cc.insert(n.to_string(), Value::String(name.to_string()));
        }
    }
    let mut rpn = Map::new();
    for n in 0..=127u16 {
        let name = yinhe_types::automation::rpn_name(n);
        if !name.is_empty() {
            rpn.insert(n.to_string(), Value::String(name.to_string()));
        }
    }
    json!({
        "gm": {
            "melodic": yinhe_types::gm::MELODIC_NAMES.as_slice(),
            "drums": yinhe_types::gm::DRUM_KIT_NAMES.as_slice(),
        },
        "cc": cc,
        "rpn": rpn,
        "notes": yinhe_types::NOTE_NAMES.as_slice(),
        "limits": {
            "tempo": 60_000_000,
            "pitch_bend": 16_383,
            "cc": 127,
            "rpn_7bit": 127,
            "rpn_14bit": 16_383,
        },
    })
    .to_string()
}

/// 常用量化档位 JSON（按 PPQ 推导）：`[{"label":"1/1","ticks":1920}, ...]`。
pub fn quantize_presets_json(ppq: f64) -> String {
    let ppq = ppq.round().clamp(1.0, 1_000_000.0) as u32;
    let items: Vec<Value> = yinhe_editor_core::quantize::presets_for_ppq(ppq)
        .into_iter()
        .map(|p| json!({"label": p.label(), "ticks": p.tick_interval(ppq)}))
        .collect();
    Value::Array(items).to_string()
}
