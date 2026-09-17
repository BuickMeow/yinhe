use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, RwLock};

use xsynth_core::channel::{ChannelConfigEvent, ChannelEvent};
use xsynth_core::channel_group::SynthEvent;

use crate::channel_set::ChannelSet;
use xsynth_core::soundfont::{SampleSoundfont, SoundfontBase, SoundfontInitOptions};
use xsynth_core::{AudioStreamParams, ChannelCount};
use yinhe_types::Interpolation;

/// 音色库缓存条目 key：(路径, 采样率, 插值方式)。
///
/// xsynth 在加载时按 `AudioStreamParams.sample_rate` 重采样样本（loop 点/包络
/// 时间也按采样率换算），同一音色库以不同采样率加载得到的是不同的内部数据。
/// 若缓存只按路径区分，切换采样率后引擎/导出会命中旧采样率的缓存版本，
/// 播放音高错误（跑调），必须重启才能恢复。
///
/// 插值方式（`SoundfontInitOptions.interpolator`）同样是**加载时**选项
/// （voice spawner 类型在加载时确定），必须进 key，否则切换设置后命中旧
/// 插值的缓存。旧插值条目由 `sweep_unused` 在引用释放后清理。
type SfCacheKey = (PathBuf, u32, Interpolation);
static GLOBAL_SF_CACHE: LazyLock<RwLock<HashMap<SfCacheKey, Arc<dyn SoundfontBase>>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// Remove cache entries that are no longer referenced outside the cache.
fn sweep_unused() {
    let mut cache = GLOBAL_SF_CACHE.write().unwrap_or_else(|e| e.into_inner());
    cache.retain(|_, sf| Arc::strong_count(sf) > 1);
}

pub struct SoundFontManager {
    /// 每源通道（0..256）的音色库列表（按源通道索引；未配置为空）。
    channel_sfs: Box<[Vec<Arc<dyn SoundfontBase>>; 256]>,
    stream_params: AudioStreamParams,
    /// 采样插值方式（加载时写入 `SoundfontInitOptions`）。
    interp: Interpolation,
}

impl SoundFontManager {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            channel_sfs: Box::new(std::array::from_fn(|_| Vec::new())),
            stream_params: AudioStreamParams {
                sample_rate,
                channels: ChannelCount::Stereo,
            },
            interp: Interpolation::default(),
        }
    }

    /// 设置采样插值方式（须在加载音色库之前；已在缓存中的旧插值条目
    /// 不会自动失效，换插值后重新加载即得到新条目）。
    pub fn set_interpolation(&mut self, interp: Interpolation) {
        self.interp = interp;
    }

    pub fn load_soundfont(&self, path: &Path) -> Result<Arc<dyn SoundfontBase>, String> {
        let key = (
            path.to_path_buf(),
            self.stream_params.sample_rate,
            self.interp,
        );
        {
            let cache = GLOBAL_SF_CACHE.read().unwrap_or_else(|e| e.into_inner());
            if let Some(sf) = cache.get(&key) {
                return Ok(Arc::clone(sf));
            }
        }

        let options = SoundfontInitOptions {
            interpolator: match self.interp {
                Interpolation::Nearest => xsynth_core::soundfont::Interpolator::Nearest,
                Interpolation::Linear => xsynth_core::soundfont::Interpolator::Linear,
            },
            ..SoundfontInitOptions::default()
        };
        let sf = yinhe_memtrace::with_tag(yinhe_memtrace::AllocTag::SoundFont, || {
            SampleSoundfont::new(path, self.stream_params, options)
                .map_err(|e| format!("Failed to load SoundFont {:?}: {}", path, e))
        })?;

        let arc: Arc<dyn SoundfontBase> = Arc::new(sf);
        let mut cache = GLOBAL_SF_CACHE.write().unwrap_or_else(|e| e.into_inner());
        cache.insert(key, Arc::clone(&arc));
        Ok(arc)
    }

    pub(crate) fn load_for_channel(
        &mut self,
        channel: u8,
        paths: &[String],
        cg: &mut ChannelSet,
        dense: u32,
    ) -> Result<(), String> {
        let soundfonts = self.load_paths(paths)?;
        self.apply_loaded_for_channel(channel, soundfonts, cg, dense);
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

    pub(crate) fn apply_loaded_for_channel(
        &mut self,
        channel: u8,
        soundfonts: Vec<Arc<dyn SoundfontBase>>,
        cg: &mut ChannelSet,
        dense: u32,
    ) {
        self.channel_sfs[channel as usize] = soundfonts;
        sweep_unused();

        let sfs = self.channel_sfs[channel as usize].clone();
        cg.send_event(SynthEvent::Channel(
            dense,
            ChannelEvent::Config(ChannelConfigEvent::SetSoundfonts(sfs)),
        ));
    }
}
