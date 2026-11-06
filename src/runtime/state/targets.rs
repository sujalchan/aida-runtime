//! Targets operations for Scratch project state.

use super::*;

fn scratch_direction(direction: f64) -> f64 {
    direction - (((direction + 179.0) / 360.0).floor() * 360.0)
}

// match scratch's display precision without changing the stored target coordinates.
pub fn scratch_motion_coordinate(coordinate: f64) -> f64 {
    let rounded = coordinate.round();
    if (coordinate - rounded).abs() < 1e-9 {
        rounded
    } else {
        coordinate
    }
}

fn costume_corners(geometry: &SpriteFenceGeometry) -> [(f64, f64); 4] {
    [
        (0.0, 0.0),
        (geometry.width, 0.0),
        (0.0, geometry.height),
        (geometry.width, geometry.height),
    ]
}

#[derive(Clone, Copy)]
struct CostumeTransform {
    rotation_center: (f64, f64),
    rotation: (f64, f64),
    flip_x: f64,
    scale: f64,
}

fn transform_costume_point(point: (f64, f64), transform: CostumeTransform) -> (f64, f64) {
    let local_x = (point.0 - transform.rotation_center.0) * transform.flip_x * transform.scale;
    let local_y = (transform.rotation_center.1 - point.1) * transform.scale;
    (
        local_x * transform.rotation.0 - local_y * transform.rotation.1,
        local_x * transform.rotation.1 + local_y * transform.rotation.0,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScratchRotationStyle {
    #[default]
    AllAround,
    LeftRight,
    DontRotate,
}

impl ScratchRotationStyle {
    pub fn from_scratch(value: &str) -> Option<Self> {
        match value {
            "all around" => Some(Self::AllAround),
            "left-right" => Some(Self::LeftRight),
            "don't rotate" => Some(Self::DontRotate),
            _ => None,
        }
    }

    pub fn rendered_direction_and_flip(self, direction: f64) -> (f64, f64) {
        match self {
            Self::AllAround => (direction, 1.0),
            Self::LeftRight => (90.0, if direction < 0.0 { -1.0 } else { 1.0 }),
            Self::DontRotate => (90.0, 1.0),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SpriteFenceGeometry {
    pub width: f64,
    pub height: f64,
    pub rotation_center_x: f64,
    pub rotation_center_y: f64,
    pub bounds_hull: Option<Vec<[f64; 2]>>,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct CachedFenceOffsets {
    pub(super) geometry: [f64; 4],
    pub(super) direction: f64,
    pub(super) size: f64,
    pub(super) rotation_style: ScratchRotationStyle,
    pub(super) points: [(f64, f64); 4],
}

#[derive(Debug, Clone)]
pub(super) struct ScratchTargetInstance {
    pub(super) target: usize,
    pub(super) original: bool,
    pub(super) alive: bool,
    pub(super) is_stage: bool,
    pub(super) x: f64,
    pub(super) y: f64,
    pub(super) direction: f64,
    pub(super) rotation_style: ScratchRotationStyle,
    pub(super) size: f64,
    pub(super) visible: bool,
    pub(super) values: Vec<Option<ScratchValue>>,
    pub(super) lists: Vec<Option<ScratchList>>,
    pub(super) bubble: Option<SpeechBubble>,
    pub(super) pen_color: PenColorState,
    pub(super) pen_down: bool,
    pub(super) pen_size: f64,
    pub(super) costume_index: usize,
    pub(super) fence_offsets: Option<CachedFenceOffsets>,
    pub(super) sound_pitch: f64,
    pub(super) sound_pan: f64,
    pub(super) volume: f64,
}

#[derive(Debug, Clone)]
pub(super) struct ScratchCostumeMetadata {
    pub(super) name: String,
    size_bounds: Option<(f64, f64)>,
}

impl ScratchCostumeMetadata {
    fn new(name: String, logical_size: Option<[f64; 2]>) -> Self {
        let size_bounds = logical_size
            .filter(|[width, height]| {
                width.is_finite() && height.is_finite() && *width >= 0.0 && *height >= 0.0
            })
            .map(|[width, height]| {
                let min_scale = 1.0_f64.min((5.0 / width).max(5.0 / height));
                let max_scale = ((1.5 * 480.0) / width).min((1.5 * 360.0) / height);
                (min_scale, max_scale)
            });
        Self { name, size_bounds }
    }
}

fn wrapped_costume_index(index: f64, costume_count: usize) -> usize {
    let rounded = super::scratch_round(index);
    if !rounded.is_finite() || costume_count == 0 {
        return 0;
    }
    rounded.rem_euclid(costume_count as f64) as usize
}

fn costume_number_from_text(text: &str) -> Option<f64> {
    let text = text.trim();
    if text.is_empty() || matches!(text, "inf" | "+inf" | "-inf" | "INF" | "+INF" | "-INF") {
        return None;
    }
    if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        return u64::from_str_radix(hex, 16).ok().map(|value| value as f64);
    }
    if let Some(binary) = text.strip_prefix("0b").or_else(|| text.strip_prefix("0B")) {
        return u64::from_str_radix(binary, 2)
            .ok()
            .map(|value| value as f64);
    }
    if let Some(octal) = text.strip_prefix("0o").or_else(|| text.strip_prefix("0O")) {
        return u64::from_str_radix(octal, 8).ok().map(|value| value as f64);
    }
    let number = text.parse::<f64>().ok()?;
    (!number.is_nan()).then_some(number)
}

impl ScratchState {
    pub fn set_backdrops(&mut self, names: Vec<String>, current_index: usize) {
        self.backdrop_names = names;
        self.backdrop_index = current_index.min(self.backdrop_names.len().saturating_sub(1));
        self.pending_backdrop_events.clear();
    }

    pub fn backdrop_number_name(&self, number: bool) -> ScratchValue {
        if number {
            ScratchValue::Number(self.backdrop_index as f64 + 1.0)
        } else {
            ScratchValue::Text(
                self.backdrop_names
                    .get(self.backdrop_index)
                    .cloned()
                    .unwrap_or_default(),
            )
        }
    }

    pub fn switch_backdrop(&mut self, value: impl super::ScratchInput) {
        self.select_backdrop(value.scratch_view(), false);
    }

    pub fn next_backdrop(&mut self) {
        self.select_backdrop(
            super::ScratchValueView::Number(self.backdrop_index as f64 + 1.0),
            true,
        );
    }

    pub fn take_pending_backdrop_events(&mut self) -> Vec<String> {
        std::mem::take(&mut self.pending_backdrop_events)
    }

    fn select_backdrop(&mut self, value: super::ScratchValueView<'_>, zero_indexed: bool) {
        if self.backdrop_names.is_empty() {
            return;
        }
        let selected = match value {
            super::ScratchValueView::Number(number) => {
                self.backdrop_index_from_number(number, zero_indexed)
            }
            super::ScratchValueView::Text(text) => {
                if let Some(index) = self.backdrop_names.iter().position(|name| name == text) {
                    Some(index)
                } else if text == "next backdrop" {
                    Some((self.backdrop_index + 1) % self.backdrop_names.len())
                } else if text == "previous backdrop" {
                    Some(
                        (self.backdrop_index + self.backdrop_names.len() - 1)
                            % self.backdrop_names.len(),
                    )
                } else if text == "random backdrop" {
                    if self.backdrop_names.len() < 2 {
                        Some(self.backdrop_index)
                    } else {
                        let other =
                            scratch_random(0.0, (self.backdrop_names.len() - 2) as f64) as usize;
                        Some(if other >= self.backdrop_index {
                            other + 1
                        } else {
                            other
                        })
                    }
                } else if text.trim().is_empty() {
                    None
                } else {
                    text.trim()
                        .parse::<f64>()
                        .ok()
                        .and_then(|number| self.backdrop_index_from_number(number, zero_indexed))
                }
            }
            super::ScratchValueView::Boolean(_) => None,
            super::ScratchValueView::Undefined => None,
        };
        if let Some(index) = selected {
            self.backdrop_index = index;
            // scratch's backdrop switch calls setCostume, which requests a
            // redraw even when the selected costume is already active.
            self.request_redraw();
        }
        if let Some(name) = self.backdrop_names.get(self.backdrop_index) {
            self.pending_backdrop_events.push(name.clone());
        }
    }

    fn backdrop_index_from_number(&self, number: f64, zero_indexed: bool) -> Option<usize> {
        let adjusted = if zero_indexed { number } else { number - 1.0 };
        let rounded = if adjusted.is_finite() {
            (adjusted + 0.5).floor()
        } else {
            0.0
        };
        Some((rounded.rem_euclid(self.backdrop_names.len() as f64)) as usize)
    }

    pub fn set_current_instance(&mut self, instance: usize) {
        self.current_instance = instance;
    }

    pub fn current_instance(&self) -> usize {
        self.current_instance
    }

    pub fn current_target(&self) -> usize {
        self.instances[self.current_instance].target
    }

    pub fn set_original_xy(&mut self, target: usize, x: f64, y: f64) {
        if let Some(instance) = self
            .instances
            .iter_mut()
            .find(|instance| instance.target == target && instance.original && !instance.is_stage)
        {
            instance.x = x;
            instance.y = y;
        }
    }

    pub fn set_target_fence_geometry(&mut self, target: usize, geometry: SpriteFenceGeometry) {
        if self.fence_geometry.len() <= target {
            self.fence_geometry.resize(target + 1, None);
        }
        self.fence_geometry[target] = Some(geometry);
        for instance in self
            .instances
            .iter_mut()
            .filter(|instance| instance.target == target)
        {
            instance.fence_offsets = None;
        }
    }

    pub fn set_original_visual(&mut self, target: usize, direction: f64, size: f64, visible: bool) {
        if let Some(instance) = self
            .instances
            .iter_mut()
            .find(|instance| instance.target == target && instance.original && !instance.is_stage)
        {
            if direction.is_finite() {
                instance.direction = scratch_direction(direction);
            }
            instance.size = size.max(0.0);
            instance.visible = visible;
        }
    }

    /// update the executing sprite's Scratch size from an input value.
    pub fn set_size(&mut self, value: impl super::ScratchInput) {
        let requested_size = super::scratch_to_number(value);
        let instance = &self.instances[self.current_instance];
        if instance.is_stage {
            return;
        }
        let size_bounds = self
            .target_costumes
            .get(instance.target)
            .and_then(|costumes| costumes.get(instance.costume_index))
            .and_then(|costume| costume.size_bounds);
        let size = if let Some((min_scale, max_scale)) = size_bounds {
            (requested_size / 100.0).max(min_scale).min(max_scale) * 100.0
        } else {
            // Older generated modules registered names without skin dimensions.
            requested_size.max(0.0)
        };
        let instance = &mut self.instances[self.current_instance];
        if instance.size == size {
            return;
        }
        instance.size = size;
        instance.fence_offsets = None;
        self.request_redraw();
    }

    /// apply Scratch's size change block to the executing sprite.
    pub fn change_size_by(&mut self, value: impl super::ScratchInput) {
        let change = super::scratch_to_number(value);
        let current_size = self.instances[self.current_instance].size;
        self.set_size(current_size + change);
    }

    pub fn set_original_rotation_style(&mut self, target: usize, style: &str) {
        let Some(style) = ScratchRotationStyle::from_scratch(style) else {
            return;
        };
        if let Some(instance) = self
            .instances
            .iter_mut()
            .find(|instance| instance.target == target && instance.original && !instance.is_stage)
        {
            instance.rotation_style = style;
            instance.fence_offsets = None;
        }
    }

    // Compatibility entry point for generated programs without resolved asset metadata.
    pub fn set_target_costumes(&mut self, target: usize, names: Vec<String>, current: usize) {
        let costumes = names
            .into_iter()
            .map(|name| ScratchCostumeMetadata::new(name, None))
            .collect();
        self.set_target_costume_metadata(target, costumes, current);
    }

    // Register immutable costume names and precomputed size limits for each target.
    pub fn set_target_costumes_with_sizes(
        &mut self,
        target: usize,
        costumes: Vec<(String, Option<[f64; 2]>)>,
        current: usize,
    ) {
        let costumes = costumes
            .into_iter()
            .map(|(name, logical_size)| ScratchCostumeMetadata::new(name, logical_size))
            .collect();
        self.set_target_costume_metadata(target, costumes, current);
    }

    fn set_target_costume_metadata(
        &mut self,
        target: usize,
        costumes: Vec<ScratchCostumeMetadata>,
        current: usize,
    ) {
        if self.target_costumes.len() <= target {
            self.target_costumes.resize_with(target + 1, Vec::new);
        }
        self.target_costumes[target] = costumes;
        if let Some(instance) = self
            .instances
            .iter_mut()
            .find(|instance| instance.target == target && instance.original)
        {
            instance.costume_index = current;
        }
    }

    // resolve costume names and 1-based numeric choices using scratch's costume wrapping.
    pub fn switch_costume_to(&mut self, value: ScratchValue) {
        let instance = &self.instances[self.current_instance];
        let target = instance.target;
        let current = instance.costume_index;
        let Some(costumes) = self.target_costumes.get(target) else {
            return;
        };
        if costumes.is_empty() {
            return;
        }
        let index = match value.scratch_view() {
            super::ScratchValueView::Number(number) => {
                Some(wrapped_costume_index(number - 1.0, costumes.len()))
            }
            super::ScratchValueView::Boolean(value) => {
                let text = if value { "true" } else { "false" };
                costumes.iter().position(|costume| costume.name == text)
            }
            super::ScratchValueView::Text(text) => costumes
                .iter()
                .position(|costume| costume.name == text)
                .or_else(|| match text {
                    "next costume" => {
                        Some(wrapped_costume_index(current as f64 + 1.0, costumes.len()))
                    }
                    "previous costume" => {
                        Some(wrapped_costume_index(current as f64 - 1.0, costumes.len()))
                    }
                    _ => costume_number_from_text(text)
                        .map(|number| wrapped_costume_index(number - 1.0, costumes.len())),
                }),
            super::ScratchValueView::Undefined => None,
        };
        let Some(index) = index else {
            return;
        };
        let instance = &mut self.instances[self.current_instance];
        if instance.costume_index != index {
            instance.costume_index = index;
            self.request_redraw();
        }
    }

    pub fn next_costume(&mut self) {
        let instance = &self.instances[self.current_instance];
        let Some(costumes) = self.target_costumes.get(instance.target) else {
            return;
        };
        if costumes.is_empty() {
            return;
        }
        let index = wrapped_costume_index(instance.costume_index as f64 + 1.0, costumes.len());
        let instance = &mut self.instances[self.current_instance];
        if instance.costume_index != index {
            instance.costume_index = index;
            self.request_redraw();
        }
    }

    // report the executing instance's selected costume using scratch's 1-based numbering.
    pub fn costume_number_name(&self, number: bool) -> ScratchValue {
        let instance = &self.instances[self.current_instance];
        let Some(costumes) = self.target_costumes.get(instance.target) else {
            return ScratchValue::Text(String::new());
        };
        if number {
            ScratchValue::Number(instance.costume_index as f64 + 1.0)
        } else {
            ScratchValue::Text(
                costumes
                    .get(instance.costume_index)
                    .map(|costume| costume.name.clone())
                    .unwrap_or_default(),
            )
        }
    }

    /// report the executing sprite's size with Scratch's reporter rounding.
    pub fn size_reporter(&self) -> f64 {
        super::scratch_round(self.instances[self.current_instance].size)
    }

    // update only the executing target's visibility and request a visual frame.
    pub fn set_visible(&mut self, visible: bool) {
        let instance = &mut self.instances[self.current_instance];
        if !instance.is_stage && instance.visible != visible {
            instance.visible = visible;
            self.request_redraw();
        }
    }

    pub fn direction(&self) -> f64 {
        self.instances[self.current_instance].direction
    }

    pub fn x_position(&self) -> f64 {
        scratch_motion_coordinate(self.instances[self.current_instance].x)
    }

    pub fn y_position(&self) -> f64 {
        scratch_motion_coordinate(self.instances[self.current_instance].y)
    }

    pub fn set_rotation_style(&mut self, style: &str) {
        let Some(style) = ScratchRotationStyle::from_scratch(style) else {
            return;
        };
        let instance = &mut self.instances[self.current_instance];
        if instance.is_stage || instance.rotation_style == style {
            return;
        }
        instance.rotation_style = style;
        instance.fence_offsets = None;
        self.request_redraw();
    }

    pub fn move_steps(&mut self, steps: f64) {
        let instance = &self.instances[self.current_instance];
        if instance.is_stage {
            return;
        }
        let radians = (90.0 - instance.direction) * std::f64::consts::PI / 180.0;
        let x = instance.x + steps * radians.cos();
        let y = instance.y + steps * radians.sin();
        self.set_xy(x, y);
    }

    pub fn bounce_on_edge(&mut self) {
        let instance_id = self.current_instance;
        let Some(bounds) = self.transformed_bounds(instance_id, true) else {
            return;
        };
        let left = (240.0 + bounds[0]).max(0.0);
        let top = (180.0 - bounds[3]).max(0.0);
        let right = (240.0 - bounds[1]).max(0.0);
        let bottom = (180.0 + bounds[2]).max(0.0);
        let mut edge = 0;
        let mut minimum = f64::INFINITY;
        for (index, distance) in [left, top, right, bottom].into_iter().enumerate() {
            if distance < minimum {
                minimum = distance;
                edge = index;
            }
        }
        if minimum > 0.0 {
            return;
        }

        let direction = self.instances[instance_id].direction;
        let radians = (90.0 - direction) * std::f64::consts::PI / 180.0;
        let mut dx = radians.cos();
        let mut dy = -radians.sin();
        match edge {
            0 => dx = dx.abs().max(0.2),
            1 => dy = dy.abs().max(0.2),
            2 => dx = -dx.abs().max(0.2),
            _ => dy = -dy.abs().max(0.2),
        }
        self.set_direction(dy.atan2(dx) * 180.0 / std::f64::consts::PI + 90.0);
        self.keep_current_instance_in_stage();
    }

    fn keep_current_instance_in_stage(&mut self) {
        let instance_id = self.current_instance;
        // scratch uses the opaque bounds for edge bounce, then keeps that outline inside the stage.
        let Some(bounds) = self.transformed_bounds(instance_id, true) else {
            return;
        };
        let mut dx = 0.0;
        let mut dy = 0.0;
        if bounds[0] < -240.0 {
            dx += -240.0 - bounds[0];
        }
        if bounds[1] > 240.0 {
            dx += 240.0 - bounds[1];
        }
        if bounds[3] > 180.0 {
            dy += 180.0 - bounds[3];
        }
        if bounds[2] < -180.0 {
            dy += -180.0 - bounds[2];
        }
        let instance = &self.instances[instance_id];
        self.set_xy(instance.x + dx, instance.y + dy);
    }

    pub fn direction_for_instance(&self, instance: usize) -> Option<f64> {
        self.instances
            .get(instance)
            .filter(|target| target.alive)
            .map(|target| target.direction)
    }

    pub fn set_direction(&mut self, direction: f64) {
        if direction.is_finite() && !self.instances[self.current_instance].is_stage {
            let direction = scratch_direction(direction);
            let instance = &mut self.instances[self.current_instance];
            if instance.direction != direction {
                instance.direction = direction;
                self.request_redraw();
            }
        }
    }

    pub fn set_xy(&mut self, x: f64, y: f64) {
        if self.instances[self.current_instance].is_stage {
            return;
        }
        let (x, y) = self.fenced_position(self.current_instance, x, y);
        let (changed, stroke) = {
            let instance = &mut self.instances[self.current_instance];
            let old = [instance.x as f32, instance.y as f32];
            let changed = instance.x != x || instance.y != y;
            instance.x = x;
            instance.y = y;
            let new = [x as f32, y as f32];
            let stroke = (changed && instance.pen_down).then_some(PenStroke {
                start: old,
                end: new,
                color: instance.pen_color.rgba,
                diameter: instance.pen_size as f32,
                pixel_offset: 0.0,
            });
            (changed, stroke)
        };
        let drew_pen_stroke = stroke.is_some();
        if let Some(stroke) = stroke {
            self.push_pen_stroke(stroke);
        }
        if changed {
            let instance = &self.instances[self.current_instance];
            if drew_pen_stroke || instance.visible || instance.bubble.is_some() {
                self.request_redraw();
            } else {
                // Keep position monitors and other presentation observers
                // current without rebuilding/rendering the hidden sprite.
                self.request_presentation();
            }
        }
    }

    fn fenced_position(&mut self, instance_id: usize, x: f64, y: f64) -> (f64, f64) {
        let instance = &self.instances[instance_id];
        let target = instance.target;
        let direction = instance.direction;
        let size = instance.size;
        let rotation_style = instance.rotation_style;
        let instance_x = instance.x;
        let instance_y = instance.y;
        let Some(geometry) = self.fence_geometry.get(target).and_then(Option::as_ref) else {
            return (x, y);
        };
        let geometry_key = [
            geometry.width,
            geometry.height,
            geometry.rotation_center_x,
            geometry.rotation_center_y,
        ];
        let cache = self.instances[instance_id].fence_offsets;
        let offsets = cache
            .filter(|cache| {
                cache.geometry == geometry_key
                    && cache.direction == direction
                    && cache.size == size
                    && cache.rotation_style == rotation_style
            })
            .map(|cache| cache.points)
            .unwrap_or_else(|| {
                // cache the transformed costume corners until its bounds or style changes.
                let (rendered_direction, flip_x) =
                    rotation_style.rendered_direction_and_flip(direction);
                let radians = (90.0 - rendered_direction) * std::f64::consts::PI / 180.0;
                let cos = radians.cos();
                let sin = radians.sin();
                let scale = size / 100.0;
                let transform = CostumeTransform {
                    rotation_center: (geometry.rotation_center_x, geometry.rotation_center_y),
                    rotation: (cos, sin),
                    flip_x,
                    scale,
                };
                let points = costume_corners(geometry).map(|(local_x, local_y)| {
                    transform_costume_point((local_x, local_y), transform)
                });
                self.instances[instance_id].fence_offsets = Some(CachedFenceOffsets {
                    geometry: geometry_key,
                    direction,
                    size,
                    rotation_style,
                    points,
                });
                points
            });
        let mut min_x = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        for (offset_x, offset_y) in offsets {
            let tx = offset_x + instance_x;
            let ty = offset_y + instance_y;
            min_x = min_x.min(tx);
            max_x = max_x.max(tx);
            min_y = min_y.min(ty);
            max_y = max_y.max(ty);
        }
        let inset = (((max_x - min_x).min(max_y - min_y) / 2.0).floor()).max(0.0);
        let margin = 15.0_f64.min(inset);
        let fence_x = 240.0 - margin;
        let fence_y = 180.0 - margin;
        let dx = x - instance_x;
        let dy = y - instance_y;
        let mut fenced_x = x;
        let mut fenced_y = y;
        if max_x + dx < -fence_x {
            fenced_x = (instance_x - (fence_x + max_x)).ceil();
        } else if min_x + dx > fence_x {
            fenced_x = (instance_x + (fence_x - min_x)).floor();
        }
        if max_y + dy < -fence_y {
            fenced_y = (instance_y - (fence_y + max_y)).ceil();
        } else if min_y + dy > fence_y {
            fenced_y = (instance_y + (fence_y - min_y)).floor();
        }
        (fenced_x, fenced_y)
    }

    fn transformed_bounds(&self, instance_id: usize, use_silhouette: bool) -> Option<[f64; 4]> {
        let instance = self.instances.get(instance_id)?;
        let geometry = self.fence_geometry.get(instance.target)?.as_ref()?;
        let (rendered_direction, flip_x) = instance
            .rotation_style
            .rendered_direction_and_flip(instance.direction);
        let radians = (90.0 - rendered_direction) * std::f64::consts::PI / 180.0;
        let cos = radians.cos();
        let sin = radians.sin();
        let scale = instance.size / 100.0;
        let transform = CostumeTransform {
            rotation_center: (geometry.rotation_center_x, geometry.rotation_center_y),
            rotation: (cos, sin),
            flip_x,
            scale,
        };
        let mut bounds = [
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
        ];
        let mut include = |x: f64, y: f64| {
            let (offset_x, offset_y) = transform_costume_point((x, y), transform);
            // scratch's renderer stores transformed hull points and drawable positions as float32.
            let (x, y) = if use_silhouette {
                (
                    (instance.x.round() + offset_x) as f32 as f64,
                    (instance.y.round() + offset_y) as f32 as f64,
                )
            } else {
                (offset_x + instance.x, offset_y + instance.y)
            };
            bounds[0] = bounds[0].min(x);
            bounds[1] = bounds[1].max(x);
            bounds[2] = bounds[2].min(y);
            bounds[3] = bounds[3].max(y);
        };
        let outline = if use_silhouette {
            geometry.bounds_hull.as_deref()
        } else {
            None
        };
        if let Some(outline) = outline {
            if outline.is_empty() {
                return None;
            }
            for point in outline {
                // scratch's drawable transform places hull samples at texel centers.
                include(point[0] + 0.5, point[1] + 0.5);
            }
        } else {
            for (x, y) in costume_corners(geometry) {
                include(x, y);
            }
        }
        (bounds[0].is_finite() && bounds[1].is_finite()).then_some(bounds)
    }

    pub fn change_x_by(&mut self, dx: f64) {
        if let Some((x, y)) = self.position(self.current_instance) {
            self.set_xy(x + dx, y);
        }
    }

    pub fn change_y_by(&mut self, dy: f64) {
        if let Some((x, y)) = self.position(self.current_instance) {
            self.set_xy(x, y + dy);
        }
    }

    pub fn position(&self, instance: usize) -> Option<(f64, f64)> {
        self.instances
            .get(instance)
            .filter(|target| target.alive)
            .map(|target| (target.x, target.y))
    }

    pub fn add_original_target(&mut self, target: usize) -> usize {
        if let Some(id) = self.original_instance(target) {
            return id;
        }
        let id = self.instances.len();
        let is_stage = self
            .target_names
            .iter()
            .find(|(index, _, _)| *index == target)
            .is_some_and(|(_, _, is_stage)| *is_stage);
        self.instances.push(ScratchTargetInstance {
            target,
            original: true,
            alive: true,
            is_stage,
            x: 0.0,
            y: 0.0,
            direction: 90.0,
            rotation_style: ScratchRotationStyle::AllAround,
            size: 100.0,
            visible: true,
            values: vec![None; self.values.len()],
            lists: vec![None; self.lists.len()],
            bubble: None,
            pen_color: PenColorState::default(),
            pen_down: false,
            pen_size: 1.0,
            costume_index: 0,
            fence_offsets: None,
            sound_pitch: 0.0,
            sound_pan: 0.0,
            volume: 100.0,
        });
        id
    }

    pub fn clone_target_for_name(&self, name: &str) -> Option<usize> {
        self.target_names
            .iter()
            .find(|(_, target_name, is_stage)| target_name == name && !is_stage)
            .map(|(target, _, _)| *target)
    }

    /// read a static `sensing_of` selection from the original named target.
    /// The optional variable slot is compiler-resolved; `None` means no scalar
    /// variable exists for this target and property.
    pub fn sensing_of_target_slot(
        &self,
        target: usize,
        property: &str,
        variable_slot: Option<usize>,
    ) -> ScratchValue {
        self.sensing_of_target(target, property, variable_slot, false)
    }

    fn sensing_of_target(
        &self,
        target: usize,
        property: &str,
        variable_slot: Option<usize>,
        lookup_variable_by_name: bool,
    ) -> ScratchValue {
        let Some(instance_id) = self.original_instance(target) else {
            return ScratchValue::Number(0.0);
        };
        let instance = &self.instances[instance_id];

        if instance.is_stage {
            let built_in = match property {
                "backdrop #" | "background #" => {
                    Some(ScratchValue::Number(self.backdrop_index as f64 + 1.0))
                }
                "backdrop name" => Some(match self.backdrop_names.get(self.backdrop_index) {
                    Some(name) => ScratchValue::Text(name.clone()),
                    None => ScratchValue::Text(String::new()),
                }),
                "volume" => Some(ScratchValue::Number(instance.volume)),
                _ => None,
            };
            if let Some(value) = built_in {
                return value;
            }
        } else {
            let built_in = match property {
                "x position" => Some(ScratchValue::Number(instance.x)),
                "y position" => Some(ScratchValue::Number(instance.y)),
                "direction" => Some(ScratchValue::Number(instance.direction)),
                "costume #" => Some(ScratchValue::Number(instance.costume_index as f64 + 1.0)),
                "costume name" => Some(ScratchValue::Text(
                    self.target_costumes
                        .get(target)
                        .and_then(|costumes| costumes.get(instance.costume_index))
                        .map(|costume| costume.name.clone())
                        .unwrap_or_default(),
                )),
                "size" => Some(ScratchValue::Number(instance.size)),
                "volume" => Some(ScratchValue::Number(instance.volume)),
                _ => None,
            };
            if let Some(value) = built_in {
                return value;
            }
        }

        let slot = variable_slot
            .filter(|slot| {
                self.variables.get(*slot).is_some_and(|metadata| {
                    metadata.name == property
                        && metadata.target_index == target
                        && metadata.is_stage == instance.is_stage
                })
            })
            .or_else(|| {
                if lookup_variable_by_name {
                    self.variables.iter().position(|metadata| {
                        metadata.name == property
                            && metadata.target_index == target
                            && metadata.is_stage == instance.is_stage
                    })
                } else {
                    None
                }
            });
        let value = slot.and_then(|slot| {
            if instance.is_stage {
                self.values.get(slot)
            } else {
                instance
                    .values
                    .get(slot)
                    .and_then(Option::as_ref)
                    .or_else(|| self.values.get(slot))
            }
        });
        value.cloned().unwrap_or(ScratchValue::Number(0.0))
    }

    /// resolve a runtime object expression using Scratch's string conversion.
    pub fn sensing_of_by_name(
        &self,
        object: impl super::ScratchInput,
        property: &str,
    ) -> ScratchValue {
        let name = super::to_text(object.scratch_view());
        let target = if name == "_stage_" {
            self.target_names
                .iter()
                .find(|(_, _, is_stage)| *is_stage)
                .map(|(target, _, _)| *target)
        } else {
            self.target_names
                .iter()
                .find(|(_, target_name, is_stage)| !is_stage && target_name == name.as_ref())
                .map(|(target, _, _)| *target)
        };
        target.map_or(ScratchValue::Number(0.0), |target| {
            self.sensing_of_target(target, property, None, true)
        })
    }

    pub fn register_target(&mut self, target: usize, name: impl Into<String>, is_stage: bool) {
        self.target_names.push((target, name.into(), is_stage));
        let instance = self.add_original_target(target);
        self.instances[instance].is_stage = is_stage;
    }

    pub(super) fn ensure_instance_slot(&mut self, slot: usize) {
        let instance = &mut self.instances[self.current_instance];
        if instance.values.len() < self.values.len() {
            instance.values.resize(self.values.len(), None);
        }
        if instance.lists.len() < self.lists.len() {
            instance.lists.resize(self.lists.len(), None);
        }
        if let Some(metadata) = self.variables.get(slot)
            && instance.values[slot].is_none()
            && metadata.target_index == instance.target
        {
            instance.values[slot] = Some(self.values[slot].clone());
        }
        if let Some(metadata) = self.list_metadata.get(slot)
            && instance.lists[slot].is_none()
            && metadata.target_index == instance.target
        {
            instance.lists[slot] = Some(self.lists[slot].clone());
        }
    }
}
