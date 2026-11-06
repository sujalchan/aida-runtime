use std::collections::HashMap;
use std::fs::File;
use std::io;
use std::path::Path;
use std::sync::Arc;

use aida_ast::parser::ScratchProject;
use rodio::{Decoder, Source};

use super::super::{ScratchValue, scratch_string};

#[derive(Debug)]
pub enum SoundAssetError {
    MissingArchivePath { sound_name: String },
    Open { path: String, source: io::Error },
}

impl std::fmt::Display for SoundAssetError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingArchivePath { sound_name } => {
                write!(formatter, "sound `{sound_name}` has no asset path")
            }
            Self::Open { path, source } => {
                write!(formatter, "failed to open sound `{path}`: {source}")
            }
        }
    }
}

impl std::error::Error for SoundAssetError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Open { source, .. } => Some(source),
            Self::MissingArchivePath { .. } => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SoundParameters {
    pub pitch: f64,
    pub pan: f64,
    pub volume: f64,
}

impl Default for SoundParameters {
    fn default() -> Self {
        Self {
            pitch: 0.0,
            pan: 0.0,
            volume: 100.0,
        }
    }
}

impl SoundParameters {
    pub fn pitch_rate(self) -> f64 {
        2.0_f64.powf(self.pitch / 120.0)
    }
}

pub fn pan_gains(pan: f64) -> (f64, f64) {
    let p = (pan.clamp(-100.0, 100.0) + 100.0) / 200.0;
    let angle = p * std::f64::consts::FRAC_PI_2;
    (angle.cos(), angle.sin())
}

#[derive(Clone, Debug)]
pub struct DecodedSound {
    pub identity: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub samples: Arc<[f32]>,
}

impl DecodedSound {
    fn silence(identity: String) -> Self {
        Self {
            identity,
            sample_rate: 44_100,
            channels: 1,
            samples: Arc::from([0.0]),
        }
    }

    pub fn frame_count(&self) -> usize {
        self.samples.len() / usize::from(self.channels.max(1))
    }

    pub fn duration_seconds(&self) -> f64 {
        self.frame_count() as f64 / f64::from(self.sample_rate.max(1))
    }
}

#[derive(Clone, Debug)]
pub(super) struct SoundEntry {
    pub(super) name: String,
    pub(super) sound: Arc<DecodedSound>,
}

#[derive(Clone, Debug, Default)]
pub struct SoundLibrary {
    pub(super) targets: Vec<Vec<SoundEntry>>,
}

impl SoundLibrary {
    pub fn load_project(
        project: &ScratchProject,
        assets_dir: &Path,
    ) -> Result<Self, SoundAssetError> {
        let mut decoded = HashMap::<String, Arc<DecodedSound>>::new();
        let mut targets = Vec::with_capacity(project.targets.len());
        for target in &project.targets {
            let mut sounds = Vec::with_capacity(target.sounds.len());
            for metadata in &target.sounds {
                let identity = metadata.identity().to_owned();
                let sound = if let Some(sound) = decoded.get(&identity) {
                    Arc::clone(sound)
                } else {
                    let archive_path = metadata.archive_path().ok_or_else(|| {
                        SoundAssetError::MissingArchivePath {
                            sound_name: metadata.name.clone(),
                        }
                    })?;
                    let path = assets_dir.join(&archive_path);
                    let sound = Arc::new(decode_sound_asset(&path, identity.clone())?);
                    decoded.insert(identity, Arc::clone(&sound));
                    sound
                };
                sounds.push(SoundEntry {
                    name: metadata.name.clone(),
                    sound,
                });
            }
            targets.push(sounds);
        }
        Ok(Self { targets })
    }

    pub(super) fn sound(
        &self,
        target: usize,
        selector: &ScratchValue,
    ) -> Option<(&SoundEntry, usize)> {
        let sounds = self.targets.get(target)?;
        if sounds.is_empty() {
            return None;
        }
        if let ScratchValue::Text(name) = selector
            && let Some((index, sound)) = sounds
                .iter()
                .enumerate()
                .find(|(_, sound)| sound.name == *name)
        {
            return Some((sound, index));
        }
        let index = scratch_parse_int(&scratch_string(selector.clone()))?;
        let wrapped = (index - 1).rem_euclid(sounds.len() as i64) as usize;
        sounds.get(wrapped).map(|sound| (sound, wrapped))
    }

    pub fn sound_count(&self) -> usize {
        self.targets.iter().map(Vec::len).sum()
    }
}

pub(super) fn scratch_parse_int(value: &str) -> Option<i64> {
    let value = value.trim_start();
    let (negative, digits) = match value.as_bytes().first().copied() {
        Some(b'-') => (true, &value[1..]),
        Some(b'+') => (false, &value[1..]),
        _ => (false, value),
    };
    let digit_count = digits.bytes().take_while(u8::is_ascii_digit).count();
    if digit_count == 0 {
        return None;
    }
    let parsed = digits[..digit_count].parse::<i64>().ok()?;
    Some(if negative { -parsed } else { parsed })
}

pub(super) fn decode_sound_asset(
    path: &Path,
    identity: String,
) -> Result<DecodedSound, SoundAssetError> {
    let file = File::open(path).map_err(|source| SoundAssetError::Open {
        path: path.display().to_string(),
        source,
    })?;
    let Ok(decoder) = Decoder::try_from(file) else {
        return Ok(DecodedSound::silence(identity));
    };
    let channels = decoder.channels();
    let sample_rate = decoder.sample_rate();
    let samples = decoder.collect::<Vec<_>>();
    if channels == 0 || sample_rate == 0 || samples.len() < usize::from(channels) {
        return Ok(DecodedSound::silence(identity));
    }
    Ok(DecodedSound {
        identity,
        sample_rate,
        channels,
        samples: samples.into(),
    })
}
