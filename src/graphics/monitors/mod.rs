use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::{Duration, Instant},
};

use crate::runtime::ScratchValue;
use ab_glyph::{FontArc, OutlineCurve};
use lyon::math::Point;
use wgpu::Queue;

use super::texture::{RenderTexture, TextureRegion, unpremultiply_rgba};
use super::{SCRATCH_HEIGHT, SCRATCH_WIDTH};

mod canvas;
mod gpu;
mod interactions;
mod list_editor;
mod menu;

use crate::graphics::text::{GlyphAtlas, load_system_fonts, text_width};

use gpu::fit_monitor_text;

use list_editor::{
    LIST_CONTENT_TOP, LIST_EMPTY_TOP_PADDING, LIST_FOOTER_HEIGHT, LIST_FOOTER_SIDE_PADDING,
    LIST_HEADER_FONT_SIZE, LIST_HEADER_HORIZONTAL_PADDING, LIST_LENGTH_FONT_SIZE,
    LIST_ROW_FONT_SIZE, LIST_ROW_HEIGHT, LIST_ROW_INDEX_GAP, LIST_ROW_INDEX_X, LIST_ROW_TEXT_INSET,
    LIST_SCROLLBAR_RIGHT_INSET, LIST_SCROLLBAR_WIDTH, ListItemEditor, ListResize,
    ListScrollbarDrag,
};
#[cfg(test)]
use list_editor::{LIST_ROW_VALUE_X, LIST_SCROLLBAR_MIN_THUMB_HEIGHT};
pub(crate) use list_editor::{ListEditorInput, ListMonitorFileAction, ListPointerResponse};

pub(crate) use menu::SliderRangeEditorInput;
use menu::{MonitorContextMenu, SliderRangeEditor, format_slider_value};

const STAGE_HALF_WIDTH: f32 = SCRATCH_WIDTH as f32 * 0.5;
const STAGE_HALF_HEIGHT: f32 = SCRATCH_HEIGHT as f32 * 0.5;
const VALUE_HORIZONTAL_PADDING: f32 = 24.0;
const SCALAR_MONITOR_SCALE: f32 = 0.55;
const LIST_MONITOR_SCALE: f32 = 0.65;

pub(crate) struct VariableMonitor {
    gpu_id: u32,
    pub(super) name: String,
    pub(super) value_key: String,
    value: String,
    x: f32,
    y: f32,
    origin_x: f32,
    origin_y: f32,
    scale: f32,
    pub(super) display_mode: MonitorDisplayMode,
    pub(super) category: MonitorCategory,
    pub(super) kind: MonitorKind,
    list_items: Arc<Vec<ScratchValue>>,
    list_scroll_offset: usize,
    list_scroll_remainder: f32,
    visibility: MonitorVisibility,
    label_width: f32,
    value_width: f32,
    value_text_width: f32,
    value_horizontal_padding: f32,
    slider_min_value_width: f32,
    slider_min: f64,
    slider_max: f64,
    pub(super) width: f32,
    pub(super) height: f32,
    shadow_tile: Option<CachedTile>,
}

struct CachedTile {
    pixels: Vec<u8>,
    width: u32,
    height: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MonitorDisplayMode {
    // expanded shows the label beside the value
    Expanded,
    // value only hides the label and fills the monitor with the orange pill
    ValueOnly,
    // slider keeps the label and value above an interactive numeric range
    Slider,
    // list uses the dedicated indexed list layout
    List,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)] // keep category tags for reporter groups not used by every project
pub(crate) enum MonitorCategory {
    Variables,
    Lists,
    Motion,
    Looks,
    Sound,
    Events,
    Control,
    Sensing,
    Operators,
    MyBlocks,
    Extension([u8; 3]),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MonitorKind {
    ScalarVariable,
    List,
    Reporter,
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
            value,
            x,
            y,
            origin_x: x,
            origin_y: y,
            scale: 0.65,
            display_mode: MonitorDisplayMode::Expanded,
            category: MonitorCategory::Variables,
            kind: MonitorKind::ScalarVariable,
            list_items: Arc::new(Vec::new()),
            list_scroll_offset: 0,
            list_scroll_remainder: 0.0,
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

    pub(crate) fn with_kind(mut self, kind: MonitorKind) -> Self {
        self.kind = kind;
        self
    }

    #[cfg(test)]
    pub(crate) fn with_list_items(mut self, items: Vec<String>, width: f32, height: f32) -> Self {
        self.list_items = Arc::new(items.into_iter().map(ScratchValue::Text).collect());
        self.with_list_size(width, height)
    }

    pub(crate) fn with_list_values(
        mut self,
        items: Arc<Vec<ScratchValue>>,
        width: f32,
        height: f32,
    ) -> Self {
        self.list_items = items;
        self.with_list_size(width, height)
    }

    fn with_list_size(mut self, width: f32, height: f32) -> Self {
        self.kind = MonitorKind::List;
        self.category = MonitorCategory::Lists;
        self.display_mode = MonitorDisplayMode::List;
        self.width = width.max(64.0);
        self.height = height.max(68.0);
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

    fn list_row_capacity(&self) -> usize {
        // include the next row when its top edge fits above the footer.
        ((self.height - (LIST_CONTENT_TOP + LIST_FOOTER_HEIGHT)) / LIST_ROW_HEIGHT)
            .ceil()
            .max(1.0) as usize
    }

    fn list_scroll_row_capacity(&self) -> usize {
        ((self.height - (LIST_CONTENT_TOP + LIST_FOOTER_HEIGHT)) / LIST_ROW_HEIGHT)
            .floor()
            .max(1.0) as usize
    }

    fn max_list_scroll_offset(&self, list_len: usize) -> usize {
        list_len.saturating_sub(self.list_scroll_row_capacity())
    }

    fn visible_list_row_count(&self, list_len: usize) -> usize {
        list_len
            .saturating_sub(self.list_scroll_offset)
            .min(self.list_row_capacity())
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
    list_editor: Option<ListItemEditor>,
    list_resize: Option<ListResize>,
    list_scrollbar_hovered: Option<usize>,
    list_scrollbar_drag: Option<ListScrollbarDrag>,
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
    glyph_atlas: GlyphAtlas,
}

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
    pub(crate) fn build_gpu_bubble_instances(
        &mut self,
        bubbles: &[crate::graphics::BubbleRenderInput],
    ) -> Result<crate::graphics::bubbles::BubbleGpuBatch, String> {
        crate::graphics::bubbles::mesh::build_gpu_instances(
            &self.regular_font,
            &mut self.glyph_atlas,
            bubbles,
        )
    }

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
                monitor.scale = if monitor.kind == MonitorKind::List {
                    LIST_MONITOR_SCALE
                } else {
                    SCALAR_MONITOR_SCALE
                };
                if monitor.kind != MonitorKind::ScalarVariable
                    && monitor.display_mode == MonitorDisplayMode::Slider
                {
                    monitor.display_mode = MonitorDisplayMode::Expanded;
                }
                monitor.label_width = text_width(&bold_font, &monitor.name, 18.0);
                monitor.value_width = text_width(&regular_font, &monitor.value, 19.0)
                    + monitor.value_horizontal_padding * 2.0;
                monitor.value_text_width =
                    monitor.value_width - monitor.value_horizontal_padding * 2.0;
                if monitor.kind == MonitorKind::List {
                    monitor.width = monitor.width.max(64.0);
                    monitor.height = monitor.height.max(68.0);
                } else {
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
            list_editor: None,
            list_resize: None,
            list_scrollbar_hovered: None,
            list_scrollbar_drag: None,
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
            glyph_atlas: GlyphAtlas::new(),
        })
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

    pub(crate) fn has_monitor_value_key(&self, key: &str) -> bool {
        self.monitors.iter().any(|monitor| monitor.value_key == key)
    }

    pub(crate) fn has_visible_monitor_value_key(&self, key: &str) -> bool {
        self.monitors.iter().any(|monitor| {
            monitor.value_key == key && monitor.visibility == MonitorVisibility::Visible
        })
    }

    pub(crate) fn has_visible_monitor_key_containing(&self, text: &str) -> bool {
        self.monitors.iter().any(|monitor| {
            monitor.value_key.contains(text) && monitor.visibility == MonitorVisibility::Visible
        })
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

    pub(super) fn list_display_len(&self, index: usize) -> usize {
        let Some(monitor) = self.monitors.get(index) else {
            return 0;
        };
        let mut len = monitor.list_items.len();
        if let Some(editor) = &self.list_editor
            && editor.monitor_index == index
            && editor.appended
            && editor.index > len
        {
            len = editor.index;
        }
        len
    }

    pub(crate) fn gpu_monitor_ids_in_draw_order(&self) -> impl Iterator<Item = u32> + '_ {
        self.monitors
            .iter()
            .filter(|monitor| {
                monitor.visibility == MonitorVisibility::Visible
                    && monitor.kind != MonitorKind::List
            })
            .chain(self.monitors.iter().filter(|monitor| {
                monitor.visibility == MonitorVisibility::Visible
                    && monitor.kind == MonitorKind::List
            }))
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
        if !self.glyph_atlas.set_scale(bucket) {
            return false;
        }
        // old scale-tier glyphs are no longer referenced after visible text
        // batches are rebuilt. drop them so a resize cannot grow the atlas
        // indefinitely across scale tiers.
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
        TextureRegion {
            x: target_x,
            y: target_y,
            width,
            height,
        },
        &tile.pixels[offset..],
        tile.width * 4,
    );
}

type Triangle = [Point; 3];

use lyon::{math::point, path::Path, tessellation::FillRule};

use canvas::{monitor_slider_ratio, tessellate_path};

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
mod tests;
