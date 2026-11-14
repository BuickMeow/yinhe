//! 主输出表分析层：响度、频谱、示波器、立体声、VU、波形。
//!
//! 纯数据处理，不依赖 egui / 音频后端；由 UI 线程按帧喂入 Master 样本
//! （交错立体声，L,R,L,R...）。所有分析器的状态都是有界的可视化/测量状态，
//! 不是业务缓存。

pub mod loudness;
pub mod scope;
pub mod spectrogram;
pub mod spectrum;
pub mod stereo;
pub mod vu;
pub mod waveform;

pub use loudness::{LoudnessAnalyzer, LoudnessReading};
pub use scope::ScopeBuffer;
pub use spectrogram::SpectrogramAnalyzer;
pub use spectrum::SpectrumAnalyzer;
pub use stereo::{StereoAnalyzer, StereoReading};
pub use vu::VuAnalyzer;
pub use waveform::{WaveColumn, WaveformHistory};
