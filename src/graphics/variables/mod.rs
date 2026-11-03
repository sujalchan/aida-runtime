use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

use ab_glyph::{Font, FontArc, OutlineCurve, ScaleFont};
use fontdb::{Database, Family, Query, Weight};
use lyon::{
    math::{Point, point},
    path::Path,
    tessellation::FillRule,
};
use wgpu::Queue;

use super::texture::{RenderTexture, unpremultiply_rgba};
use super::{SCRATCH_HEIGHT, SCRATCH_WIDTH};

mod canvas;
mod gpu;
mod menu;

pub(crate) use gpu::BubbleRenderInput;

pub(crate) use menu::SliderRangeEditorInput;
use menu::{MonitorContextMenu, SliderRangeEditor, format_slider_value};

const STAGE_HALF_WIDTH: f32 = SCRATCH_WIDTH as f32 * 0.5;
const STAGE_HALF_HEIGHT: f32 = SCRATCH_HEIGHT as f32 * 0.5;
const VALUE_HORIZONTAL_PADDING: f32 = 24.0;

pub(crate) struct VariableMonitor {
    gpu_id: u32,
    name: String,
    value_key: String,
    value: String,
    x: f32,
    y: f32,
    origin_x: f32,
    origin_y: f32,
    scale: f32,
    display_mode: MonitorDisplayMode,
    category: MonitorCategory,
    visibility: MonitorVisibility,
    label_width: f32,
    value_width: f32,
    value_text_width: f32,
    value_horizontal_padding: f32,
    slider_min_value_width: f32,
    slider_min: f64,
    slider_max: f64,
    width: f32,
    height: f32,
    shadow_tile: Option<CachedTile>,
}

struct CachedTile {
    pixels: Vec<u8>,
    width: u32,
    height: u32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum MonitorDisplayMode {
    // expanded shows the label beside the value
    Expanded,
    // value only hides the label and fills the monitor with the orange pill
    ValueOnly,
    // slider keeps the label and value above an interactive numeric range
    Slider,
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // Keep the category tags for reporters added in later steps.
pub(crate) enum MonitorCategory {
    Variables,
    Motion,
    Looks,
    Sound,
    Sensing,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum MonitorVisibility {
    Visible,
    Hidden,
}

impl VariableMonitor {
    pub(crate) fn new(name: impl Into<String>, value: impl Into<String>, x: f32, y: f32) -> Self {
        let value_key = name.into();
        let name = truncate_monitor_text(&value_key, 64);
        let value = truncate_monitor_text(&value.into(), 96);
        let value_horizontal_padding = if name == "timer" {
            6.0
        } else {
            VALUE_HORIZONTAL_PADDING
        };
        Self {
            gpu_id: 0,
            value_key,
            name,
            value: value.into(),
            x,
            y,
            origin_x: x,
            origin_y: y,
            scale: 0.65,
            display_mode: MonitorDisplayMode::Expanded,
            category: MonitorCategory::Variables,
            visibility: MonitorVisibility::Visible,
            label_width: 0.0,
            value_width: 66.0,
            value_text_width: 0.0,
            value_horizontal_padding,
            slider_min_value_width: 0.0,
            slider_min: 0.0,
            slider_max: 100.0,
            width: 0.0,
            height: 34.0,
            shadow_tile: None,
        }
    }

    // choose the monitor's initial expanded, value only, or slider layout
    pub(crate) fn with_display_mode(mut self, display_mode: MonitorDisplayMode) -> Self {
        self.display_mode = display_mode;
        self
    }

    pub(crate) fn with_category(mut self, category: MonitorCategory) -> Self {
        self.category = category;
        self
    }

    pub(crate) fn with_value_key(mut self, value_key: impl Into<String>) -> Self {
        self.value_key = value_key.into();
        self
    }

    pub(crate) fn with_visibility(mut self, visible: bool) -> Self {
        self.visibility = if visible {
            MonitorVisibility::Visible
        } else {
            MonitorVisibility::Hidden
        };
        self
    }

    pub(crate) fn with_slider_range(mut self, min: f64, max: f64) -> Self {
        self.slider_min = min;
        self.slider_max = max;
        self
    }
}

pub(crate) struct VariableLayer {
    monitors: Vec<VariableMonitor>,
    gpu_translation_enabled: bool,
    regular_font: FontArc,
    bold_font: FontArc,
    glyph_meshes: HashMap<(char, bool), Vec<[Point; 3]>>,
    value_digit_width: f32,
    dragging: Option<(usize, f32, f32)>,
    slider_dragging: Option<usize>,
    slider_hovered: Option<usize>,
    slider_thumb_hovered: Option<usize>,
    slider_width_changed: bool,
    changed_manual_values: HashMap<String, f64>,
    shadow_monitor: Option<usize>,
    shadow_opacity: f32,
    shadow_fade: Option<ShadowFade>,
    context_menu: Option<MonitorContextMenu>,
    slider_range_editor: Option<SliderRangeEditor>,
    dirty_gpu_shapes: HashSet<u32>,
    dirty_gpu_texts: HashSet<u32>,
    dirty_transforms: HashSet<u32>,
    glyph_atlas: Vec<u8>,
    glyph_atlas_size: u32,
    glyph_atlas_revision: u64,
    glyph_atlas_full_dirty: bool,
    glyph_atlas_entries: HashMap<(char, bool, u32), GlyphAtlasEntry>,
    glyph_atlas_dirty_regions: Vec<(u32, u32, u32, u32)>,
    glyph_atlas_scale_bucket: u32,
    glyph_atlas_count: u32,
    glyph_atlas_next_x: u32,
    glyph_atlas_next_y: u32,
    glyph_atlas_row_height: u32,
}

#[derive(Clone, Copy)]
pub(super) struct GlyphAtlasEntry {
    pub(super) id: u32,
    pub(super) x: u32,
    pub(super) y: u32,
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) bounds: [f32; 4],
}

pub(crate) const GLYPH_ATLAS_INITIAL_SIZE: u32 = 128;
pub(crate) const GLYPH_ATLAS_MAX_SIZE: u32 = 2048;
pub(crate) const GLYPH_ATLAS_GLYPH_CAPACITY: u32 = 4096;

#[derive(Clone, Copy)]
pub(crate) struct MonitorBounds {
    pub(crate) left: f32,
    pub(crate) top: f32,
    pub(crate) right: f32,
    pub(crate) bottom: f32,
}

struct ShadowFade {
    started: Instant,
    from: f32,
    to: f32,
}

const SHADOW_FADE_DURATION: Duration = Duration::from_millis(100);
const SHADOW_MAX_OPACITY: f32 = 0.70;

impl VariableLayer {
    pub(crate) fn new(monitors: Vec<VariableMonitor>) -> Result<Self, String> {
        let gpu_translation_enabled = monitors
            .iter()
            .filter(|monitor| monitor.visibility == MonitorVisibility::Visible)
            .take(2)
            .count()
            == 1;
        let (regular_font, bold_font) = load_system_fonts()?;
        let value_digit_width = text_width(&regular_font, "0", 19.0);
        let monitors: Vec<VariableMonitor> = monitors
            .into_iter()
            .enumerate()
            .map(|(gpu_id, mut monitor)| {
                monitor.gpu_id = gpu_id as u32;
                if monitor.category != MonitorCategory::Variables
                    && monitor.display_mode == MonitorDisplayMode::Slider
                {
                    monitor.display_mode = MonitorDisplayMode::Expanded;
                }
                monitor.label_width = text_width(&bold_font, &monitor.name, 18.0);
                monitor.value_width = text_width(&regular_font, &monitor.value, 19.0)
                    + monitor.value_horizontal_padding * 2.0;
                monitor.value_text_width =
                    monitor.value_width - monitor.value_horizontal_padding * 2.0;
                monitor.width = if monitor.display_mode == MonitorDisplayMode::ValueOnly {
                    monitor.value_width
                } else {
                    expanded_width(monitor.label_width, monitor.value_width)
                };
                monitor.height = if monitor.display_mode == MonitorDisplayMode::Slider {
                    58.0
                } else {
                    34.0
                };
                if monitor.display_mode == MonitorDisplayMode::Slider {
                    monitor.slider_min_value_width = monitor.value_width;
                }

                monitor.x = clamp_monitor_x(monitor.x, monitor.width, monitor.scale);
                monitor.y = clamp_monitor_y(monitor.y, monitor.height, monitor.scale);
                monitor.origin_x = monitor.x;
                monitor.origin_y = monitor.y;
                monitor
            })
            .collect();
        let dirty_gpu_shapes: HashSet<u32> =
            monitors.iter().map(|monitor| monitor.gpu_id).collect();
        let dirty_gpu_texts = dirty_gpu_shapes.clone();
        let dirty_transforms = dirty_gpu_shapes.clone();
        Ok(Self {
            monitors,
            gpu_translation_enabled,
            regular_font,
            bold_font,
            glyph_meshes: HashMap::new(),
            value_digit_width,
            dragging: None,
            slider_dragging: None,
            slider_hovered: None,
            slider_thumb_hovered: None,
            slider_width_changed: false,
            changed_manual_values: HashMap::new(),
            shadow_monitor: None,
            shadow_opacity: 0.0,
            shadow_fade: None,
            context_menu: None,
            slider_range_editor: None,
            dirty_gpu_shapes,
            dirty_gpu_texts,
            dirty_transforms,
            glyph_atlas: vec![0; (GLYPH_ATLAS_INITIAL_SIZE * GLYPH_ATLAS_INITIAL_SIZE) as usize],
            glyph_atlas_size: GLYPH_ATLAS_INITIAL_SIZE,
            glyph_atlas_revision: 1,
            glyph_atlas_full_dirty: false,
            glyph_atlas_entries: HashMap::new(),
            glyph_atlas_dirty_regions: Vec::new(),
            glyph_atlas_scale_bucket: 1,
            glyph_atlas_count: 0,
            glyph_atlas_next_x: 0,
            glyph_atlas_next_y: 0,
            glyph_atlas_row_height: 0,
        })
    }

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
            MonitorDisplayMode::Expanded => MonitorDisplayMode::ValueOnly,
            MonitorDisplayMode::ValueOnly if monitor.category == MonitorCategory::Variables => {
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

    fn set_monitor_display_mode(
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

    pub(crate) fn set_monitor_value(&mut self, name: &str, value: String) -> Option<(usize, bool)> {
        let value = truncate_monitor_text(&value, 96);
        let index = self
            .monitors
            .iter()
            .position(|monitor| monitor.value_key == name)?;
        if self.monitors[index].value == value {
            return None;
        }
        if self.monitors[index].visibility == MonitorVisibility::Hidden {
            self.monitors[index].value = value;
            return None;
        }
        let monitor = &self.monitors[index];
        let measured_width =
            text_width(&self.regular_font, &value, 19.0) + monitor.value_horizontal_padding * 2.0;
        let old_x = monitor.x;
        let monitor = &mut self.monitors[index];
        monitor.value = value;
        monitor.value_text_width = measured_width - monitor.value_horizontal_padding * 2.0;
        let width_changed = (monitor.value_width - measured_width).abs() > f32::EPSILON;
        monitor.value_width = measured_width;
        monitor.width = if monitor.display_mode == MonitorDisplayMode::ValueOnly {
            monitor.value_width
        } else {
            expanded_width(monitor.label_width, monitor.value_width)
        };
        monitor.x = clamp_monitor_x(monitor.x, monitor.width, monitor.scale);
        if width_changed || monitor.display_mode == MonitorDisplayMode::Slider {
            self.mark_gpu_shapes(index);
        }
        if self.monitors[index].x != old_x {
            self.mark_gpu_transform(index);
        }
        self.mark_gpu_text(index);
        Some((index, width_changed))
    }

    pub(crate) fn set_monitor_visibility(&mut self, name: &str, visible: bool) -> Option<usize> {
        let index = self
            .monitors
            .iter()
            .position(|monitor| monitor.name == name || monitor.value_key == name)?;
        let monitor = &mut self.monitors[index];
        let visibility = if visible {
            MonitorVisibility::Visible
        } else {
            MonitorVisibility::Hidden
        };
        if monitor.visibility == visibility {
            return None;
        }
        monitor.visibility = visibility;
        self.dirty_gpu_shapes.insert(monitor.gpu_id);
        self.dirty_gpu_texts.insert(monitor.gpu_id);
        self.dirty_transforms.insert(monitor.gpu_id);
        Some(index)
    }

    pub(crate) fn invalidate_shadow_tile(&mut self, index: usize) {
        self.monitors[index].shadow_tile = None;
    }

    pub(crate) fn refresh_shadow_tile_for_viewport(
        &mut self,
        index: usize,
        viewport_width: u32,
        viewport_height: u32,
    ) -> Result<(), String> {
        let monitor = &self.monitors[index];
        let canvas_width = monitor.width * monitor.scale + 28.0;
        let canvas_height = monitor.height * monitor.scale + 28.0;
        let tile_width = (canvas_width * viewport_width as f32 / SCRATCH_WIDTH as f32)
            .ceil()
            .max(1.0) as u32;
        let tile_height = (canvas_height * viewport_height as f32 / SCRATCH_HEIGHT as f32)
            .ceil()
            .max(1.0) as u32;
        self.refresh_shadow_tile(
            index,
            tile_width,
            tile_height,
            viewport_width,
            viewport_height,
        )
    }

    fn refresh_shadow_tile(
        &mut self,
        index: usize,
        tile_width: u32,
        tile_height: u32,
        viewport_width: u32,
        viewport_height: u32,
    ) -> Result<(), String> {
        let tile = self.render_shadow_tile(
            index,
            tile_width,
            tile_height,
            viewport_width,
            viewport_height,
        )?;
        self.monitors[index].shadow_tile = Some(tile);
        Ok(())
    }

    // rasterize a monitor shadow inside a padded local canvas
    fn render_shadow_tile(
        &mut self,
        index: usize,
        tile_width: u32,
        tile_height: u32,
        viewport_width: u32,
        viewport_height: u32,
    ) -> Result<CachedTile, String> {
        let pixmap = self.rasterize_shadow_tile(
            tile_width,
            tile_height,
            viewport_width as f32 / SCRATCH_WIDTH as f32,
            viewport_height as f32 / SCRATCH_HEIGHT as f32,
            index,
        )?;
        Ok(CachedTile {
            pixels: unpremultiply_rgba(pixmap.data()),
            width: tile_width,
            height: tile_height,
        })
    }

    pub(crate) fn cached_shadow_tile_size(&self, index: usize) -> Result<(u32, u32), String> {
        let tile = self.monitors[index]
            .shadow_tile
            .as_ref()
            .ok_or_else(|| "variable monitor shadow tile is not ready".to_owned())?;
        Ok((tile.width, tile.height))
    }

    // place the cached shadow at its source position for gpu compositing
    pub(crate) fn upload_cached_tile_at(
        &self,
        queue: &Queue,
        target: &RenderTexture,
        index: usize,
        left: i64,
        top: i64,
    ) -> Result<(), String> {
        let tile = self.monitors[index]
            .shadow_tile
            .as_ref()
            .ok_or_else(|| "variable monitor shadow tile is not ready".to_owned())?;
        write_cached_tile(queue, target, tile, left, top);
        Ok(())
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
        self.mark_gpu_monitor(index);
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

    fn mark_gpu_monitor(&mut self, index: usize) {
        let id = self.monitors[index].gpu_id;
        self.dirty_gpu_shapes.insert(id);
        self.dirty_gpu_texts.insert(id);
        self.dirty_transforms.insert(id);
    }

    fn mark_gpu_shapes(&mut self, index: usize) {
        self.dirty_gpu_shapes.insert(self.monitors[index].gpu_id);
    }
    fn mark_gpu_text(&mut self, index: usize) {
        self.dirty_gpu_texts.insert(self.monitors[index].gpu_id);
    }

    fn mark_gpu_transform(&mut self, index: usize) {
        self.dirty_transforms.insert(self.monitors[index].gpu_id);
    }

    pub(crate) fn take_dirty_gpu_shapes(&mut self) -> Vec<u32> {
        self.dirty_gpu_shapes.drain().collect()
    }
    pub(crate) fn take_dirty_gpu_texts(&mut self) -> Vec<u32> {
        self.dirty_gpu_texts.drain().collect()
    }
    pub(crate) fn take_dirty_transforms(&mut self) -> Vec<u32> {
        self.dirty_transforms.drain().collect()
    }

    pub(crate) fn gpu_monitor_transform(&self, gpu_id: u32) -> [f32; 4] {
        self.monitors
            .iter()
            .find(|monitor| monitor.gpu_id == gpu_id)
            .map_or([0.0; 4], |monitor| {
                [
                    monitor.x + STAGE_HALF_WIDTH,
                    STAGE_HALF_HEIGHT - monitor.y,
                    monitor.scale,
                    0.0,
                ]
            })
    }

    pub(crate) fn gpu_monitor_ids_in_draw_order(&self) -> impl Iterator<Item = u32> + '_ {
        self.monitors
            .iter()
            .filter(|monitor| monitor.visibility == MonitorVisibility::Visible)
            .map(|monitor| monitor.gpu_id)
    }

    pub(crate) fn gpu_monitor_id_at(&self, index: usize) -> Option<u32> {
        self.monitors.get(index).map(|monitor| monitor.gpu_id)
    }

    pub(crate) fn gpu_monitor_bounds(&self, gpu_id: u32) -> Option<MonitorBounds> {
        let index = self
            .monitors
            .iter()
            .position(|monitor| monitor.gpu_id == gpu_id)?;
        Some(self.monitor_bounds(index))
    }

    pub(crate) fn set_render_scale(&mut self, stage_scale: f32) -> bool {
        let scale = stage_scale.max(1.0).ceil() as u32;
        // a 4x minimum atlas tier limits glyph-bearing quantization to a
        // quarter of a stage pixel even when the window is at 1x scale.
        let bucket = scale.next_power_of_two().max(4);
        if self.glyph_atlas_scale_bucket == bucket {
            return false;
        }
        self.glyph_atlas_scale_bucket = bucket;
        // old scale-tier glyphs are no longer referenced after visible text
        // batches are rebuilt. drop them so a resize cannot grow the atlas
        // indefinitely across scale tiers.
        self.glyph_atlas_size = GLYPH_ATLAS_INITIAL_SIZE;
        self.glyph_atlas = vec![0; (self.glyph_atlas_size * self.glyph_atlas_size) as usize];
        self.glyph_atlas_entries.clear();
        self.glyph_atlas_dirty_regions.clear();
        self.glyph_atlas_full_dirty = false;
        self.glyph_atlas_count = 0;
        self.glyph_atlas_next_x = 0;
        self.glyph_atlas_next_y = 0;
        self.glyph_atlas_row_height = 0;
        self.glyph_atlas_revision = self.glyph_atlas_revision.wrapping_add(1);
        for monitor in &self.monitors {
            if monitor.visibility == MonitorVisibility::Visible {
                self.dirty_gpu_texts.insert(monitor.gpu_id);
            }
        }
        true
    }

    pub(crate) fn update_shadow(&mut self, now: Instant) -> bool {
        let Some(fade) = &self.shadow_fade else {
            return false;
        };
        let progress = (now.duration_since(fade.started).as_secs_f32()
            / SHADOW_FADE_DURATION.as_secs_f32())
        .clamp(0.0, 1.0);
        let next_opacity = fade.from + (fade.to - fade.from) * progress;
        let fade_complete = progress >= 1.0;
        let fade_target = fade.to;
        let changed = (self.shadow_opacity - next_opacity).abs() > f32::EPSILON;
        self.shadow_opacity = next_opacity;
        if fade_complete {
            self.shadow_fade = None;
            if fade_target == 0.0 {
                self.shadow_monitor = None;
            }
        }
        changed || (fade_complete && fade_target == 0.0)
    }

    pub(crate) fn composite_offset(&self) -> (f32, f32) {
        if !self.uses_gpu_translation() {
            return (0.0, 0.0);
        }
        self.monitors
            .iter()
            .find(|monitor| monitor.visibility == MonitorVisibility::Visible)
            .map_or((0.0, 0.0), |monitor| {
                (
                    (monitor.x - monitor.origin_x) / SCRATCH_WIDTH as f32,
                    -(monitor.y - monitor.origin_y) / SCRATCH_HEIGHT as f32,
                )
            })
    }

    pub(crate) fn shadow_opacity(&self) -> f32 {
        self.shadow_opacity
    }

    pub(crate) fn shadow_is_animating(&self) -> bool {
        self.shadow_fade.is_some()
    }

    pub(crate) fn uses_gpu_translation(&self) -> bool {
        self.gpu_translation_enabled
            && self.context_menu.is_none()
            && self.slider_range_editor.is_none()
    }

    fn start_shadow_fade(&mut self, target: f32) {
        self.shadow_fade = Some(ShadowFade {
            started: Instant::now(),
            from: self.shadow_opacity,
            to: target,
        });
    }

    // rasterize the complete monitor layer after a layout change
}

// copy a local tile into the visible part of the target texture
fn write_cached_tile(
    queue: &Queue,
    target: &RenderTexture,
    tile: &CachedTile,
    left: i64,
    top: i64,
) {
    let source_x = (-left).max(0) as u32;
    let source_y = (-top).max(0) as u32;
    let target_x = left.max(0) as u32;
    let target_y = top.max(0) as u32;
    let width = tile
        .width
        .saturating_sub(source_x)
        .min(target.width.saturating_sub(target_x));
    let height = tile
        .height
        .saturating_sub(source_y)
        .min(target.height.saturating_sub(target_y));
    if width == 0 || height == 0 {
        return;
    }
    let offset = ((source_y * tile.width + source_x) * 4) as usize;
    target.write_straight_rgba_region(
        queue,
        target_x,
        target_y,
        width,
        height,
        &tile.pixels[offset..],
        tile.width * 4,
    );
}

type Triangle = [Point; 3];

use canvas::{monitor_slider_ratio, tessellate_path};

fn load_system_fonts() -> Result<(FontArc, FontArc), String> {
    let mut database = Database::new();
    database.load_system_fonts();
    let regular = load_system_font(&database, false)?;
    let bold = load_system_font(&database, true)?;
    Ok((regular, bold))
}

fn load_system_font(database: &Database, bold: bool) -> Result<FontArc, String> {
    let families = [Family::Name("Arial"), Family::SansSerif];
    let query = Query {
        families: &families,
        weight: if bold {
            Weight::SEMIBOLD
        } else {
            Weight::NORMAL
        },
        ..Default::default()
    };
    let face = database
        .query(&query)
        .or_else(|| database.faces().next().map(|face| face.id))
        .ok_or_else(|| "no system font is available for variable monitors".to_owned())?;
    let bytes = database
        .with_face_data(face, |data, _| data.to_vec())
        .ok_or_else(|| "failed to read the system font for variable monitors".to_owned())?;
    FontArc::try_from_vec(bytes)
        .map_err(|error| format!("failed to parse variable monitor font: {error}"))
}

pub(super) fn text_width(font: &FontArc, text: &str, size: f32) -> f32 {
    let scaled = font.as_scaled(size);
    let mut width = 0.0;
    let mut previous = None;
    for character in text.chars() {
        let glyph = scaled.glyph_id(character);
        if let Some(previous) = previous {
            width += scaled.kern(previous, glyph);
        }
        width += scaled.h_advance(glyph);
        previous = Some(glyph);
    }
    width
}

fn tessellate_glyph_outline(
    curves: &[OutlineCurve],
    scale_x: f32,
    scale_y: f32,
) -> Result<Vec<Triangle>, String> {
    let mut builder = Path::builder().with_svg();
    let mut active = false;
    let mut previous_end: Option<Point> = None;
    let scaled_point = |source: ab_glyph::Point| point(source.x * scale_x, source.y * scale_y);
    for curve in curves {
        let (start, end) = match curve {
            OutlineCurve::Line(from, to) => (scaled_point(*from), scaled_point(*to)),
            OutlineCurve::Quad(from, _, to) => (scaled_point(*from), scaled_point(*to)),
            OutlineCurve::Cubic(from, _, _, to) => (scaled_point(*from), scaled_point(*to)),
        };
        if !active
            || previous_end.is_some_and(|last| {
                (last.x - start.x).abs() > 0.01 || (last.y - start.y).abs() > 0.01
            })
        {
            if active {
                builder.close();
            }
            builder.move_to(start);
            active = true;
        }

        match curve {
            OutlineCurve::Line(_, to) => {
                builder.line_to(scaled_point(*to));
            }
            OutlineCurve::Quad(_, control, to) => {
                builder.quadratic_bezier_to(scaled_point(*control), scaled_point(*to));
            }
            OutlineCurve::Cubic(_, control1, control2, to) => {
                builder.cubic_bezier_to(
                    scaled_point(*control1),
                    scaled_point(*control2),
                    scaled_point(*to),
                );
            }
        }
        previous_end = Some(end);
    }
    if active {
        builder.close();
    }
    tessellate_path(&builder.build(), FillRule::EvenOdd)
}

fn clamp_monitor_x(x: f32, width: f32, scale: f32) -> f32 {
    clamp_monitor_position(x, -STAGE_HALF_WIDTH, STAGE_HALF_WIDTH - width * scale)
}

fn clamp_monitor_y(y: f32, height: f32, scale: f32) -> f32 {
    clamp_monitor_position(y, -STAGE_HALF_HEIGHT + height * scale, STAGE_HALF_HEIGHT)
}

fn clamp_monitor_position(value: f32, min: f32, max: f32) -> f32 {
    if !min.is_finite() || !max.is_finite() || max < min {
        return if min.is_finite() { min } else { 0.0 };
    }
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        min
    }
}

fn truncate_monitor_text(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let prefix: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        let mut truncated = prefix;
        truncated.pop();
        truncated.push('…');
        truncated
    } else {
        prefix
    }
}

fn expanded_width(label_width: f32, value_width: f32) -> f32 {
    14.0 + label_width + 14.0 + value_width + 14.0
}

#[cfg(test)]
mod tests {
    use super::{MonitorDisplayMode, VariableLayer, VariableMonitor};
    use crate::runtime::variable_snapshot_key;

    #[test]
    fn variable_monitor_builds_gpu_instances() {
        let mut layer =
            VariableLayer::new(vec![VariableMonitor::new("my variable", "0", 8.0, 8.0)]).unwrap();
        let (shapes, glyphs) = layer.build_gpu_monitor_instances(0, true, true).unwrap();
        assert!(!shapes.is_empty());
        assert!(!glyphs.is_empty());
    }

    #[test]
    fn gpu_text_keeps_literal_special_characters() {
        let mut layer =
            VariableLayer::new(vec![VariableMonitor::new("a < b", "1 & 2", 0.0, 0.0)]).unwrap();
        let (_, glyphs) = layer.build_gpu_monitor_instances(0, false, true).unwrap();
        assert_eq!(glyphs.len(), 6);
    }

    #[test]
    fn timer_value_update_only_dirties_its_glyph_instances() {
        let mut layer = VariableLayer::new(vec![
            VariableMonitor::new("timer", "12.3456789", 0.0, 0.0),
            VariableMonitor::new("other", "static", 0.0, 80.0),
        ])
        .unwrap();
        layer.take_dirty_gpu_shapes();
        layer.take_dirty_gpu_texts();
        layer.take_dirty_transforms();
        let revision = layer.glyph_atlas_revision();

        layer
            .set_monitor_value("timer", "12.3456788".to_owned())
            .unwrap();
        assert!(layer.take_dirty_gpu_shapes().is_empty());
        assert_eq!(layer.take_dirty_gpu_texts(), vec![0]);
        assert_eq!(layer.glyph_atlas_revision(), revision);
    }

    #[test]
    fn stage_variable_snapshot_updates_its_visible_monitor() {
        let id = "`jEk@4|i[#Fk?(8x)AV.-my variable";
        let key = variable_snapshot_key("Stage", "my variable", id, None);
        let mut layer = VariableLayer::new(vec![
            VariableMonitor::new("my variable", "0", 5.0, 5.0).with_value_key(key.clone()),
        ])
        .unwrap();
        layer.take_dirty_gpu_texts();

        assert_eq!(
            layer.set_monitor_value(&key, "341".to_owned()),
            Some((0, true))
        );
        assert_eq!(layer.monitors[0].value, "341");
        assert_eq!(layer.take_dirty_gpu_texts(), vec![0]);
    }

    #[test]
    fn slider_drag_coalesces_to_the_latest_numeric_value_for_its_variable_key() {
        let id = "slider-id";
        let key = variable_snapshot_key("Stage", "limit", id, None);
        let monitor = VariableMonitor::new("limit", "10", 0.0, 0.0)
            .with_display_mode(MonitorDisplayMode::Slider)
            .with_value_key(key.clone())
            .with_slider_range(0.0, 100.0);
        let mut layer = VariableLayer::new(vec![monitor]).unwrap();
        layer.take_dirty_gpu_texts();

        let monitor = &layer.monitors[0];
        let y = monitor.y - 42.0 * monitor.scale;
        let high_x = monitor.x + (14.0 + 0.8 * (monitor.width - 28.0)) * monitor.scale;
        assert!(layer.begin_slider_drag(high_x, y));

        let monitor = &layer.monitors[0];
        let low_x = monitor.x + (14.0 + 0.02 * (monitor.width - 28.0)) * monitor.scale;
        assert!(layer.drag_to(low_x, y));

        let changes = layer.take_changed_manual_values();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].0, key);
        assert_eq!(changes[0].1, 2.0);
        assert_eq!(layer.take_dirty_gpu_texts(), vec![0]);
    }

    #[test]
    fn monitor_drag_updates_only_its_transform() {
        let mut layer =
            VariableLayer::new(vec![VariableMonitor::new("timer", "0", 0.0, 0.0)]).unwrap();
        let _ = layer.take_dirty_gpu_shapes();
        let _ = layer.take_dirty_gpu_texts();
        let _ = layer.take_dirty_transforms();
        let before = layer.gpu_monitor_transform(0);
        assert!(layer.begin_drag(1.0, -1.0));
        assert!(layer.drag_to(20.0, -1.0));
        assert!(layer.take_dirty_gpu_shapes().is_empty());
        assert!(layer.take_dirty_gpu_texts().is_empty());
        assert_eq!(layer.take_dirty_transforms(), vec![0]);
        assert_ne!(layer.gpu_monitor_transform(0), before);
    }

    #[test]
    fn bring_to_front_changes_draw_order_without_changing_gpu_ids() {
        let mut layer = VariableLayer::new(vec![
            VariableMonitor::new("front later", "1", 0.0, 0.0),
            VariableMonitor::new("back", "2", 120.0, 100.0),
        ])
        .unwrap();
        let _ = layer.take_dirty_gpu_shapes();
        let _ = layer.take_dirty_gpu_texts();
        let _ = layer.take_dirty_transforms();
        assert!(layer.begin_drag(1.0, -1.0));
        assert_eq!(
            layer.gpu_monitor_ids_in_draw_order().collect::<Vec<_>>(),
            vec![1, 0]
        );
        assert!(layer.take_dirty_gpu_shapes().is_empty());
        assert!(layer.take_dirty_gpu_texts().is_empty());
        assert!(layer.take_dirty_transforms().is_empty());
    }

    #[test]
    fn render_scale_changes_only_text_and_reuses_scale_tier() {
        let mut layer =
            VariableLayer::new(vec![VariableMonitor::new("timer", "12.345", 0.0, 0.0)]).unwrap();
        assert!(layer.set_render_scale(1.0));
        let _ = layer.take_dirty_gpu_shapes();
        let _ = layer.take_dirty_gpu_texts();
        let _ = layer.take_dirty_transforms();
        let low_scale_batch = layer.build_gpu_monitor_instances(0, true, true).unwrap();
        let low_scale_metadata = layer.glyph_metadata();
        let low_scale_bounds = low_scale_metadata[low_scale_batch.1[0].glyph_id as usize].bounds;
        let first_revision = layer.glyph_atlas_revision();

        assert!(layer.set_render_scale(5.0));
        assert_eq!(layer.take_dirty_gpu_texts(), vec![0]);
        assert!(layer.take_dirty_gpu_shapes().is_empty());
        assert!(layer.take_dirty_transforms().is_empty());
        let high_scale_batch = layer.build_gpu_monitor_instances(0, false, true).unwrap();
        let high_scale_metadata = layer.glyph_metadata();
        let high_scale_bounds = high_scale_metadata[high_scale_batch.1[0].glyph_id as usize].bounds;
        let high_scale_revision = layer.glyph_atlas_revision();
        assert!(high_scale_revision > first_revision);
        for index in 0..4 {
            assert!(
                (low_scale_bounds[index] - high_scale_bounds[index]).abs() < 0.2,
                "bounds differ at {index}: {} vs {}",
                low_scale_bounds[index],
                high_scale_bounds[index]
            );
        }
        assert_eq!(
            low_scale_batch
                .1
                .iter()
                .map(|glyph| glyph.local_position)
                .collect::<Vec<_>>(),
            high_scale_batch
                .1
                .iter()
                .map(|glyph| glyph.local_position)
                .collect::<Vec<_>>(),
        );

        assert!(!layer.set_render_scale(5.4));
        assert!(layer.take_dirty_gpu_texts().is_empty());
        assert_eq!(layer.glyph_atlas_revision(), high_scale_revision);
    }
}
