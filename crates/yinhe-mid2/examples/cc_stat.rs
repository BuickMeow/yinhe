// 诊断：统计 MIDI 的自动化（CC/bend/RPN/NRPN）分布，评估合成器 CC 覆盖
use std::collections::BTreeMap;
use yinhe_types::AutomationTarget;

fn main() {
    for path in std::env::args().skip(1) {
        let m = match yinhe_mid2::parse_path(&path) {
            Ok(m) => m,
            Err(e) => {
                println!("{path} -> ERR {e}");
                continue;
            }
        };
        let mut per_target: BTreeMap<String, (usize, usize)> = BTreeMap::new(); // target -> (events, tracks)
        let mut total_events = 0usize;
        let mut channels: BTreeMap<String, usize> = BTreeMap::new(); // (port,ch) -> tracks
        for t in &m.tracks {
            let ch_key = format!("port{} ch{}", t.port, t.channel);
            *channels.entry(ch_key).or_insert(0) += 1;
            for lane in &t.automation_lanes {
                let key = match lane.target {
                    AutomationTarget::CC { controller } => format!("CC{controller}"),
                    AutomationTarget::PitchBend => "PitchBend".into(),
                    AutomationTarget::Rpn { parameter } => format!("RPN{parameter}"),
                    AutomationTarget::Nrpn { parameter } => format!("NRPN{parameter}"),
                    AutomationTarget::Tempo => "Tempo".into(),
                };
                let e = per_target.entry(key).or_insert((0, 0));
                e.0 += lane.events.len();
                e.1 += 1;
                total_events += lane.events.len();
            }
        }
        println!(
            "== {} == ppq={} tick_length={} tracks={} notes={} total_auto_events={}",
            path,
            m.meta.ppq,
            m.tick_length,
            m.tracks.len(),
            m.tracks.iter().map(|t| t.notes.len()).sum::<usize>(),
            total_events,
        );
        for (k, (n, tr)) in &per_target {
            println!("  {k:>12}: {n:>8} events across {tr} tracks");
        }
        println!("  channels: {channels:?}");
    }
}
