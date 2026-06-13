use std::collections::{BTreeMap, BTreeSet, HashMap};

use midly::num::{u4, u7, u15};
use midly::{Format, Header, MetaMessage, MidiMessage, PitchBend, Smf, Timing, TrackEvent, TrackEventKind};
use std::io::Write;
use uuid::Uuid;
use yinhe_project::*;

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
    midi_to_archive_with_names(midi, &names)
}

/// Same as `midi_to_archive` but uses caller-provided track names (the
/// authoritative editable copy from `Document.track_names`).
pub fn midi_to_archive_with_names(
    midi: &yinhe_midi::MidiFile,
    track_names: &[String],
) -> ProjectArchive {
    let mut archive = ProjectArchive::new();

    // ── Conductor events ──
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

    // ── Group events by (track, channel) ──
    let num_tracks = midi.track_ports.len();

    struct TrackChannelData {
        notes: Vec<yinhe_project::Note>,
        cc_events: Vec<(u8, yinhe_project::CcEvent)>,
        rpn_events: Vec<(u8, yinhe_project::RpnEvent)>,
        pitch_events: Vec<yinhe_project::PitchBendEvent>,
        pc_events: Vec<yinhe_project::PcEvent>,
    }

    let mut groups: HashMap<(usize, u8), TrackChannelData> = HashMap::new();

    for (key_idx, key_notes) in midi.key_notes.iter().enumerate() {
        for note in key_notes {
            let idx = note.track as usize;
            if idx < num_tracks {
                groups
                    .entry((idx, note.channel))
                    .or_insert_with(|| TrackChannelData {
                        notes: Vec::new(),
                        cc_events: Vec::new(),
                        rpn_events: Vec::new(),
                        pitch_events: Vec::new(),
                        pc_events: Vec::new(),
                    })
                    .notes
                    .push(yinhe_project::Note {
                        start_tick: note.start_tick,
                        end_tick: note.end_tick,
                        key: key_idx as u8,
                        velocity: note.velocity,
                    });
            }
        }
    }

    for ev in &midi.control_events {
        let (ev_track, ev_channel) = match ev {
            yinhe_midi::MidiControlEvent::ControlChange { track, channel, .. }
            | yinhe_midi::MidiControlEvent::ProgramChange { track, channel, .. }
            | yinhe_midi::MidiControlEvent::PitchBend { track, channel, .. } => (*track, *channel),
        };
        let idx = ev_track as usize;
        if idx >= num_tracks {
            continue;
        }
        let data = groups.entry((idx, ev_channel)).or_insert_with(|| TrackChannelData {
            notes: Vec::new(),
            cc_events: Vec::new(),
            rpn_events: Vec::new(),
            pitch_events: Vec::new(),
            pc_events: Vec::new(),
        });
        match ev {
            yinhe_midi::MidiControlEvent::ControlChange {
                tick, controller, value, ..
            } => {
                data.cc_events.push((*controller, yinhe_project::CcEvent { tick: *tick, value: *value }));
            }
            yinhe_midi::MidiControlEvent::PitchBend { tick, value, .. } => {
                data.pitch_events.push(yinhe_project::PitchBendEvent {
                    tick: *tick,
                    value: *value,
                });
            }
            yinhe_midi::MidiControlEvent::ProgramChange { tick, program, .. } => {
                data.pc_events.push(yinhe_project::PcEvent {
                    tick: *tick,
                    program: *program,
                    bank_msb: 0xFF,
                    bank_lsb: 0xFF,
                });
            }
        }
    }

    for data in groups.values_mut() {
        data.notes.sort_by_key(|n| n.start_tick);
    }

    // ── Discover channels per original track ──
    let mut channels_per_track: Vec<Vec<u8>> = (0..num_tracks).map(|_| Vec::new()).collect();
    for (&(track_idx, channel), data) in &groups {
        let has_notes = !data.notes.is_empty();
        let has_cc = !data.cc_events.is_empty();
        let has_rpn = !data.rpn_events.is_empty();
        let has_pitch = !data.pitch_events.is_empty();
        let has_pc = !data.pc_events.is_empty();
        if has_notes || has_cc || has_rpn || has_pitch || has_pc {
            if !channels_per_track[track_idx].contains(&channel) {
                channels_per_track[track_idx].push(channel);
            }
        }
    }
    for channels in &mut channels_per_track {
        channels.sort();
    }

    // ── Assign new track indices ──
    // First channel per original track keeps the original index.
    // Extra channels are inserted after the original; subsequent tracks shift.
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

    let total_new_tracks = num_tracks + extra_before;

    // ── Build track metadata arrays ──
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
        let prefix = midi.track_channel_prefixes.get(original_idx).copied().flatten();

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

    // ── Write archive entries per (track, channel) group ──
    let mut port_map: HashMap<u8, Vec<(u8, Vec<TrackMapping>)>> = HashMap::new();

    for original_idx in 0..num_tracks {
        let channels = &channels_per_track[original_idx];
        for (ch_idx, &ch) in channels.iter().enumerate() {
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

            // Collect CC events per controller number for this (track, channel) group,
            // extracting known RPN sequences (CC 101/100/6/38) into dedicated RPN entries.
            let mut cc_by_tick: BTreeMap<u32, Vec<(u8, u8)>> = BTreeMap::new();
            let mut pitch_events: Vec<PitchBendEvent> = Vec::new();
            let mut pc_by_tick: BTreeMap<u32, Vec<u8>> = BTreeMap::new();
            for (controller, ev) in &data.cc_events {
                cc_by_tick.entry(ev.tick).or_default().push((*controller, ev.value));
            }
            for ev in &data.pitch_events {
                pitch_events.push(PitchBendEvent {
                    tick: ev.tick,
                    value: ev.value,
                });
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

                let consumed_rpn = if let (Some(msb_v), Some(lsb_v), Some(dv)) = (msb, lsb, data_msb.or(data_lsb))
                    && let Some(rpn_num) = rpn_number(msb_v, lsb_v)
                {
                    let value = match (data_msb, data_lsb) {
                        (Some(m), Some(l)) => ((m as u16) << 7) | (l as u16),
                        (Some(m), None) => m as u16,
                        (None, Some(l)) => l as u16,
                        _ => 0,
                    };
                    rpn_events.entry(rpn_num).or_default().push(RpnEvent { tick, value });
                    true
                } else {
                    false
                };

                let bank_msb = ccs.iter().find(|(c, _)| *c == 0).map(|(_, v)| *v).unwrap_or(0xFF);
                let bank_lsb = ccs.iter().find(|(c, _)| *c == 32).map(|(_, v)| *v).unwrap_or(0xFF);

                if !pcs.is_empty() {
                    for &prog in pcs {
                        pc_events.push(PcEvent { tick, program: prog, bank_msb, bank_lsb });
                    }
                }

                for (ctrl, val) in ccs {
                    if consumed_rpn && matches!(ctrl, 101 | 100 | 6 | 38) {
                        continue;
                    }
                    if !pcs.is_empty() && matches!(ctrl, 0 | 32) {
                        continue;
                    }
                    cc_events.entry(*ctrl).or_default().push(CcEvent { tick, value: *val });
                }
            }

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

            if !pitch_events.is_empty() {
                archive.set_delta_events_with_inner(
                    pitch_path(ch, &uuid),
                    FileHeader::new(*b"YHPB", port, raw_channel, 0),
                    inner,
                    &pitch_events,
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

            // Add to mapping (group by (port, raw_channel))
            let channels_entry = port_map.entry(port).or_default();
            let ch_entry = if let Some(existing) = channels_entry.iter_mut().find(|(c, _)| *c == raw_channel) {
                existing
            } else {
                channels_entry.push((raw_channel, Vec::new()));
                channels_entry.last_mut().unwrap()
            };
            ch_entry.1.push(TrackMapping {
                uuid,
                name,
                color: [0.5, 0.5, 0.5],
                track_index: new_idx as u16,
                channel_prefix: new_track_channel_prefixes[new_idx],
            });
        }
    }

    // ── Ensure empty tracks (e.g. conductor) still have mapping entries ──
    for original_idx in 0..num_tracks {
        if !channels_per_track[original_idx].is_empty() {
            continue;
        }
        let port = midi.track_ports.get(original_idx).copied().unwrap_or(0);
        let raw_channel = 0;
        let uuid = Uuid::new_v4().to_string();
        let name = track_names
            .get(original_idx)
            .cloned()
            .or_else(|| midi.track_names.get(original_idx).cloned())
            .unwrap_or_else(|| format!("Track {}", original_idx + 1));

        let channels_entry = port_map.entry(port).or_default();
        let ch_entry = if let Some(existing) = channels_entry.iter_mut().find(|(c, _)| *c == raw_channel) {
            existing
        } else {
            channels_entry.push((raw_channel, Vec::new()));
            channels_entry.last_mut().unwrap()
        };
        ch_entry.1.push(TrackMapping {
            uuid,
            name,
            color: [0.5, 0.5, 0.5],
            track_index: original_idx as u16,
            channel_prefix: midi.track_channel_prefixes.get(original_idx).copied().flatten(),
        });
    }

    // ── Write mapping.json ──
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
    archive.set_events("mapping.json", FileHeader::new(*b"YHMP", 0, 0, 0), &[mapping]);

    // ── Write project.json ──
    let proj = ProjectJson {
        version: 1,
        name: String::new(),
        artist: String::new(),
        ppq: midi.ticks_per_beat,
        zstd_level: 0,
        description: String::new(),
        soundfont_project_mode: false,
        soundfont_overrides: Vec::new(),
    };
    archive.set_events("project.json", FileHeader::new(*b"YHPR", 0, 0, 0), &[proj]);

    archive
}

/// Convert a ProjectArchive back into a MidiFile.
pub fn archive_to_midi(archive: &ProjectArchive) -> yinhe_midi::MidiFile {
    let mut midi = yinhe_midi::MidiFile::default();

    // ── Read project.json for ppq ──
    if let Some(proj) = archive.get_events::<ProjectJson>("project.json") {
        if let Some(p) = proj.first() {
            midi.ticks_per_beat = p.ppq;
        }
    }

    // ── Read conductor events ──
    {
        let tempos = archive
            .get_delta_events::<TempoEvent>(&conductor_path("tempo.zst"))
            .unwrap_or_default();
        let mut segments: Vec<yinhe_midi::TempoSegment> = tempos
            .iter()
            .map(|t| yinhe_midi::TempoSegment {
                start_tick: t.tick,
                start_time: 0.0,
                micros_per_quarter: yinhe_midi::mpq_from_bpm(t.bpm),
            })
            .collect();
        // Ensure there is always a tempo segment at tick 0 (matches MidiParser
        // behaviour). Without this, time-based seeking before the first tempo
        // event would fall through to the default-mpq fallback path.
        if segments.first().map(|s| s.start_tick).unwrap_or(u32::MAX) > 0 {
            segments.insert(0, yinhe_midi::TempoSegment {
                start_tick: 0,
                start_time: 0.0,
                micros_per_quarter: yinhe_midi::mpq_from_bpm(120.0),
            });
        }
        yinhe_midi::recompute_tempo_start_times(&mut segments, midi.ticks_per_beat);
        midi.tempo_segments = segments;
    }

    if let Some(time_sigs) = archive.get_delta_events::<TimeSigEvent>(&conductor_path("time_sig.zst")) {
        midi.time_sig_events = time_sigs
            .iter()
            .map(|e| yinhe_types::TimeSigEvent {
                tick: e.tick,
                numerator: e.numerator,
                denominator: e.denominator_power,
            })
            .collect();
    }

    // ── Read mapping ──
    let mapping: Vec<MappingJson> = archive.get_events("mapping.json").unwrap_or_default();
    let mapping = mapping.into_iter().next();

    // Build uuid → track_index lookup from mapping.
    let mut uuid_to_track: HashMap<&str, usize> = HashMap::new();
    if let Some(mapping) = &mapping {
        for port_mapping in &mapping.ports {
            for ch_mapping in &port_mapping.channels {
                for track_mapping in &ch_mapping.tracks {
                    uuid_to_track.insert(
                        track_mapping.uuid.as_str(),
                        track_mapping.track_index as usize,
                    );
                    let _ = port_mapping.port;
                    let _ = ch_mapping.channel;
                }
            }
        }
    }

    // Extract uuid from a path matching "channels/{label}/{uuid}/...".
    fn extract_uuid(path: &str) -> Option<&str> {
        let rest = path.strip_prefix("channels/")?;
        let slash1 = rest.find('/')?;
        let after_label = &rest[slash1 + 1..];
        let slash2 = after_label.find('/')?;
        Some(&after_label[..slash2])
    }

    // ── Rebuild track data from archive entries ──
    // Each archive entry carries its own channel in the InnerHeader.
    // We group by track_index from mapping; multiple channels per track
    // are allowed (they'll be split into separate tracks later).
    let mut track_data: Vec<Option<TrackReadData>> = Vec::new();

    struct TrackReadData {
        notes: Vec<yinhe_project::Note>,
        cc_events: Vec<(u8, CcEvent)>,
        rpn_events: Vec<(u8, RpnEvent)>,
        pitch_events: Vec<PitchBendEvent>,
        pc_events: Vec<PcEvent>,
    }

    for (path, entry) in &archive.entries {
        let path = path.as_str();
        if path.starts_with("conductor/") || path == "mapping.json" || path == "project.json" {
            continue;
        }
        let Some(uuid_str) = extract_uuid(path) else { continue };
        let Some(&track_idx) = uuid_to_track.get(uuid_str) else { continue };

        while track_data.len() <= track_idx {
            track_data.push(None);
        }
        let data = track_data[track_idx].get_or_insert_with(|| {
            TrackReadData {
                notes: Vec::new(),
                cc_events: Vec::new(),
                rpn_events: Vec::new(),
                pitch_events: Vec::new(),
                pc_events: Vec::new(),
            }
        });

        let h = entry.header;
        match h.magic {
            magic::TRACK_NOTES => {
                if let Some((_inner, notes)) = archive.get_notes(path) {
                    data.notes.extend(notes);
                }
            }
            magic::CC => {
                if let Some((_inner, events)) = archive.get_delta_events_with_inner::<CcEvent>(path) {
                    for ev in &events {
                        data.cc_events.push((h.extra, *ev));
                    }
                }
            }
            magic::PITCH_BEND => {
                if let Some((_inner, events)) = archive.get_delta_events_with_inner::<PitchBendEvent>(path) {
                    data.pitch_events.extend(events);
                }
            }
            magic::PC => {
                if let Some((_inner, events)) = archive.get_delta_events_with_inner::<PcEvent>(path) {
                    data.pc_events.extend(events);
                }
            }
            magic::RPN => {
                if let Some((_inner, events)) = archive.get_delta_events_with_inner::<RpnEvent>(path) {
                    for ev in &events {
                        data.rpn_events.push((h.extra, *ev));
                    }
                }
            }
            _ => {}
        }
    }

    let num_tracks_from_entries = track_data.len();
    let num_tracks_from_mapping = mapping
        .as_ref()
        .map(|m| {
            m.ports
                .iter()
                .flat_map(|p| p.channels.iter())
                .flat_map(|c| c.tracks.iter())
                .map(|t| t.track_index as usize + 1)
                .max()
                .unwrap_or(0)
        })
        .unwrap_or(0);
    let num_tracks = num_tracks_from_entries.max(num_tracks_from_mapping).max(1);
    while track_data.len() < num_tracks {
        track_data.push(None);
    }
    midi.track_ports = vec![0; num_tracks];
    midi.track_channel_prefixes = vec![None; num_tracks];
    midi.track_names = (0..num_tracks)
        .map(|i| format!("Track {}", i + 1))
        .collect();

    if let Some(mapping) = &mapping {
        for port_mapping in &mapping.ports {
            for ch_mapping in &port_mapping.channels {
                for track_mapping in &ch_mapping.tracks {
                    let idx = track_mapping.track_index as usize;
                    if idx < num_tracks {
                        midi.track_names[idx] = track_mapping.name.clone();
                        midi.track_ports[idx] = port_mapping.port;
                        midi.track_channel_prefixes[idx] = track_mapping.channel_prefix;
                    }
                }
            }
        }
    }

    // ── Rebuild key_notes and control_events ──
    // Read the channel from each archive entry by re-scanning the archive.
    // For each track, we collect the channel from its entries.
    for (track_idx, data) in track_data.iter().enumerate() {
        let Some(data) = data else { continue };

        // Determine the channel for this track from its archive entries.
        let global_ch = {
            let mut ch = None;
            for (path, entry) in &archive.entries {
                let path = path.as_str();
                if path.starts_with("conductor/") || path == "mapping.json" || path == "project.json" {
                    continue;
                }
                let Some(uuid_str) = extract_uuid(path) else { continue };
                let Some(&tid) = uuid_to_track.get(uuid_str) else { continue };
                if tid != track_idx {
                    continue;
                }
                // Read inner header to get channel.
                let inner = match entry.header.magic {
                    magic::TRACK_NOTES => {
                        archive.get_notes(path).map(|(i, _)| i)
                    }
                    magic::CC => {
                        archive.get_delta_events_with_inner::<CcEvent>(path).map(|(i, _)| i)
                    }
                    magic::PITCH_BEND => {
                        archive.get_delta_events_with_inner::<PitchBendEvent>(path).map(|(i, _)| i)
                    }
                    magic::PC => {
                        archive.get_delta_events_with_inner::<PcEvent>(path).map(|(i, _)| i)
                    }
                    magic::RPN => {
                        archive.get_delta_events_with_inner::<RpnEvent>(path).map(|(i, _)| i)
                    }
                    _ => None,
                };
                if let Some(inner) = inner {
                    ch = Some(inner.channel);
                    break;
                }
            }
            ch.unwrap_or_else(|| {
                let port = midi.track_ports.get(track_idx).copied().unwrap_or(0);
                port * 16
            })
        };

        let port = global_ch >> 4;
        midi.track_ports[track_idx] = port;

        for note in &data.notes {
            let key = note.key as usize;
            if key < 128 {
                midi.key_notes[key].push(yinhe_types::Note {
                    start_tick: note.start_tick,
                    end_tick: note.end_tick,
                    velocity: note.velocity,
                    channel: global_ch,
                    track: track_idx as u16,
                });
            }
        }

        for (controller, ev) in &data.cc_events {
            midi.control_events.push(yinhe_midi::MidiControlEvent::ControlChange {
                tick: ev.tick,
                channel: global_ch,
                controller: *controller,
                value: ev.value,
                track: track_idx as u16,
            });
        }

        for ev in &data.pitch_events {
            midi.control_events.push(yinhe_midi::MidiControlEvent::PitchBend {
                tick: ev.tick,
                channel: global_ch,
                value: ev.value,
                track: track_idx as u16,
            });
        }

        for ev in &data.pc_events {
            midi.control_events.push(yinhe_midi::MidiControlEvent::ProgramChange {
                tick: ev.tick,
                channel: global_ch,
                program: ev.program,
                track: track_idx as u16,
            });
            if ev.bank_msb != 0xFF {
                midi.control_events.push(yinhe_midi::MidiControlEvent::ControlChange {
                    tick: ev.tick,
                    channel: global_ch,
                    controller: 0,
                    value: ev.bank_msb,
                    track: track_idx as u16,
                });
            }
            if ev.bank_lsb != 0xFF {
                midi.control_events.push(yinhe_midi::MidiControlEvent::ControlChange {
                    tick: ev.tick,
                    channel: global_ch,
                    controller: 32,
                    value: ev.bank_lsb,
                    track: track_idx as u16,
                });
            }
        }

        for (rpn_num, ev) in &data.rpn_events {
            midi.control_events.push(yinhe_midi::MidiControlEvent::ControlChange {
                tick: ev.tick,
                channel: global_ch,
                controller: 101,
                value: *rpn_num,
                track: track_idx as u16,
            });
            midi.control_events.push(yinhe_midi::MidiControlEvent::ControlChange {
                tick: ev.tick,
                channel: global_ch,
                controller: 100,
                value: 0,
                track: track_idx as u16,
            });
            midi.control_events.push(yinhe_midi::MidiControlEvent::ControlChange {
                tick: ev.tick,
                channel: global_ch,
                controller: 6,
                value: ev.value.min(127) as u8,
                track: track_idx as u16,
            });
        }
    }

    for notes in &mut midi.key_notes {
        notes.sort_by_key(|n| n.start_tick);
    }

    midi.control_events.sort_by_key(|e| match e {
        yinhe_midi::MidiControlEvent::ControlChange { tick, .. } => *tick,
        yinhe_midi::MidiControlEvent::ProgramChange { tick, .. } => *tick,
        yinhe_midi::MidiControlEvent::PitchBend { tick, .. } => *tick,
    });

    // ── Recompute derived fields ──
    midi.note_count = midi.key_notes.iter().map(|n| n.len() as u64).sum();
    midi.tick_length = midi
        .key_notes
        .iter()
        .flat_map(|notes| notes.iter())
        .map(|n| n.end_tick as u64)
        .max()
        .unwrap_or(0);

    midi.scan_index = Some(yinhe_types::NoteScanIndex::build(
        &midi.key_notes,
        midi.tick_length,
    ));

    midi.automation_lanes = yinhe_midi::build_automation_lanes(&midi.control_events, &midi.key_notes);

    midi
}

/// Save a document as a .yin file.
pub fn save_project(
    doc: &crate::document::Document,
    path: &str,
    global_enabled: bool,
) -> std::io::Result<()> {
    let archive = build_archive(doc, global_enabled);
    archive.write_to(path)
}

/// Build a ProjectArchive from a Document (without writing to disk).
pub fn build_archive(
    doc: &crate::document::Document,
    global_enabled: bool,
) -> ProjectArchive {
    build_archive_from(
        &doc.midi,
        &doc.track_names,
        &doc.project_name,
        &doc.project_artist,
        doc.project_ppq,
        doc.archive.as_ref().map(|a| a.compression_level).unwrap_or(0),
        &doc.project_description,
        &doc.project_sf,
        global_enabled,
    )
}

/// Build a ProjectArchive from raw fields (usable from a background thread).
pub fn build_archive_from(
    midi: &yinhe_midi::MidiFile,
    track_names: &[String],
    project_name: &str,
    project_artist: &str,
    project_ppq: u32,
    compression_level: i32,
    project_description: &str,
    project_sf: &crate::right_panel::config::ProjectSfConfig,
    global_enabled: bool,
) -> ProjectArchive {
    let mut archive = midi_to_archive_with_names(midi, track_names);

    let soundfont_overrides: Vec<yinhe_project::SfPortOverride> = project_sf
        .overrides
        .iter()
        .map(|(port, entries)| yinhe_project::SfPortOverride {
            port: *port,
            entries: entries
                .iter()
                .map(|e| yinhe_project::SfEntryJson {
                    path: e.path.clone(),
                    name: e.name.clone(),
                    enabled: e.enabled,
                })
                .collect(),
        })
        .collect();

    let proj = ProjectJson {
        version: 1,
        name: project_name.to_string(),
        artist: project_artist.to_string(),
        ppq: project_ppq,
        zstd_level: compression_level,
        description: project_description.to_string(),
        soundfont_project_mode: !global_enabled,
        soundfont_overrides,
    };
    archive.set_events("project.json", FileHeader::new(*b"YHPR", 0, 0, 0), &[proj]);

    archive.compression_level = compression_level;
    archive
}

/// Load a .yin file and return a MidiFile + file stem name + the archive.
pub fn load_project_full(path: &str) -> std::io::Result<(yinhe_midi::MidiFile, String, ProjectArchive)> {
    let archive = ProjectArchive::read_from(path)?;
    let midi = archive_to_midi(&archive);

    let file_name = std::path::Path::new(path)
        .file_stem()
        .and_then(|n| n.to_str())
        .map(|s| s.to_string())
        .unwrap_or_default();

    Ok((midi, file_name, archive))
}

/// Load a .yin file and return a MidiFile + file stem name.
pub fn load_project(path: &str) -> std::io::Result<(yinhe_midi::MidiFile, String)> {
    let (midi, file_name, _) = load_project_full(path)?;
    Ok((midi, file_name))
}

/// Export the current document as a standard MIDI file.
pub fn export_midi(doc: &crate::document::Document, path: &str) -> Result<(), String> {
    let midi = &doc.midi;
    let num_tracks = midi.track_ports.len();
    let ppq = midi.ticks_per_beat;

    let mut tracks: Vec<Vec<TrackEvent>> = Vec::new();

    for track_idx in 0..num_tracks {
        let mut events: Vec<(u32, TrackEventKind)> = Vec::new();

        for (key_idx, key_notes) in midi.key_notes.iter().enumerate() {
            let key_u7 = u7::new(key_idx as u8);
            for note in key_notes {
                if note.track as usize != track_idx {
                    continue;
                }
                events.push((
                    note.start_tick,
                    TrackEventKind::Midi {
                        channel: u4::new(note.channel & 0x0F),
                        message: MidiMessage::NoteOn {
                            key: key_u7,
                            vel: u7::new(note.velocity),
                        },
                    },
                ));
                events.push((
                    note.end_tick,
                    TrackEventKind::Midi {
                        channel: u4::new(note.channel & 0x0F),
                        message: MidiMessage::NoteOff {
                            key: key_u7,
                            vel: u7::new(0),
                        },
                    },
                ));
            }
        }

        for ev in &midi.control_events {
            let (tick, track, channel, kind) = match ev {
                yinhe_midi::MidiControlEvent::ControlChange {
                    tick, channel, controller, value, track
                } => (*tick, *track, *channel, TrackEventKind::Midi {
                    channel: u4::new(*channel & 0x0F),
                    message: MidiMessage::Controller {
                        controller: u7::new(*controller),
                        value: u7::new(*value),
                    },
                }),
                yinhe_midi::MidiControlEvent::ProgramChange {
                    tick, channel, program, track
                } => (*tick, *track, *channel, TrackEventKind::Midi {
                    channel: u4::new(*channel & 0x0F),
                    message: MidiMessage::ProgramChange {
                        program: u7::new(*program),
                    },
                }),
                yinhe_midi::MidiControlEvent::PitchBend {
                    tick, channel, value, track
                } => (*tick, *track, *channel, TrackEventKind::Midi {
                    channel: u4::new(*channel & 0x0F),
                    message: MidiMessage::PitchBend {
                        bend: PitchBend::from_int(*value),
                    },
                }),
            };
            if track as usize == track_idx {
                events.push((tick, kind));
            }
        }

        events.sort_by_key(|e| e.0);

        let mut abs_tick = 0u32;
        let mut track_events = Vec::new();

        if let Some(name) = doc.track_names.get(track_idx) {
            track_events.push(TrackEvent {
                delta: 0.into(),
                kind: TrackEventKind::Meta(MetaMessage::TrackName(name.as_bytes())),
            });
        }

        for (tick, kind) in events {
            let delta = tick.saturating_sub(abs_tick);
            track_events.push(TrackEvent {
                delta: delta.into(),
                kind,
            });
            abs_tick = tick;
        }

        track_events.push(TrackEvent {
            delta: 0.into(),
            kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
        });

        tracks.push(track_events);
    }

    // Build conductor track
    let mut conductor_events: Vec<(u32, TrackEventKind)> = Vec::new();

    for seg in &midi.tempo_segments {
        conductor_events.push((
            seg.start_tick,
            TrackEventKind::Meta(MetaMessage::Tempo(midly::num::u24::new(
                seg.micros_per_quarter as u32,
            ))),
        ));
    }

    for ev in &midi.time_sig_events {
        conductor_events.push((
            ev.tick,
            TrackEventKind::Meta(MetaMessage::TimeSignature(
                ev.numerator,
                ev.denominator,
                24,
                8,
            )),
        ));
    }

    conductor_events.sort_by_key(|e| e.0);

    let mut abs_tick = 0u32;
    let mut conductor_track = Vec::new();
    for (tick, kind) in conductor_events {
        let delta = tick.saturating_sub(abs_tick);
        conductor_track.push(TrackEvent {
            delta: delta.into(),
            kind,
        });
        abs_tick = tick;
    }
    conductor_track.push(TrackEvent {
        delta: 0.into(),
        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
    });

    tracks.insert(0, conductor_track);

    let smf = Smf {
        header: Header {
            format: Format::SingleTrack,
            timing: Timing::Metrical(u15::new(ppq as u16)),
        },
        tracks: tracks.into_iter().map(|t| t.into_iter().collect()).collect(),
    };

    let mut buf = Vec::new();
    smf.write(&mut buf).map_err(|e| format!("{e}"))?;
    std::fs::write(path, &buf).map_err(|e| format!("{e}"))?;
    Ok(())
}

#[cfg(test)]
mod roundtrip_tests {
    use super::*;
    use yinhe_midi::MidiFile;
    use yinhe_midi::MidiControlEvent;
    use yinhe_types::{Note, TimeSigEvent as TypesTimeSigEvent};

    fn make_test_midi() -> MidiFile {
        let mut m = MidiFile::default();
        m.ticks_per_beat = 480;
        m.track_ports = vec![0, 0, 1];
        m.track_channel_prefixes = vec![None, None, None];
        m.track_names = vec!["Lead".into(), "Bass".into(), "Drums".into()];

        // Notes: track 0 channel 0, track 1 channel 1, track 2 channel 16 (port 1, raw 0)
        m.key_notes[60].push(Note { start_tick: 0, end_tick: 480, velocity: 100, channel: 0, track: 0 });
        m.key_notes[60].push(Note { start_tick: 480, end_tick: 960, velocity: 100, channel: 0, track: 0 });
        m.key_notes[48].push(Note { start_tick: 0, end_tick: 1920, velocity: 90, channel: 1, track: 1 });
        m.key_notes[36].push(Note { start_tick: 0, end_tick: 240, velocity: 120, channel: 16, track: 2 });

        // Control events
        m.control_events.push(MidiControlEvent::ControlChange { tick: 0, channel: 0, controller: 7, value: 100, track: 0 });
        m.control_events.push(MidiControlEvent::ControlChange { tick: 240, channel: 0, controller: 7, value: 80, track: 0 });
        m.control_events.push(MidiControlEvent::PitchBend { tick: 100, channel: 1, value: 1024, track: 1 });
        m.control_events.push(MidiControlEvent::ProgramChange { tick: 0, channel: 16, program: 7, track: 2 });

        // Tempo: 120 -> 140 at tick 1920
        m.tempo_segments = vec![
            yinhe_midi::TempoSegment { start_tick: 0, start_time: 0.0, micros_per_quarter: yinhe_midi::mpq_from_bpm(120.0) },
            yinhe_midi::TempoSegment { start_tick: 1920, start_time: 0.0, micros_per_quarter: yinhe_midi::mpq_from_bpm(140.0) },
        ];
        yinhe_midi::recompute_tempo_start_times(&mut m.tempo_segments, m.ticks_per_beat);

        // Time signature: 4/4 then 3/4 at bar 2
        m.time_sig_events = vec![
            TypesTimeSigEvent { tick: 0, numerator: 4, denominator: 2 },
            TypesTimeSigEvent { tick: 1920, numerator: 3, denominator: 2 },
        ];

        m.note_count = m.key_notes.iter().map(|n| n.len() as u64).sum();
        m.tick_length = 1920;
        m
    }

    #[test]
    fn roundtrip_preserves_notes_and_channels() {
        let original = make_test_midi();
        let archive = midi_to_archive(&original);
        let restored = archive_to_midi(&archive);

        assert_eq!(restored.ticks_per_beat, 480);
        assert_eq!(restored.track_ports.len(), 3);
        assert_eq!(restored.track_ports, vec![0, 0, 1]);

        // Notes preserved with correct channels
        assert_eq!(restored.key_notes[60].len(), 2, "track 0 notes at key 60");
        assert!(restored.key_notes[60].iter().all(|n| n.channel == 0 && n.track == 0));
        assert_eq!(restored.key_notes[48].len(), 1);
        assert_eq!(restored.key_notes[48][0].channel, 1);
        assert_eq!(restored.key_notes[48][0].track, 1);
        assert_eq!(restored.key_notes[36].len(), 1);
        assert_eq!(restored.key_notes[36][0].channel, 16);
        assert_eq!(restored.key_notes[36][0].track, 2);
    }

    #[test]
    fn roundtrip_preserves_control_events() {
        let original = make_test_midi();
        let archive = midi_to_archive(&original);
        let restored = archive_to_midi(&archive);

        let cc_count = restored.control_events.iter().filter(|e| matches!(e, MidiControlEvent::ControlChange { .. })).count();
        assert_eq!(cc_count, 2);
        let pb_count = restored.control_events.iter().filter(|e| matches!(e, MidiControlEvent::PitchBend { .. })).count();
        assert_eq!(pb_count, 1);
        let pc_count = restored.control_events.iter().filter(|e| matches!(e, MidiControlEvent::ProgramChange { .. })).count();
        assert_eq!(pc_count, 1);

        // Verify channel & track on PB
        let pb = restored.control_events.iter().find_map(|e| match e {
            MidiControlEvent::PitchBend { tick, channel, value, track } => Some((*tick, *channel, *value, *track)),
            _ => None,
        }).unwrap();
        assert_eq!(pb, (100, 1, 1024, 1));
    }

    #[test]
    fn roundtrip_preserves_tempo_and_time_sig() {
        let original = make_test_midi();
        let archive = midi_to_archive(&original);
        let restored = archive_to_midi(&archive);

        // Tempo: should have segment at 0 and 1920
        assert!(!restored.tempo_segments.is_empty());
        let bpm0 = yinhe_midi::bpm_from_mpq(restored.tempo_segments[0].micros_per_quarter);
        assert!((bpm0 - 120.0).abs() < 0.5, "expected ~120 BPM at tick 0, got {bpm0}");
        // Find the 140 BPM segment
        let has_140 = restored.tempo_segments.iter().any(|s| {
            s.start_tick == 1920 && (yinhe_midi::bpm_from_mpq(s.micros_per_quarter) - 140.0).abs() < 0.5
        });
        assert!(has_140, "expected 140 BPM segment at tick 1920, got {:?}", restored.tempo_segments);

        // Time sig
        assert_eq!(restored.time_sig_events.len(), 2);
        assert_eq!(restored.time_sig_events[0].numerator, 4);
        assert_eq!(restored.time_sig_events[1].numerator, 3);
        assert_eq!(restored.time_sig_events[1].tick, 1920);
    }

    #[test]
    fn roundtrip_no_tempo_yields_default_segment_at_zero() {
        // An archive with no tempo entries (e.g. brand-new project) should
        // still produce a tempo_segments[0] at tick 0 so that timing math
        // doesn't fall back to the global default path.
        let mut m = MidiFile::default();
        m.ticks_per_beat = 480;
        m.track_ports = vec![0];
        m.track_names = vec!["t".into()];
        m.tempo_segments.clear();
        m.time_sig_events.clear();

        let archive = midi_to_archive(&m);
        let restored = archive_to_midi(&archive);

        assert_eq!(restored.tempo_segments.len(), 1);
        assert_eq!(restored.tempo_segments[0].start_tick, 0);
        let bpm = yinhe_midi::bpm_from_mpq(restored.tempo_segments[0].micros_per_quarter);
        assert!((bpm - 120.0).abs() < 0.5);
    }

    #[test]
    fn roundtrip_preserves_track_names() {
        let original = make_test_midi();
        let archive = midi_to_archive(&original);
        let restored = archive_to_midi(&archive);

        assert_eq!(restored.track_names, vec!["Lead", "Bass", "Drums"]);
    }

    #[test]
    fn roundtrip_rpn_events() {
        let mut m = make_test_midi();
        // RPN 0 at tick 100: CC 6 before CC 101/100 (order shouldn't matter)
        m.control_events.push(yinhe_midi::MidiControlEvent::ControlChange {
            tick: 100, channel: 0, controller: 6, value: 2, track: 0,
        });
        m.control_events.push(yinhe_midi::MidiControlEvent::ControlChange {
            tick: 100, channel: 0, controller: 101, value: 0, track: 0,
        });
        m.control_events.push(yinhe_midi::MidiControlEvent::ControlChange {
            tick: 100, channel: 0, controller: 100, value: 0, track: 0,
        });
        // RPN 1 at tick 200: only CC 6, no CC 38
        m.control_events.push(yinhe_midi::MidiControlEvent::ControlChange {
            tick: 200, channel: 0, controller: 101, value: 1, track: 0,
        });
        m.control_events.push(yinhe_midi::MidiControlEvent::ControlChange {
            tick: 200, channel: 0, controller: 100, value: 0, track: 0,
        });
        m.control_events.push(yinhe_midi::MidiControlEvent::ControlChange {
            tick: 200, channel: 0, controller: 6, value: 50, track: 0,
        });
        // RPN 2 at tick 300: CC 6 + CC 38 (14-bit value)
        m.control_events.push(yinhe_midi::MidiControlEvent::ControlChange {
            tick: 300, channel: 0, controller: 101, value: 2, track: 0,
        });
        m.control_events.push(yinhe_midi::MidiControlEvent::ControlChange {
            tick: 300, channel: 0, controller: 100, value: 0, track: 0,
        });
        m.control_events.push(yinhe_midi::MidiControlEvent::ControlChange {
            tick: 300, channel: 0, controller: 6, value: 24, track: 0,
        });
        m.control_events.push(yinhe_midi::MidiControlEvent::ControlChange {
            tick: 300, channel: 0, controller: 38, value: 127, track: 0,
        });

        let archive = midi_to_archive(&m);
        let restored = archive_to_midi(&archive);

        let rpn_ccs: Vec<_> = restored
            .control_events
            .iter()
            .filter_map(|ev| match ev {
                yinhe_midi::MidiControlEvent::ControlChange {
                    controller: 101 | 100 | 6, ..
                } => Some(ev),
                _ => None,
            })
            .collect();
        // 3 RPN events × 3 CCs each = 9 CCs (CC 38 is consumed, not stored)
        assert_eq!(rpn_ccs.len(), 9, "expected 9 RPN-related CCs");
    }
}

/// Map (RPN MSB, RPN LSB) to a canonical RPN number for storage.
/// Returns `Some(rpn_num)` for known RPNs (0, 1, 2), `None` for unknown.
fn rpn_number(msb: u8, lsb: u8) -> Option<u8> {
    match (msb, lsb) {
        (0, 0) => Some(0),
        (1, 0) => Some(1),
        (2, 0) => Some(2),
        _ => None,
    }
}
