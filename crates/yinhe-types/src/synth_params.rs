//! 内置音源的可调参数（项目级、按全局通道）。
//!
//! 这些参数不改音色库文件，只作为**相对偏移/倍率**叠加到 `KeyInfo` 的
//! per-voice 展开上；默认值全部是"不改变"。参数目录与 UI 见
//! `docs/spec-instrument-params.md`。

use serde::{Deserialize, Serialize};

/// 内置音源参数。默认值 = 不改变音色库原始行为。
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SynthParams {
    /// 亮度偏移（音分）：叠加到每个 voice 的 cutoff。
    pub brightness_cents: f32,
    /// 共振偏移（dB）：叠加到每个 voice 的 Q。
    pub resonance_db: f32,
    /// 音量偏移（dB）。
    pub volume_db: f32,
    /// 声像偏移（-1 = 左，0 = 不变，1 = 右）。
    pub pan: f32,
    /// 攻击时长倍率（1 = 原值）。
    pub attack_scale: f32,
    /// 释放时长倍率（1 = 原值）。
    pub release_scale: f32,
    /// 音高偏移（音分）。
    pub tune_cents: f32,
}

impl Default for SynthParams {
    fn default() -> Self {
        Self {
            brightness_cents: 0.0,
            resonance_db: 0.0,
            volume_db: 0.0,
            pan: 0.0,
            attack_scale: 1.0,
            release_scale: 1.0,
            tune_cents: 0.0,
        }
    }
}

impl SynthParams {
    /// 是否全部为默认值（保存时可跳过）。
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// 按参数 id 读取值。
    pub fn get(&self, id: SynthParamId) -> f32 {
        match id {
            SynthParamId::Brightness => self.brightness_cents,
            SynthParamId::Resonance => self.resonance_db,
            SynthParamId::Volume => self.volume_db,
            SynthParamId::Pan => self.pan,
            SynthParamId::Attack => self.attack_scale,
            SynthParamId::Release => self.release_scale,
            SynthParamId::Tune => self.tune_cents,
        }
    }

    /// 按参数 id 写入值。
    pub fn set(&mut self, id: SynthParamId, value: f32) {
        match id {
            SynthParamId::Brightness => self.brightness_cents = value,
            SynthParamId::Resonance => self.resonance_db = value,
            SynthParamId::Volume => self.volume_db = value,
            SynthParamId::Pan => self.pan = value,
            SynthParamId::Attack => self.attack_scale = value,
            SynthParamId::Release => self.release_scale = value,
            SynthParamId::Tune => self.tune_cents = value,
        }
    }
}

/// 参数分类（UI 左栏）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SynthParamCategory {
    Tone,
    Level,
    Envelope,
    Pitch,
}

impl SynthParamCategory {
    pub const ALL: [SynthParamCategory; 4] = [
        SynthParamCategory::Tone,
        SynthParamCategory::Level,
        SynthParamCategory::Envelope,
        SynthParamCategory::Pitch,
    ];
}

/// 参数 id（跨版本稳定，用于持久化与 UI）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SynthParamId {
    Brightness,
    Resonance,
    Volume,
    Pan,
    Attack,
    Release,
    Tune,
}

/// 一个参数的范围与显示信息（UI 用；引擎只读 [`SynthParams`]）。
#[derive(Clone, Copy, Debug)]
pub struct SynthParamInfo {
    pub id: SynthParamId,
    pub category: SynthParamCategory,
    pub name: &'static str,
    pub unit: &'static str,
    pub min: f32,
    pub max: f32,
    /// UI 是否按对数刻度拖动。
    pub log: bool,
    /// 小数位数（显示用）。
    pub decimals: u8,
}

impl SynthParamInfo {
    /// 把实际值归一化到 0..1（log 参数按对数）。
    pub fn normalize(&self, value: f32) -> f32 {
        let v = value.clamp(self.min, self.max);
        if self.log {
            ((v / self.min).ln() / (self.max / self.min).ln()).clamp(0.0, 1.0)
        } else {
            ((v - self.min) / (self.max - self.min)).clamp(0.0, 1.0)
        }
    }

    /// 从 0..1 还原实际值。
    pub fn denormalize(&self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        if self.log {
            self.min * (self.max / self.min).powf(t)
        } else {
            self.min + t * (self.max - self.min)
        }
    }

    /// 当前值是否等于默认值（默认值 = `SynthParams::default().get(id)`）。
    pub fn is_default(&self, value: f32) -> bool {
        (value - SynthParams::default().get(self.id)).abs() < 1e-6
    }
}

/// 参数表（权威唯一：UI、持久化、测试共用）。
pub const SYNTH_PARAMS: &[SynthParamInfo] = &[
    SynthParamInfo {
        id: SynthParamId::Brightness,
        category: SynthParamCategory::Tone,
        name: "Brightness",
        unit: "ct",
        min: -4800.0,
        max: 4800.0,
        log: false,
        decimals: 0,
    },
    SynthParamInfo {
        id: SynthParamId::Resonance,
        category: SynthParamCategory::Tone,
        name: "Resonance",
        unit: "dB",
        min: -25.0,
        max: 25.0,
        log: false,
        decimals: 1,
    },
    SynthParamInfo {
        id: SynthParamId::Volume,
        category: SynthParamCategory::Level,
        name: "Volume",
        unit: "dB",
        min: -24.0,
        max: 24.0,
        log: false,
        decimals: 1,
    },
    SynthParamInfo {
        id: SynthParamId::Pan,
        category: SynthParamCategory::Level,
        name: "Pan",
        unit: "",
        min: -1.0,
        max: 1.0,
        log: false,
        decimals: 2,
    },
    SynthParamInfo {
        id: SynthParamId::Attack,
        category: SynthParamCategory::Envelope,
        name: "Attack",
        unit: "x",
        min: 0.1,
        max: 10.0,
        log: true,
        decimals: 2,
    },
    SynthParamInfo {
        id: SynthParamId::Release,
        category: SynthParamCategory::Envelope,
        name: "Release",
        unit: "x",
        min: 0.1,
        max: 10.0,
        log: true,
        decimals: 2,
    },
    SynthParamInfo {
        id: SynthParamId::Tune,
        category: SynthParamCategory::Pitch,
        name: "Tune",
        unit: "ct",
        min: -100.0,
        max: 100.0,
        log: false,
        decimals: 1,
    },
];

/// 按 id 查参数信息。
pub fn synth_param_info(id: SynthParamId) -> &'static SynthParamInfo {
    SYNTH_PARAMS
        .iter()
        .find(|p| p.id == id)
        .expect("SynthParamId 必须在 SYNTH_PARAMS 中")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_neutral() {
        let p = SynthParams::default();
        assert!(p.is_default());
        for info in SYNTH_PARAMS {
            assert!(
                info.is_default(p.get(info.id)),
                "{} 默认值应中性",
                info.name
            );
        }
    }

    /// 全部参数必须是双极性的：默认值落在旋钮正中（`knob_centered` 的
    /// 0.5 = 归零），否则中位看起来归零、实际不是。
    #[test]
    fn defaults_sit_at_knob_center() {
        for info in SYNTH_PARAMS {
            let t = info.normalize(SynthParams::default().get(info.id));
            assert!((t - 0.5).abs() < 1e-6, "{} 默认值不在正中: {t}", info.name);
        }
    }

    #[test]
    fn normalize_roundtrip() {
        for info in SYNTH_PARAMS {
            for t in [0.0, 0.25, 0.5, 0.75, 1.0] {
                let v = info.denormalize(t);
                let back = info.normalize(v);
                assert!((back - t).abs() < 1e-5, "{} t={t} back={back}", info.name);
            }
        }
    }

    #[test]
    fn all_ids_have_info() {
        for id in [
            SynthParamId::Brightness,
            SynthParamId::Resonance,
            SynthParamId::Volume,
            SynthParamId::Pan,
            SynthParamId::Attack,
            SynthParamId::Release,
            SynthParamId::Tune,
        ] {
            let _ = synth_param_info(id);
        }
    }
}
