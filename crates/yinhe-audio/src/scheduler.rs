use xsynth_core::channel::{ChannelAudioEvent, ChannelEvent, ControlEvent};
use xsynth_core::channel_group::{ChannelGroup, SynthEvent};
use yinhe_midi::MidiFile;
use yinhe_types::MidiControlEvent;

const CHASE_CHECKPOINT_INTERVAL: u64 = 1000;

#[derive(Clone, Debug)]
struct ScheduledEvent {
    sample: u64,
    tick: u64,
    channel: u32,
    event: ChannelAudioEvent,
}

#[derive(Clone, Default)]
struct ChannelState {
    bank_msb: u8,
    bank_lsb: u8,
    program: u8,
    volume: u8,
    pan: u8,
    expression: u8,
    sustain: u8,
    cutoff: u8,
    resonance: u8,
    attack: u8,
    release: u8,
    pitch_bend: f32,
    rpn_msb: u8,
    rpn_lsb: u8,
    data_entry_msb: u8,
    data_entry_lsb: u8,
}

struct ChaseCheckpoint {
    tick: u64,
    channels: [ChannelState; 256],
}

pub struct MidiEventScheduler {
    events: Vec<ScheduledEvent>,
    cursor: usize,
    checkpoints: Vec<ChaseCheckpoint>,
}

impl MidiEventScheduler {
    pub fn new() -> Self {
        Self {
            events: Vec::new(),
            cursor: 0,
            checkpoints: Vec::new(),
        }
    }

    pub fn build(&mut self, midi: &MidiFile, sample_rate: u32) {
        self.events.clear();
        self.checkpoints.clear();
        self.cursor = 0;

        let sr = sample_rate as f64;

        for key in 0..128u8 {
            for note in &midi.key_notes[key as usize] {
                if note.velocity <= 1 {
                    continue;
                }
                let start_sample = (note.start * sr) as u64;
                let end_sample = (note.end * sr) as u64;
                let ch = note.channel as u32;

                self.events.push(ScheduledEvent {
                    sample: start_sample,
                    tick: note.start_tick as u64,
                    channel: ch,
                    event: ChannelAudioEvent::NoteOn {
                        key: note.key,
                        vel: note.velocity,
                    },
                });

                if end_sample > start_sample {
                    self.events.push(ScheduledEvent {
                        sample: end_sample,
                        tick: note.end_tick as u64,
                        channel: ch,
                        event: ChannelAudioEvent::NoteOff { key: note.key },
                    });
                }
            }
        }

        for evt in &midi.control_events {
            match evt {
                MidiControlEvent::ControlChange {
                    tick,
                    channel,
                    controller,
                    value,
                    ..
                } => {
                    let sample = (midi.tick_to_seconds(*tick) * sr) as u64;
                    self.events.push(ScheduledEvent {
                        sample,
                        tick: *tick as u64,
                        channel: *channel as u32,
                        event: ChannelAudioEvent::Control(ControlEvent::Raw(
                            *controller, *value,
                        )),
                    });
                }
                MidiControlEvent::ProgramChange {
                    tick,
                    channel,
                    program,
                    ..
                } => {
                    let sample = (midi.tick_to_seconds(*tick) * sr) as u64;
                    self.events.push(ScheduledEvent {
                        sample,
                        tick: *tick as u64,
                        channel: *channel as u32,
                        event: ChannelAudioEvent::ProgramChange(*program),
                    });
                }
                MidiControlEvent::PitchBend {
                    tick,
                    channel,
                    value,
                    ..
                } => {
                    let sample = (midi.tick_to_seconds(*tick) * sr) as u64;
                    let normalized = *value as f32 / 8192.0;
                    self.events.push(ScheduledEvent {
                        sample,
                        tick: *tick as u64,
                        channel: *channel as u32,
                        event: ChannelAudioEvent::Control(ControlEvent::PitchBendValue(
                            normalized,
                        )),
                    });
                }
            }
        }

        self.events.sort_by(|a, b| {
            a.sample
                .cmp(&b.sample)
                .then_with(|| a.tick.cmp(&b.tick))
        });

        self.build_checkpoints();
    }

    fn build_checkpoints(&mut self) {
        if self.events.is_empty() {
            return;
        }

        let mut state = [(); 256].map(|_| ChannelState::default());
        for i in 0..16 {
            state[i * 16 + 9].pan = 64;
        }
        for s in state.iter_mut() {
            s.volume = 100;
            s.pan = 64;
            s.expression = 127;
        }

        let mut next_checkpoint_tick = 0u64;
        self.checkpoints.push(ChaseCheckpoint {
            tick: 0,
            channels: state.clone(),
        });

        for evt in &self.events {
            while evt.tick >= next_checkpoint_tick + CHASE_CHECKPOINT_INTERVAL {
                next_checkpoint_tick += CHASE_CHECKPOINT_INTERVAL;
                self.checkpoints.push(ChaseCheckpoint {
                    tick: next_checkpoint_tick,
                    channels: state.clone(),
                });
            }

            let ch = evt.channel as usize;
            if ch >= 256 {
                continue;
            }
            match &evt.event {
                ChannelAudioEvent::Control(ControlEvent::Raw(cc, val)) => match cc {
                    0 => state[ch].bank_msb = *val,
                    6 => state[ch].data_entry_msb = *val,
                    7 => state[ch].volume = *val,
                    10 => state[ch].pan = *val,
                    32 => state[ch].bank_lsb = *val,
                    38 => state[ch].data_entry_lsb = *val,
                    64 => state[ch].sustain = *val,
                    71 => state[ch].resonance = *val,
                    72 => state[ch].release = *val,
                    73 => state[ch].attack = *val,
                    74 => state[ch].cutoff = *val,
                    100 => state[ch].rpn_lsb = *val,
                    101 => state[ch].rpn_msb = *val,
                    11 => state[ch].expression = *val,
                    _ => {}
                },
                ChannelAudioEvent::Control(ControlEvent::PitchBendValue(v)) => {
                    state[ch].pitch_bend = *v;
                }
                ChannelAudioEvent::ProgramChange(p) => {
                    state[ch].program = *p;
                }
                _ => {}
            }
        }
    }

    pub fn push_events(&mut self, _start: u64, end: u64, cg: &mut ChannelGroup) {
        while self.cursor < self.events.len() && self.events[self.cursor].sample < end {
            let evt = &self.events[self.cursor];
            cg.send_event(SynthEvent::Channel(
                evt.channel,
                ChannelEvent::Audio(evt.event.clone()),
            ));
            self.cursor += 1;
        }
    }

    pub fn seek(&mut self, sample: u64) {
        self.cursor = 0;
        for (i, evt) in self.events.iter().enumerate() {
            if evt.sample >= sample {
                self.cursor = i;
                return;
            }
        }
        self.cursor = self.events.len();
    }

    pub fn inject_chase(&mut self, sample: u64, cg: &mut ChannelGroup) {
        if self.checkpoints.is_empty() {
            return;
        }
        let tick = self.estimate_tick_for_sample(sample);

        let cp_idx = match self.checkpoints.binary_search_by(|cp| cp.tick.cmp(&tick)) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        };

        let mut state = self.checkpoints[cp_idx].channels.clone();
        let cp_tick = self.checkpoints[cp_idx].tick;

        for evt in &self.events {
            if evt.tick <= cp_tick {
                continue;
            }
            if evt.tick > tick {
                break;
            }
            let ch = evt.channel as usize;
            if ch >= 256 {
                continue;
            }
            match &evt.event {
                ChannelAudioEvent::Control(ControlEvent::Raw(cc, val)) => match cc {
                    0 => state[ch].bank_msb = *val,
                    6 => state[ch].data_entry_msb = *val,
                    7 => state[ch].volume = *val,
                    10 => state[ch].pan = *val,
                    32 => state[ch].bank_lsb = *val,
                    38 => state[ch].data_entry_lsb = *val,
                    64 => state[ch].sustain = *val,
                    71 => state[ch].resonance = *val,
                    72 => state[ch].release = *val,
                    73 => state[ch].attack = *val,
                    74 => state[ch].cutoff = *val,
                    100 => state[ch].rpn_lsb = *val,
                    101 => state[ch].rpn_msb = *val,
                    11 => state[ch].expression = *val,
                    _ => {}
                },
                ChannelAudioEvent::Control(ControlEvent::PitchBendValue(v)) => {
                    state[ch].pitch_bend = *v;
                }
                ChannelAudioEvent::ProgramChange(p) => {
                    state[ch].program = *p;
                }
                _ => {}
            }
        }

        self.send_chase_state(&state, cg);
    }

    fn send_chase_state(&self, state: &[ChannelState; 256], cg: &mut ChannelGroup) {
        for ch in 0..256u32 {
            let s = &state[ch as usize];
            let mut send = |event: ChannelAudioEvent| {
                cg.send_event(SynthEvent::Channel(ch, ChannelEvent::Audio(event)));
            };

            send(ChannelAudioEvent::Control(ControlEvent::Raw(101, s.rpn_msb)));
            send(ChannelAudioEvent::Control(ControlEvent::Raw(100, s.rpn_lsb)));
            send(ChannelAudioEvent::Control(ControlEvent::Raw(6, s.data_entry_msb)));
            send(ChannelAudioEvent::Control(ControlEvent::Raw(38, s.data_entry_lsb)));
            send(ChannelAudioEvent::Control(ControlEvent::Raw(0, s.bank_msb)));
            send(ChannelAudioEvent::Control(ControlEvent::Raw(32, s.bank_lsb)));
            send(ChannelAudioEvent::Control(ControlEvent::Raw(7, s.volume)));
            send(ChannelAudioEvent::Control(ControlEvent::Raw(10, s.pan)));
            send(ChannelAudioEvent::Control(ControlEvent::Raw(11, s.expression)));
            send(ChannelAudioEvent::Control(ControlEvent::Raw(64, s.sustain)));
            send(ChannelAudioEvent::Control(ControlEvent::Raw(73, s.attack)));
            send(ChannelAudioEvent::Control(ControlEvent::Raw(72, s.release)));
            send(ChannelAudioEvent::Control(ControlEvent::Raw(74, s.cutoff)));
            send(ChannelAudioEvent::Control(ControlEvent::Raw(71, s.resonance)));
            send(ChannelAudioEvent::ProgramChange(s.program));
            send(ChannelAudioEvent::Control(ControlEvent::PitchBendValue(
                s.pitch_bend,
            )));
        }
    }

    fn estimate_tick_for_sample(&self, _sample: u64) -> u64 {
        if self.events.is_empty() {
            return 0;
        }
        let idx = self.cursor.min(self.events.len() - 1);
        self.events[idx].tick
    }

    pub fn reset(&mut self) {
        self.cursor = 0;
    }
}
