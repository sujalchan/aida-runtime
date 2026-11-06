use super::*;

#[test]
fn edge_hat_state_fires_once_per_rising_edge_and_tracks_each_instance() {
    let mut edges = EdgeHatState::default();
    assert!(edges.rising_edge(4, 10, true));
    assert!(!edges.rising_edge(4, 10, true));
    assert!(!edges.rising_edge(4, 10, false));
    assert!(edges.rising_edge(4, 10, true));
    assert!(edges.rising_edge(4, 11, true));

    edges.retain_live_instances(|instance| instance == 10);
    assert!(!edges.rising_edge(4, 10, true));
    assert!(edges.rising_edge(4, 11, true));
    edges.clear();
    assert!(edges.rising_edge(4, 10, true));
}

#[test]
fn external_audio_wait_suspends_only_its_thread_and_resumes_on_its_signal() {
    let mut state = ScratchState::new(vec![0.0.into(), 0.0.into()]);
    let mut scheduler = ScriptScheduler::new(vec![
        SignalWaitScript {
            waiting: true,
            steps: 0,
            slot: 0,
        },
        SignalWaitScript {
            waiting: false,
            steps: 0,
            slot: 1,
        },
    ]);

    assert_eq!(
        scheduler.advance(&mut state),
        super::SchedulerStatus::Progressed
    );
    assert_eq!(state.get(0), &ScratchValue::Number(1.0));
    assert_eq!(
        scheduler.advance(&mut state),
        super::SchedulerStatus::Progressed
    );
    assert_eq!(state.get(1), &ScratchValue::Number(1.0));
    assert_eq!(
        scheduler.advance(&mut state),
        super::SchedulerStatus::WaitingForSignal
    );
    assert_eq!(scheduler.active_scripts(), 1);
    assert_eq!(
        scheduler.idle_status(),
        super::SchedulerStatus::WaitingForSignal
    );

    scheduler.signal(91);
    assert_eq!(
        scheduler.advance(&mut state),
        super::SchedulerStatus::Progressed
    );
    assert_eq!(state.get(0), &ScratchValue::Number(2.0));
    assert_eq!(scheduler.active_scripts(), 0);
    assert_eq!(scheduler.idle_status(), super::SchedulerStatus::Complete);
}

#[test]
fn timer_edge_can_start_after_idle_and_rearms_after_a_timer_reset() {
    let mut state = ScratchState::new(vec![0.0.into()]);
    state.reset_project_timer(500);
    let mut edges = EdgeHatState::default();
    let mut scheduler = ScriptScheduler::new(Vec::<EventHatScript>::new());
    assert_eq!(scheduler.idle_status(), super::SchedulerStatus::Complete);

    let poll_timer = |now_millis,
                      state: &ScratchState,
                      edges: &mut EdgeHatState,
                      scheduler: &mut ScriptScheduler<EventHatScript>| {
        let active = state.project_timer_seconds(now_millis) > 10.0;
        if edges.rising_edge(9, 0, active) {
            scheduler.start_edge_hats([EventHatScript {
                key: (9, 0),
                edge: true,
                record_instance: false,
                steps: 0,
            }]);
        }
    };
    poll_timer(10_500, &state, &mut edges, &mut scheduler);
    assert_eq!(scheduler.active_scripts(), 0);
    poll_timer(10_501, &state, &mut edges, &mut scheduler);
    assert_eq!(scheduler.active_scripts(), 1);
    scheduler.advance(&mut state);
    scheduler.advance(&mut state);
    assert_eq!(scheduler.active_scripts(), 0);
    poll_timer(10_502, &state, &mut edges, &mut scheduler);
    assert_eq!(scheduler.active_scripts(), 0);

    state.reset_project_timer(11_000);
    poll_timer(11_000, &state, &mut edges, &mut scheduler);
    assert_eq!(scheduler.active_scripts(), 0);
    poll_timer(21_001, &state, &mut edges, &mut scheduler);
    assert_eq!(scheduler.active_scripts(), 1);
}
