use super::{Clock, ResumableScript, ScriptScheduler, ScriptStatus, WarpBudget};
use crate::runtime::{
    ScratchState, ScratchValue, ScratchVariableMetadata, scratch_bool, scratch_round,
    scratch_to_number,
};

struct FirstScript {
    pc: u8,
}

impl ResumableScript for FirstScript {
    fn step(&mut self, state: &mut ScratchState, _clock: &dyn super::Clock) -> ScriptStatus {
        match self.pc {
            0 => {
                let starts = state.get(1).clone();
                state.set(1, crate::runtime::scratch_add(&starts, 1.0_f64).into());
                state.set(0, 1.0_f64.into());
                self.pc = 1;
                ScriptStatus::Yield
            }
            1 => {
                assert_eq!(state.get(0), &ScratchValue::Text("2".to_owned()));
                state.set(0, ScratchValue::Boolean(false));
                self.pc = 2;
                ScriptStatus::Complete
            }
            _ => panic!("completed script was stepped again"),
        }
    }
}

struct SecondScript {
    pc: u8,
}

impl ResumableScript for SecondScript {
    fn step(&mut self, state: &mut ScratchState, _clock: &dyn super::Clock) -> ScriptStatus {
        match self.pc {
            0 => {
                let starts = state.get(2).clone();
                state.set(2, crate::runtime::scratch_add(&starts, 1.0_f64).into());
                assert_eq!(state.get(0), &ScratchValue::Number(1.0));
                state.set(0, ScratchValue::Text("2".to_owned()));
                self.pc = 1;
                ScriptStatus::Yield
            }
            1 => {
                assert_eq!(state.get(0), &ScratchValue::Boolean(false));
                let resumes = state.get(3).clone();
                state.set(3, crate::runtime::scratch_add(&resumes, 1.0_f64).into());
                self.pc = 2;
                ScriptStatus::Complete
            }
            _ => panic!("completed script was stepped again"),
        }
    }
}

enum FixtureScript {
    First(FirstScript),
    Second(SecondScript),
}

impl ResumableScript for FixtureScript {
    fn step(&mut self, state: &mut ScratchState, clock: &dyn super::Clock) -> ScriptStatus {
        match self {
            Self::First(script) => script.step(state, clock),
            Self::Second(script) => script.step(state, clock),
        }
    }
}

#[derive(Clone)]
struct CountingClock(std::rc::Rc<std::cell::Cell<usize>>);

impl Clock for CountingClock {
    fn now_millis(&self) -> u64 {
        self.0.set(self.0.get() + 1);
        0
    }
}

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

enum LoopRole {
    Repeat {
        count: RepeatCount,
        remaining: Option<usize>,
        hits_slot: usize,
    },
    Write {
        slot: usize,
        value: ScratchValue,
    },
    RepeatUntil {
        condition_slot: usize,
        hits_slot: usize,
    },
    NestedRepeat {
        outer_remaining: usize,
        inner_remaining: usize,
        outer_hits_slot: usize,
        inner_hits_slot: usize,
        done: bool,
    },
    Forever {
        hits_slot: usize,
    },
    Observe {
        source_slot: usize,
        observed_slot: usize,
    },
}

enum RepeatCount {
    Fixed(usize),
    Variable(usize),
}

struct LoopScript {
    role: LoopRole,
}

impl ResumableScript for LoopScript {
    fn step(&mut self, state: &mut ScratchState, _clock: &dyn super::Clock) -> ScriptStatus {
        match &mut self.role {
            LoopRole::Repeat {
                count,
                remaining,
                hits_slot,
            } => {
                let remaining = remaining.get_or_insert_with(|| match count {
                    RepeatCount::Fixed(count) => *count,
                    RepeatCount::Variable(slot) => {
                        scratch_round(state.get(*slot)).max(0.0) as usize
                    }
                });
                if *remaining == 0 {
                    return ScriptStatus::Complete;
                }
                increment(state, *hits_slot);
                *remaining -= 1;
                ScriptStatus::Yield
            }
            LoopRole::Write { slot, value } => {
                state.set(*slot, value.clone());
                ScriptStatus::Complete
            }
            LoopRole::RepeatUntil {
                condition_slot,
                hits_slot,
            } => {
                if scratch_bool(state.get(*condition_slot)) {
                    ScriptStatus::Complete
                } else {
                    increment(state, *hits_slot);
                    ScriptStatus::Yield
                }
            }
            LoopRole::NestedRepeat {
                outer_remaining,
                inner_remaining,
                outer_hits_slot,
                inner_hits_slot,
                done,
            } => {
                if *done {
                    return ScriptStatus::Complete;
                }
                if *inner_remaining > 0 {
                    increment(state, *inner_hits_slot);
                    *inner_remaining -= 1;
                } else {
                    increment(state, *outer_hits_slot);
                    *outer_remaining -= 1;
                    if *outer_remaining == 0 {
                        *done = true;
                    } else {
                        *inner_remaining = 3;
                    }
                }
                ScriptStatus::Yield
            }
            LoopRole::Forever { hits_slot } => {
                increment(state, *hits_slot);
                ScriptStatus::Yield
            }
            LoopRole::Observe {
                source_slot,
                observed_slot,
            } => {
                if scratch_to_number(state.get(*source_slot)) > 0.0 {
                    state.set(*observed_slot, true.into());
                    ScriptStatus::Complete
                } else {
                    ScriptStatus::Yield
                }
            }
        }
    }
}

fn increment(state: &mut ScratchState, slot: usize) {
    let value = scratch_to_number(state.get(slot)) + 1.0;
    state.set(slot, value.into());
}

fn number_state(values: &[f64]) -> ScratchState {
    ScratchState::new(values.iter().copied().map(ScratchValue::Number).collect())
}

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

#[derive(Clone)]
struct ManualClock(std::rc::Rc<std::cell::Cell<u64>>);

impl ManualClock {
    fn new() -> Self {
        Self(std::rc::Rc::new(std::cell::Cell::new(0)))
    }

    fn set(&self, milliseconds: u64) {
        self.0.set(milliseconds);
    }
}

impl Clock for ManualClock {
    fn now_millis(&self) -> u64 {
        self.0.get()
    }
}

struct WaitScript {
    slot: usize,
    duration: u64,
    phase: u8,
}

impl ResumableScript for WaitScript {
    fn step(&mut self, state: &mut ScratchState, clock: &dyn Clock) -> ScriptStatus {
        match self.phase {
            0 => {
                increment(state, self.slot);
                self.phase = 1;
                ScriptStatus::WaitUntil(clock.now_millis().saturating_add(self.duration))
            }
            1 => {
                state.set(
                    self.slot,
                    (scratch_to_number(state.get(self.slot)) + 10.0).into(),
                );
                self.phase = 2;
                ScriptStatus::Complete
            }
            _ => panic!("completed wait script was stepped again"),
        }
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

struct RestartScript {
    finish: bool,
}

impl ResumableScript for RestartScript {
    fn step(&mut self, state: &mut ScratchState, _clock: &dyn Clock) -> ScriptStatus {
        if self.finish {
            let next = scratch_to_number(state.get(0)) + 1.0;
            state.set(0, next.into());
            ScriptStatus::Complete
        } else {
            state.set(0, 100.0.into());
            self.finish = true;
            ScriptStatus::Yield
        }
    }
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

use std::cell::Cell;

struct TestClock {
    now: Cell<u64>,
    advance_per_read: u64,
}

impl TestClock {
    fn advancing_by(milliseconds: u64) -> Self {
        Self {
            now: Cell::new(0),
            advance_per_read: milliseconds,
        }
    }
}

impl Clock for TestClock {
    fn now_millis(&self) -> u64 {
        let now = self.now.get();
        self.now.set(now.saturating_add(self.advance_per_read));
        now
    }
}

enum WarpRole {
    Finite {
        iterations: usize,
        budget: WarpBudget,
    },
    Infinite {
        iterations: usize,
        budget: WarpBudget,
    },
    Observer {
        seen: usize,
    },
}

struct WarpScript(WarpRole);

impl ResumableScript for WarpScript {
    fn step(&mut self, state: &mut ScratchState, clock: &dyn Clock) -> ScriptStatus {
        match &mut self.0 {
            WarpRole::Finite { iterations, budget } => {
                while *iterations > 0 {
                    increment(state, 0);
                    *iterations -= 1;
                    if budget.expired(clock) {
                        return ScriptStatus::Yield;
                    }
                }
                ScriptStatus::Complete
            }
            WarpRole::Infinite { iterations, budget } => loop {
                increment(state, 0);
                *iterations += 1;
                if budget.expired(clock) {
                    return ScriptStatus::Yield;
                }
            },
            WarpRole::Observer { seen } => {
                *seen += 1;
                state.set(1, (*seen as f64).into());
                ScriptStatus::Complete
            }
        }
    }
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

#[derive(Clone, Copy)]
enum CallKind {
    Root,
    Warp,
    Normal,
}

struct CallFrame {
    kind: CallKind,
    pc: usize,
    remaining: usize,
    warp_mode: bool,
}

struct CallFixture {
    enter_warp: bool,
    frames: Vec<CallFrame>,
}

enum CallRunner {
    Call(CallFixture),
    Observer(WarpScript),
}

impl ResumableScript for CallRunner {
    fn step(&mut self, state: &mut ScratchState, clock: &dyn Clock) -> ScriptStatus {
        match self {
            Self::Call(script) => script.step(state, clock),
            Self::Observer(script) => script.step(state, clock),
        }
    }
}

impl ResumableScript for CallFixture {
    fn step(&mut self, state: &mut ScratchState, _clock: &dyn Clock) -> ScriptStatus {
        loop {
            let Some(frame) = self.frames.last_mut() else {
                return ScriptStatus::Complete;
            };
            match frame.kind {
                CallKind::Root if frame.pc == 0 => {
                    frame.pc = 1;
                    self.frames.push(CallFrame {
                        kind: if self.enter_warp {
                            CallKind::Warp
                        } else {
                            CallKind::Normal
                        },
                        pc: 0,
                        remaining: 2,
                        warp_mode: self.enter_warp,
                    });
                }
                CallKind::Root => {
                    state.set(2, true.into());
                    return ScriptStatus::Complete;
                }
                CallKind::Warp if frame.pc == 0 => {
                    frame.pc = 1;
                    let inherited_warp = frame.warp_mode;
                    self.frames.push(CallFrame {
                        kind: CallKind::Normal,
                        pc: 0,
                        remaining: 2,
                        warp_mode: inherited_warp,
                    });
                }
                CallKind::Warp => {
                    self.frames.pop();
                }
                CallKind::Normal if frame.pc == 0 => {
                    increment(state, 0);
                    frame.remaining -= 1;
                    if frame.remaining == 0 {
                        frame.pc = 1;
                    } else if !frame.warp_mode {
                        return ScriptStatus::Yield;
                    }
                }
                CallKind::Normal => {
                    self.frames.pop();
                }
            }
        }
    }
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

struct TargetScript(usize);

impl ResumableScript for TargetScript {
    fn step(&mut self, _state: &mut ScratchState, _clock: &dyn Clock) -> ScriptStatus {
        ScriptStatus::Complete
    }

    fn target_instance(&self) -> usize {
        self.0
    }
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

struct CloneWaitFrame {
    pc: usize,
    warp_mode: bool,
    arguments: Vec<ScratchValue>,
}

struct CloneWaitScript {
    instance: usize,
    phase: u8,
    frames: Vec<CloneWaitFrame>,
}

enum CloneWaitRunner {
    Clone(CloneWaitScript),
    StageWriter,
}

impl ResumableScript for CloneWaitRunner {
    fn step(&mut self, state: &mut ScratchState, clock: &dyn Clock) -> ScriptStatus {
        match self {
            Self::StageWriter => {
                state.set(0, 100.0.into());
                ScriptStatus::Complete
            }
            Self::Clone(script) => {
                state.set_current_instance(script.instance);
                match script.phase {
                    0 => {
                        increment(state, 0);
                        increment(state, 1);
                        script.frames[0].pc = 1;
                        script.frames.push(CloneWaitFrame {
                            pc: 0,
                            warp_mode: true,
                            arguments: vec![3.0.into()],
                        });
                        script.phase = 1;
                        ScriptStatus::WaitUntil(clock.now_millis().saturating_add(20))
                    }
                    1 => {
                        let procedure = script.frames.last().unwrap();
                        assert!(procedure.warp_mode);
                        assert_eq!(procedure.pc, 0);
                        let argument = scratch_to_number(&procedure.arguments[0]);
                        let local = scratch_to_number(state.get(1));
                        let shared = scratch_to_number(state.get(0));
                        state.set(1, (local + argument + shared).into());
                        script.frames.pop();
                        assert_eq!(script.frames.last().unwrap().pc, 1);
                        increment(state, 0);
                        script.phase = 2;
                        ScriptStatus::Complete
                    }
                    _ => panic!("completed clone script was stepped again"),
                }
            }
        }
    }

    fn target_instance(&self) -> usize {
        match self {
            Self::Clone(script) => script.instance,
            Self::StageWriter => 0,
        }
    }
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

enum BroadcastTestScript {
    Sender { pc: u8 },
    Receiver { hat: usize, instance: usize, pc: u8 },
    Rebroadcast { pc: u8 },
}

impl ResumableScript for BroadcastTestScript {
    fn step(&mut self, state: &mut ScratchState, clock: &dyn Clock) -> ScriptStatus {
        match self {
            Self::Sender { pc } if *pc == 0 => {
                *pc = 1;
                ScriptStatus::Broadcast {
                    message: 1,
                    wait: true,
                }
            }
            Self::Sender { pc } => {
                assert_eq!(*pc, 1);
                state.set(2, true.into());
                ScriptStatus::Complete
            }
            Self::Receiver { pc, .. } if *pc == 0 => {
                *pc = 1;
                ScriptStatus::WaitUntil(clock.now_millis().saturating_add(5))
            }
            Self::Receiver { instance, pc, .. } => {
                assert_eq!(*pc, 1);
                state.set_current_instance(*instance);
                increment(state, 0);
                increment(state, 1);
                ScriptStatus::Complete
            }
            Self::Rebroadcast { pc } if *pc == 0 => {
                *pc = 1;
                ScriptStatus::Yield
            }
            Self::Rebroadcast { pc } if *pc == 1 => {
                *pc = 2;
                ScriptStatus::Broadcast {
                    message: 1,
                    wait: false,
                }
            }
            Self::Rebroadcast { .. } => ScriptStatus::Complete,
        }
    }

    fn target_instance(&self) -> usize {
        match self {
            Self::Sender { .. } => 0,
            Self::Receiver { instance, .. } => *instance,
            Self::Rebroadcast { .. } => 0,
        }
    }

    fn broadcast_receiver(&self) -> Option<(usize, usize, usize)> {
        match self {
            Self::Receiver { hat, instance, .. } => Some((1, *hat, *instance)),
            Self::Sender { .. } => None,
            Self::Rebroadcast { .. } => None,
        }
    }

    fn restart_for_broadcast(&mut self) {
        if let Self::Receiver { pc, .. } = self {
            *pc = 0;
        }
    }
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
