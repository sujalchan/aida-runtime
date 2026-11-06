use super::*;

#[test]
fn sound_effects_clamp_copy_to_clones_and_clear_without_resetting_volume() {
    let mut state = ScratchState::new(Vec::new());
    state.register_target(0, "Stage", true);
    state.register_target(1, "Sprite1", false);
    let original = state.original_instance(1).unwrap();
    state.set_current_instance(original);
    state.set_sound_effect(ScratchSoundEffect::Pitch, 500.0, true, 0);
    state.set_sound_effect(ScratchSoundEffect::Pitch, 100.0, true, 0);
    state.set_sound_effect(ScratchSoundEffect::Pitch, -500.0, true, 0);
    state.set_sound_effect(ScratchSoundEffect::Pan, 150.0, true, 0);
    state.set_sound_effect(ScratchSoundEffect::Pan, 150.0, true, 0);
    state.set_sound_effect(ScratchSoundEffect::Pan, -150.0, true, 0);
    state.set_sound_volume(90.0, false, 0);
    state.set_sound_volume(100.0, true, 0);

    let clone = state.create_clone(original, 1, 300).unwrap();
    assert_eq!(state.instances[clone].sound_pitch, -140.0);
    assert_eq!(state.instances[clone].sound_pan, -50.0);
    assert_eq!(state.instances[clone].volume, 100.0);

    state.set_current_instance(clone);
    state.set_sound_effect(ScratchSoundEffect::Pitch, -500.0, false, 1);
    state.set_sound_effect(ScratchSoundEffect::Pitch, -100.0, true, 1);
    state.set_sound_effect(ScratchSoundEffect::Pan, 150.0, false, 1);
    state.set_sound_effect(ScratchSoundEffect::Pan, 10.0, true, 1);
    state.set_sound_effect(ScratchSoundEffect::Pan, -500.0, true, 1);
    state.set_sound_volume(15.0, false, 1);
    state.clear_sound_effects(1);
    assert_eq!(state.instances[clone].sound_pitch, 0.0);
    assert_eq!(state.instances[clone].sound_pan, 0.0);
    assert_eq!(state.instances[clone].volume, 15.0);

    state.set_current_instance(original);
    assert_eq!(state.instances[original].sound_pitch, -140.0);
    assert_eq!(state.instances[original].sound_pan, -50.0);
    assert_eq!(state.instances[original].volume, 100.0);

    state.restart_sounds(2);
    assert_eq!(state.instances[original].sound_pitch, 0.0);
    assert_eq!(state.instances[original].sound_pan, 0.0);
    assert_eq!(state.instances[original].volume, 100.0);
    assert_eq!(state.instances[clone].sound_pitch, 0.0);
    assert_eq!(state.instances[clone].sound_pan, 0.0);
    assert_eq!(state.instances[clone].volume, 15.0);
}

#[test]
fn project_volume_is_loaded_for_the_original_and_inherited_by_clones() {
    let mut state = ScratchState::new(Vec::new());
    state.register_target(0, "Stage", true);
    state.register_target(1, "Sprite1", false);
    state.set_original_volume(1, 42.0);
    let original = state.original_instance(1).unwrap();
    state.set_current_instance(original);
    assert_eq!(state.sound_volume(), 42.0);
    assert_eq!(state.sound_volume_for_target(1), 42.0);
    assert_eq!(state.sound_volume_for_target(0), 100.0);

    let clone = state.create_clone(original, 1, 300).unwrap();
    state.set_current_instance(clone);
    assert_eq!(state.sound_volume(), 42.0);
    state.set_sound_volume(20.0, true, 0);
    assert_eq!(state.sound_volume(), 62.0);
    assert_eq!(state.sound_volume_for_target(1), 42.0);
    state.set_current_instance(original);
    assert_eq!(state.sound_volume(), 42.0);
}
