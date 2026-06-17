use std::collections::BTreeMap;

use uuid::Uuid;

use yinhe_project::{
    FileHeader, InnerHeader, ProjectArchive, ProjectJson, SfPortOverride,
    cc_path, conductor_path, pc_path, pitch_path, rpn_path, track_notes_path,
};

use crate::model::YinModel;

/// Convert a `YinModel` into a `ProjectArchive` for saving to .yin files.
pub fn yinmodel_to_archive(model: &YinModel) -> ProjectArchive {
    let mut archive = ProjectArchive::new();
    archive.compression_level = model.meta.compression_level;

    // ── Conductor events ──
    let tempo_events: Vec<yinhe_project::TempoEvent> = model
        .conductor
        .tempo
        .iter()
        .map(|e| yinhe_project::TempoEvent {
            tick: e.tick,
            bpm: e.bpm as f32,
        })
        .collect();
    if !tempo_events.is_empty() {
        archive.set_delta_events(
            conductor_path("tempo.zst"),
            FileHeader::new(*b"YHTM", 0, 0, 0),
            &tempo_events,
        );
    }

    let ts_events: Vec<yinhe_project::TimeSigEvent> = model
        .conductor
        .time_sig
        .iter()
        .map(|e| yinhe_project::TimeSigEvent {
            tick: e.tick,
            numerator: e.numerator,
            denominator_power: e.denominator,
        })
        .collect();
    if !ts_events.is_empty() {
        archive.set_delta_events(
            conductor_path("time_sig.zst"),
            FileHeader::new(*b"YHTS", 0, 0, 0),
            &ts_events,
        );
    }

    // ── Track entries ──
    let mut port_map: std::collections::HashMap<u8, Vec<(u8, Vec<yinhe_project::TrackMapping>)>> =
        std::collections::HashMap::new();

    for (track_idx, track) in model.tracks.iter().enumerate() {
        let uuid = if track.uuid.is_empty() {
            Uuid::new_v4().to_string()
        } else {
            track.uuid.clone()
        };
        let ch = track.channel;
        let port = track.port;
        let raw_channel = ch & 0x0F;
        let inner = InnerHeader::new(track_idx as u16, ch);

        // Notes
        if !track.notes.is_empty() {
            let notes: Vec<yinhe_project::Note> = track
                .notes
                .iter()
                .map(|n| yinhe_project::Note {
                    start_tick: n.tick,
                    end_tick: n.tick + n.duration,
                    key: n.key,
                    velocity: n.velocity,
                })
                .collect();
            archive.set_notes(
                track_notes_path(ch, &uuid),
                FileHeader::new(*b"YHTK", port, raw_channel, track_idx as u8),
                inner,
                &notes,
            );
        }

        // CC events
        for (&cc_num, events) in &track.cc {
            let cc_events: Vec<yinhe_project::CcEvent> = events
                .iter()
                .map(|e| yinhe_project::CcEvent {
                    tick: e.tick,
                    value: e.value,
                })
                .collect();
            archive.set_delta_events_with_inner(
                cc_path(ch, &uuid, cc_num),
                FileHeader::new(*b"YHCC", port, raw_channel, cc_num),
                inner,
                &cc_events,
            );
        }

        // RPN events
        for (&rpn_num, events) in &track.rpn {
            let rpn_events: Vec<yinhe_project::RpnEvent> = events
                .iter()
                .map(|e| yinhe_project::RpnEvent {
                    tick: e.tick,
                    value: e.value,
                })
                .collect();
            archive.set_delta_events_with_inner(
                rpn_path(ch, &uuid, rpn_num),
                FileHeader::new(*b"YHRP", port, raw_channel, rpn_num),
                inner,
                &rpn_events,
            );
        }

        // Pitch bend
        if !track.pitch_bend.is_empty() {
            let pb_events: Vec<yinhe_project::PitchBendEvent> = track
                .pitch_bend
                .iter()
                .map(|e| yinhe_project::PitchBendEvent {
                    tick: e.tick,
                    value: e.value,
                })
                .collect();
            archive.set_delta_events_with_inner(
                pitch_path(ch, &uuid),
                FileHeader::new(*b"YHPB", port, raw_channel, 0),
                inner,
                &pb_events,
            );
        }

        // Program change
        if !track.program_change.is_empty() {
            let pc_events: Vec<yinhe_project::PcEvent> = track
                .program_change
                .iter()
                .map(|e| yinhe_project::PcEvent {
                    tick: e.tick,
                    program: e.program,
                    bank_msb: e.bank_msb,
                    bank_lsb: e.bank_lsb,
                })
                .collect();
            archive.set_delta_events_with_inner(
                pc_path(ch, &uuid),
                FileHeader::new(*b"YHPC", port, raw_channel, 0),
                inner,
                &pc_events,
            );
        }

        // Build port map
        let channels_entry = port_map.entry(port).or_default();
        let ch_entry = if let Some(existing) = channels_entry
            .iter_mut()
            .find(|(c, _)| *c == raw_channel)
        {
            existing
        } else {
            channels_entry.push((raw_channel, Vec::new()));
            channels_entry.last_mut().unwrap()
        };
        ch_entry.1.push(yinhe_project::TrackMapping {
            uuid: uuid.clone(),
            name: track.name.clone(),
            color: [0.5, 0.5, 0.5],
            track_index: track_idx as u16,
            channel_prefix: None,
        });
    }

    // ── mapping.json ──
    let mapping = yinhe_project::MappingJson {
        ports: port_map
            .into_iter()
            .map(|(port, channels)| yinhe_project::PortMapping {
                port,
                channels: channels
                    .into_iter()
                    .map(|(channel, tracks)| yinhe_project::ChannelMapping { channel, tracks })
                    .collect(),
            })
            .collect(),
    };
    archive.set_json(
        "mapping.json",
        FileHeader::new(*b"YHMP", 0, 0, 0),
        &mapping,
    );

    // ── project.json ──
    let song_title = model
        .tracks
        .first()
        .map(|t| t.name.clone())
        .filter(|n| !n.is_empty() && !n.starts_with("Track "))
        .unwrap_or_default();
    let proj = ProjectJson {
        version: 1,
        name: if model.meta.name.is_empty() {
            song_title
        } else {
            model.meta.name.clone()
        },
        artist: model.meta.artist.clone(),
        ppq: model.meta.ppq,
        zstd_level: model.meta.compression_level,
        description: model.meta.description.clone(),
        soundfont_project_mode: false,
        soundfont_overrides: Vec::new(),
    };
    archive.set_json(
        "project.json",
        FileHeader::new(*b"YHPR", 0, 0, 0),
        &proj,
    );

    archive
}
