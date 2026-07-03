use std::sync::Arc;

use yinhe_core::{ConductorData, NoteEvent, PcEvent, ProjectMeta, TempoEvent, TrackData, YinModel};
use yinhe_types::{AutomationEvent, AutomationLane, AutomationTarget, Note, NoteSource, TimeSigEvent};
use yinhe_editor_core::document::Document;
use yinhe_editor_core::quantize::QuantizePreset;

/// Mock MIDI data for testing.
pub struct MockMidi {
    pub notes: [Vec<Note>; 128],
    pub tpb: u32,
    pub tick_len: u64,
}

impl NoteSource for MockMidi {
    fn key_notes(&self, key: u8) -> &[Note] {
        &self.notes[key as usize]
    }
    fn duration(&self) -> f64 {
        10.0
    }
    fn ticks_per_beat(&self) -> Option<u32> {
        Some(self.tpb)
    }
    fn tick_length(&self) -> Option<u64> {
        Some(self.tick_len)
    }
}

pub fn make_midi(notes: Vec<(u8, u32, u32, u16, u8)>) -> MockMidi {
    let mut key_notes: [Vec<Note>; 128] = core::array::from_fn(|_| Vec::new());
    let mut max_tick: u64 = 0;
    for (key, start_tick, end_tick, track, vel) in notes {
        let n = Note {
            start_tick,
            end_tick,
            velocity: vel,
            dup_index: 0,
            track,
        };
        if (end_tick as u64) > max_tick {
            max_tick = end_tick as u64;
        }
        key_notes[key as usize].push(n);
    }
    MockMidi {
        notes: key_notes,
        tpb: 480,
        tick_len: max_tick,
    }
}

/// Create a multi-track test YinModel (programmatically, no raw bytes).
pub fn make_test_model() -> YinModel {
    let conductor = ConductorData {
        tempo: vec![
            TempoEvent { tick: 0, bpm: 120.0 },
            TempoEvent { tick: 1920, bpm: 140.0 },
        ],
        time_sig: vec![
            TimeSigEvent { tick: 0, numerator: 4, denominator: 2 },
            TimeSigEvent { tick: 1920, numerator: 3, denominator: 2 },
        ],
    };

    let mut t0 = TrackData::new(0, 0); // port 0, ch 0
    t0.name = "Lead".into();
    t0.automation_lanes = vec![AutomationLane {
        target: AutomationTarget::CC { controller: 7 },
        track: 0,
        events: vec![
            AutomationEvent { tick: 0, value: 100 },
            AutomationEvent { tick: 240, value: 80 },
        ],
    }];

    let mut t1 = TrackData::new(0, 1); // port 0, ch 1
    t1.name = "Bass".into();
    t1.automation_lanes = vec![AutomationLane {
        target: AutomationTarget::PitchBend,
        track: 1,
        events: vec![AutomationEvent { tick: 100, value: 1024 }],
    }];

    let mut t2 = TrackData::new(1, 0); // port 1, ch 0
    t2.name = "Drums".into();
    t2.program_change = vec![PcEvent { tick: 0, program: 7, bank_msb: 0xFF, bank_lsb: 0xFF }];

    let per_track_notes: Vec<Vec<NoteEvent>> = vec![
        vec![
            NoteEvent { start_tick: 0, end_tick: 480, key: 60, velocity: 100, dup_index: 0 },
            NoteEvent { start_tick: 480, end_tick: 960, key: 60, velocity: 100, dup_index: 0 },
        ],
        vec![
            NoteEvent { start_tick: 0, end_tick: 1920, key: 48, velocity: 90, dup_index: 0 },
        ],
        vec![
            NoteEvent { start_tick: 0, end_tick: 240, key: 36, velocity: 120, dup_index: 0 },
        ],
    ];

    let meta = ProjectMeta { ppq: 480, ..ProjectMeta::default() };
    let mut model = YinModel {
        conductor: Arc::new(conductor),
        tracks: vec![Arc::new(t0), Arc::new(t1), Arc::new(t2)],
        meta,
        ..Default::default()
    };
    model.load_track_notes(per_track_notes);
    model.rebuild();
    model
}

/// Create a Document from a test model.
pub fn make_test_document() -> Document {
    let model = make_test_model();
    Document::from_model("test.mid", model, QuantizePreset::default(), Default::default(), Default::default()).expect("from_model failed")
}

/// Create a multi-track YinModel with notes on many keys for stress testing.
pub fn make_stress_model(track_count: u16, notes_per_track: u32) -> YinModel {
    let conductor = ConductorData::default();

    let tracks: Vec<Arc<TrackData>> = (0..track_count)
        .map(|t| {
            Arc::new(TrackData::new(0, t as u8))
        })
        .collect();

    let per_track_notes: Vec<Vec<NoteEvent>> = (0..track_count)
        .map(|t| {
            let mut notes = Vec::with_capacity(notes_per_track as usize);
            for n in 0..notes_per_track {
                let key = (n % 128) as u8;
                let start = n * 120;
                notes.push(NoteEvent {
                    start_tick: start,
                    end_tick: start + 100,
                    key,
                    velocity: 80,
                    dup_index: 0,
                });
            }
            notes
        })
        .collect();

    let meta = ProjectMeta { ppq: 480, ..ProjectMeta::default() };
    let mut model = YinModel {
        conductor: Arc::new(conductor),
        tracks,
        meta,
        ..Default::default()
    };
    model.load_track_notes(per_track_notes);
    model.rebuild();
    model
}
