use super::{
    Clock, EdgeHatState, ResumableScript, SchedulerStatus, ScriptScheduler, ScriptStatus,
    WarpBudget,
};
use crate::runtime::{
    BubbleKind, RuntimeInputEvent, ScratchState, ScratchValue, ScratchVariableMetadata,
    pointer_click_activates, scratch_bool, scratch_round, scratch_to_number,
};

struct SignalWaitScript {
    waiting: bool,
    steps: u8,
    slot: usize,
}

impl ResumableScript for SignalWaitScript {
    fn step(&mut self, state: &mut ScratchState, _clock: &dyn Clock) -> ScriptStatus {
        let next = crate::runtime::scratch_to_number(state.get(self.slot)) + 1.0;
        state.set(self.slot, next.into());
        self.steps += 1;
        if self.waiting && self.steps == 1 {
            ScriptStatus::WaitForSignal(91)
        } else {
            ScriptStatus::Complete
        }
    }
}

// exercise generic hat scheduling without coupling the runtime to Scratch opcodes.
struct EventHatScript {
    key: (usize, usize),
    edge: bool,
    record_instance: bool,
    steps: u8,
}

impl ResumableScript for EventHatScript {
    fn step(&mut self, state: &mut ScratchState, _clock: &dyn Clock) -> ScriptStatus {
        if self.record_instance {
            state.set_shared(0, (state.current_instance() as f64).into());
            return ScriptStatus::Complete;
        }
        state.set(
            0,
            (crate::runtime::scratch_to_number(state.get(0)) + 1.0).into(),
        );
        self.steps += 1;
        if self.steps == 1 {
            ScriptStatus::Yield
        } else {
            ScriptStatus::Complete
        }
    }

    fn target_instance(&self) -> usize {
        self.key.1
    }

    fn edge_hat_key(&self) -> Option<(usize, usize)> {
        self.edge.then_some(self.key)
    }

    fn click_hat_key(&self) -> Option<(usize, usize)> {
        (!self.edge).then_some(self.key)
    }
}

struct KeyHatScript {
    key: (usize, usize),
    steps: u8,
}

impl ResumableScript for KeyHatScript {
    fn step(&mut self, state: &mut ScratchState, _clock: &dyn Clock) -> ScriptStatus {
        if self.steps == 0 {
            state.set_shared(0, (scratch_to_number(state.get(0)) + 1.0).into());
            self.steps = 1;
            ScriptStatus::Yield
        } else {
            ScriptStatus::Complete
        }
    }

    fn target_instance(&self) -> usize {
        self.key.1
    }

    fn key_hat_key(&self) -> Option<(usize, usize)> {
        Some(self.key)
    }
}

fn dispatch_pointer_event(
    scheduler: &mut ScriptScheduler<EventHatScript>,
    event: RuntimeInputEvent,
) {
    let RuntimeInputEvent::PointerButton {
        pressed,
        inside_stage,
        target_instance,
        draggable,
        was_dragged,
        monitor_interaction,
    } = event
    else {
        return;
    };
    if pointer_click_activates(
        pressed,
        inside_stage,
        draggable,
        was_dragged,
        monitor_interaction,
    ) {
        let instance = target_instance.unwrap_or(0);
        scheduler.start_click_hats([EventHatScript {
            key: (8, instance),
            edge: false,
            record_instance: true,
            steps: 0,
        }]);
    }
}

// inject resolved pointer events through the same click predicate and scheduler path.
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

#[derive(Clone)]
struct ControlledClock {
    calls: std::rc::Rc<std::cell::Cell<usize>>,
    now: std::rc::Rc<std::cell::Cell<u64>>,
}

impl Clock for ControlledClock {
    fn now_millis(&self) -> u64 {
        self.calls.set(self.calls.get() + 1);
        self.now.get()
    }
}

struct DeadlineScript {
    deadline: u64,
    steps: std::rc::Rc<std::cell::Cell<usize>>,
}

impl ResumableScript for DeadlineScript {
    fn step(&mut self, _state: &mut ScratchState, _clock: &dyn Clock) -> ScriptStatus {
        self.steps.set(self.steps.get() + 1);
        ScriptStatus::WaitUntil(self.deadline)
    }
}

struct ClockAdvancingScript {
    clock: ControlledClock,
    values: Vec<u64>,
    steps: usize,
}

impl ResumableScript for ClockAdvancingScript {
    fn step(&mut self, _state: &mut ScratchState, _clock: &dyn Clock) -> ScriptStatus {
        if let Some(value) = self.values.get(self.steps) {
            self.clock.now.set(*value);
            self.steps += 1;
            ScriptStatus::Yield
        } else {
            ScriptStatus::Complete
        }
    }
}

enum ClockFixtureScript {
    Deadline(DeadlineScript),
    Advancing(ClockAdvancingScript),
}

impl ResumableScript for ClockFixtureScript {
    fn step(&mut self, state: &mut ScratchState, clock: &dyn Clock) -> ScriptStatus {
        match self {
            Self::Deadline(script) => script.step(state, clock),
            Self::Advancing(script) => script.step(state, clock),
        }
    }
}

enum FrameOrderScript {
    Wait { started: bool },
    Runnable,
}

enum FrameRateScript {
    Wait { started: bool },
    Counter,
}

impl ResumableScript for FrameRateScript {
    fn step(&mut self, state: &mut ScratchState, _clock: &dyn Clock) -> ScriptStatus {
        match self {
            Self::Wait { started: false } => {
                *self = Self::Wait { started: true };
                ScriptStatus::WaitUntil(120)
            }
            Self::Wait { started: true } => {
                state.set_shared(1, state.get_shared(0).clone());
                ScriptStatus::Complete
            }
            Self::Counter => {
                state.change_shared(0, 1.0);
                ScriptStatus::Yield
            }
        }
    }
}

impl ResumableScript for FrameOrderScript {
    fn step(&mut self, _state: &mut ScratchState, _clock: &dyn Clock) -> ScriptStatus {
        match self {
            Self::Wait { started: false } => {
                *self = Self::Wait { started: true };
                ScriptStatus::WaitUntil(10)
            }
            Self::Wait { started: true } => ScriptStatus::Complete,
            Self::Runnable => ScriptStatus::Yield,
        }
    }
}

enum StopFixtureScript {
    All {
        target: usize,
    },
    Others {
        target: usize,
        pc: u8,
    },
    Write {
        target: usize,
        slot: usize,
        value: f64,
    },
    Broadcast {
        target: usize,
    },
    Receiver {
        target: usize,
    },
}

impl ResumableScript for StopFixtureScript {
    fn step(&mut self, state: &mut ScratchState, _clock: &dyn Clock) -> ScriptStatus {
        match self {
            Self::All { .. } => ScriptStatus::StopAll,
            Self::Others { pc, .. } if *pc == 0 => {
                *pc = 1;
                ScriptStatus::StopOtherScripts
            }
            Self::Others { pc, .. } => {
                *pc = 2;
                state.set_shared(0, ScratchValue::Number(1.0));
                ScriptStatus::Complete
            }
            Self::Write { slot, value, .. } => {
                state.set_shared(*slot, ScratchValue::Number(*value));
                ScriptStatus::Complete
            }
            Self::Broadcast { .. } => ScriptStatus::Broadcast {
                message: 7,
                wait: true,
            },
            Self::Receiver { .. } => ScriptStatus::Yield,
        }
    }

    fn target_instance(&self) -> usize {
        match self {
            Self::All { target }
            | Self::Others { target, .. }
            | Self::Write { target, .. }
            | Self::Broadcast { target }
            | Self::Receiver { target } => *target,
        }
    }

    fn broadcast_receiver(&self) -> Option<(usize, usize, usize)> {
        match self {
            Self::Receiver { target } => Some((7, 0, *target)),
            _ => None,
        }
    }
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

struct TargetScript(usize);

impl ResumableScript for TargetScript {
    fn step(&mut self, _state: &mut ScratchState, _clock: &dyn Clock) -> ScriptStatus {
        ScriptStatus::Complete
    }

    fn target_instance(&self) -> usize {
        self.0
    }
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

mod broadcast;
mod clones;
mod events;
mod lifecycle;
mod timing;
mod warp;
