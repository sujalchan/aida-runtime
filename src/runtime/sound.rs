use std::collections::HashMap;
use std::fmt;
use std::fs::File;
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use aida_ast::parser::ScratchProject;
use rodio::{Decoder, OutputStreamBuilder, Sink, Source};

use super::{ScratchValue, scratch_string};

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
struct SoundEntry {
    name: String,
    sound: Arc<DecodedSound>,
}

#[derive(Clone, Debug, Default)]
pub struct SoundLibrary {
    targets: Vec<Vec<SoundEntry>>,
}

impl SoundLibrary {
    pub fn load_project(project: &ScratchProject, assets_dir: &Path) -> Result<Self, String> {
        let mut decoded = HashMap::<String, Arc<DecodedSound>>::new();
        let mut targets = Vec::with_capacity(project.targets.len());
        for target in &project.targets {
            let mut sounds = Vec::with_capacity(target.sounds.len());
            for metadata in &target.sounds {
                let identity = metadata.identity().to_owned();
                let sound = if let Some(sound) = decoded.get(&identity) {
                    Arc::clone(sound)
                } else {
                    let archive_path = metadata
                        .archive_path()
                        .ok_or_else(|| format!("sound `{}` has no asset path", metadata.name))?;
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

    fn sound(&self, target: usize, selector: &ScratchValue) -> Option<(&SoundEntry, usize)> {
        let sounds = self.targets.get(target)?;
        if sounds.is_empty() {
            return None;
        }
        if let ScratchValue::Text(name) = selector {
            if let Some((index, sound)) = sounds
                .iter()
                .enumerate()
                .find(|(_, sound)| sound.name == *name)
            {
                return Some((sound, index));
            }
        }
        let index = scratch_parse_int(&scratch_string(selector.clone()))?;
        let wrapped = (index - 1).rem_euclid(sounds.len() as i64) as usize;
        sounds.get(wrapped).map(|sound| (sound, wrapped))
    }

    pub fn sound_count(&self) -> usize {
        self.targets.iter().map(Vec::len).sum()
    }
}

fn scratch_parse_int(value: &str) -> Option<i64> {
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

fn decode_sound_asset(path: &Path, identity: String) -> Result<DecodedSound, String> {
    let file = File::open(path)
        .map_err(|error| format!("failed to open sound `{}`: {error}", path.display()))?;
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

#[derive(Clone, Debug, PartialEq)]
pub enum AudioEvent {
    Started {
        playback_id: u64,
        target: usize,
        instance: usize,
        sound_name: String,
        parameters: SoundParameters,
    },
    Updated {
        playback_id: u64,
        parameters: SoundParameters,
    },
    Stopped {
        playback_id: u64,
    },
    Completed {
        playback_id: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct VoiceKey {
    target: usize,
    identity: String,
}

#[derive(Clone, Debug)]
struct ActivePlayback {
    key: VoiceKey,
    target: usize,
    instance: usize,
    sound: Arc<DecodedSound>,
    parameters: SoundParameters,
    position_frames: f64,
    updated_at_millis: u64,
}

enum AudioCommand {
    Start {
        key: VoiceKey,
        playback_id: u64,
        instance: usize,
        sound: Arc<DecodedSound>,
        parameters: SoundParameters,
    },
    Update {
        key: VoiceKey,
        playback_id: u64,
        instance: usize,
        parameters: SoundParameters,
    },
    StopAll,
    StopTarget {
        target: usize,
        instance: usize,
    },
    Shutdown,
}

struct PlaybackControls {
    pan: AtomicU32,
    volume: AtomicU32,
    pitch_rate: AtomicU32,
    fade_out: std::sync::atomic::AtomicBool,
}

impl PlaybackControls {
    fn new(parameters: SoundParameters) -> Self {
        Self {
            pan: AtomicU32::new((parameters.pan as f32).to_bits()),
            volume: AtomicU32::new((parameters.volume as f32).to_bits()),
            pitch_rate: AtomicU32::new((parameters.pitch_rate() as f32).to_bits()),
            fade_out: std::sync::atomic::AtomicBool::new(false),
        }
    }

    fn update(&self, parameters: SoundParameters) {
        self.pan
            .store((parameters.pan as f32).to_bits(), Ordering::Relaxed);
        self.volume
            .store((parameters.volume as f32).to_bits(), Ordering::Relaxed);
        self.pitch_rate.store(
            (parameters.pitch_rate() as f32).to_bits(),
            Ordering::Relaxed,
        );
    }
}

pub struct RealtimeAudio {
    commands: mpsc::Sender<AudioCommand>,
    completed: mpsc::Receiver<u64>,
    worker: Option<JoinHandle<()>>,
}

impl RealtimeAudio {
    pub fn open() -> Result<Self, String> {
        let (commands, command_rx) = mpsc::channel();
        let (completed_tx, completed) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("aida-audio-output".to_owned())
            .spawn(move || {
                let stream = match OutputStreamBuilder::open_default_stream() {
                    Ok(output) => output,
                    Err(error) => {
                        let _ = ready_tx.send(Err(format!(
                            "could not open the default audio output: {error}"
                        )));
                        return;
                    }
                };
                if ready_tx.send(Ok(())).is_err() {
                    return;
                }
                let mut voices = HashMap::<VoiceKey, OutputVoice>::new();
                let mut fading = HashMap::<u64, (OutputVoice, std::time::Instant)>::new();
                loop {
                    match command_rx.recv_timeout(Duration::from_millis(5)) {
                        Ok(AudioCommand::Start {
                            key,
                            playback_id,
                            instance,
                            sound,
                            parameters,
                        }) => {
                            if let Some(previous) = voices.remove(&key) {
                                begin_audio_fade(previous, &mut fading);
                            }
                            let sink = Sink::connect_new(stream.mixer());
                            sink.set_speed(parameters.pitch_rate() as f32);
                            let controls = Arc::new(PlaybackControls::new(parameters));
                            sink.append(PannedSound::new(
                                Arc::clone(&sound),
                                Arc::clone(&controls),
                            ));
                            voices.insert(
                                key,
                                OutputVoice {
                                    playback_id,
                                    instance,
                                    sink,
                                    controls,
                                },
                            );
                        }
                        Ok(AudioCommand::Update {
                            key,
                            playback_id,
                            instance,
                            parameters,
                        }) => {
                            if let Some(voice) = voices.get_mut(&key)
                                && voice.playback_id == playback_id
                                && voice.instance == instance
                            {
                                voice.sink.set_speed(parameters.pitch_rate() as f32);
                                voice.controls.update(parameters);
                            }
                        }
                        Ok(AudioCommand::StopAll) => {
                            for (_, voice) in voices.drain() {
                                begin_audio_fade(voice, &mut fading);
                            }
                        }
                        Ok(AudioCommand::StopTarget { target, instance }) => {
                            let keys = voices
                                .iter()
                                .filter_map(|(key, voice)| {
                                    (key.target == target && voice.instance == instance)
                                        .then_some(key.clone())
                                })
                                .collect::<Vec<_>>();
                            for key in keys {
                                if let Some(voice) = voices.remove(&key) {
                                    begin_audio_fade(voice, &mut fading);
                                }
                            }
                        }
                        Ok(AudioCommand::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                            break;
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                    }
                    let finished = voices
                        .iter()
                        .filter_map(|(key, voice)| voice.sink.empty().then_some(key.clone()))
                        .collect::<Vec<_>>();
                    for key in finished {
                        if let Some(voice) = voices.remove(&key) {
                            let _ = completed_tx.send(voice.playback_id);
                        }
                    }
                    let expired_fades = fading
                        .iter()
                        .filter_map(|(playback_id, (_, stop_at))| {
                            (std::time::Instant::now() >= *stop_at).then_some(*playback_id)
                        })
                        .collect::<Vec<_>>();
                    for playback_id in expired_fades {
                        if let Some((voice, _)) = fading.remove(&playback_id) {
                            voice.sink.stop();
                        }
                    }
                }
                for (_, voice) in voices.drain() {
                    voice.sink.stop();
                    let _ = completed_tx.send(voice.playback_id);
                }
                for (_, (voice, _)) in fading.drain() {
                    voice.sink.stop();
                }
                drop(stream);
            })
            .map_err(|error| format!("could not start audio output thread: {error}"))?;
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                commands,
                completed,
                worker: Some(worker),
            }),
            Ok(Err(error)) => {
                let _ = worker.join();
                Err(error)
            }
            Err(error) => {
                let _ = worker.join();
                Err(format!(
                    "audio output thread exited during startup: {error}"
                ))
            }
        }
    }
}

impl Drop for RealtimeAudio {
    fn drop(&mut self) {
        let _ = self.commands.send(AudioCommand::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

struct OutputVoice {
    playback_id: u64,
    instance: usize,
    sink: Sink,
    controls: Arc<PlaybackControls>,
}

fn begin_audio_fade(
    voice: OutputVoice,
    fading: &mut HashMap<u64, (OutputVoice, std::time::Instant)>,
) {
    voice.controls.fade_out.store(true, Ordering::Relaxed);
    let stop_at = std::time::Instant::now() + Duration::from_millis(25);
    fading.insert(voice.playback_id, (voice, stop_at));
}

struct PannedSound {
    sound: Arc<DecodedSound>,
    controls: Arc<PlaybackControls>,
    frame: usize,
    output_right: bool,
    current_sample: f32,
    left_gain: f32,
    right_gain: f32,
}

impl PannedSound {
    fn new(sound: Arc<DecodedSound>, controls: Arc<PlaybackControls>) -> Self {
        let pan = f64::from(f32::from_bits(controls.pan.load(Ordering::Relaxed)));
        let volume = f32::from_bits(controls.volume.load(Ordering::Relaxed)) / 100.0;
        let (left, right) = pan_gains(pan);
        Self {
            sound,
            controls,
            frame: 0,
            output_right: false,
            current_sample: 0.0,
            left_gain: left as f32 * volume,
            right_gain: right as f32 * volume,
        }
    }

    fn sample_mono(&self, frame: usize) -> f32 {
        let channels = usize::from(self.sound.channels);
        let start = frame * channels;
        self.sound.samples[start..start + channels]
            .iter()
            .sum::<f32>()
            / channels as f32
    }

    fn update_gains(&mut self) {
        let pan = f64::from(f32::from_bits(self.controls.pan.load(Ordering::Relaxed)));
        let volume = f32::from_bits(self.controls.volume.load(Ordering::Relaxed)) / 100.0;
        let (left, right) = pan_gains(pan);
        let (left, right) = if self.controls.fade_out.load(Ordering::Relaxed) {
            (0.0, 0.0)
        } else {
            (left as f32 * volume, right as f32 * volume)
        };
        let playback_rate = f64::from(f32::from_bits(
            self.controls.pitch_rate.load(Ordering::Relaxed),
        ))
        .max(0.001);
        let effective_rate = f64::from(self.sound.sample_rate.max(1)) * playback_rate;
        let blend = (1.0 - (-1.0 / (effective_rate * 0.025).max(1.0)).exp()) as f32;
        self.left_gain += (left - self.left_gain) * blend;
        self.right_gain += (right - self.right_gain) * blend;
    }
}

impl Iterator for PannedSound {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        if self.frame >= self.sound.frame_count() {
            return None;
        }
        if !self.output_right {
            self.current_sample = self.sample_mono(self.frame);
            self.output_right = true;
            Some(self.current_sample * self.left_gain)
        } else {
            self.output_right = false;
            self.frame += 1;
            self.update_gains();
            Some(self.current_sample * self.right_gain)
        }
    }
}

impl Source for PannedSound {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> u16 {
        2
    }
    fn sample_rate(&self) -> u32 {
        self.sound.sample_rate
    }
    fn total_duration(&self) -> Option<Duration> {
        Some(Duration::from_secs_f64(self.sound.duration_seconds()))
    }
}

pub struct SoundRuntime {
    library: Option<Arc<SoundLibrary>>,
    output: Option<RealtimeAudio>,
    active: HashMap<u64, ActivePlayback>,
    current: HashMap<VoiceKey, u64>,
    completed: Vec<u64>,
    next_playback_id: u64,
    events: Vec<AudioEvent>,
}

impl fmt::Debug for SoundRuntime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SoundRuntime")
            .field(
                "sound_count",
                &self
                    .library
                    .as_ref()
                    .map_or(0, |library| library.sound_count()),
            )
            .field("active_voices", &self.active.len())
            .finish()
    }
}

impl Default for SoundRuntime {
    fn default() -> Self {
        Self {
            library: None,
            output: None,
            active: HashMap::new(),
            current: HashMap::new(),
            completed: Vec::new(),
            next_playback_id: 1,
            events: Vec::new(),
        }
    }
}

impl SoundRuntime {
    pub fn set_library(&mut self, library: SoundLibrary) {
        self.library = Some(Arc::new(library));
    }

    pub fn set_output(&mut self, output: RealtimeAudio) {
        self.output = Some(output);
    }

    pub fn start(
        &mut self,
        target: usize,
        instance: usize,
        selector: ScratchValue,
        parameters: SoundParameters,
        now_millis: u64,
    ) -> Option<u64> {
        self.advance_to(now_millis);
        let (entry, _) = self.library.as_ref()?.sound(target, &selector)?;
        let sound = Arc::clone(&entry.sound);
        let sound_name = entry.name.clone();
        let key = VoiceKey {
            target,
            identity: sound.identity.clone(),
        };
        if let Some(previous) = self.current.get(&key).copied() {
            self.finish(previous, true);
        }
        let playback_id = self.next_playback_id;
        self.next_playback_id = self.next_playback_id.wrapping_add(1).max(1);
        self.current.insert(key.clone(), playback_id);
        self.active.insert(
            playback_id,
            ActivePlayback {
                key: key.clone(),
                target,
                instance,
                sound: Arc::clone(&sound),
                parameters,
                position_frames: 0.0,
                updated_at_millis: now_millis,
            },
        );
        self.events.push(AudioEvent::Started {
            playback_id,
            target,
            instance,
            sound_name,
            parameters,
        });
        if let Some(output) = &self.output {
            let _ = output.commands.send(AudioCommand::Start {
                key,
                playback_id,
                instance,
                sound,
                parameters,
            });
        }
        Some(playback_id)
    }

    pub fn update_instance(
        &mut self,
        target: usize,
        instance: usize,
        parameters: SoundParameters,
        now_millis: u64,
    ) {
        self.advance_to(now_millis);
        for (playback_id, playback) in &mut self.active {
            if playback.target == target && playback.instance == instance {
                playback.parameters = parameters;
                self.events.push(AudioEvent::Updated {
                    playback_id: *playback_id,
                    parameters,
                });
                if let Some(output) = &self.output {
                    let _ = output.commands.send(AudioCommand::Update {
                        key: playback.key.clone(),
                        playback_id: *playback_id,
                        instance,
                        parameters,
                    });
                }
            }
        }
    }

    pub fn stop_target(&mut self, target: usize, instance: usize) {
        let ids = self
            .active
            .iter()
            .filter_map(|(id, playback)| {
                (playback.target == target && playback.instance == instance).then_some(*id)
            })
            .collect::<Vec<_>>();
        for id in ids {
            self.finish(id, true);
        }
        if let Some(output) = &self.output {
            let _ = output
                .commands
                .send(AudioCommand::StopTarget { target, instance });
        }
    }

    pub fn stop_all(&mut self) {
        let ids = self.active.keys().copied().collect::<Vec<_>>();
        for id in ids {
            self.finish(id, true);
        }
        if let Some(output) = &self.output {
            let _ = output.commands.send(AudioCommand::StopAll);
        }
    }

    pub fn advance_to(&mut self, now_millis: u64) {
        if self.output.is_none() {
            let mut finished = Vec::new();
            for (id, playback) in &mut self.active {
                let elapsed = now_millis.saturating_sub(playback.updated_at_millis) as f64 / 1000.0;
                playback.position_frames += elapsed
                    * f64::from(playback.sound.sample_rate)
                    * playback.parameters.pitch_rate();
                playback.updated_at_millis = now_millis;
                if playback.position_frames >= playback.sound.frame_count() as f64 {
                    finished.push(*id);
                }
            }
            for id in finished {
                self.finish(id, false);
            }
        }
        if let Some(output) = &self.output {
            let finished = output.completed.try_iter().collect::<Vec<_>>();
            for id in finished {
                self.finish(id, false);
            }
        }
    }

    pub fn has_active(&self) -> bool {
        !self.active.is_empty()
    }

    pub fn drain_completed(&mut self) -> Vec<u64> {
        if let Some(output) = &self.output {
            let finished = output.completed.try_iter().collect::<Vec<_>>();
            for id in finished {
                self.finish(id, false);
            }
        }
        std::mem::take(&mut self.completed)
    }

    pub fn take_events(&mut self) -> Vec<AudioEvent> {
        std::mem::take(&mut self.events)
    }

    fn finish(&mut self, playback_id: u64, stopped: bool) {
        if let Some(playback) = self.active.remove(&playback_id) {
            if self.current.get(&playback.key) == Some(&playback_id) {
                self.current.remove(&playback.key);
            }
            self.completed.push(playback_id);
            self.events.push(if stopped {
                AudioEvent::Stopped { playback_id }
            } else {
                AudioEvent::Completed { playback_id }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{
        DecodedSound, PannedSound, PlaybackControls, SoundEntry, SoundLibrary, SoundParameters,
        SoundRuntime, decode_sound_asset, pan_gains, scratch_parse_int,
    };
    use crate::runtime::ScratchValue;
    use aida_ast::load_sb3;

    fn sound_library() -> SoundLibrary {
        let sounds = vec![
            SoundEntry {
                name: "Bass".to_owned(),
                sound: Arc::new(DecodedSound {
                    identity: "bass-id".to_owned(),
                    sample_rate: 100,
                    channels: 1,
                    samples: vec![0.25; 100].into(),
                }),
            },
            SoundEntry {
                name: "Piano".to_owned(),
                sound: Arc::new(DecodedSound {
                    identity: "piano-id".to_owned(),
                    sample_rate: 100,
                    channels: 1,
                    samples: vec![0.5; 100].into(),
                }),
            },
            SoundEntry {
                name: "1".to_owned(),
                sound: Arc::new(DecodedSound {
                    identity: "numeric-name-id".to_owned(),
                    sample_rate: 100,
                    channels: 1,
                    samples: vec![0.75; 100].into(),
                }),
            },
        ];
        SoundLibrary {
            targets: vec![sounds.clone(), sounds],
        }
    }

    #[test]
    fn pitch_and_pan_follow_scratch_audio_mapping() {
        assert!(
            (SoundParameters {
                pitch: 10.0,
                ..Default::default()
            }
            .pitch_rate()
                - 2.0_f64.powf(1.0 / 12.0))
            .abs()
                < 1e-12
        );
        assert_eq!(
            SoundParameters {
                pitch: 120.0,
                ..Default::default()
            }
            .pitch_rate(),
            2.0
        );
        assert_eq!(
            SoundParameters {
                pitch: -120.0,
                ..Default::default()
            }
            .pitch_rate(),
            0.5
        );
        assert_eq!(
            SoundParameters {
                pitch: 360.0,
                ..Default::default()
            }
            .pitch_rate(),
            8.0
        );
        assert_eq!(
            SoundParameters {
                pitch: -360.0,
                ..Default::default()
            }
            .pitch_rate(),
            0.125
        );
        let (left, right) = pan_gains(0.0);
        assert!((left - std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-12);
        assert!((right - std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-12);
        assert_eq!(pan_gains(-100.0), (1.0, 0.0));
        let (left, right) = pan_gains(100.0);
        assert!(left.abs() < 1e-12 && (right - 1.0).abs() < 1e-12);
    }

    #[test]
    fn mixer_applies_equal_power_pan_and_target_volume() {
        let sound = Arc::new(DecodedSound {
            identity: "mixer-test".to_owned(),
            sample_rate: 100,
            channels: 1,
            samples: vec![1.0, 1.0].into(),
        });
        let controls = Arc::new(PlaybackControls::new(SoundParameters::default()));
        let mut centered = PannedSound::new(Arc::clone(&sound), Arc::clone(&controls));
        let center_left = centered.next().unwrap();
        let center_right = centered.next().unwrap();
        assert!((center_left - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        assert!((center_right - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);

        controls.update(SoundParameters {
            pan: -100.0,
            volume: 50.0,
            ..Default::default()
        });
        let mut left = PannedSound::new(sound, Arc::clone(&controls));
        assert!((left.next().unwrap() - 0.5).abs() < 1e-6);
        assert!(left.next().unwrap().abs() < 1e-6);
    }

    #[test]
    fn sound_index_conversion_matches_parse_int_and_wraps_at_the_caller() {
        assert_eq!(scratch_parse_int("  -2.75tail"), Some(-2));
        assert_eq!(scratch_parse_int("first"), None);
    }

    #[test]
    fn project_assets_decode_wav_and_mp3_once_into_shared_playback_data() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../aida-ast/tests/src/sound.sb3");
        let asset_root =
            std::env::temp_dir().join(format!("aida-runtime-sounds-{}", std::process::id()));
        let loaded = load_sb3(fixture, &asset_root).unwrap();
        let library = SoundLibrary::load_project(&loaded.project, &loaded.assets_dir).unwrap();
        assert_eq!(library.sound_count(), 2);
        let sprite_sounds = &library.targets[1];
        assert_eq!(sprite_sounds[0].name, "A Bass");
        assert_eq!(sprite_sounds[1].name, "A Elec Piano");
        assert_eq!(sprite_sounds[1].sound.sample_rate, 22_050);
        assert!((sprite_sounds[1].sound.duration_seconds() - 2.0).abs() < 0.002);

        let mp3_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/sine.mp3");
        let mp3 = decode_sound_asset(&mp3_path, "mp3-test".to_owned()).unwrap();
        assert_eq!(mp3.sample_rate, 22_050);
        assert!(mp3.frame_count() >= 2_000);
        assert!(mp3.samples.iter().any(|sample| sample.abs() > 0.01));

        let adpcm_path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/sine-adpcm.wav");
        let adpcm = decode_sound_asset(&adpcm_path, "adpcm-test".to_owned()).unwrap();
        assert_eq!(adpcm.sample_rate, 22_050);
        assert!(adpcm.frame_count() >= 2_000);
        assert!(adpcm.samples.iter().any(|sample| sample.abs() > 0.01));
        let _ = std::fs::remove_dir_all(asset_root);
    }

    #[test]
    fn sound_lookup_prefers_exact_names_then_wraps_numeric_indexes() {
        let library = sound_library();
        assert_eq!(
            library
                .sound(0, &ScratchValue::Text("1".to_owned()))
                .unwrap()
                .1,
            2
        );
        assert_eq!(
            library
                .sound(0, &ScratchValue::Text("4tail".to_owned()))
                .unwrap()
                .1,
            0
        );
        assert_eq!(
            library
                .sound(0, &ScratchValue::Text("0".to_owned()))
                .unwrap()
                .1,
            2
        );
        assert!(
            library
                .sound(0, &ScratchValue::Text("missing".to_owned()))
                .is_none()
        );
        assert!(
            library
                .sound(3, &ScratchValue::Text("Bass".to_owned()))
                .is_none()
        );
    }

    #[test]
    fn deterministic_playback_completion_tracks_pitch_changes_and_voice_restarts() {
        let mut runtime = SoundRuntime::default();
        runtime.set_library(sound_library());
        let original = runtime
            .start(0, 0, "Bass".into(), SoundParameters::default(), 0)
            .unwrap();
        runtime.advance_to(250);
        assert!(runtime.has_active());
        runtime.update_instance(
            0,
            0,
            SoundParameters {
                pitch: 120.0,
                pan: 100.0,
                volume: 75.0,
            },
            250,
        );
        runtime.advance_to(625);
        assert_eq!(runtime.drain_completed(), vec![original]);
        assert!(!runtime.has_active());

        let first = runtime
            .start(0, 0, "Piano".into(), SoundParameters::default(), 1_000)
            .unwrap();
        let restarted = runtime
            .start(0, 0, "Piano".into(), SoundParameters::default(), 1_001)
            .unwrap();
        let other_target = runtime
            .start(1, 2, "Piano".into(), SoundParameters::default(), 1_002)
            .unwrap();
        let events = runtime.take_events();
        assert!(events.contains(&super::AudioEvent::Stopped { playback_id: first }));
        assert!(events.contains(&super::AudioEvent::Started {
            playback_id: restarted,
            target: 0,
            instance: 0,
            sound_name: "Piano".to_owned(),
            parameters: SoundParameters::default(),
        }));
        assert!(events.contains(&super::AudioEvent::Updated {
            playback_id: original,
            parameters: SoundParameters {
                pitch: 120.0,
                pan: 100.0,
                volume: 75.0
            },
        }));
        assert!(events.contains(&super::AudioEvent::Completed {
            playback_id: original
        }));
        assert!(events.contains(&super::AudioEvent::Started {
            playback_id: other_target,
            target: 1,
            instance: 2,
            sound_name: "Piano".to_owned(),
            parameters: SoundParameters::default(),
        }));
        runtime.stop_target(0, 0);
        assert!(runtime.has_active());
        assert_eq!(runtime.drain_completed(), vec![first, restarted]);
        runtime.stop_all();
        assert!(!runtime.has_active());
        assert_eq!(runtime.drain_completed(), vec![other_target]);
        assert!(runtime.take_events().contains(&super::AudioEvent::Stopped {
            playback_id: other_target
        }));
    }
}
