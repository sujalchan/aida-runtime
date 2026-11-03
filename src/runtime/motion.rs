/// time-based motion state for a Scratch glide block.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlideState {
    started_at_millis: u64,
    duration_millis: f64,
    start_x: f64,
    start_y: f64,
    end_x: f64,
    end_y: f64,
}

impl GlideState {
    pub fn new(
        started_at_millis: u64,
        duration_seconds: f64,
        start: (f64, f64),
        end: (f64, f64),
    ) -> Self {
        Self {
            started_at_millis,
            duration_millis: duration_seconds * 1000.0,
            start_x: start.0,
            start_y: start.1,
            end_x: end.0,
            end_y: end.1,
        }
    }

    /// return the interpolated position while the glide is in progress.
    pub fn position_at(&self, now_millis: u64) -> Option<(f64, f64)> {
        if !(self.duration_millis > 0.0) {
            return None;
        }
        let elapsed_millis = now_millis.saturating_sub(self.started_at_millis) as f64;
        if elapsed_millis >= self.duration_millis {
            return None;
        }
        let fraction = elapsed_millis / self.duration_millis;
        Some((
            self.start_x + fraction * (self.end_x - self.start_x),
            self.start_y + fraction * (self.end_y - self.start_y),
        ))
    }

    pub fn destination(&self) -> (f64, f64) {
        (self.end_x, self.end_y)
    }
}

#[cfg(test)]
mod tests {
    use super::GlideState;

    #[test]
    fn glide_interpolates_from_start_through_midpoint_to_destination() {
        let glide = GlideState::new(100, 1.0, (-10.0, 10.0), (30.0, -30.0));
        assert_eq!(glide.position_at(100), Some((-10.0, 10.0)));
        assert_eq!(glide.position_at(600), Some((10.0, -10.0)));
        assert_eq!(glide.position_at(1100), None);
        assert_eq!(glide.destination(), (30.0, -30.0));
    }

    #[test]
    fn non_positive_and_nan_durations_finish_immediately() {
        for duration in [0.0, -1.0, f64::NAN] {
            let glide = GlideState::new(0, duration, (2.0, 3.0), (8.0, 9.0));
            assert_eq!(glide.position_at(0), None);
            assert_eq!(glide.destination(), (8.0, 9.0));
        }
    }
}
