//! Sound operations for Scratch project state.

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScratchSoundEffect {
    Pitch,
    Pan,
}

impl ScratchState {
    pub fn set_original_volume(&mut self, target: usize, volume: f64) {
        if let Some(instance) = self
            .instances
            .iter_mut()
            .find(|instance| instance.target == target && instance.original)
        {
            instance.volume = scratch_to_number(volume).clamp(0.0, 100.0);
        }
    }

    pub fn set_sound_library(&mut self, library: SoundLibrary) {
        self.sound_runtime.set_library(library);
    }

    pub fn set_realtime_audio(&mut self, output: RealtimeAudio) {
        self.sound_runtime.set_output(output);
    }

    pub fn start_sound(&mut self, selector: ScratchValue, now_millis: u64) -> Option<u64> {
        let instance = self.current_instance;
        let target = self.instances.get(instance)?.target;
        let parameters = self.sound_parameters(instance);
        self.sound_runtime
            .start(target, instance, selector, parameters, now_millis)
    }

    pub fn set_sound_effect(
        &mut self,
        effect: ScratchSoundEffect,
        value: f64,
        change: bool,
        now_millis: u64,
    ) {
        let instance_id = self.current_instance;
        let instance = &mut self.instances[instance_id];
        let value = scratch_to_number(value);
        match effect {
            ScratchSoundEffect::Pitch => {
                let next = if change {
                    instance.sound_pitch + value
                } else {
                    value
                };
                instance.sound_pitch = next.clamp(-360.0, 360.0);
            }
            ScratchSoundEffect::Pan => {
                let next = if change {
                    instance.sound_pan + value
                } else {
                    value
                };
                instance.sound_pan = next.clamp(-100.0, 100.0);
            }
        }
        let target = instance.target;
        let parameters = self.sound_parameters(instance_id);
        self.sound_runtime
            .update_instance(target, instance_id, parameters, now_millis);
    }

    pub fn clear_sound_effects(&mut self, now_millis: u64) {
        let instance_id = self.current_instance;
        let instance = &mut self.instances[instance_id];
        instance.sound_pitch = 0.0;
        instance.sound_pan = 0.0;
        let target = instance.target;
        let parameters = self.sound_parameters(instance_id);
        self.sound_runtime
            .update_instance(target, instance_id, parameters, now_millis);
    }

    pub fn reset_sound_effects(&mut self, now_millis: u64) {
        for instance_id in 0..self.instances.len() {
            let instance = &mut self.instances[instance_id];
            instance.sound_pitch = 0.0;
            instance.sound_pan = 0.0;
            let parameters = SoundParameters {
                pitch: instance.sound_pitch,
                pan: instance.sound_pan,
                volume: instance.volume,
            };
            self.sound_runtime.update_instance(
                instance.target,
                instance_id,
                parameters,
                now_millis,
            );
        }
    }

    pub fn set_sound_volume(&mut self, value: f64, change: bool, now_millis: u64) {
        let instance_id = self.current_instance;
        let instance = &mut self.instances[instance_id];
        let value = scratch_to_number(value);
        let next = if change {
            instance.volume + value
        } else {
            value
        };
        instance.volume = next.clamp(0.0, 100.0);
        let target = instance.target;
        let parameters = self.sound_parameters(instance_id);
        self.sound_runtime
            .update_instance(target, instance_id, parameters, now_millis);
    }

    pub fn sound_volume(&self) -> f64 {
        self.instances[self.current_instance].volume
    }

    /// read a sprite or stage's original target volume for its reporter monitor.
    pub fn sound_volume_for_target(&self, target: usize) -> f64 {
        self.original_instance(target)
            .map_or(100.0, |instance| self.instances[instance].volume)
    }

    pub fn stop_all_sounds(&mut self, now_millis: u64) {
        self.sound_runtime.advance_to(now_millis);
        self.sound_runtime.stop_all();
    }

    pub fn restart_sounds(&mut self, now_millis: u64) {
        self.stop_all_sounds(now_millis);
        self.reset_sound_effects(now_millis);
    }

    pub fn stop_project_sounds(&mut self, now_millis: u64) {
        self.restart_sounds(now_millis);
    }

    pub fn advance_sounds(&mut self, now_millis: u64) {
        self.sound_runtime.advance_to(now_millis);
    }

    pub fn drain_finished_sounds(&mut self) -> Vec<u64> {
        self.sound_runtime.drain_completed()
    }

    pub fn has_active_sounds(&self) -> bool {
        self.sound_runtime.has_active()
    }

    pub fn take_audio_events(&mut self) -> Vec<super::super::AudioEvent> {
        self.sound_runtime.take_events()
    }

    fn sound_parameters(&self, instance: usize) -> SoundParameters {
        let instance = &self.instances[instance];
        SoundParameters {
            pitch: instance.sound_pitch,
            pan: instance.sound_pan,
            volume: instance.volume,
        }
    }
}
