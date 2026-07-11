//! Minimal SFZ parser — extracts key-to-sample mapping for GPU rendering.
//!
//! Parses only the subset of SFZ needed for voice state construction:
//! key ranges, sample paths, pitch_keycenter, and basic envelope params.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A single SFZ region (sample zone).
#[derive(Debug, Clone)]
pub struct SfzRegion {
    pub sample: String,
    pub key: Option<u8>,
    pub lo_key: u8,
    pub hi_key: u8,
    pub lo_vel: u8,
    pub hi_vel: u8,
    pub pitch_keycenter: Option<u8>,
    pub ampeg_release: f32,
}

/// Parsed SFZ file — a list of regions.
#[derive(Debug, Clone)]
pub struct SfzSoundfont {
    pub regions: Vec<SfzRegion>,
    pub default_path: String,
    /// Base directory of the SFZ file (for resolving relative paths).
    pub base_dir: PathBuf,
}

impl SfzSoundfont {
    /// Parse an SFZ file, resolving #include directives.
    pub fn parse(path: &Path) -> Result<Self, String> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read SFZ {:?}: {}", path, e))?;
        let base_dir = path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_default();
        Self::parse_content(&content, &base_dir, 0)
    }

    fn parse_content(content: &str, base_dir: &Path, depth: u32) -> Result<Self, String> {
        if depth > 10 {
            return Err("SFZ include depth exceeded".into());
        }

        let mut regions = Vec::new();
        let mut default_path = String::new();
        let mut current_region: Option<SfzRegion> = None;

        for line in content.lines() {
            let line = line.trim();

            // Skip comments and empty lines
            if line.starts_with("//") || line.is_empty() {
                continue;
            }

            // Handle #include
            if let Some(include_path) = line.strip_prefix("#include") {
                let include_path = include_path.trim().trim_matches('"').trim_matches('\'');
                let include_full = base_dir.join(include_path);
                if include_full.exists() {
                    let included = std::fs::read_to_string(&include_full)
                        .map_err(|e| format!("Failed to read include {:?}: {}", include_full, e))?;
                    let included_base = include_full
                        .parent()
                        .map(|p| p.to_path_buf())
                        .unwrap_or_else(|| base_dir.to_path_buf());
                    let sub = Self::parse_content(&included, &included_base, depth + 1)?;
                    regions.extend(sub.regions);
                }
                continue;
            }

            // Handle <region> tag
            if line.starts_with("<region>") || line.starts_with("<master>") || line.starts_with("<group>") || line.starts_with("<control>") {
                // Save previous region
                if let Some(r) = current_region.take() {
                    regions.push(r);
                }
                if line.starts_with("<region>") {
                    current_region = Some(SfzRegion {
                        sample: String::new(),
                        key: None,
                        lo_key: 0,
                        hi_key: 127,
                        lo_vel: 0,
                        hi_vel: 127,
                        pitch_keycenter: None,
                        ampeg_release: 0.0,
                    });
                }
                continue;
            }

            // Parse key=value pairs
            if let Some((key, value)) = line.split_once('=') {
                let key = key.trim();
                let value = value.trim();

                match key {
                    "default_path" => {
                        default_path = value.to_string();
                    }
                    "sample" => {
                        if let Some(ref mut r) = current_region {
                            r.sample = value.to_string();
                        }
                    }
                    "key" => {
                        if let Some(ref mut r) = current_region {
                            if let Some(k) = parse_key(value) {
                                r.key = Some(k);
                                r.lo_key = k;
                                r.hi_key = k;
                            }
                        }
                    }
                    "lokey" => {
                        if let Some(ref mut r) = current_region {
                            if let Some(k) = parse_key(value) {
                                r.lo_key = k;
                            }
                        }
                    }
                    "hikey" => {
                        if let Some(ref mut r) = current_region {
                            if let Some(k) = parse_key(value) {
                                r.hi_key = k;
                            }
                        }
                    }
                    "lovel" => {
                        if let Some(ref mut r) = current_region {
                            if let Ok(v) = value.parse::<u8>() {
                                r.lo_vel = v;
                            }
                        }
                    }
                    "hivel" => {
                        if let Some(ref mut r) = current_region {
                            if let Ok(v) = value.parse::<u8>() {
                                r.hi_vel = v;
                            }
                        }
                    }
                    "pitch_keycenter" => {
                        if let Some(ref mut r) = current_region {
                            if let Some(k) = parse_key(value) {
                                r.pitch_keycenter = Some(k);
                            }
                        }
                    }
                    "ampeg_release" => {
                        if let Some(ref mut r) = current_region {
                            if let Ok(v) = value.parse::<f32>() {
                                r.ampeg_release = v;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        // Save last region
        if let Some(r) = current_region {
            regions.push(r);
        }

        Ok(Self {
            regions,
            default_path,
            base_dir: base_dir.to_path_buf(),
        })
    }

    /// Resolve a sample path to an absolute file path.
    pub fn resolve_sample(&self, sample: &str) -> PathBuf {
        // SFZ uses backslashes (Windows convention) — normalize to forward slashes
        let sample = sample.replace('\\', "/");
        let default = self.default_path.replace('\\', "/");
        let without_ext = sample.strip_suffix(".wav").unwrap_or(&sample);
        let raw = self.base_dir.join(&default).join(format!("{}.wav", without_ext));
        // Canonicalize to resolve ".." components
        std::fs::canonicalize(&raw).unwrap_or(raw)
    }

    /// Build a lookup table: MIDI key → (sample_path, pitch_keycenter, release_time).
    /// For each key, picks the best matching region (highest lo_key that fits).
    pub fn build_key_map(&self) -> Vec<(PathBuf, u8, f32)> {
        let mut key_map: Vec<Option<(PathBuf, u8, f32)>> = vec![None; 128];

        for region in &self.regions {
            if region.sample.is_empty() {
                continue;
            }
            let path = self.resolve_sample(&region.sample);
            let pkc = region.pitch_keycenter.unwrap_or(region.key.unwrap_or(60));
            for key in region.lo_key..=region.hi_key {
                let k = key as usize;
                if k < 128 {
                    // Prefer regions with tighter key ranges (more specific)
                    let replace = match &key_map[k] {
                        None => true,
                        Some((_, prev_pkc, _)) => {
                            // Prefer the region whose pitch_keycenter is closest to the key
                            (pkc as i32 - key as i32).unsigned_abs()
                                < (*prev_pkc as i32 - key as i32).unsigned_abs()
                        }
                    };
                    if replace {
                        key_map[k] = Some((path.clone(), pkc, region.ampeg_release));
                    }
                }
            }
        }

        key_map
            .into_iter()
            .map(|opt| {
                opt.unwrap_or_else(|| {
                    // Fallback: middle C sample
                    (
                        PathBuf::from("missing"),
                        60,
                        0.0,
                    )
                })
            })
            .collect()
    }
}

/// Parse an SFZ key name (e.g. "c4", "C#5", "a-1") to MIDI note number.
fn parse_key(s: &str) -> Option<u8> {
    let s = s.trim().to_lowercase();
    let (note_str, octave_str) = if s.ends_with('-') || s.ends_with('+') {
        // Handle "c-1" style
        let last = s.len() - 1;
        (&s[..last], &s[last..])
    } else {
        // Split at the boundary between letters and digits
        let split = s
            .char_indices()
            .find(|(_, c)| c.is_ascii_digit() || *c == '-')
            .map(|(i, _)| i)
            .unwrap_or(s.len());
        (&s[..split], &s[split..])
    };

    let note_val = match note_str {
        "c" => 0,
        "c#" | "db" => 1,
        "d" => 2,
        "d#" | "eb" => 3,
        "e" => 4,
        "f" => 5,
        "f#" | "gb" => 6,
        "g" => 7,
        "g#" | "ab" => 8,
        "a" => 9,
        "a#" | "bb" => 10,
        "b" => 11,
        _ => return None,
    };

    let octave: i8 = octave_str.parse().ok()?;
    // MIDI note = (octave + 1) * 12 + note_val
    let midi = (octave + 1) * 12 + note_val as i8;
    if midi >= 0 && midi <= 127 {
        Some(midi as u8)
    } else {
        None
    }
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
                // hound reads 24-bit as i32 with value in upper 24 bits
                (v >> 8) as f32 / (i16::MAX as f32)
            })
            .collect(),
        32 => reader
            .samples::<i32>()
            .map(|s| s.unwrap() as f32 / i32::MAX as f32)
            .collect(),
        _ => return Err(format!("Unsupported bit depth: {}", spec.bits_per_sample)),
    };

    // Convert stereo to mono by averaging channels
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_sfz_key() {
        assert_eq!(parse_key("c4"), Some(60));
        assert_eq!(parse_key("C#5"), Some(73));
        assert_eq!(parse_key("a-1"), Some(9));
        assert_eq!(parse_key("c-1"), Some(0));
        assert_eq!(parse_key("a4"), Some(69));
        assert_eq!(parse_key("g9"), Some(131)); // out of range
    }

    #[test]
    fn parse_real_sfz() {
        let sfz_path = "/Users/jieneng/Music/Soundfonts/Starry Studio Grand v2.7~/Presets/A_Standard/Studio Grand - Standard (No Hammer).sfz";
        if std::path::Path::new(sfz_path).exists() {
            let sfz = SfzSoundfont::parse(std::path::Path::new(sfz_path)).unwrap();
            eprintln!("Parsed {} regions", sfz.regions.len());
            let key_map = sfz.build_key_map();
            eprintln!("Key map: {} entries", key_map.len());
            // Check that key 60 (middle C) has a sample
            let (ref path, pkc, release) = key_map[60];
            eprintln!("Key 60: sample={:?} pkc={} release={}", path, pkc, release);
            assert!(!path.to_string_lossy().contains("missing"));
        }
    }
}
