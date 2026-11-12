//! SFZ 运行期门控：`xfin/xfout`、`on_cc`、`sw_last`、`seq_*`、`off_by`、
//! `pitch_random`、LFO。
//!
//! 这些参数是**每 region 一份**，不是每 (key, vel) 一份：直接塞进 `KeyInfo`
//! 会让 GeneralUser 的 670 万条快照再涨数百 MB（见 `spec-keymap-memory.md`）。
//! 因此放在 `KeyMapEntry` 的侧表里，`KeyInfo` 只存 `extra: u32` 索引。

pub use crate::sfz::{CcGate, GateDir, Lfo, OnGate};

/// 一条 region 的运行期附加行为。全部为纯数据，可在解析期算好。
#[derive(Clone, Debug, Default)]
pub struct LayerExtra {
    /// `xfin_*` 淡入 / `xfout_*` 淡出（线性）。
    pub cc_gates: Vec<CcGate>,
    /// `on_loccN/hiccN`：区间外整层不发声。
    pub on_gates: Vec<OnGate>,
    /// round-robin：0 表示未启用。
    pub seq_length: u16,
    pub seq_position: u16,
    pub group: u16,
    pub off_by: u16,
    /// keyswitch：0xFF 表示未启用。
    pub sw_last: u8,
    pub sw_lokey: u8,
    pub sw_hikey: u8,
    pub has_sw: bool,
    /// `trigger=release`：在音符结束时发声（释放采样）。
    pub release_trigger: bool,
    /// `one_shot`：忽略 note_off，播到采样结束。
    pub one_shot: bool,
    /// 速度抖动的层（`lorand`/`hirand`）：命中区间才发声。
    pub lorand: f32,
    pub hirand: f32,
    /// 音分随机（`pitch_random`），±range。
    pub pitch_random: f32,
    pub amplfo: Option<Lfo>,
    pub pitchlfo: Option<Lfo>,
}

impl LayerExtra {
    /// 是否完全没有附加行为（热路径可跳过求值）。
    pub fn is_plain(&self) -> bool {
        self.cc_gates.is_empty()
            && self.on_gates.is_empty()
            && self.seq_length == 0
            && self.off_by == 0
            && !self.has_sw
            && !self.release_trigger
            && !self.one_shot
            && self.lorand <= 0.0
            && self.hirand >= 1.0
            && self.pitch_random == 0.0
            && self.amplfo.is_none()
            && self.pitchlfo.is_none()
    }
}

/// note_on 时的求值上下文。
pub struct GateCtx<'a> {
    pub cc: &'a [u8; 128],
    pub sw_last: u8,
    /// 该 (channel, key) 的轮次计数（每次 note_on +1）。
    pub seq_counter: u32,
    /// 确定性随机值 0..1（由 (channel,key,note_serial) 哈希而来）。
    pub rand: f32,
}

/// 门控求值结果。
pub struct GateResult {
    /// 线性增益（`xfin/xfout` 相乘）。
    pub gain: f32,
    /// 音高偏移（音分，含 `pitch_random`）。
    pub pitch_cents: f32,
}

/// 判定一层是否发声；不发声返回 `None`。
///
/// 只在 note_on 求值（CC 变化不追溯已发声 voice）——与主流播放器一致，
/// 也避免 GPU 端逐块回写。
pub fn evaluate(extra: &LayerExtra, key: u8, ctx: &GateCtx) -> Option<GateResult> {
    // keyswitch：落在开关区内的键不发声，只改变通道的 sw_last。
    if extra.has_sw && key >= extra.sw_lokey && key <= extra.sw_hikey {
        return None;
    }
    if extra.has_sw && extra.sw_last != 0xFF && extra.sw_last != ctx.sw_last {
        return None;
    }
    for g in &extra.on_gates {
        if !g.passes(ctx.cc) {
            return None;
        }
    }
    if extra.seq_length > 0
        && extra.seq_position > 0
        && (ctx.seq_counter % extra.seq_length as u32) + 1 != extra.seq_position as u32
    {
        return None;
    }
    if ctx.rand < extra.lorand || ctx.rand > extra.hirand {
        return None;
    }
    let mut gain = 1.0;
    for g in &extra.cc_gates {
        gain *= g.gain(ctx.cc);
        if gain <= 0.0 {
            return None;
        }
    }
    let pitch_cents = if extra.pitch_random != 0.0 {
        (ctx.rand * 2.0 - 1.0) * extra.pitch_random
    } else {
        0.0
    };
    Some(GateResult { gain, pitch_cents })
}

/// 确定性随机：同类事件顺序回放时结果一致（离线导出 = 实时回放）。
#[inline]
pub fn hash_rand(channel: u8, key: u8, serial: u32) -> f32 {
    let mut h = (channel as u32).wrapping_mul(0x9E37_79B9)
        ^ (key as u32).wrapping_mul(0x85EB_CA6B)
        ^ serial.wrapping_mul(0xC2B2_AE35);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2545_F491);
    h ^= h >> 13;
    (h & 0xFFFF) as f32 / 65535.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx<'a>(cc: &'a [u8; 128], rand: f32) -> GateCtx<'a> {
        GateCtx {
            cc,
            sw_last: 60,
            seq_counter: 0,
            rand,
        }
    }

    #[test]
    fn xfin_fades_linearly() {
        let mut cc = [0u8; 128];
        let g = CcGate {
            cc: 20,
            lo: 0,
            hi: 100,
            dir: GateDir::In,
        };
        cc[20] = 0;
        assert_eq!(g.gain(&cc), 0.0);
        cc[20] = 50;
        assert!((g.gain(&cc) - 0.5).abs() < 1e-6);
        cc[20] = 100;
        assert_eq!(g.gain(&cc), 1.0);
        cc[20] = 127;
        assert_eq!(g.gain(&cc), 1.0);
    }

    #[test]
    fn xfout_is_inverse() {
        let mut cc = [0u8; 128];
        let g = CcGate {
            cc: 20,
            lo: 0,
            hi: 100,
            dir: GateDir::Out,
        };
        cc[20] = 0;
        assert_eq!(g.gain(&cc), 1.0);
        cc[20] = 100;
        assert_eq!(g.gain(&cc), 0.0);
    }

    #[test]
    fn on_gate_is_binary() {
        let mut cc = [0u8; 128];
        let g = OnGate {
            cc: 64,
            lo: 127,
            hi: 127,
        };
        cc[64] = 0;
        assert!(!g.passes(&cc));
        cc[64] = 127;
        assert!(g.passes(&cc));
    }

    #[test]
    fn seq_position_selects_one_of_n() {
        let mut e = LayerExtra {
            seq_length: 4,
            seq_position: 2,
            ..Default::default()
        };
        e.hirand = 1.0;
        let c = ctx(&[0u8; 128], 0.5);
        // counter 0 → pos 1（不命中）；counter 1 → pos 2（命中）。
        assert!(evaluate(&e, 60, &c).is_none());
        let c1 = GateCtx {
            seq_counter: 1,
            ..ctx(&[0u8; 128], 0.5)
        };
        assert!(evaluate(&e, 60, &c1).is_some());
    }

    #[test]
    fn keyswitch_swallows_keys_in_range() {
        let e = LayerExtra {
            has_sw: true,
            sw_lokey: 24,
            sw_hikey: 30,
            sw_last: 0xFF,
            hirand: 1.0,
            ..Default::default()
        };
        let c = ctx(&[0u8; 128], 0.5);
        assert!(evaluate(&e, 26, &c).is_none());
        assert!(evaluate(&e, 60, &c).is_some());
    }

    #[test]
    fn pitch_random_is_bipolar_and_bounded() {
        let e = LayerExtra {
            pitch_random: 100.0,
            hirand: 1.0,
            ..Default::default()
        };
        let lo = evaluate(&e, 60, &ctx(&[0u8; 128], 0.0)).unwrap();
        let hi = evaluate(&e, 60, &ctx(&[0u8; 128], 1.0)).unwrap();
        assert!((lo.pitch_cents + 100.0).abs() < 1e-4);
        assert!((hi.pitch_cents - 100.0).abs() < 1e-4);
    }

    #[test]
    fn hash_rand_is_deterministic_and_in_range() {
        let a = hash_rand(3, 60, 7);
        assert_eq!(a, hash_rand(3, 60, 7));
        assert!((0.0..=1.0).contains(&a));
        assert_ne!(a, hash_rand(3, 61, 7));
    }

    #[test]
    fn plain_extra_reports_plain() {
        let e = LayerExtra {
            hirand: 1.0,
            sw_last: 0xFF,
            ..Default::default()
        };
        assert!(e.is_plain());
    }
}
