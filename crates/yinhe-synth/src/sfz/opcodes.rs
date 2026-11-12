//! SFZ opcode 解析：`name=value` → 类型化字段。
//!
//! SFZ 的 opcode 名带编号后缀（`xfin_locc14`、`on_hicc64`、`set_cc20`），
//! 统一按「前缀 + 尾随数字」拆分，避免为 128 个 CC 各写一条分支。

use xsynth_soundfonts::FilterType;

use super::types::{Ampeg, CcGate, GateDir, Lfo, LoopMode, OnGate, Trigger};

/// 一条 region 的**可继承**参数集合（control/global/master/group/region 逐层覆盖）。
///
/// 与 SFZ 规范一致：所有字段都参与继承；列表型字段（CC 门）整体替换，不追加。
#[derive(Clone, Debug)]
pub(super) struct RegionFields {
    pub lokey: Option<i32>,
    pub hikey: Option<i32>,
    pub lovel: u8,
    pub hivel: u8,
    pub pitch_keycenter: Option<i32>,
    pub tune: f32,
    pub volume_db: f32,
    pub pan: f32,
    pub amp_veltrack: f32,
    pub amp_keytrack: f32,
    pub amp_keycenter: i32,
    pub pan_veltrack: f32,
    pub pan_keytrack: f32,
    pub pan_keycenter: i32,
    pub fil_veltrack: f32,
    pub fil_keytrack: f32,
    pub fil_keycenter: i32,
    pub filter_type: FilterType,
    pub cutoff: Option<f32>,
    pub resonance_db: f32,
    pub ampeg: Ampeg,
    pub loop_mode: LoopMode,
    pub loop_start: u32,
    pub loop_end: u32,
    pub offset: u32,
    pub trigger: Trigger,
    pub group: u32,
    pub off_by: u32,
    pub seq_length: u32,
    pub seq_position: u32,
    pub sw_last: Option<u8>,
    pub sw_lokey: Option<u8>,
    pub sw_hikey: Option<u8>,
    pub cc_gates: Vec<CcGate>,
    pub on_gates: Vec<OnGate>,
    pub lorand: Option<f32>,
    pub hirand: Option<f32>,
    pub pitch_random: f32,
    /// (depth_db, freq, phase)
    pub amplfo: Option<Lfo>,
    pub pitchlfo: Option<Lfo>,
    pub fillfo: Option<Lfo>,
    pub effect1: f32,
    pub sample: Option<String>,
    pub default_path: Option<String>,
}

impl Default for RegionFields {
    fn default() -> Self {
        Self {
            lokey: None,
            hikey: None,
            lovel: 0,
            hivel: 127,
            pitch_keycenter: None,
            tune: 0.0,
            volume_db: 0.0,
            pan: 0.0,
            amp_veltrack: 100.0,
            amp_keytrack: 0.0,
            amp_keycenter: 60,
            pan_veltrack: 0.0,
            pan_keytrack: 0.0,
            pan_keycenter: 0,
            fil_veltrack: 0.0,
            fil_keytrack: 0.0,
            fil_keycenter: 60,
            filter_type: FilterType::default(),
            cutoff: None,
            resonance_db: 0.0,
            ampeg: Ampeg::default(),
            loop_mode: LoopMode::NoLoop,
            loop_start: 0,
            loop_end: 0,
            offset: 0,
            trigger: Trigger::Attack,
            group: 0,
            off_by: 0,
            seq_length: 0,
            seq_position: 0,
            sw_last: None,
            sw_lokey: None,
            sw_hikey: None,
            cc_gates: Vec::new(),
            on_gates: Vec::new(),
            lorand: None,
            hirand: None,
            pitch_random: 0.0,
            amplfo: None,
            pitchlfo: None,
            fillfo: None,
            effect1: 0.0,
            sample: None,
            default_path: None,
        }
    }
}

/// 前缀表命中：返回 (前缀, 是否为 xfin/xfout 的 lo 端, 尾随编号)。
fn split_indexed(name: &str) -> Option<(&str, u8)> {
    const PREFIXES: &[&str] = &[
        "xfin_locc",
        "xfin_hicc",
        "xfout_locc",
        "xfout_hicc",
        "on_locc",
        "on_hicc",
        "set_cc",
    ];
    for p in PREFIXES {
        if let Some(rest) = name.strip_prefix(p)
            && !rest.is_empty()
            && rest.bytes().all(|b| b.is_ascii_digit())
        {
            return rest.parse::<u8>().ok().map(|n| (*p, n));
        }
    }
    None
}

/// 应用一条 opcode。`unknown` 记录本解析器不认识的键（诊断用）。
pub(super) fn apply(
    f: &mut RegionFields,
    name: &str,
    value: &str,
    unknown: &mut std::collections::BTreeMap<String, usize>,
) {
    // 编号型 opcode 先处理（其余走精确匹配）。
    if let Some((prefix, n)) = split_indexed(name)
        && n < 128
    {
        apply_indexed(f, prefix, n, value);
        return;
    }

    let ok_num = value.trim().parse::<f32>();
    match name {
        "lokey" => f.lokey = parse_key(value),
        "hikey" => f.hikey = parse_key(value),
        "key" => {
            if let Some(k) = parse_key(value) {
                f.lokey = Some(k);
                f.hikey = Some(k);
                f.pitch_keycenter = Some(k);
            }
        }
        "lovel" => f.lovel = value.trim().parse().unwrap_or(0).min(127),
        "hivel" => f.hivel = value.trim().parse().unwrap_or(127).min(127),
        "pitch_keycenter" => f.pitch_keycenter = parse_key(value),
        "tune" => f.tune = ok_num.unwrap_or(0.0),
        "volume" => f.volume_db = ok_num.unwrap_or(0.0),
        "pan" => f.pan = ok_num.unwrap_or(0.0),
        "amp_veltrack" => f.amp_veltrack = ok_num.unwrap_or(100.0),
        "amp_keytrack" => f.amp_keytrack = ok_num.unwrap_or(0.0),
        "amp_keycenter" => f.amp_keycenter = parse_key(value).unwrap_or(60),
        "pan_veltrack" => f.pan_veltrack = ok_num.unwrap_or(0.0),
        "pan_keytrack" => f.pan_keytrack = ok_num.unwrap_or(0.0),
        "pan_keycenter" => f.pan_keycenter = parse_key(value).unwrap_or(0),
        "fil_veltrack" => f.fil_veltrack = ok_num.unwrap_or(0.0),
        "fil_keytrack" => f.fil_keytrack = ok_num.unwrap_or(0.0),
        "fil_keycenter" => f.fil_keycenter = parse_key(value).unwrap_or(60),
        "fil_type" => f.filter_type = parse_filter(value),
        "cutoff" => f.cutoff = ok_num.ok(),
        "resonance" => f.resonance_db = ok_num.unwrap_or(0.0),
        "ampeg_start" => f.ampeg.start = ok_num.unwrap_or(0.0),
        "ampeg_delay" => f.ampeg.delay = ok_num.unwrap_or(0.0),
        "ampeg_attack" => f.ampeg.attack = ok_num.unwrap_or(0.0),
        "ampeg_hold" => f.ampeg.hold = ok_num.unwrap_or(0.0),
        "ampeg_decay" => f.ampeg.decay = ok_num.unwrap_or(0.0),
        "ampeg_sustain" => f.ampeg.sustain = ok_num.unwrap_or(100.0),
        "ampeg_release" => f.ampeg.release = ok_num.unwrap_or(0.0),
        "ampeg_vel2release" => f.ampeg.vel2release = ok_num.unwrap_or(0.0),
        "loop_mode" | "loopmode" => f.loop_mode = parse_loop(value),
        "loop_start" | "loopstart" => f.loop_start = ok_num.unwrap_or(0.0) as u32,
        "loop_end" | "loopend" => f.loop_end = ok_num.unwrap_or(0.0) as u32,
        "offset" => f.offset = ok_num.unwrap_or(0.0) as u32,
        "trigger" => f.trigger = parse_trigger(value),
        "group" => f.group = ok_num.unwrap_or(0.0) as u32,
        "off_by" => f.off_by = ok_num.unwrap_or(0.0) as u32,
        "seq_length" => f.seq_length = ok_num.unwrap_or(0.0) as u32,
        "seq_position" => f.seq_position = ok_num.unwrap_or(0.0) as u32,
        "sw_last" => f.sw_last = Some(parse_key(value).unwrap_or(0).clamp(0, 127) as u8),
        "sw_lokey" => f.sw_lokey = Some(parse_key(value).unwrap_or(0).clamp(0, 127) as u8),
        "sw_hikey" => f.sw_hikey = Some(parse_key(value).unwrap_or(127).clamp(0, 127) as u8),
        "lorand" => f.lorand = ok_num.ok(),
        "hirand" => f.hirand = ok_num.ok(),
        "pitch_random" => f.pitch_random = ok_num.unwrap_or(0.0),
        "effect1" => f.effect1 = ok_num.unwrap_or(0.0),
        // LFO：编号后缀只在带 phase 编号时出现；depth/freq 是普通键。
        "amplfo_depth" => upsert_lfo(&mut f.amplfo).depth = ok_num.unwrap_or(0.0),
        "amplfo_freq" => upsert_lfo(&mut f.amplfo).freq = ok_num.unwrap_or(0.0),
        "amplfo_phase" => upsert_lfo(&mut f.amplfo).phase = ok_num.unwrap_or(0.0),
        "pitchlfo_depth" => upsert_lfo(&mut f.pitchlfo).depth = ok_num.unwrap_or(0.0),
        "pitchlfo_freq" => upsert_lfo(&mut f.pitchlfo).freq = ok_num.unwrap_or(0.0),
        "pitchlfo_phase" => upsert_lfo(&mut f.pitchlfo).phase = ok_num.unwrap_or(0.0),
        "fillfo_depth" => upsert_lfo(&mut f.fillfo).depth = ok_num.unwrap_or(0.0),
        "fillfo_freq" => upsert_lfo(&mut f.fillfo).freq = ok_num.unwrap_or(0.0),
        "fillfo_phase" => upsert_lfo(&mut f.fillfo).phase = ok_num.unwrap_or(0.0),
        "sample" => f.sample = Some(value.trim().replace('\\', "/")),
        "default_path" => f.default_path = Some(value.trim().replace('\\', "/")),
        // 已知但不参与发声的 opcode：不记为 unknown（否则诊断全是噪音）。
        _ if name.starts_with("ampeg_") => {}
        _ if name.starts_with("fileg_")
            || name.starts_with("pitcheg_")
            || name.starts_with("eg")
            || name.starts_with("velocity_")
            || name.starts_with("rt_")
            || name.starts_with("controller_")
            || name.starts_with("lfo")
            || name.starts_with("label_")
            || name.starts_with("hint_")
            || name.starts_with("sw_")
            || name.starts_with("xfin_lokey")
            || name.starts_with("xfin_hikey")
            || name.starts_with("xfout_lokey")
            || name.starts_with("xfout_hikey")
            || name.starts_with("xfin_lovel")
            || name.starts_with("xfin_hivel")
            || name.starts_with("xfout_lovel")
            || name.starts_with("xfout_hivel")
            || name == "sustain_sw"
            || name == "pitch_keytrack"
            || name == "transpose"
            || name == "bend_up"
            || name == "bend_down"
            || name == "fil_keytrack"
            || name == "note_off_velocity"
            || name == "delay"
            || name == "end"
            || name == "width"
            || name == "position"
            || name == "direction"
            || name == "start"
            || name == "count"
            || name == "loop_crossfade"
            || name == "off_mode"
            || name == "rt_dead"
            || name == "fixed_cc" =>
        {
            // 后续轮次实现的目标：先计数供 lint 用，但不当作"未识别"。
            *unknown.entry(name.to_string()).or_insert(0) += 1;
        }
        _ => {
            *unknown.entry(name.to_string()).or_insert(0) += 1;
        }
    }
}

fn apply_indexed(f: &mut RegionFields, prefix: &str, cc: u8, value: &str) {
    let v = value.trim().parse::<f32>().unwrap_or(0.0).clamp(0.0, 127.0);
    let vi = v as u8;
    match prefix {
        "xfin_locc" => upsert_gate(f, cc, GateDir::In, true, vi),
        "xfin_hicc" => upsert_gate(f, cc, GateDir::In, false, vi),
        "xfout_locc" => upsert_gate(f, cc, GateDir::Out, true, vi),
        "xfout_hicc" => upsert_gate(f, cc, GateDir::Out, false, vi),
        "on_locc" => upsert_on_gate(f, cc, true, vi),
        "on_hicc" => upsert_on_gate(f, cc, false, vi),
        "set_cc" => {
            // <control> 段的 set_ccN：由上层单独收集（此处不落到 region）。
        }
        _ => {}
    }
}

fn upsert_lfo(slot: &mut Option<Lfo>) -> &mut Lfo {
    slot.get_or_insert(Lfo {
        depth: 0.0,
        freq: 0.0,
        phase: 0.0,
    })
}

/// `xfin_loccN` / `xfin_hiccN` 可能先后出现（任意顺序）：按 (cc, dir) 合并。
fn upsert_gate(f: &mut RegionFields, cc: u8, dir: GateDir, is_lo: bool, v: u8) {
    if let Some(g) = f.cc_gates.iter_mut().find(|g| g.cc == cc && g.dir == dir) {
        if is_lo {
            g.lo = v;
        } else {
            g.hi = v;
        }
        return;
    }
    f.cc_gates.push(CcGate {
        cc,
        lo: if is_lo { v } else { 0 },
        hi: if is_lo { 127 } else { v },
        dir,
    });
}

fn upsert_on_gate(f: &mut RegionFields, cc: u8, is_lo: bool, v: u8) {
    if let Some(g) = f.on_gates.iter_mut().find(|g| g.cc == cc) {
        if is_lo {
            g.lo = v;
        } else {
            g.hi = v;
        }
        return;
    }
    f.on_gates.push(OnGate {
        cc,
        lo: if is_lo { v } else { 0 },
        hi: if is_lo { 127 } else { v },
    });
}

/// SFZ 键名：数字或 `c4`/`db3`/`c#-1`。
pub(super) fn parse_key(val: &str) -> Option<i32> {
    let s = val.trim();
    if let Ok(n) = s.parse::<i32>() {
        return Some(n);
    }
    let lower = s.to_ascii_lowercase();
    let (name, rest) = lower.split_at(lower.find(|c: char| c.is_ascii_digit() || c == '-')?);
    let semitone = match name {
        "c" => 0,
        "c#" | "db" => 1,
        "d" => 2,
        "d#" | "eb" => 3,
        "e" => 4,
        "f" => 5,
        "f#" | "gb" => 6,
        "g" => 7,
        "g#" | "ab" => 8,
        "a" => 9,
        "a#" | "bb" => 10,
        "b" => 11,
        _ => return None,
    };
    let octave: i32 = rest.parse().ok()?;
    Some(12 + semitone + octave * 12)
}

fn parse_filter(val: &str) -> FilterType {
    let v = val.trim().to_ascii_lowercase();
    if v.starts_with("hpf") {
        FilterType::HighPass
    } else if v.starts_with("bpf") {
        FilterType::BandPass
    } else if v == "lpf_1p" {
        FilterType::LowPassPole
    } else {
        FilterType::LowPass
    }
}

fn parse_loop(val: &str) -> LoopMode {
    match val.trim().to_ascii_lowercase().as_str() {
        "one_shot" => LoopMode::OneShot,
        "loop_continuous" => LoopMode::LoopContinuous,
        "loop_sustain" => LoopMode::LoopSustain,
        _ => LoopMode::NoLoop,
    }
}

fn parse_trigger(val: &str) -> Trigger {
    match val.trim().to_ascii_lowercase().as_str() {
        "release" => Trigger::Release,
        _ => Trigger::Attack,
    }
}

/// 调试/测试辅助：解析一串 opcode（不含标签）。
#[cfg(test)]
#[allow(dead_code)]
pub(super) fn apply_all(pairs: &[(&str, &str)]) -> RegionFields {
    let mut f = RegionFields::default();
    let mut unknown = std::collections::BTreeMap::new();
    for (k, v) in pairs {
        apply(&mut f, k, v, &mut unknown);
    }
    f
}
