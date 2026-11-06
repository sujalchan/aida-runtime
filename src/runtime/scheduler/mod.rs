// Copyright (c) 2026 AIDA AST contributers (see AUTHORS.md)
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use super::ScratchState;
use std::collections::{HashMap, HashSet};
use std::time::Instant;

/// keep each compiled edge hat's last predicate value per target instance.
#[derive(Debug, Default)]
pub struct EdgeHatState {
    previous: HashMap<(usize, usize), bool>,
}

impl EdgeHatState {
    /// return true only when a hat's predicate changes from false to true.
    pub fn rising_edge(&mut self, hat: usize, instance: usize, active: bool) -> bool {
        !self
            .previous
            .insert((hat, instance), active)
            .unwrap_or(false)
            && active
    }

    /// clear predicate history when Scratch starts a new project run.
    pub fn clear(&mut self) {
        self.previous.clear();
    }

    /// discard clone history once the corresponding runtime instance is gone.
    pub fn retain_live_instances(&mut self, mut is_alive: impl FnMut(usize) -> bool) {
        self.previous.retain(|(_, instance), _| is_alive(*instance));
    }
}

pub trait Clock {
    fn now_millis(&self) -> u64;
}

pub struct WarpBudget {
    started_at: Option<u64>,
    limit_millis: u64,
    safe_points_until_check: u16,
}

// clock reads dominate very small native loops; 1024 safe points are still
// only a few microseconds of unchecked work in the measured numeric hot path.
const WARP_CLOCK_CHECK_INTERVAL: u16 = 1024;

impl WarpBudget {
    pub fn new(limit_millis: u64) -> Self {
        Self {
            started_at: None,
            limit_millis,
            safe_points_until_check: 0,
        }
    }

    pub fn reset(&mut self) {
        self.started_at = None;
        self.safe_points_until_check = 0;
    }

    pub fn expired(&mut self, clock: &dyn Clock) -> bool {
        if self.safe_points_until_check > 0 {
            self.safe_points_until_check -= 1;
            return false;
        }
        self.safe_points_until_check = WARP_CLOCK_CHECK_INTERVAL - 1;

        let now = clock.now_millis();
        let started = *self.started_at.get_or_insert(now);
        if now.saturating_sub(started) > self.limit_millis {
            self.reset();
            true
        } else {
            false
        }
    }

    // force the watchdog at generated loop chunk boundaries, never per list item.
    pub fn expired_now(&mut self, clock: &dyn Clock) -> bool {
        self.safe_points_until_check = 0;
        self.expired(clock)
    }
}

struct MonotonicClock {
    start: Instant,
}

impl MonotonicClock {
    fn new() -> Self {
        Self {
            start: Instant::now(),
        }
    }
}

impl Clock for MonotonicClock {
    fn now_millis(&self) -> u64 {
        self.start.elapsed().as_millis().min(u64::MAX as u128) as u64
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptStatus {
    Yield,
    /// suspend until the scheduler clock reaches the deadline; the scheduler will not step the script earlier.
    WaitUntil(u64),
    /// suspend until a runtime event completes the supplied wait token.
    WaitForSignal(u64),
    Complete,
    StopAll,
    StopOtherScripts,
    Broadcast {
        message: usize,
        wait: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerStatus {
    /// one or more scripts advanced and another host turn may run immediately.
    Progressed,
    /// all active scripts are waiting until the given clock time.
    WaitingUntil(u64),
    /// every active script is suspended on an external runtime event.
    WaitingForSignal,
    /// no active scripts remain.
    Complete,
}

pub trait ResumableScript {
    /// advance one script continuation; wait deadlines are enforced by the scheduler.
    fn step(&mut self, state: &mut ScratchState, clock: &dyn Clock) -> ScriptStatus;
    fn target_instance(&self) -> usize {
        0
    }
    fn broadcast_receiver(&self) -> Option<(usize, usize, usize)> {
        None
    }
    fn restart_for_broadcast(&mut self) {}
    /// identify an active edge hat so the scheduler does not start duplicates.
    fn edge_hat_key(&self) -> Option<(usize, usize)> {
        None
    }
    /// identify click hats that restart when the same target is clicked again.
    fn click_hat_key(&self) -> Option<(usize, usize)> {
        None
    }
    /// identify key hats so a held key does not restart an active target thread.
    fn key_hat_key(&self) -> Option<(usize, usize)> {
        None
    }
}

pub struct ScriptScheduler<S> {
    scripts: Vec<S>,
    wake_at: Vec<Option<u64>>,
    waiting_for: Vec<Option<u64>>,
    completed_signals: HashSet<u64>,
    script_ids: Vec<u64>,
    joins: Vec<Option<Vec<u64>>>,
    has_waiting_senders: bool,
    next_script_id: u64,
    pending_broadcast: Option<(u64, usize, bool)>,
    next: usize,
    clock: Box<dyn Clock>,
}

impl<S: ResumableScript> ScriptScheduler<S> {
    pub fn new(scripts: Vec<S>) -> Self {
        Self::with_clock(scripts, Box::new(MonotonicClock::new()))
    }

    pub fn with_clock(scripts: Vec<S>, clock: Box<dyn Clock>) -> Self {
        let script_count = scripts.len();
        Self {
            wake_at: vec![None; scripts.len()],
            waiting_for: vec![None; scripts.len()],
            completed_signals: HashSet::new(),
            script_ids: (0..script_count as u64).collect(),
            joins: vec![None; script_count],
            has_waiting_senders: false,
            next_script_id: script_count as u64,
            pending_broadcast: None,
            scripts,
            next: 0,
            clock,
        }
    }

    pub fn run_until_complete(&mut self, state: &mut ScratchState) {
        loop {
            match self.advance(state) {
                SchedulerStatus::Progressed => {}
                SchedulerStatus::WaitingUntil(deadline) => {
                    let now = self.clock.now_millis();
                    if deadline > now {
                        std::thread::sleep(std::time::Duration::from_millis(deadline - now));
                    }
                }
                SchedulerStatus::WaitingForSignal => {
                    std::thread::sleep(std::time::Duration::from_millis(8))
                }
                SchedulerStatus::Complete => return,
            }
        }
    }

    /// run one runnable script until it yields, waits, or completes.
    pub fn step(&mut self, state: &mut ScratchState) -> bool {
        matches!(self.advance(state), SchedulerStatus::Progressed)
    }

    /// advance one runnable script, or report when the host should wake again.
    pub fn advance(&mut self, state: &mut ScratchState) -> SchedulerStatus {
        if self.scripts.is_empty() {
            return SchedulerStatus::Complete;
        }
        let mut now = None;
        for offset in 0..self.scripts.len() {
            let index = (self.next + offset) % self.scripts.len();
            let join_finished = if self.has_waiting_senders {
                if let Some(waiting) = &self.joins[index] {
                    let finished = waiting.iter().all(|id| !self.script_ids.contains(id));
                    if finished {
                        self.joins[index] = None;
                        self.has_waiting_senders = self.joins.iter().any(Option::is_some);
                    }
                    finished
                } else {
                    true
                }
            } else {
                true
            };
            let is_runnable = self.is_runnable(index, &mut now) && join_finished;
            if is_runnable {
                return self.run_script_at(index, state);
            }
        }
        self.idle_status()
    }

    /// return active scripts before sleeping scripts at a frame boundary.
    pub fn script_ids_in_frame_order(&self) -> Vec<u64> {
        let mut ids = Vec::with_capacity(self.scripts.len());
        self.script_ids_in_frame_order_into(&mut ids);
        ids
    }

    /// fill a reusable buffer with runnable scripts before scripts waiting for deadlines.
    pub fn script_ids_in_frame_order_into(&self, output: &mut Vec<u64>) {
        output.clear();
        if self.scripts.is_empty() {
            return;
        }
        for offset in 0..self.scripts.len() {
            let index = (self.next + offset) % self.scripts.len();
            if self.waiting_for[index].is_none() && self.wake_at[index].is_none() {
                output.push(self.script_ids[index]);
            }
        }
        for offset in 0..self.scripts.len() {
            let index = (self.next + offset) % self.scripts.len();
            if self.waiting_for[index].is_some() || self.wake_at[index].is_some() {
                output.push(self.script_ids[index]);
            }
        }
    }

    /// return newly assigned script ids so frame stepping can avoid rescanning unchanged scripts.
    pub fn append_new_script_ids_since(&self, output: &mut Vec<u64>, first_id: u64) {
        output.extend(self.script_ids.iter().copied().filter(|id| *id >= first_id));
    }

    pub fn next_script_id(&self) -> u64 {
        self.next_script_id
    }

    /// advance one selected script if it is runnable.
    pub fn advance_script_id(
        &mut self,
        id: u64,
        state: &mut ScratchState,
    ) -> Option<SchedulerStatus> {
        let index = self
            .script_ids
            .iter()
            .position(|script_id| *script_id == id)?;
        let join_finished = if self.has_waiting_senders {
            if let Some(waiting) = &self.joins[index] {
                let finished = waiting.iter().all(|id| !self.script_ids.contains(id));
                if finished {
                    self.joins[index] = None;
                    self.has_waiting_senders = self.joins.iter().any(Option::is_some);
                }
                finished
            } else {
                true
            }
        } else {
            true
        };
        let mut now = None;
        let is_runnable = self.is_runnable(index, &mut now);
        (join_finished && is_runnable).then(|| self.run_script_at(index, state))
    }

    /// report completion or the next deadline when no frame script ran.
    pub fn idle_status(&self) -> SchedulerStatus {
        if self.scripts.is_empty() {
            SchedulerStatus::Complete
        } else if let Some(deadline) = self.wake_at.iter().flatten().copied().min() {
            SchedulerStatus::WaitingUntil(deadline)
        } else {
            SchedulerStatus::WaitingForSignal
        }
    }

    fn is_runnable(&self, index: usize, now: &mut Option<u64>) -> bool {
        if let Some(signal) = self.waiting_for[index] {
            return self.completed_signals.contains(&signal);
        }
        match self.wake_at[index] {
            None => true,
            Some(deadline) => deadline <= *now.get_or_insert_with(|| self.clock.now_millis()),
        }
    }

    /// wake every script suspended on a completed runtime signal.
    pub fn signal(&mut self, signal: u64) {
        if self.waiting_for.contains(&Some(signal)) {
            self.completed_signals.insert(signal);
        }
    }

    fn run_script_at(&mut self, mut index: usize, state: &mut ScratchState) -> SchedulerStatus {
        if let Some(signal) = self.waiting_for[index].take()
            && !self.waiting_for.contains(&Some(signal))
        {
            self.completed_signals.remove(&signal);
        }
        self.wake_at[index] = None;
        self.next = index;
        state.set_current_instance(self.scripts[index].target_instance());
        let mut status = self.scripts[index].step(state, self.clock.as_ref());
        while status == ScriptStatus::StopOtherScripts {
            let target_instance = self.scripts[index].target_instance();
            let current_id = self.script_ids[index];
            self.remove_other_scripts(target_instance, current_id);
            index = self
                .script_ids
                .iter()
                .position(|id| *id == current_id)
                .expect("the stopping script remains active");
            self.next = index;
            state.set_current_instance(target_instance);
            status = self.scripts[index].step(state, self.clock.as_ref());
        }
        match status {
            ScriptStatus::Yield => {
                self.wake_at[index] = None;
                self.next = (index + 1) % self.scripts.len();
            }
            ScriptStatus::WaitUntil(deadline) => {
                self.wake_at[index] = Some(deadline);
                self.waiting_for[index] = None;
                self.next = (index + 1) % self.scripts.len();
            }
            ScriptStatus::WaitForSignal(signal) => {
                self.wake_at[index] = None;
                self.waiting_for[index] = Some(signal);
                self.next = (index + 1) % self.scripts.len();
            }
            ScriptStatus::Complete => {
                self.scripts.remove(index);
                self.wake_at.remove(index);
                self.script_ids.remove(index);
                self.joins.remove(index);
                self.waiting_for.remove(index);
                self.has_waiting_senders = self.joins.iter().any(Option::is_some);
                self.next = if self.scripts.is_empty() {
                    0
                } else {
                    index % self.scripts.len()
                };
            }
            ScriptStatus::StopAll => {
                self.stop_all(state);
                return SchedulerStatus::Complete;
            }
            ScriptStatus::StopOtherScripts => unreachable!(),
            ScriptStatus::Broadcast { message, wait } => {
                self.wake_at[index] = None;
                self.waiting_for[index] = None;
                self.pending_broadcast = Some((self.script_ids[index], message, wait));
                self.next = (index + 1) % self.scripts.len();
            }
        }
        SchedulerStatus::Progressed
    }

    pub fn stop_all(&mut self, state: &mut ScratchState) {
        state.stop_project_sounds(self.clock.now_millis());
        state.clear_speech_bubbles();
        state.remove_clones();
        self.scripts.clear();
        self.wake_at.clear();
        self.waiting_for.clear();
        self.completed_signals.clear();
        self.script_ids.clear();
        self.joins.clear();
        self.has_waiting_senders = false;
        self.pending_broadcast = None;
        self.next = 0;
    }

    /// stop active scripts and start a fresh matching event set.
    pub fn restart(&mut self, scripts: Vec<S>) {
        self.next_script_id = scripts.len() as u64;
        self.script_ids = (0..scripts.len() as u64).collect();
        self.joins = vec![None; scripts.len()];
        self.has_waiting_senders = false;
        self.pending_broadcast = None;
        self.scripts = scripts;
        self.wake_at = vec![None; self.scripts.len()];
        self.waiting_for = vec![None; self.scripts.len()];
        self.completed_signals.clear();
        self.next = 0;
    }

    pub fn active_scripts(&self) -> usize {
        self.scripts.len()
    }

    pub fn add_scripts(&mut self, scripts: impl IntoIterator<Item = S>) {
        let first_added = self.scripts.len();
        for script in scripts {
            self.scripts.push(script);
            self.wake_at.push(None);
            self.waiting_for.push(None);
            self.script_ids.push(self.next_script_id);
            self.next_script_id += 1;
            self.joins.push(None);
        }
        if self.scripts.len() > first_added && self.next == 0 && first_added > 0 {
            self.next = first_added;
        }
    }

    /// start edge hats only when that hat has no active thread for its target.
    pub fn start_edge_hats(&mut self, candidates: impl IntoIterator<Item = S>) {
        let mut starters = Vec::new();
        for candidate in candidates {
            let Some(key) = candidate.edge_hat_key() else {
                continue;
            };
            if !self
                .scripts
                .iter()
                .any(|script| script.edge_hat_key() == Some(key))
            {
                starters.push(candidate);
            }
        }
        self.add_scripts(starters);
    }

    /// start click hats, replacing any still-running thread for the same hat.
    pub fn start_click_hats(&mut self, candidates: impl IntoIterator<Item = S>) {
        for candidate in candidates {
            let Some(key) = candidate.click_hat_key() else {
                continue;
            };
            if let Some(index) = self
                .scripts
                .iter()
                .position(|script| script.click_hat_key() == Some(key))
            {
                self.scripts[index] = candidate;
                self.wake_at[index] = None;
                self.waiting_for[index] = None;
                self.joins[index] = None;
                self.script_ids[index] = self.next_script_id;
                self.next_script_id += 1;
            } else {
                self.add_scripts(std::iter::once(candidate));
            }
        }
        self.has_waiting_senders = self.joins.iter().any(Option::is_some);
    }

    /// start key hats once per hat and target instance while an earlier thread is active.
    pub fn start_key_hats(&mut self, candidates: impl IntoIterator<Item = S>) {
        let mut starters = Vec::new();
        for candidate in candidates {
            let Some(key) = candidate.key_hat_key() else {
                continue;
            };
            if !self
                .scripts
                .iter()
                .any(|script| script.key_hat_key() == Some(key))
            {
                starters.push(candidate);
            }
        }
        self.add_scripts(starters);
    }

    pub fn remove_target_scripts(&mut self, target_instance: usize) {
        let mut index = 0;
        while index < self.scripts.len() {
            if self.scripts[index].target_instance() == target_instance {
                self.scripts.remove(index);
                self.wake_at.remove(index);
                self.waiting_for.remove(index);
                self.script_ids.remove(index);
                self.joins.remove(index);
                if index < self.next {
                    self.next -= 1;
                }
            } else {
                index += 1;
            }
        }
        if self.scripts.is_empty() {
            self.next = 0;
        } else {
            self.next %= self.scripts.len();
        }
        self.has_waiting_senders = self.joins.iter().any(Option::is_some);
    }

    fn remove_other_scripts(&mut self, target_instance: usize, current_id: u64) {
        let mut index = 0;
        while index < self.scripts.len() {
            if self.script_ids[index] != current_id
                && self.scripts[index].target_instance() == target_instance
            {
                self.scripts.remove(index);
                self.wake_at.remove(index);
                self.waiting_for.remove(index);
                self.script_ids.remove(index);
                self.joins.remove(index);
                if index < self.next {
                    self.next -= 1;
                }
            } else {
                index += 1;
            }
        }
        self.has_waiting_senders = self.joins.iter().any(Option::is_some);
    }

    /// start matching receivers and return identities for broadcast-and-wait.
    pub fn broadcast(&mut self, sender: u64, candidates: Vec<S>, wait: bool) {
        let mut receiver_ids = Vec::new();
        for candidate in candidates {
            let Some(key) = candidate.broadcast_receiver() else {
                continue;
            };
            let existing = self
                .scripts
                .iter()
                .position(|script| script.broadcast_receiver() == Some(key));
            let id = if let Some(index) = existing {
                self.scripts[index].restart_for_broadcast();
                self.wake_at[index] = None;
                self.waiting_for[index] = None;
                self.joins[index] = None;
                let restarted_id = self.next_script_id;
                self.next_script_id += 1;
                self.script_ids[index] = restarted_id;
                restarted_id
            } else {
                let id = self.next_script_id;
                self.next_script_id += 1;
                self.scripts.push(candidate);
                self.wake_at.push(None);
                self.waiting_for.push(None);
                self.script_ids.push(id);
                self.joins.push(None);
                id
            };
            receiver_ids.push(id);
        }
        if wait
            && !receiver_ids.is_empty()
            && !receiver_ids.contains(&sender)
            && let Some(index) = self.script_ids.iter().position(|id| *id == sender)
        {
            self.joins[index] = Some(receiver_ids);
            self.has_waiting_senders = true;
        }
        self.has_waiting_senders = self.joins.iter().any(Option::is_some);
    }

    pub fn take_broadcast(&mut self) -> Option<(u64, usize, bool)> {
        self.pending_broadcast.take()
    }

    pub fn duration_until(&self, deadline: u64) -> std::time::Duration {
        std::time::Duration::from_millis(deadline.saturating_sub(self.clock.now_millis()))
    }

    pub fn now_millis(&self) -> u64 {
        self.clock.now_millis()
    }
}

#[cfg(test)]
mod tests;
