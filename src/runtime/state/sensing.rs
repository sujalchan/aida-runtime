//! Sensing operations for Scratch project state.

use super::*;

pub(super) fn default_username() -> String {
    let suffix = scratch_random(1000.0, 9999.0) as u16;
    format!("player{suffix:04}")
}

/// input collected by the presentation thread and consumed by generated code.
#[derive(Debug, Clone, PartialEq)]
pub enum RuntimeInputEvent {
    Key {
        name: String,
        pressed: bool,
    },
    // release all held keys when the window loses focus.
    ClearKeys,
    MousePosition {
        x: f64,
        y: f64,
    },
    MouseButton {
        pressed: bool,
    },
    // carry renderer-picked click targets to the scheduler without gpu state.
    PointerButton {
        pressed: bool,
        inside_stage: bool,
        target_instance: Option<usize>,
        draggable: bool,
        was_dragged: bool,
        monitor_interaction: bool,
    },
    VariableValue {
        key: String,
        value: f64,
    },
    ListAppend {
        key: String,
    },
    ListSetItem {
        key: String,
        index: usize,
        value: String,
    },
    ListDeleteItem {
        key: String,
        index: usize,
    },
    ListReplace {
        key: String,
        items: Vec<String>,
    },
}

// mirror Scratch's mouse-down versus mouse-up activation rule after renderer picking.
pub fn pointer_click_activates(
    pressed: bool,
    inside_stage: bool,
    draggable: bool,
    was_dragged: bool,
    monitor_interaction: bool,
) -> bool {
    inside_stage
        && !was_dragged
        && !monitor_interaction
        && if draggable { !pressed } else { pressed }
}

#[derive(Debug, Default)]
pub(super) struct RuntimeInputState {
    pressed_keys: HashSet<String>,
    mouse_x: f64,
    mouse_y: f64,
    mouse_down: bool,
}

impl RuntimeInputState {
    fn apply(&mut self, event: RuntimeInputEvent) -> (Option<String>, bool) {
        match event {
            RuntimeInputEvent::Key { name, pressed } => {
                let Some(name) = scratch_key_from_host(&name) else {
                    return (None, false);
                };
                if pressed {
                    let changed = self.pressed_keys.insert(name.clone());
                    (Some(name.to_lowercase()), changed)
                } else {
                    let changed = self.pressed_keys.remove(&name);
                    (None, changed)
                }
            }
            RuntimeInputEvent::ClearKeys => {
                let changed = !self.pressed_keys.is_empty();
                self.pressed_keys.clear();
                (None, changed)
            }
            RuntimeInputEvent::MousePosition { x, y } => {
                let changed = self.mouse_x != x || self.mouse_y != y;
                self.mouse_x = x;
                self.mouse_y = y;
                (None, changed)
            }
            RuntimeInputEvent::MouseButton { pressed } => {
                let changed = self.mouse_down != pressed;
                self.mouse_down = pressed;
                (None, changed)
            }
            RuntimeInputEvent::PointerButton { .. } => (None, false),
            RuntimeInputEvent::VariableValue { .. }
            | RuntimeInputEvent::ListAppend { .. }
            | RuntimeInputEvent::ListSetItem { .. }
            | RuntimeInputEvent::ListDeleteItem { .. }
            | RuntimeInputEvent::ListReplace { .. } => (None, false),
        }
    }
}

// match the key names accepted from a logical host keyboard event by scratch vm.
fn scratch_key_from_host(key: &str) -> Option<String> {
    let special = match key {
        " " => Some("space"),
        "ArrowLeft" | "Left" => Some("left arrow"),
        "ArrowUp" | "Up" => Some("up arrow"),
        "ArrowRight" | "Right" => Some("right arrow"),
        "ArrowDown" | "Down" => Some("down arrow"),
        "Enter" => Some("enter"),
        _ => None,
    };
    if let Some(special) = special {
        return Some(special.to_owned());
    }

    // scratch ignores modifier and named keys whose logical name has multiple code units.
    if key.encode_utf16().count() != 1 {
        return None;
    }
    Some(key.to_uppercase())
}

// convert the sensing block argument separately from host events, as scratch does.
fn scratch_key_from_argument(value: ScratchValueView<'_>) -> Option<String> {
    if let ScratchValueView::Number(number) = value {
        if (48.0..=90.0).contains(&number) {
            return char::from_u32(number as u32).map(|key| key.to_uppercase().to_string());
        }
        let key = match number {
            32.0 => "space",
            37.0 => "left arrow",
            38.0 => "up arrow",
            39.0 => "right arrow",
            40.0 => "down arrow",
            _ => "",
        };
        if !key.is_empty() {
            return Some(key.to_owned());
        }
    }

    let ScratchValueView::Text(text) = value else {
        let text = scratch_string(match value {
            ScratchValueView::Number(number) => ScratchValue::Number(number),
            ScratchValueView::Boolean(value) => ScratchValue::Boolean(value),
            ScratchValueView::Undefined => ScratchValue::Undefined,
            ScratchValueView::Text(_) => unreachable!(),
        });
        return scratch_key_from_text(&text);
    };
    if matches!(
        text,
        "space" | "left arrow" | "up arrow" | "right arrow" | "down arrow" | "enter"
    ) {
        return Some(text.to_owned());
    }
    scratch_key_from_text(text)
}

fn scratch_key_from_text(text: &str) -> Option<String> {
    let key = text.chars().next()?;
    if key == ' ' {
        Some("space".to_owned())
    } else {
        Some(key.to_uppercase().to_string())
    }
}

use time::OffsetDateTime;

fn local_datetime() -> OffsetDateTime {
    OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc())
}

impl ScratchState {
    /// update canonical input state from the renderer's queued events.
    pub fn apply_input(&mut self, event: RuntimeInputEvent) -> Option<String> {
        match event {
            RuntimeInputEvent::VariableValue { key, value } => {
                self.apply_variable_edit(&key, value);
                None
            }
            RuntimeInputEvent::ListAppend { key } => {
                self.apply_list_monitor_edit(&key, ListMonitorEdit::Append);
                None
            }
            RuntimeInputEvent::ListSetItem { key, index, value } => {
                self.apply_list_monitor_edit(&key, ListMonitorEdit::SetItem { index, value });
                None
            }
            RuntimeInputEvent::ListDeleteItem { key, index } => {
                self.apply_list_monitor_edit(&key, ListMonitorEdit::DeleteItem { index });
                None
            }
            RuntimeInputEvent::ListReplace { key, items } => {
                self.apply_list_monitor_edit(&key, ListMonitorEdit::Replace(items));
                None
            }
            event => {
                let (keypress, changed) = self.input.apply(event);
                if changed {
                    self.request_presentation();
                }
                keypress
            }
        }
    }

    pub(super) fn apply_variable_edit(&mut self, key: &str, value: f64) -> bool {
        let target = self
            .variables
            .iter()
            .enumerate()
            .find_map(|(slot, metadata)| {
                let instance = if metadata.is_stage {
                    None
                } else {
                    Some(self.original_instance(metadata.target_index)?)
                };
                (variable_snapshot_key(&metadata.target, &metadata.name, &metadata.id, instance)
                    == key)
                    .then_some((slot, instance))
            });
        let Some((slot, instance)) = target else {
            return false;
        };
        let next = ScratchValue::Number(value);
        if let Some(instance) = instance {
            let previous_instance = self.current_instance;
            self.current_instance = instance;
            self.set(slot, next);
            self.current_instance = previous_instance;
        } else {
            self.set_shared(slot, next);
        }
        true
    }

    // keep reporter argument conversion separate from normalized host key state.
    pub fn key_pressed(&self, key: impl ScratchInput) -> bool {
        let value = key.scratch_view();
        if matches!(value, ScratchValueView::Text("any")) {
            return !self.input.pressed_keys.is_empty();
        }
        scratch_key_from_argument(value).is_some_and(|key| self.input.pressed_keys.contains(&key))
    }

    pub fn mouse_x(&self) -> f64 {
        self.input.mouse_x
    }

    pub fn mouse_y(&self) -> f64 {
        self.input.mouse_y
    }

    pub fn mouse_down(&self) -> bool {
        self.input.mouse_down
    }

    /// reset the project-wide Scratch timer against the runtime clock.
    pub fn reset_project_timer(&mut self, now_millis: u64) {
        self.project_timer_started_at_millis = now_millis;
        self.project_timer_epoch = self.project_timer_epoch.wrapping_add(1);
        if self.project_timer_monitor_visible {
            self.request_presentation();
        }
    }

    pub fn set_username(&mut self, username: String) {
        self.username = username;
    }

    pub fn username(&self) -> &str {
        &self.username
    }

    pub fn days_since_2000(&self) -> f64 {
        const UNIX_DAYS_TO_2000: f64 = 10_957.0;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        now.as_secs_f64() / 86_400.0 - UNIX_DAYS_TO_2000
    }

    pub fn current_year(&self) -> f64 {
        local_datetime().year() as f64
    }

    pub fn current_month(&self) -> f64 {
        local_datetime().month() as u8 as f64
    }

    pub fn current_date(&self) -> f64 {
        local_datetime().day() as f64
    }

    pub fn current_day_of_week(&self) -> f64 {
        local_datetime().weekday().number_from_sunday() as f64
    }

    pub fn current_hour(&self) -> f64 {
        local_datetime().hour() as f64
    }

    pub fn current_minute(&self) -> f64 {
        local_datetime().minute() as f64
    }

    pub fn current_second(&self) -> f64 {
        local_datetime().second() as f64
    }

    /// microphone input is unavailable until AIDA has an audio input device.
    pub fn loudness(&self) -> f64 {
        self.loudness
    }

    /// set the latest audio input sample without polling from script execution.
    pub fn set_loudness(&mut self, loudness: f64) {
        let next = if loudness.is_finite() { loudness } else { -1.0 };
        if self.loudness != next {
            self.loudness = next;
            self.request_presentation();
        }
    }

    /// return elapsed project time without adding clock state to script frames.
    pub fn project_timer_seconds(&self, now_millis: u64) -> f64 {
        self.project_timer_millis(now_millis) as f64 / 1000.0
    }

    pub fn project_timer_millis(&self, now_millis: u64) -> u64 {
        now_millis.saturating_sub(self.project_timer_started_at_millis)
    }
}
