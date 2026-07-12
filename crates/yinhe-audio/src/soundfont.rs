use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, RwLock};

use xsynth_core::channel::{ChannelConfigEvent, ChannelEvent};
use xsynth_core::channel_group::{ChannelGroup, SynthEvent};
use xsynth_core::soundfont::{SampleSoundfont, SoundfontBase, SoundfontInitOptions};
use xsynth_core::{AudioStreamParams, ChannelCount};

static GLOBAL_SF_CACHE: LazyLock<RwLock<HashMap<PathBuf, Arc<dyn SoundfontBase>>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// Remove cache entries that are no longer referenced outside the cache.
fn sweep_unused() {
    let mut cache = GLOBAL_SF_CACHE.write().unwrap_or_else(|e| e.into_inner());
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
            let cache = GLOBAL_SF_CACHE.read().unwrap_or_else(|e| e.into_inner());
            if let Some(sf) = cache.get(path) {
                return Ok(Arc::clone(sf));
            }
        }

        let sf = yinhe_memtrace::with_tag(yinhe_memtrace::AllocTag::SoundFont, || {
            SampleSoundfont::new(path, self.stream_params, SoundfontInitOptions::default())
                .map_err(|e| format!("Failed to load SoundFont {:?}: {}", path, e))
        })?;

        let arc: Arc<dyn SoundfontBase> = Arc::new(sf);
        let mut cache = GLOBAL_SF_CACHE.write().unwrap_or_else(|e| e.into_inner());
        cache.insert(path.to_path_buf(), Arc::clone(&arc));
        Ok(arc)
    }

    pub fn load_for_port_with_dense(
        &mut self,
        port: u8,
        paths: &[String],
        cg: &mut ChannelGroup,
        dense_channels: &[u32],
    ) -> Result<(), String> {
        let soundfonts = self.load_paths(paths)?;
        self.apply_loaded_for_port_with_dense(port, soundfonts, cg, dense_channels);
        Ok(())
    }

    pub fn load_paths(&self, paths: &[String]) -> Result<Vec<Arc<dyn SoundfontBase>>, String> {
        let mut soundfonts = Vec::new();
        for p in paths {
            let path = Path::new(p);
            let sf = self.load_soundfont(path)?;
            soundfonts.push(sf);
        }
        Ok(soundfonts)
    }

    pub fn apply_loaded_for_port_with_dense(
        &mut self,
        port: u8,
        soundfonts: Vec<Arc<dyn SoundfontBase>>,
        cg: &mut ChannelGroup,
        dense_channels: &[u32],
    ) {
        self.port_sfs[port as usize] = soundfonts;
        sweep_unused();

        for &dense in dense_channels {
            let sfs = self.port_sfs[port as usize].clone();
            cg.send_event(SynthEvent::Channel(
                dense,
                ChannelEvent::Config(ChannelConfigEvent::SetSoundfonts(sfs)),
            ));
        }
    }
}
