use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use rodio::{OutputStreamBuilder, Sink, Source};

use super::assets::{DecodedSound, SoundParameters, pan_gains};

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
pub(super) struct VoiceKey {
    pub(super) target: usize,
    pub(super) identity: String,
}

pub(super) enum AudioCommand {
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

pub(super) struct PlaybackControls {
    pan: AtomicU32,
    volume: AtomicU32,
    pitch_rate: AtomicU32,
    fade_out: std::sync::atomic::AtomicBool,
}

impl PlaybackControls {
    pub(super) fn new(parameters: SoundParameters) -> Self {
        Self {
            pan: AtomicU32::new((parameters.pan as f32).to_bits()),
            volume: AtomicU32::new((parameters.volume as f32).to_bits()),
            pitch_rate: AtomicU32::new((parameters.pitch_rate() as f32).to_bits()),
            fade_out: std::sync::atomic::AtomicBool::new(false),
        }
    }

    pub(super) fn update(&self, parameters: SoundParameters) {
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
    pub(super) commands: mpsc::Sender<AudioCommand>,
    pub(super) completed: mpsc::Receiver<u64>,
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

pub(super) struct PannedSound {
    sound: Arc<DecodedSound>,
    controls: Arc<PlaybackControls>,
    frame: usize,
    output_right: bool,
    current_sample: f32,
    left_gain: f32,
    right_gain: f32,
}

impl PannedSound {
    pub(super) fn new(sound: Arc<DecodedSound>, controls: Arc<PlaybackControls>) -> Self {
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
