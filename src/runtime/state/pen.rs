//! Pen operations for Scratch project state.

use super::*;

impl ScratchState {
    /// update the current target's pen color using scratch's parameter ranges.
    // accept the generated menu string directly so fixed dropdowns allocate nothing per block.
    pub fn set_pen_color_parameter(
        &mut self,
        parameter: impl Into<PenColorParameter>,
        value: f64,
        change: bool,
    ) {
        self.instances[self.current_instance]
            .pen_color
            .set_or_change_parameter(parameter, value, change);
    }

    /// set the current target's pen color from a scratch rgb input.
    pub fn set_pen_rgb_color(&mut self, color: impl super::ScratchInput) {
        self.instances[self.current_instance]
            .pen_color
            .set_rgb_color(color);
    }

    /// use the generated fast path when this target has no hsv-based pen controls.
    pub fn set_pen_rgb_color_without_hsv(&mut self, color: impl super::ScratchInput) {
        self.instances[self.current_instance]
            .pen_color
            .set_rgb_color_without_hsv(color);
    }

    pub fn clear_pen(&mut self) {
        self.pending_pen_commands.push(PenCommand::Clear);
        self.request_redraw();
    }

    pub fn pen_down(&mut self) {
        let instance_id = self.current_instance;
        let stroke = {
            let instance = &mut self.instances[instance_id];
            if instance.is_stage {
                return;
            }
            instance.pen_down = true;
            let point = [instance.x as f32, instance.y as f32];
            PenDot {
                position: point,
                color: instance.pen_color.rgba,
                diameter: instance.pen_size as f32,
                pixel_offset: 0.0,
            }
        };
        self.push_pen_dot(stroke);
        self.request_redraw();
    }

    pub fn pen_up(&mut self) {
        self.instances[self.current_instance].pen_down = false;
    }

    pub fn set_pen_size(&mut self, size: f64) {
        let size = scratch_to_number(size).clamp(1.0, 1200.0);
        self.instances[self.current_instance].pen_size = size;
    }

    pub fn change_pen_size(&mut self, change: f64) {
        let size = self.instances[self.current_instance].pen_size + scratch_to_number(change);
        self.set_pen_size(size);
    }

    pub fn stamp(&mut self) {
        let instance = &self.instances[self.current_instance];
        if instance.is_stage {
            return;
        }
        let Some((_, target_name, _)) = self
            .target_names
            .iter()
            .find(|(target, _, _)| *target == instance.target)
        else {
            return;
        };
        self.pending_pen_commands.push(PenCommand::Stamp {
            target_name: target_name.clone(),
            position: [instance.x as f32, instance.y as f32],
            direction: instance.direction as f32,
            scale: instance.size as f32 / 100.0,
            costume_index: instance.costume_index,
            effects_0: [0.0; 4],
            effects_1: [0.0; 4],
        });
        self.request_redraw();
    }

    pub fn take_pending_pen_commands(&mut self) -> Vec<PenCommand> {
        std::mem::take(&mut self.pending_pen_commands)
    }

    /// report whether ordered pen marks can be accumulated before presentation.
    pub fn has_pending_pen_commands(&self) -> bool {
        !self.pending_pen_commands.is_empty()
    }

    #[inline]
    pub(super) fn push_pen_dot(&mut self, dot: PenDot) {
        match self.pending_pen_commands.last_mut() {
            Some(PenCommand::Dots(dots)) => dots.push(dot),
            Some(PenCommand::Mixed(marks)) => marks.push(dot.as_stroke()),
            _ => {
                self.pending_pen_commands.push(PenCommand::Dots(vec![dot]));
                self.coalesce_small_pen_batches();
            }
        }
    }

    #[inline]
    pub(super) fn push_pen_stroke(&mut self, stroke: PenStroke) {
        match self.pending_pen_commands.last_mut() {
            Some(PenCommand::Strokes(strokes) | PenCommand::Mixed(strokes)) => strokes.push(stroke),
            _ => {
                self.pending_pen_commands
                    .push(PenCommand::Strokes(vec![stroke]));
                self.coalesce_small_pen_batches();
            }
        }
    }

    // Promote only a run of small alternating batches. Large homogeneous dot
    // buffers keep their compact representation and clears/stamps stay barriers.
    fn coalesce_small_pen_batches(&mut self) {
        const RUN: usize = 16;
        let Some(start) = self.pending_pen_commands.len().checked_sub(RUN) else {
            return;
        };
        if !self.pending_pen_commands[start..]
            .iter()
            .all(|command| match command {
                PenCommand::Dots(dots) => dots.len() <= 4,
                PenCommand::Strokes(strokes) => strokes.len() <= 4,
                _ => false,
            })
        {
            return;
        }
        let batches = self.pending_pen_commands.split_off(start);
        let mut marks = Vec::with_capacity(RUN * 2);
        for batch in batches {
            match batch {
                PenCommand::Dots(dots) => marks.extend(dots.into_iter().map(PenDot::as_stroke)),
                PenCommand::Strokes(strokes) => marks.extend(strokes),
                _ => unreachable!(),
            }
        }
        self.pending_pen_commands.push(PenCommand::Mixed(marks));
    }

    pub fn set_legacy_pen_hue(&mut self, hue: f64) {
        self.instances[self.current_instance]
            .pen_color
            .set_legacy_hue(hue);
    }

    pub fn change_legacy_pen_hue(&mut self, hue: f64) {
        self.instances[self.current_instance]
            .pen_color
            .change_legacy_hue(hue);
    }

    pub fn set_legacy_pen_shade(&mut self, shade: f64) {
        self.instances[self.current_instance]
            .pen_color
            .set_legacy_shade(shade);
    }

    pub fn change_legacy_pen_shade(&mut self, shade: f64) {
        self.instances[self.current_instance]
            .pen_color
            .change_legacy_shade(shade);
    }

    /// read the current target's calculated pen color for renderer commands.
    pub fn current_pen_rgba(&self) -> [f32; 4] {
        self.instances[self.current_instance].pen_color.rgba
    }
}
