//! 层级继承：`<control>` < `<global>` < `<master>` < `<group>` < `<region>`。
//!
//! 与 SFZ 规范（以及旧 xsynth 实现）一致：进入更内层标签时复制外层并集，
//! 返回外层时丢弃内层。`<region>` 结束时落成一条 `SfzRegion`。

use std::path::{Path, PathBuf};

use super::lexer::{Token, read_tokens};
use super::opcodes::{RegionFields, apply};
use super::types::{SfzFile, SfzRegion};

/// 标签层级深度：`<control>` 是最外层基底（它提供 `default_path` 与默认 CC，
/// 也被后续层级继承），`<region>` 是最内层。
fn level(tag: &str) -> Option<usize> {
    match tag {
        "control" => Some(0),
        "global" => Some(1),
        "master" => Some(2),
        "group" => Some(3),
        "region" => Some(4),
        _ => None,
    }
}

/// 解析一个 SFZ 文件（含 include 展开）。
pub fn parse(path: &Path) -> Result<SfzFile, String> {
    let tokens = read_tokens(path)?;
    let base = path.parent().unwrap_or(Path::new(".")).to_path_buf();
    Ok(build(&tokens, &base))
}

fn build(tokens: &[Token], base: &Path) -> SfzFile {
    let mut out = SfzFile::default();
    let mut stack: Vec<RegionFields> = Vec::new();
    let mut current: Option<usize> = None;
    let mut pending_cc: Vec<(u8, u8)> = Vec::new();
    for token in tokens {
        match token {
            Token::Group(tag) => {
                if current == Some(4) {
                    emit(&mut out, &mut stack, base);
                }
                match level(tag) {
                    Some(lv) => {
                        current = Some(lv);
                        // 进入更内层：以当前栈顶为基底复制；返回外层时截断。
                        while stack.len() <= lv {
                            let parent = stack.last().cloned().unwrap_or_default();
                            stack.push(parent);
                        }
                        stack.truncate(lv + 1);
                    }
                    None => current = None,
                }
            }
            Token::Opcode(name, value) => {
                let Some(lv) = current else { continue };
                let Some(fields) = stack.get_mut(lv) else {
                    continue;
                };
                // `<control>` 的 `set_ccN` 不进 region 继承链（它是通道初值）。
                if lv == 0
                    && let Some(rest) = name.strip_prefix("set_cc")
                    && let (Ok(cc), Ok(v)) = (rest.parse::<u8>(), value.trim().parse::<f32>())
                    && cc < 128
                {
                    pending_cc.push((cc, v.clamp(0.0, 127.0) as u8));
                    continue;
                }
                apply(fields, name, value, &mut out.unknown_opcodes);
            }
            Token::Include(_) => {}
        }
    }
    if current == Some(4) {
        emit(&mut out, &mut stack, base);
    }
    out.cc_defaults = pending_cc;
    out
}

/// 把当前 `<region>` 字段落成 `SfzRegion`（缺 sample 的静默跳过）。
fn emit(out: &mut SfzFile, stack: &mut [RegionFields], base: &Path) {
    let Some(fields) = stack.last().cloned() else {
        return;
    };
    let Some(sample) = fields.sample.clone() else {
        return;
    };
    let rel = match &fields.default_path {
        Some(dp) if !Path::new(&sample).is_absolute() => Path::new(dp).join(&sample),
        _ => PathBuf::from(&sample),
    };
    let sample_path = if rel.is_absolute() {
        rel
    } else {
        base.join(rel)
    };
    if !sample_path.exists() {
        out.missing_samples += 1;
    }

    // `key=` 已同时写 lokey/hikey/pitch_keycenter；只给 lokey/hikey 时
    // pitch_keycenter 默认取 lokey（SFZ 规范），再退到 60。
    let lokey = fields.lokey.unwrap_or(0);
    let hikey = fields.hikey.unwrap_or(127);
    let pitch_keycenter = fields.pitch_keycenter.unwrap_or(lokey.clamp(0, 127));

    out.regions.push(SfzRegion {
        sample_path,
        keyrange: lokey..=hikey,
        velrange: fields.lovel.min(fields.hivel)..=fields.hivel.max(fields.lovel),
        pitch_keycenter,
        tune: fields.tune,
        volume_db: fields.volume_db,
        pan: fields.pan,
        amp_veltrack: fields.amp_veltrack,
        amp_keytrack: fields.amp_keytrack,
        amp_keycenter: fields.amp_keycenter,
        pan_veltrack: fields.pan_veltrack,
        pan_keytrack: fields.pan_keytrack,
        pan_keycenter: fields.pan_keycenter,
        fil_veltrack: fields.fil_veltrack,
        fil_keytrack: fields.fil_keytrack,
        fil_keycenter: fields.fil_keycenter,
        filter_type: fields.filter_type,
        cutoff: fields.cutoff,
        resonance_db: fields.resonance_db,
        ampeg: fields.ampeg,
        loop_mode: fields.loop_mode,
        loop_start: fields.loop_start,
        loop_end: fields.loop_end,
        offset: fields.offset,
        trigger: fields.trigger,
        group: fields.group,
        off_by: fields.off_by,
        seq_length: fields.seq_length,
        seq_position: fields.seq_position,
        sw_last: fields.sw_last,
        sw_lokey: fields.sw_lokey,
        sw_hikey: fields.sw_hikey,
        cc_gates: fields.cc_gates.clone(),
        on_gates: fields.on_gates.clone(),
        lorand: fields.lorand,
        hirand: fields.hirand,
        pitch_random: fields.pitch_random,
        amplfo: fields.amplfo,
        pitchlfo: fields.pitchlfo,
        fillfo: fields.fillfo,
        effect1: fields.effect1,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(dir: &Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        fs::write(&p, body).unwrap();
        p
    }

    #[test]
    fn inherits_global_master_group_region() {
        let dir = std::env::temp_dir().join(format!("yinhe-sfz-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.wav"), b"RIFF").unwrap();
        let p = write(
            &dir,
            "t.sfz",
            "<global>\nvolume=-2\n<master>\npan=10\n<group>\nlokey=60 hikey=61\n<region>\nsample=a.wav\n",
        );
        let out = parse(&p).unwrap();
        assert_eq!(out.regions.len(), 1);
        let r = &out.regions[0];
        assert_eq!(*r.keyrange.start(), 60);
        assert_eq!(*r.keyrange.end(), 61);
        assert!((r.volume_db + 2.0).abs() < 1e-6);
        assert!((r.pan - 10.0).abs() < 1e-6);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn collects_set_cc_defaults() {
        let dir = std::env::temp_dir().join(format!("yinhe-sfz-cc-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.wav"), b"RIFF").unwrap();
        let p = write(
            &dir,
            "t.sfz",
            "<control>\nset_cc14=127\nset_cc15=85\n<region>\nsample=a.wav\n",
        );
        let out = parse(&p).unwrap();
        assert_eq!(out.cc_defaults, vec![(14, 127), (15, 85)]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn parses_xfin_and_trigger_and_lfo() {
        let dir = std::env::temp_dir().join(format!("yinhe-sfz-xf-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("p.wav"), b"RIFF").unwrap();
        let p = write(
            &dir,
            "t.sfz",
            "<region>\nsample=p.wav\nxfin_locc20=0 xfin_hicc20=127\ntrigger=release\n\
             on_locc64=127 on_hicc64=127\npitch_random=182\namplfo_depth=-0.8 amplfo_freq=0.5\n\
             seq_length=4 seq_position=2\n",
        );
        let out = parse(&p).unwrap();
        let r = &out.regions[0];
        assert_eq!(r.cc_gates.len(), 1);
        assert_eq!(r.cc_gates[0].cc, 20);
        assert_eq!(r.cc_gates[0].lo, 0);
        assert_eq!(r.cc_gates[0].hi, 127);
        assert_eq!(r.on_gates.len(), 1);
        assert!((r.pitch_random - 182.0).abs() < 1e-6);
        assert_eq!(r.seq_length, 4);
        assert_eq!(r.seq_position, 2);
        let lfo = r.amplfo.unwrap();
        assert!((lfo.depth + 0.8).abs() < 1e-6);
        assert!((lfo.freq - 0.5).abs() < 1e-6);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn follows_include_and_default_path() {
        let dir = std::env::temp_dir().join(format!("yinhe-sfz-inc-{}", std::process::id()));
        fs::create_dir_all(dir.join("Samples")).unwrap();
        fs::write(dir.join("Samples/k.wav"), b"RIFF").unwrap();
        write(
            &dir,
            "inc.sfz",
            "<region>\nsample=k.wav\nlokey=60 hikey=60\n",
        );
        let p = write(
            &dir,
            "root.sfz",
            "<control>\ndefault_path=Samples/\n<group>\n#include \"inc.sfz\"\n",
        );
        // include 位于 group 内：继承 default_path。
        let out = parse(&p).unwrap();
        assert_eq!(out.regions.len(), 1);
        assert!(out.regions[0].sample_path.ends_with("Samples/k.wav"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn counts_missing_samples() {
        let dir = std::env::temp_dir().join(format!("yinhe-sfz-miss-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let p = write(&dir, "t.sfz", "<region>\nsample=nope.wav\n");
        let out = parse(&p).unwrap();
        assert_eq!(out.missing_samples, 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn key_names_parse_like_sfz() {
        assert_eq!(super::super::opcodes::parse_key("c4"), Some(60));
        assert_eq!(super::super::opcodes::parse_key("db3"), Some(49));
        // SFZ 的 `c#-1` = MIDI 1（C-1 是 note 0）
        assert_eq!(super::super::opcodes::parse_key("c#-1"), Some(1));
        assert_eq!(super::super::opcodes::parse_key("60"), Some(60));
    }
}
