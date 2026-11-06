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
    let fixture =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../aida-ast/tests/src/sound.sb3");
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
