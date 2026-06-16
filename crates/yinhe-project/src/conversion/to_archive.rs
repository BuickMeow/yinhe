use std::collections::{BTreeMap, BTreeSet, HashMap};

use uuid::Uuid;

use crate::archive::ProjectArchive;
use crate::events::*;
use crate::header::*;
use crate::paths::*;
use crate::schema::*;

use super::{rpn_number, TrackEventGroup};

/// Convert a MidiFile into a ProjectArchive.
pub fn midi_to_archive(midi: &yinhe_midi::MidiFile) -> ProjectArchive {
    let names: Vec<String> = (0..midi.track_ports.len())
        .map(|i| {
            midi.track_names
                .get(i)
                .cloned()
                .unwrap_or_else(|| format!("Track {}", i + 1))
        })
        .collect();
    midi_to_archive_with_names(midi, &names, None)
}

/// Same as `midi_to_archive` but uses caller-provided track names (the
/// authoritative editable copy from `Document.track_names`).
pub fn midi_to_archive_with_names(
    midi: &yinhe_midi::MidiFile,
    track_names: &[String],
    progress: Option<&dyn Fn(f32, &str)>,
) -> ProjectArchive {
    let mut archive = ProjectArchive::new();

    write_conductor_events(&mut archive, midi);

    let num_tracks = midi.track_ports.len();
    let groups = group_events_by_track_channel(midi, num_tracks, progress);
    let channels_per_track = discover_channels_per_track(&groups, num_tracks);
    let (group_to_new_idx, total_new_tracks) =
        assign_track_indices(&channels_per_track, num_tracks);
    let (new_track_names, new_track_ports, new_track_channel_prefixes) = build_track_metadata(
        midi,
        track_names,
        &channels_per_track,
        &group_to_new_idx,
        total_new_tracks,
    );

    let mut port_map: HashMap<u8, Vec<(u8, Vec<TrackMapping>)>> = HashMap::new();

    write_group_entries(
        &mut archive,
        &groups,
        &channels_per_track,
        &group_to_new_idx,
        &new_track_names,
        &new_track_ports,
        &new_track_channel_prefixes,
        num_tracks,
        &mut port_map,
    );

    write_empty_track_mappings(midi, track_names, &channels_per_track, num_tracks, &mut port_map);
    write_mapping_and_project(&mut archive, port_map, track_names, midi);

    archive
}

fn write_conductor_events(archive: &mut ProjectArchive, midi: &yinhe_midi::MidiFile) {
    let tempos: Vec<TempoEvent> = midi
        .tempo_segments
        .iter()
        .map(|s| TempoEvent {
            tick: s.start_tick,
            bpm: yinhe_midi::bpm_from_mpq(s.micros_per_quarter) as f32,
        })
        .collect();
    if !tempos.is_empty() {
        archive.set_delta_events(
            conductor_path("tempo.zst"),
            FileHeader::new(*b"YHTM", 0, 0, 0),
            &tempos,
        );
    }

    let time_sigs: Vec<TimeSigEvent> = midi
        .time_sig_events
        .iter()
        .map(|e| TimeSigEvent {
            tick: e.tick,
            numerator: e.numerator,
            denominator_power: e.denominator,
        })
        .collect();
    if !time_sigs.is_empty() {
        archive.set_delta_events(
            conductor_path("time_sig.zst"),
            FileHeader::new(*b"YHTS", 0, 0, 0),
            &time_sigs,
        );
    }
}

fn group_events_by_track_channel(
    midi: &yinhe_midi::MidiFile,
    num_tracks: usize,
    progress: Option<&dyn Fn(f32, &str)>,
) -> HashMap<(usize, u8), TrackEventGroup> {
    let mut groups: HashMap<(usize, u8), TrackEventGroup> = HashMap::new();

    let total_notes: usize = midi.key_notes.iter().map(|n| n.len()).sum();
    let total_events = total_notes + midi.control_events.len();
    let mut processed: usize = 0;
    let progress_step = (total_events.max(1) / 100).max(1000);

    for (key_idx, key_notes) in midi.key_notes.iter().enumerate() {
        for note in key_notes {
            let idx = note.track as usize;
            if idx < num_tracks {
                let ch = midi.track_channels.get(idx).copied().unwrap_or(0);
                groups
                    .entry((idx, ch))
                    .or_insert_with(TrackEventGroup::new)
                    .notes
                    .push(Note {
                        start_tick: note.start_tick,
                        end_tick: note.end_tick,
                        key: key_idx as u8,
                        velocity: note.velocity,
                    });
            }
            processed += 1;
            if processed % progress_step == 0 {
                if let Some(cb) = progress {
                    cb(
                        processed as f32 / total_events.max(1) as f32,
                        &format!("{}/{}", processed.min(total_events), total_events),
                    );
                }
            }
        }
    }

    for ev in &midi.control_events {
        let ev_track = match ev {
            yinhe_midi::MidiControlEvent::ControlChange { track, .. }
            | yinhe_midi::MidiControlEvent::ProgramChange { track, .. }
            | yinhe_midi::MidiControlEvent::PitchBend { track, .. } => *track,
        };
        let idx = ev_track as usize;
        if idx >= num_tracks {
            continue;
        }
        let ev_channel = midi.track_channels.get(idx).copied().unwrap_or(0);
        let data = groups
            .entry((idx, ev_channel))
            .or_insert_with(TrackEventGroup::new);
        match ev {
            yinhe_midi::MidiControlEvent::ControlChange {
                tick, controller, value, ..
            } => {
                data.cc_events
                    .push((*controller, CcEvent { tick: *tick, value: *value }));
            }
            yinhe_midi::MidiControlEvent::PitchBend { tick, value, .. } => {
                data.pitch_events.push(PitchBendEvent {
                    tick: *tick,
                    value: *value,
                });
            }
            yinhe_midi::MidiControlEvent::ProgramChange { tick, program, .. } => {
                data.pc_events.push(PcEvent {
                    tick: *tick,
                    program: *program,
                    bank_msb: 0xFF,
                    bank_lsb: 0xFF,
                });
            }
        }
        processed += 1;
        if processed % progress_step == 0 {
            if let Some(cb) = progress {
                cb(
                    processed as f32 / total_events.max(1) as f32,
                    &format!("{}/{}", processed.min(total_events), total_events),
                );
            }
        }
    }

    for data in groups.values_mut() {
        data.notes.sort_by_key(|n| n.start_tick);
    }

    groups
}

fn discover_channels_per_track(
    groups: &HashMap<(usize, u8), TrackEventGroup>,
    num_tracks: usize,
) -> Vec<Vec<u8>> {
    let mut channels_per_track: Vec<Vec<u8>> = (0..num_tracks).map(|_| Vec::new()).collect();
    for (&(track_idx, channel), data) in groups {
        if data.has_any_events() && !channels_per_track[track_idx].contains(&channel) {
            channels_per_track[track_idx].push(channel);
        }
    }
    for channels in &mut channels_per_track {
        channels.sort();
    }
    channels_per_track
}

fn assign_track_indices(
    channels_per_track: &[Vec<u8>],
    num_tracks: usize,
) -> (HashMap<(usize, u8), usize>, usize) {
    let mut extra_before = 0usize;
    let mut group_to_new_idx: HashMap<(usize, u8), usize> = HashMap::new();

    for original_idx in 0..num_tracks {
        let channels = &channels_per_track[original_idx];
        for (ch_idx, &ch) in channels.iter().enumerate() {
            let new_idx = original_idx + extra_before + ch_idx;
            group_to_new_idx.insert((original_idx, ch), new_idx);
        }
        extra_before += channels.len().saturating_sub(1);
    }

    (group_to_new_idx, num_tracks + extra_before)
}

fn build_track_metadata(
    midi: &yinhe_midi::MidiFile,
    track_names: &[String],
    channels_per_track: &[Vec<u8>],
    group_to_new_idx: &HashMap<(usize, u8), usize>,
    total_new_tracks: usize,
) -> (Vec<String>, Vec<u8>, Vec<Option<u8>>) {
    let num_tracks = channels_per_track.len();
    let mut new_track_names: Vec<String> = vec![String::new(); total_new_tracks];
    let mut new_track_ports: Vec<u8> = vec![0; total_new_tracks];
    let mut new_track_channel_prefixes: Vec<Option<u8>> = vec![None; total_new_tracks];

    for original_idx in 0..num_tracks {
        let channels = &channels_per_track[original_idx];
        let base_name = track_names
            .get(original_idx)
            .cloned()
            .or_else(|| midi.track_names.get(original_idx).cloned())
            .unwrap_or_else(|| format!("Track {}", original_idx + 1));
        let port = midi.track_ports.get(original_idx).copied().unwrap_or(0);
        let prefix = midi
            .track_channel_prefixes
            .get(original_idx)
            .copied()
            .flatten();

        for (ch_idx, &ch) in channels.iter().enumerate() {
            let new_idx = group_to_new_idx[&(original_idx, ch)];
            let name = if ch_idx == 0 {
                base_name.clone()
            } else {
                format!("{} (ch {})", base_name, ch & 0x0F)
            };
            new_track_names[new_idx] = name;
            new_track_ports[new_idx] = port;
            new_track_channel_prefixes[new_idx] = prefix;
        }
    }

    (new_track_names, new_track_ports, new_track_channel_prefixes)
}

#[allow(clippy::too_many_arguments)]
fn write_group_entries(
    archive: &mut ProjectArchive,
    groups: &HashMap<(usize, u8), TrackEventGroup>,
    channels_per_track: &[Vec<u8>],
    group_to_new_idx: &HashMap<(usize, u8), usize>,
    new_track_names: &[String],
    new_track_ports: &[u8],
    new_track_channel_prefixes: &[Option<u8>],
    num_tracks: usize,
    port_map: &mut HashMap<u8, Vec<(u8, Vec<TrackMapping>)>>,
) {
    for original_idx in 0..num_tracks {
        let channels = &channels_per_track[original_idx];
        for &ch in channels {
            let new_idx = group_to_new_idx[&(original_idx, ch)];
            let data = &groups[&(original_idx, ch)];
            let port = new_track_ports[new_idx];
            let raw_channel = ch & 0x0F;
            let uuid = Uuid::new_v4().to_string();
            let name = new_track_names[new_idx].clone();
            let inner = InnerHeader::new(new_idx as u16, ch);

            if !data.notes.is_empty() {
                archive.set_notes(
                    track_notes_path(ch, &uuid),
                    FileHeader::new(*b"YHTK", port, raw_channel, new_idx as u8),
                    inner,
                    &data.notes,
                );
            }

            let (cc_events, rpn_events, pc_events) = classify_control_events(data);

            for (cc_num, events) in &cc_events {
                archive.set_delta_events_with_inner(
                    cc_path(ch, &uuid, *cc_num),
                    FileHeader::new(*b"YHCC", port, raw_channel, *cc_num),
                    inner,
                    events,
                );
            }

            for (rpn_num, events) in &rpn_events {
                archive.set_delta_events_with_inner(
                    rpn_path(ch, &uuid, *rpn_num),
                    FileHeader::new(*b"YHRP", port, raw_channel, *rpn_num),
                    inner,
                    events,
                );
            }

            if !data.pitch_events.is_empty() {
                archive.set_delta_events_with_inner(
                    pitch_path(ch, &uuid),
                    FileHeader::new(*b"YHPB", port, raw_channel, 0),
                    inner,
                    &data.pitch_events,
                );
            }

            if !pc_events.is_empty() {
                archive.set_delta_events_with_inner(
                    pc_path(ch, &uuid),
                    FileHeader::new(*b"YHPC", port, raw_channel, 0),
                    inner,
                    &pc_events,
                );
            }

            super::add_to_port_map(
                port_map,
                port,
                raw_channel,
                &uuid,
                name,
                new_idx,
                new_track_channel_prefixes[new_idx],
            );
        }
    }
}

fn classify_control_events(
    data: &TrackEventGroup,
) -> (
    HashMap<u8, Vec<CcEvent>>,
    HashMap<u8, Vec<RpnEvent>>,
    Vec<PcEvent>,
) {
    let mut cc_by_tick: BTreeMap<u32, Vec<(u8, u8)>> = BTreeMap::new();
    let mut pc_by_tick: BTreeMap<u32, Vec<u8>> = BTreeMap::new();

    for (controller, ev) in &data.cc_events {
        cc_by_tick
            .entry(ev.tick)
            .or_default()
            .push((*controller, ev.value));
    }
    for ev in &data.pc_events {
        pc_by_tick.entry(ev.tick).or_default().push(ev.program);
    }

    let mut cc_events: HashMap<u8, Vec<CcEvent>> = HashMap::new();
    let mut rpn_events: HashMap<u8, Vec<RpnEvent>> = HashMap::new();
    let mut pc_events: Vec<PcEvent> = Vec::new();

    let mut all_ticks: BTreeSet<u32> = cc_by_tick.keys().copied().collect();
    all_ticks.extend(pc_by_tick.keys().copied());

    for tick in all_ticks {
        let ccs = cc_by_tick.get(&tick).map(|v| v.as_slice()).unwrap_or(&[]);
        let pcs = pc_by_tick.get(&tick).map(|v| v.as_slice()).unwrap_or(&[]);

        let msb = ccs.iter().find(|(c, _)| *c == 101).map(|(_, v)| *v);
        let lsb = ccs.iter().find(|(c, _)| *c == 100).map(|(_, v)| *v);
        let data_msb = ccs.iter().find(|(c, _)| *c == 6).map(|(_, v)| *v);
        let data_lsb = ccs.iter().find(|(c, _)| *c == 38).map(|(_, v)| *v);

        let consumed_rpn =
            if let (Some(msb_v), Some(lsb_v), Some(_)) = (msb, lsb, data_msb.or(data_lsb))
                && let Some(rpn_num) = rpn_number(msb_v, lsb_v)
            {
                let value = match (data_msb, data_lsb) {
                    (Some(m), Some(l)) => ((m as u16) << 7) | (l as u16),
                    (Some(m), None) => m as u16,
                    (None, Some(l)) => l as u16,
                    _ => 0,
                };
                rpn_events
                    .entry(rpn_num)
                    .or_default()
                    .push(RpnEvent { tick, value });
                true
            } else {
                false
            };

        let bank_msb = ccs
            .iter()
            .find(|(c, _)| *c == 0)
            .map(|(_, v)| *v)
            .unwrap_or(0xFF);
        let bank_lsb = ccs
            .iter()
            .find(|(c, _)| *c == 32)
            .map(|(_, v)| *v)
            .unwrap_or(0xFF);

        if !pcs.is_empty() {
            for &prog in pcs {
                pc_events.push(PcEvent {
                    tick,
                    program: prog,
                    bank_msb,
                    bank_lsb,
                });
            }
        }

        for (ctrl, val) in ccs {
            if consumed_rpn && matches!(ctrl, 101 | 100 | 6 | 38) {
                continue;
            }
            if !pcs.is_empty() && matches!(ctrl, 0 | 32) {
                continue;
            }
            cc_events
                .entry(*ctrl)
                .or_default()
                .push(CcEvent { tick, value: *val });
        }
    }

    (cc_events, rpn_events, pc_events)
}

fn write_empty_track_mappings(
    midi: &yinhe_midi::MidiFile,
    track_names: &[String],
    channels_per_track: &[Vec<u8>],
    num_tracks: usize,
    port_map: &mut HashMap<u8, Vec<(u8, Vec<TrackMapping>)>>,
) {
    for original_idx in 0..num_tracks {
        if !channels_per_track[original_idx].is_empty() {
            continue;
        }
        let port = midi.track_ports.get(original_idx).copied().unwrap_or(0);
        let uuid = Uuid::new_v4().to_string();
        let name = track_names
            .get(original_idx)
            .cloned()
            .or_else(|| midi.track_names.get(original_idx).cloned())
            .unwrap_or_else(|| format!("Track {}", original_idx + 1));

        super::add_to_port_map(
            port_map,
            port,
            0,
            &uuid,
            name,
            original_idx,
            midi.track_channel_prefixes
                .get(original_idx)
                .copied()
                .flatten(),
        );
    }
}

fn write_mapping_and_project(
    archive: &mut ProjectArchive,
    port_map: HashMap<u8, Vec<(u8, Vec<TrackMapping>)>>,
    track_names: &[String],
    midi: &yinhe_midi::MidiFile,
) {
    let mapping = MappingJson {
        ports: port_map
            .into_iter()
            .map(|(port, channels)| PortMapping {
                port,
                channels: channels
                    .into_iter()
                    .map(|(channel, tracks)| ChannelMapping { channel, tracks })
                    .collect(),
            })
            .collect(),
    };
    archive.set_json("mapping.json", FileHeader::new(*b"YHMP", 0, 0, 0), &mapping);

    let song_title = track_names
        .first()
        .filter(|n| !n.is_empty() && !n.starts_with("Track "))
        .cloned()
        .unwrap_or_default();
    let proj = ProjectJson {
        version: 1,
        name: song_title,
        artist: String::new(),
        ppq: midi.ticks_per_beat,
        zstd_level: 0,
        description: String::new(),
        soundfont_project_mode: false,
        soundfont_overrides: Vec::new(),
    };
    archive.set_json("project.json", FileHeader::new(*b"YHPR", 0, 0, 0), &proj);
}
