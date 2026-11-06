use super::*;

#[test]
fn repeat_zero_and_positive_counts_yield_once_per_body_iteration() {
    let mut state = number_state(&[0.0, 0.0]);
    let scripts = vec![
        LoopScript {
            role: LoopRole::Repeat {
                count: RepeatCount::Fixed(0),
                remaining: None,
                hits_slot: 0,
            },
        },
        LoopScript {
            role: LoopRole::Repeat {
                count: RepeatCount::Fixed(2),
                remaining: None,
                hits_slot: 1,
            },
        },
    ];
    let mut scheduler = ScriptScheduler::new(scripts);

    scheduler.run_until_complete(&mut state);

    assert_eq!(state.get(0), &ScratchValue::Number(0.0));
    assert_eq!(state.get(1), &ScratchValue::Number(2.0));
}

#[test]
fn repeat_keeps_its_entry_count_when_the_source_variable_changes() {
    let mut state = ScratchState::new(vec![
        ScratchValue::Text("3".to_owned()),
        ScratchValue::Number(0.0),
    ]);
    let scripts = vec![
        LoopScript {
            role: LoopRole::Repeat {
                count: RepeatCount::Variable(0),
                remaining: None,
                hits_slot: 1,
            },
        },
        LoopScript {
            role: LoopRole::Write {
                slot: 0,
                value: ScratchValue::Boolean(false),
            },
        },
    ];
    let mut scheduler = ScriptScheduler::new(scripts);

    scheduler.run_until_complete(&mut state);

    assert_eq!(state.get(0), &ScratchValue::Boolean(false));
    assert_eq!(state.get(1), &ScratchValue::Number(3.0));
}

#[test]
fn repeat_until_rechecks_a_condition_changed_by_another_script() {
    let mut state = ScratchState::new(vec![
        ScratchValue::Boolean(false),
        ScratchValue::Number(0.0),
    ]);
    let scripts = vec![
        LoopScript {
            role: LoopRole::RepeatUntil {
                condition_slot: 0,
                hits_slot: 1,
            },
        },
        LoopScript {
            role: LoopRole::Write {
                slot: 0,
                value: ScratchValue::Boolean(true),
            },
        },
    ];
    let mut scheduler = ScriptScheduler::new(scripts);

    scheduler.run_until_complete(&mut state);

    assert_eq!(state.get(1), &ScratchValue::Number(1.0));
    assert_eq!(state.get(0), &ScratchValue::Boolean(true));
}

#[test]
fn nested_repeat_progress_survives_each_yield() {
    let mut state = number_state(&[0.0, 0.0]);
    let scripts = vec![LoopScript {
        role: LoopRole::NestedRepeat {
            outer_remaining: 2,
            inner_remaining: 3,
            outer_hits_slot: 0,
            inner_hits_slot: 1,
            done: false,
        },
    }];
    let mut scheduler = ScriptScheduler::new(scripts);

    scheduler.run_until_complete(&mut state);

    assert_eq!(state.get(0), &ScratchValue::Number(2.0));
    assert_eq!(state.get(1), &ScratchValue::Number(6.0));
}

#[test]
fn forever_yields_so_another_green_flag_script_can_finish() {
    let mut state = ScratchState::new(vec![
        ScratchValue::Number(0.0),
        ScratchValue::Boolean(false),
    ]);
    let scripts = vec![
        LoopScript {
            role: LoopRole::Forever { hits_slot: 0 },
        },
        LoopScript {
            role: LoopRole::Observe {
                source_slot: 0,
                observed_slot: 1,
            },
        },
    ];
    let mut scheduler = ScriptScheduler::new(scripts);

    assert!(scheduler.step(&mut state));
    assert_eq!(state.get(0), &ScratchValue::Number(1.0));
    assert!(scheduler.step(&mut state));
    assert_eq!(state.get(1), &ScratchValue::Boolean(true));
    assert_eq!(scheduler.active_scripts(), 1);
}

#[test]
fn finite_warp_work_finishes_before_another_script_runs() {
    let state = &mut number_state(&[0.0, 0.0]);
    let scripts = vec![
        WarpScript(WarpRole::Finite {
            iterations: 4,
            budget: WarpBudget::new(500),
        }),
        WarpScript(WarpRole::Observer { seen: 0 }),
    ];
    let mut scheduler =
        ScriptScheduler::with_clock(scripts, Box::new(TestClock::advancing_by(100)));

    assert!(scheduler.step(state));
    assert_eq!(state.get(0), &ScratchValue::Number(4.0));
    assert_eq!(state.get(1), &ScratchValue::Number(0.0));
    assert!(scheduler.step(state));
    assert_eq!(state.get(1), &ScratchValue::Number(1.0));
}

#[test]
fn warp_budget_yields_an_infinite_loop_and_keeps_its_progress() {
    let state = &mut number_state(&[0.0, 0.0]);
    let scripts = vec![
        WarpScript(WarpRole::Infinite {
            iterations: 0,
            budget: WarpBudget::new(500),
        }),
        WarpScript(WarpRole::Observer { seen: 0 }),
    ];
    let mut scheduler =
        ScriptScheduler::with_clock(scripts, Box::new(TestClock::advancing_by(501)));

    assert!(scheduler.step(state));
    // the clock is checked at the first safe point and then every 1024.
    assert_eq!(state.get(0), &ScratchValue::Number(1025.0));
    assert!(scheduler.step(state));
    assert_eq!(state.get(1), &ScratchValue::Number(1.0));
    assert_eq!(scheduler.active_scripts(), 1);
    assert!(scheduler.step(state));
    assert_eq!(state.get(0), &ScratchValue::Number(2050.0));
}

#[test]
fn nested_procedures_inherit_warp_and_normal_calls_resume_after_yield() {
    let state = &mut number_state(&[0.0, 0.0, 0.0]);
    let scripts = vec![
        CallRunner::Call(CallFixture {
            enter_warp: true,
            frames: vec![CallFrame {
                kind: CallKind::Root,
                pc: 0,
                remaining: 0,
                warp_mode: false,
            }],
        }),
        CallRunner::Observer(WarpScript(WarpRole::Observer { seen: 0 })),
    ];
    let mut scheduler =
        ScriptScheduler::with_clock(scripts, Box::new(TestClock::advancing_by(100)));

    assert!(scheduler.step(state));
    assert_eq!(state.get(0), &ScratchValue::Number(2.0));
    assert_eq!(state.get(1), &ScratchValue::Number(0.0));
    assert_eq!(state.get(2), &ScratchValue::Boolean(true));
    assert!(scheduler.step(state));
    assert_eq!(state.get(1), &ScratchValue::Number(1.0));

    let state = &mut number_state(&[0.0, 0.0, 0.0]);
    let scripts = vec![
        CallRunner::Call(CallFixture {
            enter_warp: false,
            frames: vec![CallFrame {
                kind: CallKind::Root,
                pc: 0,
                remaining: 0,
                warp_mode: false,
            }],
        }),
        CallRunner::Observer(WarpScript(WarpRole::Observer { seen: 0 })),
    ];
    let mut scheduler =
        ScriptScheduler::with_clock(scripts, Box::new(TestClock::advancing_by(100)));

    assert!(scheduler.step(state));
    assert_eq!(state.get(0), &ScratchValue::Number(1.0));
    assert_eq!(state.get(2), &ScratchValue::Number(0.0));
    assert!(scheduler.step(state));
    assert_eq!(state.get(1), &ScratchValue::Number(1.0));
    assert!(scheduler.step(state));
    assert_eq!(state.get(0), &ScratchValue::Number(2.0));
    assert_eq!(state.get(2), &ScratchValue::Boolean(true));
}
