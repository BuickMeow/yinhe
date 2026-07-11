//! SFZ key-map builder using xsynth-soundfonts' standard parser.
//!
//! Delegates SFZ parsing to `xsynth_soundfonts::sfz::parse_soundfont` which
//! correctly handles `<group>`, `<master>`, `#include`, inheritance, etc.

use std::path::{Path, PathBuf};

/// Build a lookup table: MIDI key → (sample_path, pitch_keycenter, release_time).
/// Uses xsynth-soundfonts' SFZ parser for correct SFZ semantics.
pub fn build_key_map_from_sfz(sfz_path: &Path) -> Result<Vec<(PathBuf, u8, f32)>, String> {
    let regions = xsynth_soundfonts::sfz::parse_soundfont(sfz_path)
        .map_err(|e| format!("SFZ parse error: {}", e))?;

    let mut key_map: Vec<Option<(PathBuf, u8, f32, u8)>> = vec![None; 128];

    for region in &regions {
        let path = sfz_path
            .parent()
            .unwrap_or(Path::new("."))
            .join(&region.sample_path);

        // Resolve the actual file path (handle relative paths, backslashes, etc.)
        let resolved = std::fs::canonicalize(&path).unwrap_or(path);

        let pkc = region.pitch_keycenter as u8;
        let release = region.ampeg_envelope.ampeg_release;

        for key in region.keyrange.clone() {
            let k = key as usize;
            if k < 128 {
                let range_width = (*region.keyrange.end() as i16 - *region.keyrange.start() as i16).unsigned_abs() as u8;
                let replace = match &key_map[k] {
                    None => true,
                    Some((_, _, _, prev_width)) => range_width < *prev_width,
                };
                if replace {
                    key_map[k] = Some((resolved.clone(), pkc, release, range_width));
                }
            }
        }
    }

    Ok(key_map
        .into_iter()
        .map(|opt| opt.unwrap_or_else(|| (PathBuf::from("missing"), 60u8, 0.0, 0u8)))
        .map(|(p, pkc, rel, _)| (p, pkc, rel))
        .collect())
}

/// Load a WAV file as f32 samples (mono, normalized to -1..1).
pub fn load_wav_as_f32(path: &Path) -> Result<Vec<f32>, String> {
    let mut reader = hound::WavReader::open(path)
        .map_err(|e| format!("Failed to open WAV {:?}: {}", path, e))?;

    let spec = reader.spec();
    let samples: Vec<f32> = match spec.bits_per_sample {
        16 => reader
            .samples::<i16>()
            .map(|s| s.unwrap() as f32 / i16::MAX as f32)
            .collect(),
        24 => reader
            .samples::<i32>()
            .map(|s| {
                let v = s.unwrap();
                (v >> 8) as f32 / (i16::MAX as f32)
            })
            .collect(),
        32 => reader
            .samples::<i32>()
            .map(|s| s.unwrap() as f32 / i32::MAX as f32)
            .collect(),
        _ => return Err(format!("Unsupported bit depth: {}", spec.bits_per_sample)),
    };

    if spec.channels == 2 {
        let mono: Vec<f32> = samples
            .chunks(2)
            .map(|pair| {
                if pair.len() == 2 {
                    (pair[0] + pair[1]) * 0.5
                } else {
                    pair[0]
                }
            })
            .collect();
        Ok(mono)
    } else {
        Ok(samples)
    }
}
