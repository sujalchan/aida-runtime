use super::*;

#[test]
fn key_hats_suppress_restarts_per_hat_and_instance_and_allow_clone_threads() {
    let mut state = ScratchState::new(vec![0.0_f64.into()]);
    state.register_target(0, "Stage", true);
    state.register_target(1, "Sprite", false);
    let first_clone = state.create_clone(1, 1, 300).unwrap();
    let second_clone = state.create_clone(1, 1, 300).unwrap();
    let mut scheduler = ScriptScheduler::new(Vec::<KeyHatScript>::new());
    let make = |hat, instance| KeyHatScript {
        key: (hat, instance),
        steps: 0,
    };

    scheduler.start_key_hats([make(4, 1), make(4, first_clone), make(4, second_clone)]);
    assert_eq!(scheduler.active_scripts(), 3);
    // repeat keydown events do not restart those active threads, per clone.
    scheduler.start_key_hats([make(4, 1), make(4, first_clone), make(4, second_clone)]);
    assert_eq!(scheduler.active_scripts(), 3);
    // a different key hat on the same target remains independently runnable.
    scheduler.start_key_hats([make(5, 1)]);
    assert_eq!(scheduler.active_scripts(), 4);

    for _ in 0..12 {
        if scheduler.active_scripts() == 0 {
            break;
        }
        scheduler.advance(&mut state);
    }
    assert_eq!(scheduler.active_scripts(), 0);
    assert_eq!(state.get(0), &ScratchValue::Number(4.0));

    // once a key hat finishes, a later repeated keydown can start it again.
    scheduler.start_key_hats([make(4, 1)]);
    assert_eq!(scheduler.active_scripts(), 1);
}

#[test]
fn key_dispatch_candidates_follow_live_original_and_clone_instances() {
    let mut state = ScratchState::new(vec![0.0_f64.into()]);
    state.register_target(0, "Stage", true);
    state.register_target(1, "Sprite", false);
    let first_clone = state.create_clone(1, 1, 300).unwrap();
    let second_clone = state.create_clone(1, 1, 300).unwrap();
    let mut scheduler = ScriptScheduler::new(Vec::<KeyHatScript>::new());
    let live_candidates = |state: &ScratchState| {
        state
            .instances_for_target(1)
            .into_iter()
            .map(|instance| KeyHatScript {
                key: (7, instance),
                steps: 0,
            })
            .collect::<Vec<_>>()
    };

    assert_eq!(
        state.instances_for_target(1),
        vec![1, first_clone, second_clone]
    );
    scheduler.start_key_hats(live_candidates(&state));
    assert_eq!(scheduler.active_scripts(), 3);

    state.set_current_instance(first_clone);
    assert!(state.delete_current_clone());
    scheduler.remove_target_scripts(first_clone);
    assert_eq!(state.instances_for_target(1), vec![1, second_clone]);
    assert_eq!(scheduler.active_scripts(), 2);

    // another press can target only the original and still-live clone.
    scheduler.start_key_hats(live_candidates(&state));
    assert_eq!(scheduler.active_scripts(), 2);
    for _ in 0..8 {
        if scheduler.active_scripts() == 0 {
            break;
        }
        scheduler.advance(&mut state);
    }
    scheduler.start_key_hats(live_candidates(&state));
    assert_eq!(scheduler.active_scripts(), 2);
}

#[test]
fn pointer_events_route_stage_original_and_clone_clicks_to_distinct_instances() {
    let mut state = ScratchState::new(vec![(-1.0).into()]);
    state.register_target(0, "Stage", true);
    state.register_target(1, "Sprite1", false);
    let clone = state.create_clone(1, 1, 300).unwrap();

    let mut scheduler = ScriptScheduler::new(Vec::<EventHatScript>::new());
    dispatch_pointer_event(
        &mut scheduler,
        RuntimeInputEvent::PointerButton {
            pressed: true,
            inside_stage: true,
            target_instance: None,
            draggable: false,
            was_dragged: false,
            monitor_interaction: false,
        },
    );
    scheduler.advance(&mut state);
    assert_eq!(state.get_shared(0), &ScratchValue::Number(0.0));

    let mut scheduler = ScriptScheduler::new(Vec::<EventHatScript>::new());
    dispatch_pointer_event(
        &mut scheduler,
        RuntimeInputEvent::PointerButton {
            pressed: true,
            inside_stage: true,
            target_instance: Some(1),
            draggable: false,
            was_dragged: false,
            monitor_interaction: false,
        },
    );
    scheduler.advance(&mut state);
    assert_eq!(state.get_shared(0), &ScratchValue::Number(1.0));

    let mut scheduler = ScriptScheduler::new(Vec::<EventHatScript>::new());
    dispatch_pointer_event(
        &mut scheduler,
        RuntimeInputEvent::PointerButton {
            pressed: true,
            inside_stage: true,
            target_instance: Some(clone),
            draggable: false,
            was_dragged: false,
            monitor_interaction: false,
        },
    );
    scheduler.advance(&mut state);
    assert_eq!(state.get_shared(0), &ScratchValue::Number(clone as f64));
}

#[test]
fn deleting_a_clone_removes_its_other_active_scripts() {
    let mut scheduler =
        ScriptScheduler::new(vec![TargetScript(1), TargetScript(2), TargetScript(2)]);
    scheduler.remove_target_scripts(2);
    assert_eq!(scheduler.active_scripts(), 1);
    assert_eq!(
        scheduler.advance(&mut ScratchState::new(Vec::new())),
        super::SchedulerStatus::Progressed
    );
    assert_eq!(scheduler.active_scripts(), 0);
}

#[test]
fn clone_warp_custom_call_waits_without_losing_instance_or_frames() {
    let mut state = ScratchState::with_metadata(
        vec![0.0.into(), 10.0.into()],
        vec![
            ScratchVariableMetadata::for_target("stage", "shared", "Stage", 0, true),
            ScratchVariableMetadata::for_target("local", "local", "Sprite1", 1, false),
        ],
    );
    state.register_target(0, "Stage", true);
    state.register_target(1, "Sprite1", false);
    let original = state.original_instance(1).unwrap();
    let first = state.create_clone(original, 1, 300).unwrap();
    let second = state.create_clone(original, 1, 300).unwrap();
    let clock = ManualClock::new();
    let mut scheduler = ScriptScheduler::with_clock(
        vec![
            CloneWaitRunner::Clone(CloneWaitScript {
                instance: first,
                phase: 0,
                frames: vec![CloneWaitFrame {
                    pc: 0,
                    warp_mode: false,
                    arguments: Vec::new(),
                }],
            }),
            CloneWaitRunner::Clone(CloneWaitScript {
                instance: second,
                phase: 0,
                frames: vec![CloneWaitFrame {
                    pc: 0,
                    warp_mode: false,
                    arguments: Vec::new(),
                }],
            }),
            CloneWaitRunner::StageWriter,
        ],
        Box::new(clock.clone()),
    );

    assert_eq!(
        scheduler.advance(&mut state),
        super::SchedulerStatus::Progressed
    );
    assert_eq!(
        scheduler.advance(&mut state),
        super::SchedulerStatus::Progressed
    );
    assert_eq!(
        scheduler.advance(&mut state),
        super::SchedulerStatus::Progressed
    );
    assert_eq!(state.get(0), &ScratchValue::Number(100.0));
    assert_eq!(
        scheduler.advance(&mut state),
        super::SchedulerStatus::WaitingUntil(20)
    );

    clock.set(20);
    assert_eq!(
        scheduler.advance(&mut state),
        super::SchedulerStatus::Progressed
    );
    assert_eq!(
        scheduler.advance(&mut state),
        super::SchedulerStatus::Progressed
    );
    state.set_current_instance(original);
    assert_eq!(state.get(1), &ScratchValue::Number(10.0));
    state.set_current_instance(first);
    assert_eq!(state.get(1), &ScratchValue::Number(114.0));
    state.set_current_instance(second);
    assert_eq!(state.get(1), &ScratchValue::Number(115.0));
    assert_eq!(state.get(0), &ScratchValue::Number(102.0));
}

#[test]
fn deleting_a_receiving_clone_releases_its_waiting_sender() {
    let mut state = ScratchState::with_metadata(
        vec![0.0.into(), 0.0.into(), false.into()],
        vec![
            ScratchVariableMetadata::for_target("shared", "shared", "Stage", 0, true),
            ScratchVariableMetadata::for_target("local", "local", "Sprite1", 1, false),
            ScratchVariableMetadata::for_target("done", "done", "Stage", 0, true),
        ],
    );
    state.register_target(0, "Stage", true);
    state.register_target(1, "Sprite1", false);
    let original = state.original_instance(1).unwrap();
    let clone = state.create_clone(original, 1, 300).unwrap();
    let mut scheduler = ScriptScheduler::new(vec![BroadcastTestScript::Sender { pc: 0 }]);

    assert_eq!(
        scheduler.advance(&mut state),
        super::SchedulerStatus::Progressed
    );
    let (sender, _, wait) = scheduler.take_broadcast().unwrap();
    scheduler.broadcast(
        sender,
        vec![BroadcastTestScript::Receiver {
            hat: 1,
            instance: clone,
            pc: 0,
        }],
        wait,
    );
    scheduler.remove_target_scripts(clone);
    assert_eq!(
        scheduler.advance(&mut state),
        super::SchedulerStatus::Progressed
    );
    assert_eq!(
        scheduler.advance(&mut state),
        super::SchedulerStatus::Complete
    );
}
