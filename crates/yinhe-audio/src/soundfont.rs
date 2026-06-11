use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, RwLock};

use xsynth_core::channel::{ChannelConfigEvent, ChannelEvent};
use xsynth_core::channel_group::{ChannelGroup, SynthEvent};
use xsynth_core::soundfont::{SampleSoundfont, SoundfontBase, SoundfontInitOptions};
use xsynth_core::{AudioStreamParams, ChannelCount};
use yinhe_midi::MidiFile;

static GLOBAL_SF_CACHE: LazyLock<RwLock<HashMap<PathBuf, Arc<dyn SoundfontBase>>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// Remove cache entries that are no longer referenced outside the cache.
fn sweep_unused() {
    let mut cache = GLOBAL_SF_CACHE.write().unwrap();
    cache.retain(|_, sf| Arc::strong_count(sf) > 1);
}

pub struct SoundFontManager {
    port_sfs: [Vec<Arc<dyn SoundfontBase>>; 16],
    stream_params: AudioStreamParams,
}

impl SoundFontManager {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            port_sfs: std::array::from_fn(|_| Vec::new()),
            stream_params: AudioStreamParams {
                sample_rate,
                channels: ChannelCount::Stereo,
            },
        }
    }

    pub fn load_soundfont(&self, path: &Path) -> Result<Arc<dyn SoundfontBase>, String> {
        {
            let cache = GLOBAL_SF_CACHE.read().unwrap();
            if let Some(sf) = cache.get(path) {
                return Ok(Arc::clone(sf));
            }
        }

        let sf = yinhe_memtrace::with_tag(yinhe_memtrace::AllocTag::SoundFont, || {
            SampleSoundfont::new(path, self.stream_params, SoundfontInitOptions::default())
                .map_err(|e| format!("Failed to load SoundFont {:?}: {}", path, e))
        })?;

        let arc: Arc<dyn SoundfontBase> = Arc::new(sf);
        let mut cache = GLOBAL_SF_CACHE.write().unwrap();
        cache.insert(path.to_path_buf(), Arc::clone(&arc));
        Ok(arc)
    }

    pub fn load_for_port(
        &mut self,
        port: u8,
        paths: &[String],
        cg: &mut ChannelGroup,
        active_mask: &[bool],
    ) -> Result<(), String> {
        let mut soundfonts = Vec::new();
        for p in paths {
            let path = Path::new(p);
            let sf = self.load_soundfont(path)?;
            soundfonts.push(sf);
        }

        // Move into port_sfs (no clone) — clone from stored reference for channels
        self.port_sfs[port as usize] = soundfonts;
        // Clean up cache entries no longer referenced by any port
        sweep_unused();

        let base_ch = (port as u32) * 16;
        for ch in base_ch..base_ch + 16 {
            if active_mask.get(ch as usize).copied().unwrap_or(false) {
                let sfs = self.port_sfs[port as usize].clone();
                cg.send_event(SynthEvent::Channel(
                    ch,
                    ChannelEvent::Config(ChannelConfigEvent::SetSoundfonts(sfs)),
                ));
            }
        }

        Ok(())
    }

    /// Load soundfonts and bind them to an explicit set of dense (XSynth)
    /// channel indices.
    ///
    /// The caller still passes the source-side `port` so we can keep
    /// `port_sfs` indexed by port (preserves `load_for_midi`'s port-based
    /// re-send behaviour). `dense_channels` should be the compacted XSynth
    /// channel indices corresponding to the alive source channels of this
    /// port.
    pub fn load_for_port_with_dense(
        &mut self,
        port: u8,
        paths: &[String],
        cg: &mut ChannelGroup,
        dense_channels: &[u32],
    ) -> Result<(), String> {
        let mut soundfonts = Vec::new();
        for p in paths {
            let path = Path::new(p);
            let sf = self.load_soundfont(path)?;
            soundfonts.push(sf);
        }

        self.port_sfs[port as usize] = soundfonts;
        sweep_unused();

        for &dense in dense_channels {
            let sfs = self.port_sfs[port as usize].clone();
            cg.send_event(SynthEvent::Channel(
                dense,
                ChannelEvent::Config(ChannelConfigEvent::SetSoundfonts(sfs)),
            ));
        }

        Ok(())
    }

    pub fn load_for_midi(
        &mut self,
        midi: &MidiFile,
        cg: &mut ChannelGroup,
        channel_map: &[u32; 256],
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
                let base_src = (port as u32 * 16) as usize;
                for src_ch in base_src..base_src + 16 {
                    let dense = channel_map[src_ch];
                    if dense != u32::MAX {
                        cg.send_event(SynthEvent::Channel(
                            dense,
                            ChannelEvent::Config(ChannelConfigEvent::SetSoundfonts(sfs.clone())),
                        ));
                    }
                }
            }
        }

        Ok(())
    }

    pub fn port_soundfonts(&self, port: u8) -> &[Arc<dyn SoundfontBase>] {
        &self.port_sfs[port as usize]
    }
}
