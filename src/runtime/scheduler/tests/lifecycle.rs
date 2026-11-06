use super::*;

#[test]
fn edge_hat_threads_do_not_duplicate_and_clicks_restart_the_same_target_hat() {
    let mut state = ScratchState::new(vec![0.0.into()]);
    let mut scheduler = ScriptScheduler::new(Vec::<EventHatScript>::new());
    let edge = || EventHatScript {
        key: (2, 0),
        edge: true,
        record_instance: false,
        steps: 0,
    };
    scheduler.start_edge_hats([edge()]);
    scheduler.start_edge_hats([edge()]);
    assert_eq!(scheduler.active_scripts(), 1);
    scheduler.advance(&mut state);
    scheduler.advance(&mut state);
    assert_eq!(scheduler.active_scripts(), 0);

    let mut scheduler = ScriptScheduler::new(Vec::<EventHatScript>::new());
    let click = || EventHatScript {
        key: (3, 0),
        edge: false,
        record_instance: false,
        steps: 0,
    };
    scheduler.start_click_hats([click()]);
    scheduler.advance(&mut state);
    assert_eq!(state.get(0), &ScratchValue::Number(3.0));
    scheduler.start_click_hats([click()]);
    assert_eq!(scheduler.active_scripts(), 1);
    scheduler.advance(&mut state);
    assert_eq!(state.get(0), &ScratchValue::Number(4.0));
}

#[test]
fn stop_other_scripts_only_stops_the_current_target_and_continues_the_stopper() {
    let mut state = ScratchState::new(vec![ScratchValue::Number(0.0), ScratchValue::Number(0.0)]);
    let clone = state.create_clone(0, 1, 300).unwrap();
    let scripts = vec![
        StopFixtureScript::Others { target: 0, pc: 0 },
        StopFixtureScript::Write {
            target: 0,
            slot: 0,
            value: 99.0,
        },
        StopFixtureScript::Write {
            target: clone,
            slot: 1,
            value: 2.0,
        },
    ];
    let mut scheduler = ScriptScheduler::with_clock(
        scripts,
        Box::new(CountingClock(std::rc::Rc::new(std::cell::Cell::new(0)))),
    );

    assert_eq!(
        scheduler.advance(&mut state),
        super::SchedulerStatus::Progressed
    );
    assert_eq!(scheduler.active_scripts(), 1);
    assert_eq!(state.get_shared(0), &ScratchValue::Number(1.0));
    assert_eq!(
        scheduler.advance(&mut state),
        super::SchedulerStatus::Progressed
    );
    assert_eq!(state.get_shared(1), &ScratchValue::Number(2.0));
    assert_eq!(scheduler.active_scripts(), 0);
}

#[test]
fn resumes_green_flag_scripts_with_live_shared_variables() {
    let mut state = ScratchState::new(vec![
        ScratchValue::Number(0.0),
        ScratchValue::Number(0.0),
        ScratchValue::Number(0.0),
        ScratchValue::Number(0.0),
    ]);
    let scripts = vec![
        FixtureScript::First(FirstScript { pc: 0 }),
        FixtureScript::Second(SecondScript { pc: 0 }),
    ];
    let mut scheduler = ScriptScheduler::new(scripts);

    assert_eq!(scheduler.active_scripts(), 2);
    scheduler.run_until_complete(&mut state);

    assert_eq!(scheduler.active_scripts(), 0);
    assert_eq!(state.get(0), &ScratchValue::Boolean(false));
    assert_eq!(state.get(1), &ScratchValue::Number(1.0));
    assert_eq!(state.get(2), &ScratchValue::Number(1.0));
    assert_eq!(state.get(3), &ScratchValue::Number(1.0));
    scheduler.run_until_complete(&mut state);
    assert_eq!(state.get(1), &ScratchValue::Number(1.0));
    assert_eq!(state.get(2), &ScratchValue::Number(1.0));
    assert_eq!(state.get(3), &ScratchValue::Number(1.0));
}

#[test]
fn restart_drops_active_frames_and_starts_fresh_scripts_with_live_state() {
    let mut state = number_state(&[4.0]);
    let mut scheduler = ScriptScheduler::new(vec![RestartScript { finish: false }]);
    assert_eq!(
        scheduler.advance(&mut state),
        super::SchedulerStatus::Progressed
    );
    assert_eq!(state.get(0), &ScratchValue::Number(100.0));

    scheduler.restart(vec![RestartScript { finish: true }]);
    assert_eq!(scheduler.active_scripts(), 1);
    assert_eq!(
        scheduler.advance(&mut state),
        super::SchedulerStatus::Progressed
    );
    assert_eq!(state.get(0), &ScratchValue::Number(101.0));
    assert_eq!(scheduler.active_scripts(), 0);
}
