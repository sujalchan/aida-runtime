use super::*;

#[test]
fn stop_all_clears_scripts_clones_and_broadcast_and_wait_joins() {
    let mut state = ScratchState::new(vec![]);
    state.register_target(0, "sprite", false);
    state.register_target(1, "clone target", false);
    state.set_speech_bubble(BubbleKind::Say, "hello".to_owned(), None);
    let clone = state.create_clone(0, 1, 300).unwrap();
    state.set_current_instance(clone);
    state.set_speech_bubble(BubbleKind::Think, "hmm".to_owned(), None);
    state.begin_tick();
    let scripts = vec![
        StopFixtureScript::Broadcast { target: 0 },
        StopFixtureScript::All { target: 0 },
    ];
    let mut scheduler = ScriptScheduler::with_clock(
        scripts,
        Box::new(CountingClock(std::rc::Rc::new(std::cell::Cell::new(0)))),
    );

    assert_eq!(
        scheduler.advance(&mut state),
        super::SchedulerStatus::Progressed
    );
    let (sender, message, wait) = scheduler.take_broadcast().unwrap();
    assert!(wait);
    scheduler.broadcast(
        sender,
        vec![StopFixtureScript::Receiver { target: clone }],
        wait,
    );
    assert_eq!(scheduler.active_scripts(), 3);
    assert_eq!(
        scheduler.advance(&mut state),
        super::SchedulerStatus::Complete
    );
    assert_eq!(scheduler.active_scripts(), 0);
    assert_eq!(state.clone_count(), 0);
    assert!(state.take_pending_clones().is_empty());
    assert!(state.redraw_requested());
    assert!(
        state
            .observable_snapshot(0)
            .layers
            .iter()
            .all(|layer| layer.bubble.is_none())
    );
    assert!(scheduler.take_broadcast().is_none());
    assert_eq!(message, 7);
}

#[test]
fn broadcast_and_wait_joins_multiple_waiting_receivers_on_original_and_clone() {
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
    let clock = ManualClock::new();
    let mut scheduler = ScriptScheduler::with_clock(
        vec![BroadcastTestScript::Sender { pc: 0 }],
        Box::new(clock.clone()),
    );

    assert_eq!(
        scheduler.advance(&mut state),
        super::SchedulerStatus::Progressed
    );
    let (sender, message, wait) = scheduler.take_broadcast().unwrap();
    assert_eq!(message, 1);
    scheduler.broadcast(
        sender,
        vec![
            BroadcastTestScript::Receiver {
                hat: 1,
                instance: original,
                pc: 0,
            },
            BroadcastTestScript::Receiver {
                hat: 1,
                instance: clone,
                pc: 0,
            },
        ],
        wait,
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
        super::SchedulerStatus::WaitingUntil(5)
    );

    clock.set(5);
    while scheduler.advance(&mut state) != super::SchedulerStatus::Complete {}
    assert_eq!(state.get(0), &ScratchValue::Number(2.0));
    assert_eq!(state.get(2), &ScratchValue::Boolean(true));
    state.set_current_instance(original);
    assert_eq!(state.get(1), &ScratchValue::Number(1.0));
    state.set_current_instance(clone);
    assert_eq!(state.get(1), &ScratchValue::Number(1.0));
}

#[test]
fn broadcast_and_wait_with_no_receivers_does_not_suspend_the_sender() {
    let mut state = ScratchState::new(vec![0.0.into(), 0.0.into(), false.into()]);
    let mut scheduler = ScriptScheduler::new(vec![BroadcastTestScript::Sender { pc: 0 }]);
    assert_eq!(
        scheduler.advance(&mut state),
        super::SchedulerStatus::Progressed
    );
    let (sender, _, wait) = scheduler.take_broadcast().unwrap();
    scheduler.broadcast(sender, Vec::new(), wait);
    assert_eq!(
        scheduler.advance(&mut state),
        super::SchedulerStatus::Progressed
    );
    assert_eq!(
        scheduler.advance(&mut state),
        super::SchedulerStatus::Complete
    );
}

#[test]
fn restarting_a_receiver_releases_waiters_holding_its_previous_thread() {
    let mut state = ScratchState::new(vec![0.0.into(), 0.0.into(), false.into()]);
    let mut scheduler = ScriptScheduler::with_clock(
        vec![
            BroadcastTestScript::Sender { pc: 0 },
            BroadcastTestScript::Receiver {
                hat: 1,
                instance: 0,
                pc: 0,
            },
            BroadcastTestScript::Rebroadcast { pc: 0 },
        ],
        Box::new(ManualClock::new()),
    );

    assert_eq!(
        scheduler.advance(&mut state),
        super::SchedulerStatus::Progressed
    );
    let (sender, _, wait) = scheduler.take_broadcast().unwrap();
    scheduler.broadcast(
        sender,
        vec![BroadcastTestScript::Receiver {
            hat: 1,
            instance: 0,
            pc: 0,
        }],
        wait,
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
    let (rebroadcaster, _, wait) = scheduler.take_broadcast().unwrap();
    assert!(!wait);
    scheduler.broadcast(
        rebroadcaster,
        vec![BroadcastTestScript::Receiver {
            hat: 1,
            instance: 0,
            pc: 0,
        }],
        false,
    );

    assert_eq!(
        scheduler.advance(&mut state),
        super::SchedulerStatus::Progressed
    );
    assert_eq!(state.get(2), &ScratchValue::Boolean(true));
}
