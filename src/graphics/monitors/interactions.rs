use super::*;

impl VariableLayer {
    // bring the selected monitor to the front before moving it
    pub(crate) fn begin_drag(&mut self, x: f32, y: f32) -> bool {
        let hit = self.hit_test(x, y).map(|index| {
            let monitor = &self.monitors[index];
            (index, x - monitor.x, monitor.y - y)
        });

        if let Some((index, offset_x, offset_y)) = hit {
            let monitor = self.monitors.remove(index);
            self.monitors.push(monitor);
            let index = self.monitors.len() - 1;
            self.slider_hovered = None;
            self.slider_thumb_hovered = None;
            self.dragging = Some((index, offset_x, offset_y));
            self.shadow_monitor = Some(index);
            self.shadow_opacity = 0.0;
            self.start_shadow_fade(SHADOW_MAX_OPACITY);
            true
        } else {
            false
        }
    }

    // cycle through expanded, value only, and slider modes at the pointer
    pub(crate) fn toggle_compact_at(&mut self, x: f32, y: f32) -> Option<(usize, MonitorBounds)> {
        let index = self.hit_test(x, y)?;
        let monitor = &self.monitors[index];
        let display_mode = match monitor.display_mode {
            MonitorDisplayMode::List => return None,
            MonitorDisplayMode::Expanded => MonitorDisplayMode::ValueOnly,
            MonitorDisplayMode::ValueOnly if monitor.kind == MonitorKind::ScalarVariable => {
                MonitorDisplayMode::Slider
            }
            MonitorDisplayMode::ValueOnly | MonitorDisplayMode::Slider => {
                MonitorDisplayMode::Expanded
            }
        };
        let old_bounds = self.set_monitor_display_mode(index, display_mode);
        self.slider_hovered = None;
        self.slider_thumb_hovered = None;
        Some((index, old_bounds))
    }

    pub(super) fn set_monitor_display_mode(
        &mut self,
        index: usize,
        display_mode: MonitorDisplayMode,
    ) -> MonitorBounds {
        let old_bounds = self.monitor_bounds(index);
        let current_value_width = {
            let monitor = &self.monitors[index];
            text_width(&self.regular_font, &monitor.value, 19.0)
                + monitor.value_horizontal_padding * 2.0
        };
        let monitor = &mut self.monitors[index];
        monitor.display_mode = display_mode;
        monitor.value_width = current_value_width;
        monitor.value_text_width = current_value_width - monitor.value_horizontal_padding * 2.0;
        if display_mode == MonitorDisplayMode::Slider {
            monitor.slider_min_value_width = current_value_width;
        }
        if monitor.kind == MonitorKind::List {
            monitor.width = monitor.width.max(64.0);
            monitor.height = monitor.height.max(68.0);
        } else {
            monitor.width = if display_mode == MonitorDisplayMode::ValueOnly {
                monitor.value_width
            } else {
                expanded_width(monitor.label_width, monitor.value_width)
            };
            monitor.height = if display_mode == MonitorDisplayMode::Slider {
                58.0
            } else {
                34.0
            };
        }
        monitor.x = clamp_monitor_x(monitor.x, monitor.width, monitor.scale);
        monitor.y = clamp_monitor_y(monitor.y, monitor.height, monitor.scale);
        self.mark_gpu_monitor(index);
        old_bounds
    }

    pub(crate) fn monitor_bounds(&self, index: usize) -> MonitorBounds {
        let monitor = &self.monitors[index];
        let (left, top) = if self.uses_gpu_translation() {
            (
                monitor.origin_x + STAGE_HALF_WIDTH,
                STAGE_HALF_HEIGHT - monitor.origin_y,
            )
        } else {
            (monitor.x + STAGE_HALF_WIDTH, STAGE_HALF_HEIGHT - monitor.y)
        };
        MonitorBounds {
            left,
            top,
            right: left + monitor.width * monitor.scale,
            bottom: top + monitor.height * monitor.scale,
        }
    }

    pub(crate) fn visual_monitor_bounds(&self, index: usize) -> MonitorBounds {
        let monitor = &self.monitors[index];
        let left = monitor.x + STAGE_HALF_WIDTH;
        let top = STAGE_HALF_HEIGHT - monitor.y;
        MonitorBounds {
            left,
            top,
            right: left + monitor.width * monitor.scale,
            bottom: top + monitor.height * monitor.scale,
        }
    }

    // start a slider interaction when the pointer is over its track
    pub(crate) fn begin_slider_drag(&mut self, x: f32, y: f32) -> bool {
        let Some(index) = self.hit_test(x, y) else {
            return false;
        };
        let monitor = &self.monitors[index];
        if monitor.display_mode != MonitorDisplayMode::Slider {
            return false;
        }
        let local_x = (x - monitor.x) / monitor.scale;
        let local_y = (monitor.y - y) / monitor.scale;
        if !(14.0..=monitor.width - 14.0).contains(&local_x) || (local_y - 42.0).abs() > 11.0 {
            return false;
        }
        self.slider_dragging = Some(index);
        self.slider_hovered = None;
        self.slider_thumb_hovered = None;
        self.set_slider_value(index, x);
        true
    }

    // track hover affects the bar while thumb hover affects only the handle
    pub(crate) fn update_slider_hover(
        &mut self,
        pointer: Option<(f32, f32)>,
    ) -> Option<(Option<usize>, Option<usize>)> {
        let previous_track = self.slider_hovered;
        let previous_thumb = self.slider_thumb_hovered;
        let hovered = if self.slider_dragging.is_some() {
            None
        } else {
            pointer
                .and_then(|(x, y)| self.hit_test(x, y))
                .filter(|&index| {
                    let (x, y) = pointer.expect("hovered slider must have a pointer position");
                    let monitor = &self.monitors[index];
                    let local_x = (x - monitor.x) / monitor.scale;
                    let local_y = (monitor.y - y) / monitor.scale;
                    monitor.display_mode == MonitorDisplayMode::Slider
                        && (14.0..=monitor.width - 14.0).contains(&local_x)
                        && (local_y - 42.0).abs() <= 11.0
                })
        };
        let hovered_thumb = hovered.filter(|&index| {
            let (x, y) = pointer.expect("hovered thumb must have a pointer position");
            let monitor = &self.monitors[index];
            let local_x = (x - monitor.x) / monitor.scale;
            let local_y = (monitor.y - y) / monitor.scale;
            let thumb_x = 14.0 + (monitor.width - 28.0) * monitor_slider_ratio(monitor);
            (local_x - thumb_x).powi(2) + (local_y - 42.0).powi(2) <= 11.0_f32.powi(2)
        });
        if self.slider_hovered == hovered && self.slider_thumb_hovered == hovered_thumb {
            return None;
        }
        self.slider_hovered = hovered;
        self.slider_thumb_hovered = hovered_thumb;
        if let Some(previous) = previous_track.or(previous_thumb) {
            self.mark_gpu_shapes(previous);
        }
        if let Some(next) = hovered.or(hovered_thumb) {
            self.mark_gpu_shapes(next);
        }
        Some((previous_track.or(previous_thumb), hovered.or(hovered_thumb)))
    }

    // find the topmost monitor under a logical stage coordinate
    pub(crate) fn hit_test(&self, x: f32, y: f32) -> Option<usize> {
        let pointer_x = x + STAGE_HALF_WIDTH;
        let pointer_y = STAGE_HALF_HEIGHT - y;
        self.monitors
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, monitor)| {
                if monitor.visibility == MonitorVisibility::Hidden {
                    return None;
                }
                let left = monitor.x + STAGE_HALF_WIDTH;
                let top = STAGE_HALF_HEIGHT - monitor.y;
                (pointer_x >= left
                    && pointer_x <= left + monitor.width * monitor.scale
                    && pointer_y >= top
                    && pointer_y <= top + monitor.height * monitor.scale)
                    .then_some(index)
            })
    }

    // move a monitor or update its value depending on the active interaction
    pub(crate) fn drag_to(&mut self, x: f32, y: f32) -> bool {
        if let Some(index) = self.slider_dragging {
            return self.set_slider_value(index, x);
        }
        if let Some(drag) = self.list_scrollbar_drag {
            let Some(scrollbar) = self.list_scrollbar_geometry(drag.monitor_index) else {
                return false;
            };
            let list_len = self.list_display_len(drag.monitor_index);
            let monitor = &self.monitors[drag.monitor_index];
            let local_y = (monitor.y - y) / monitor.scale;
            let thumb_travel = scrollbar.track_height - scrollbar.thumb_height;
            if thumb_travel <= 0.0 {
                return false;
            }
            let progress = ((local_y - drag.pointer_offset - scrollbar.track_y) / thumb_travel)
                .clamp(0.0, 1.0);
            let max_offset = monitor.max_list_scroll_offset(list_len);
            return self.set_list_scroll_offset(
                drag.monitor_index,
                (progress * max_offset as f32).round() as usize,
            );
        }
        if let Some(resize) = self.list_resize.as_ref() {
            let index = resize.monitor_index;
            let display_len = self.list_display_len(index);
            let monitor = &mut self.monitors[index];
            let max_width =
                ((SCRATCH_WIDTH as f32 - (monitor.x + STAGE_HALF_WIDTH)) / monitor.scale).max(64.0);
            let max_height = ((SCRATCH_HEIGHT as f32 - (STAGE_HALF_HEIGHT - monitor.y))
                / monitor.scale)
                .max(68.0);
            let width =
                (resize.width + (x - resize.pointer_x) / monitor.scale).clamp(64.0, max_width);
            let height =
                (resize.height - (y - resize.pointer_y) / monitor.scale).clamp(68.0, max_height);
            if monitor.width == width && monitor.height == height {
                return false;
            }
            monitor.width = width;
            monitor.height = height;
            monitor.list_scroll_offset = monitor
                .list_scroll_offset
                .min(monitor.max_list_scroll_offset(display_len));
            self.mark_gpu_shapes(index);
            self.mark_gpu_text(index);
            return true;
        }
        let Some((index, offset_x, offset_y)) = self.dragging else {
            return false;
        };
        let monitor = &mut self.monitors[index];
        let next_x = clamp_monitor_x(x - offset_x, monitor.width, monitor.scale);
        let next_y = clamp_monitor_y(y + offset_y, monitor.height, monitor.scale);
        if monitor.x == next_x && monitor.y == next_y {
            return false;
        }
        monitor.x = next_x;
        monitor.y = next_y;
        self.mark_gpu_transform(index);
        true
    }

    pub(crate) fn slider_drag_index(&self) -> Option<usize> {
        self.slider_dragging
    }

    pub(crate) fn take_changed_manual_values(&mut self) -> Vec<(String, f64)> {
        self.changed_manual_values.drain().collect()
    }

    pub(crate) fn drag_index(&self) -> Option<usize> {
        self.dragging.map(|(index, _, _)| index)
    }

    pub(crate) fn take_slider_width_changed(&mut self) -> bool {
        std::mem::take(&mut self.slider_width_changed)
    }

    pub(crate) fn end_drag(&mut self) {
        self.slider_dragging = None;
        self.dragging = None;
        self.list_resize = None;
        if let Some(drag) = self.list_scrollbar_drag.take() {
            self.mark_gpu_shapes(drag.monitor_index);
        }
        if self.shadow_monitor.is_some() {
            self.start_shadow_fade(0.0);
        }
    }

    // map the pointer across the slider range and grow its layout when needed
    fn set_slider_value(&mut self, index: usize, pointer_x: f32) -> bool {
        let (value, local_value) = {
            let monitor = &self.monitors[index];
            let local_x = (pointer_x - monitor.x) / monitor.scale;
            let ratio = ((local_x - 14.0) / (monitor.width - 28.0)).clamp(0.0, 1.0);
            let value =
                monitor.slider_min + ratio as f64 * (monitor.slider_max - monitor.slider_min);
            let text = format_slider_value(value, monitor.slider_min, monitor.slider_max);
            (text.parse().unwrap_or(value), text)
        };
        let value_width = local_value.chars().count() as f32 * self.value_digit_width
            + self.monitors[index].value_horizontal_padding * 2.0;
        let old_x = self.monitors[index].x;
        let monitor = &mut self.monitors[index];
        if monitor.value == local_value {
            return false;
        }
        monitor.value = local_value;
        monitor.value_text_width = value_width - monitor.value_horizontal_padding * 2.0;
        let previous_width = monitor.width;
        monitor.value_width = if monitor.display_mode == MonitorDisplayMode::Slider {
            monitor
                .value_width
                .max(value_width)
                .max(monitor.slider_min_value_width)
        } else {
            value_width
        };
        if monitor.display_mode == MonitorDisplayMode::Slider {
            monitor.width = expanded_width(monitor.label_width, monitor.value_width);
            monitor.x = clamp_monitor_x(monitor.x, monitor.width, monitor.scale);
            self.slider_width_changed |= monitor.width != previous_width;
        }
        self.mark_gpu_text(index);
        if self.monitors[index].display_mode == MonitorDisplayMode::Slider {
            self.mark_gpu_shapes(index);
        }
        if self.monitors[index].x != old_x {
            self.mark_gpu_transform(index);
        }
        self.changed_manual_values
            .insert(self.monitors[index].value_key.clone(), value);
        true
    }
}
