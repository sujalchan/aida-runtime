use super::*;

#[test]
fn runnable_scripts_do_not_read_the_clock_when_no_script_is_waiting() {
    let clock = CountingClock(std::rc::Rc::new(std::cell::Cell::new(0)));
    let clock_view = clock.clone();
    let scripts = vec![FixtureScript::First(FirstScript { pc: 0 })];
    let mut scheduler = ScriptScheduler::with_clock(scripts, Box::new(clock));
    let mut state = ScratchState::new(vec![
        ScratchValue::Number(0.0),
        ScratchValue::Number(0.0),
        ScratchValue::Number(0.0),
        ScratchValue::Number(0.0),
    ]);

    assert_eq!(
        scheduler.advance(&mut state),
        super::SchedulerStatus::Progressed
    );
    assert_eq!(clock_view.0.get(), 0);
}

#[test]
fn scanning_multiple_sleepers_reads_the_clock_once() {
    let clock = ControlledClock {
        calls: std::rc::Rc::new(std::cell::Cell::new(0)),
        now: std::rc::Rc::new(std::cell::Cell::new(0)),
    };
    let clock_view = clock.clone();
    let first_steps = std::rc::Rc::new(std::cell::Cell::new(0));
    let second_steps = std::rc::Rc::new(std::cell::Cell::new(0));
    let scripts = vec![
        ClockFixtureScript::Deadline(DeadlineScript {
            deadline: 10,
            steps: first_steps.clone(),
        }),
        ClockFixtureScript::Deadline(DeadlineScript {
            deadline: 20,
            steps: second_steps.clone(),
        }),
    ];
    let mut scheduler = ScriptScheduler::with_clock(scripts, Box::new(clock));
    let mut state = ScratchState::new(vec![]);

    assert_eq!(
        scheduler.advance(&mut state),
        super::SchedulerStatus::Progressed
    );
    assert_eq!(
        scheduler.advance(&mut state),
        super::SchedulerStatus::Progressed
    );
    assert_eq!(clock_view.calls.get(), 0);
    assert_eq!(
        scheduler.advance(&mut state),
        super::SchedulerStatus::WaitingUntil(10)
    );
    assert_eq!(clock_view.calls.get(), 1);
    assert_eq!(first_steps.get(), 1);
    assert_eq!(second_steps.get(), 1);
}

#[test]
fn sleeping_script_uses_a_fresh_clock_read_after_another_script_runs() {
    let clock = ControlledClock {
        calls: std::rc::Rc::new(std::cell::Cell::new(0)),
        now: std::rc::Rc::new(std::cell::Cell::new(0)),
    };
    let clock_view = clock.clone();
    let completed_waiter_steps = std::rc::Rc::new(std::cell::Cell::new(0));
    let scripts = vec![
        ClockFixtureScript::Deadline(DeadlineScript {
            deadline: 5,
            steps: completed_waiter_steps.clone(),
        }),
        ClockFixtureScript::Advancing(ClockAdvancingScript {
            clock,
            values: vec![1, 5],
            steps: 0,
        }),
    ];
    let mut scheduler = ScriptScheduler::with_clock(scripts, Box::new(clock_view.clone()));
    let mut state = ScratchState::new(vec![]);

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
    assert_eq!(clock_view.calls.get(), 1);
    assert_eq!(completed_waiter_steps.get(), 1);
    assert_eq!(
        scheduler.advance(&mut state),
        super::SchedulerStatus::Progressed
    );
    assert_eq!(clock_view.calls.get(), 2);
    assert_eq!(completed_waiter_steps.get(), 2);
}

#[test]
fn frame_order_runs_active_scripts_before_a_script_at_its_wake_deadline() {
    let now = std::rc::Rc::new(std::cell::Cell::new(0));
    let clock = ControlledClock {
        calls: std::rc::Rc::new(std::cell::Cell::new(0)),
        now: now.clone(),
    };
    let mut scheduler = ScriptScheduler::with_clock(
        vec![
            FrameOrderScript::Wait { started: false },
            FrameOrderScript::Runnable,
        ],
        Box::new(clock),
    );
    let mut state = ScratchState::new(vec![]);

    assert_eq!(
        scheduler.advance_script_id(0, &mut state),
        Some(super::SchedulerStatus::Progressed)
    );
    assert_eq!(scheduler.script_ids_in_frame_order(), vec![1, 0]);
    now.set(9);
    assert_eq!(scheduler.advance_script_id(0, &mut state), None);
    assert_eq!(
        scheduler.advance_script_id(1, &mut state),
        Some(super::SchedulerStatus::Progressed)
    );
    now.set(10);
    assert_eq!(
        scheduler.advance_script_id(0, &mut state),
        Some(super::SchedulerStatus::Progressed)
    );
}

#[test]
fn scheduler_reports_only_scripts_added_since_the_last_frame_scan() {
    let steps = std::rc::Rc::new(std::cell::Cell::new(0));
    let script = || DeadlineScript {
        deadline: 10,
        steps: steps.clone(),
    };
    let mut scheduler = ScriptScheduler::new(vec![script()]);

    assert_eq!(scheduler.next_script_id(), 1);
    scheduler.add_scripts(vec![script()]);
    let mut ids = Vec::new();
    scheduler.append_new_script_ids_since(&mut ids, 1);
    assert_eq!(ids, vec![1]);
    scheduler.append_new_script_ids_since(&mut ids, 2);
    assert_eq!(ids, vec![1]);
}

#[test]
fn frame_order_can_reuse_its_buffer_between_scheduler_rounds() {
    let steps = std::rc::Rc::new(std::cell::Cell::new(0));
    let scripts = (0..2)
        .map(|_| DeadlineScript {
            deadline: 10,
            steps: steps.clone(),
        })
        .collect();
    let scheduler = ScriptScheduler::new(scripts);
    let mut frame_order = Vec::with_capacity(2);

    scheduler.script_ids_in_frame_order_into(&mut frame_order);
    assert_eq!(frame_order, vec![0, 1]);
    let capacity = frame_order.capacity();
    scheduler.script_ids_in_frame_order_into(&mut frame_order);
    assert_eq!(frame_order, vec![0, 1]);
    assert_eq!(frame_order.capacity(), capacity);
}

#[test]
fn paced_frame_rate_controls_work_completed_before_a_wait_resumes() {
    for (ticks_per_second, expected) in [(30_usize, 31.0), (40, 41.0)] {
        let now = std::rc::Rc::new(std::cell::Cell::new(0));
        let clock = ControlledClock {
            calls: std::rc::Rc::new(std::cell::Cell::new(0)),
            now: now.clone(),
        };
        let mut scheduler = ScriptScheduler::with_clock(
            vec![
                FrameRateScript::Wait { started: false },
                FrameRateScript::Counter,
            ],
            Box::new(clock),
        );
        let mut state = ScratchState::new(vec![0.0_f64.into(), (-1.0_f64).into()]);
        let frame_units = 120 / ticks_per_second;

        for frame in (0..=120).step_by(frame_units) {
            now.set(frame as u64);
            for id in scheduler.script_ids_in_frame_order() {
                scheduler.advance_script_id(id, &mut state);
            }
            if state.get_shared(1) != &ScratchValue::Number(-1.0) {
                break;
            }
        }

        assert_eq!(state.get_shared(1), &ScratchValue::Number(expected));
    }
}

#[test]
fn zero_duration_wait_yields_once_to_the_next_script() {
    let clock = ManualClock::new();
    let mut state = number_state(&[0.0, 0.0]);
    let scripts = vec![
        WaitScript {
            slot: 0,
            duration: 0,
            phase: 0,
        },
        WaitScript {
            slot: 1,
            duration: 0,
            phase: 1,
        },
    ];
    let mut scheduler = ScriptScheduler::with_clock(scripts, Box::new(clock));

    assert_eq!(
        scheduler.advance(&mut state),
        super::SchedulerStatus::Progressed
    );
    assert_eq!(state.get(0), &ScratchValue::Number(1.0));
    assert_eq!(state.get(1), &ScratchValue::Number(0.0));
    assert_eq!(
        scheduler.advance(&mut state),
        super::SchedulerStatus::Progressed
    );
    assert_eq!(state.get(0), &ScratchValue::Number(1.0));
    assert_eq!(state.get(1), &ScratchValue::Number(10.0));
    assert_eq!(
        scheduler.advance(&mut state),
        super::SchedulerStatus::Progressed
    );
    assert_eq!(state.get(0), &ScratchValue::Number(11.0));
}

#[test]
fn positive_wait_reports_a_deadline_without_polling() {
    let clock = ManualClock::new();
    let mut state = number_state(&[0.0]);
    let mut scheduler = ScriptScheduler::with_clock(
        vec![WaitScript {
            slot: 0,
            duration: 250,
            phase: 0,
        }],
        Box::new(clock.clone()),
    );

    assert_eq!(
        scheduler.advance(&mut state),
        super::SchedulerStatus::Progressed
    );
    assert_eq!(
        scheduler.advance(&mut state),
        super::SchedulerStatus::WaitingUntil(250)
    );
    clock.set(249);
    assert_eq!(
        scheduler.advance(&mut state),
        super::SchedulerStatus::WaitingUntil(250)
    );
    assert_eq!(state.get(0), &ScratchValue::Number(1.0));
    clock.set(250);
    assert_eq!(
        scheduler.advance(&mut state),
        super::SchedulerStatus::Progressed
    );
    assert_eq!(state.get(0), &ScratchValue::Number(11.0));
    assert_eq!(scheduler.active_scripts(), 0);
}

#[test]
fn multiple_waits_report_the_earliest_wake_time() {
    let clock = ManualClock::new();
    let mut state = number_state(&[0.0, 0.0]);
    let mut scheduler = ScriptScheduler::with_clock(
        vec![
            WaitScript {
                slot: 0,
                duration: 80,
                phase: 0,
            },
            WaitScript {
                slot: 1,
                duration: 160,
                phase: 0,
            },
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
        super::SchedulerStatus::WaitingUntil(80)
    );
    clock.set(80);
    assert_eq!(
        scheduler.advance(&mut state),
        super::SchedulerStatus::Progressed
    );
    assert_eq!(state.get(0), &ScratchValue::Number(11.0));
    assert_eq!(state.get(1), &ScratchValue::Number(1.0));
}
