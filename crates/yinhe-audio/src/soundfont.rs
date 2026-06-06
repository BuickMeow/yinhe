use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use xsynth_core::channel::{ChannelConfigEvent, ChannelEvent};
use xsynth_core::channel_group::{ChannelGroup, SynthEvent};
use xsynth_core::soundfont::{SampleSoundfont, SoundfontBase, SoundfontInitOptions};
use xsynth_core::{AudioStreamParams, ChannelCount};
use yinhe_midi::MidiFile;

pub struct SoundFontManager {
    cache: HashMap<PathBuf, Arc<dyn SoundfontBase>>,
    port_sfs: [Vec<Arc<dyn SoundfontBase>>; 16],
    stream_params: AudioStreamParams,
}

impl SoundFontManager {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            cache: HashMap::new(),
            port_sfs: std::array::from_fn(|_| Vec::new()),
            stream_params: AudioStreamParams {
                sample_rate,
                channels: ChannelCount::Stereo,
            },
        }
    }

    pub fn load_soundfont(&mut self, path: &Path) -> Result<Arc<dyn SoundfontBase>, String> {
        if let Some(sf) = self.cache.get(path) {
            return Ok(Arc::clone(sf));
        }

        let sf = SampleSoundfont::new(
            path,
            self.stream_params,
            SoundfontInitOptions::default(),
        )
        .map_err(|e| format!("Failed to load SoundFont {:?}: {}", path, e))?;

        let arc: Arc<dyn SoundfontBase> = Arc::new(sf);
        self.cache.insert(path.to_path_buf(), Arc::clone(&arc));
        Ok(arc)
    }

    pub fn load_for_port(
        &mut self,
        port: u8,
        paths: &[String],
        cg: &mut ChannelGroup,
    ) -> Result<(), String> {
        let mut soundfonts = Vec::new();
        for p in paths {
            let path = Path::new(p);
            let sf = self.load_soundfont(path)?;
            soundfonts.push(sf);
        }

        self.port_sfs[port as usize] = soundfonts.clone();

        let base_ch = (port as u32) * 16;
        for ch in base_ch..base_ch + 16 {
            cg.send_event(SynthEvent::Channel(
                ch,
                ChannelEvent::Config(ChannelConfigEvent::SetSoundfonts(soundfonts.clone())),
            ));
        }

        Ok(())
    }

    pub fn load_for_midi(
        &mut self,
        midi: &MidiFile,
        cg: &mut ChannelGroup,
    ) -> Result<(), String> {
        let mut used_ports = [false; 16];
        for &port in &midi.track_ports {
            if (port as usize) < 16 {
                used_ports[port as usize] = true;
            }
        }

        for (port, &used) in used_ports.iter().enumerate() {
            if used && !self.port_sfs[port].is_empty() {
                let sfs = self.port_sfs[port].clone();
                let base_ch = (port as u32) * 16;
                for ch in base_ch..base_ch + 16 {
                    cg.send_event(SynthEvent::Channel(
                        ch,
                        ChannelEvent::Config(ChannelConfigEvent::SetSoundfonts(sfs.clone())),
                    ));
                }
            }
        }

        Ok(())
    }

    pub fn port_soundfonts(&self, port: u8) -> &[Arc<dyn SoundfontBase>] {
        &self.port_sfs[port as usize]
    }
}
