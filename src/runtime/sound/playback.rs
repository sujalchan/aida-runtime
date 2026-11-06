use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use super::super::ScratchValue;
use super::assets::{DecodedSound, SoundLibrary, SoundParameters};
use super::source::{AudioCommand, AudioEvent, RealtimeAudio, VoiceKey};

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
