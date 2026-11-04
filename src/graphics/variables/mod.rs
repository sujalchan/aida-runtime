use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::{Duration, Instant},
};

use crate::runtime::{RuntimeInputEvent, ScratchValue};
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

use gpu::fit_monitor_text;

pub(crate) use gpu::BubbleRenderInput;

pub(crate) use menu::SliderRangeEditorInput;
use menu::{MonitorContextMenu, SliderRangeEditor, format_slider_value};

const STAGE_HALF_WIDTH: f32 = SCRATCH_WIDTH as f32 * 0.5;
const STAGE_HALF_HEIGHT: f32 = SCRATCH_HEIGHT as f32 * 0.5;
const VALUE_HORIZONTAL_PADDING: f32 = 24.0;
const SCALAR_MONITOR_SCALE: f32 = 0.55;
const LIST_MONITOR_SCALE: f32 = 0.65;
const LIST_ROW_HEIGHT: f32 = 24.0;
const LIST_FOOTER_HEIGHT: f32 = 22.0;
const LIST_RESIZE_HANDLE_SIZE: f32 = 22.0;
const LIST_SCROLLBAR_WIDTH: f32 = 5.0;
const LIST_SCROLLBAR_RIGHT_INSET: f32 = 2.0;
const LIST_SCROLLBAR_MIN_THUMB_HEIGHT: f32 = 16.0;
pub(super) const LIST_ROW_INDEX_X: f32 = 7.0;
pub(super) const LIST_ROW_VALUE_X: f32 = 20.0;
pub(super) const LIST_ROW_TEXT_INSET: f32 = 6.0;
// keep a small gutter between the index column and the orange value field.
pub(super) const LIST_ROW_INDEX_GAP: f32 = 2.0;
// allow for glyph overhang and floating-point rounding in the index width.
const LIST_ROW_INDEX_CLEARANCE: f32 = 1.5;
pub(super) const LIST_CONTENT_TOP: f32 = 27.0;
pub(super) const LIST_EMPTY_TOP_PADDING: f32 = 6.0;
pub(super) const LIST_ROW_FONT_SIZE: f32 = 12.0;
pub(super) const LIST_HEADER_FONT_SIZE: f32 = 14.0;
pub(super) const LIST_LENGTH_FONT_SIZE: f32 = 12.0;
pub(super) const LIST_HEADER_HORIZONTAL_PADDING: f32 = 12.0;
pub(super) const LIST_FOOTER_SIDE_PADDING: f32 = 7.0;

pub(crate) enum ListEditorInput<'a> {
    Text(&'a str),
    Backspace,
    Delete,
    Left,
    Right,
    Home,
    End,
    Commit,
    Cancel,
}

pub(crate) struct ListPointerResponse {
    pub(crate) consumed: bool,
    pub(crate) resizing: bool,
    pub(crate) scrollbar_dragging: bool,
    pub(crate) events: Vec<RuntimeInputEvent>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ListScrollbarGeometry {
    track_x: f32,
    track_y: f32,
    track_width: f32,
    track_height: f32,
    thumb_y: f32,
    thumb_height: f32,
}

pub(crate) enum ListMonitorFileAction {
    Import { key: String },
    Export { file_name: String, contents: String },
}

struct ListItemEditor {
    monitor_index: usize,
    index: usize,
    text: String,
    caret: usize,
    select_all: bool,
    appended: bool,
    dirty: bool,
}

struct ListResize {
    monitor_index: usize,
    pointer_x: f32,
    pointer_y: f32,
    width: f32,
    height: f32,
}

#[derive(Clone, Copy)]
struct ListScrollbarDrag {
    monitor_index: usize,
    pointer_offset: f32,
}

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
            value: value.into(),
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

    pub(crate) fn set_monitor_list(
        &mut self,
        key: &str,
        items: Arc<Vec<ScratchValue>>,
    ) -> Option<(usize, bool)> {
        let index = self
            .monitors
            .iter()
            .position(|monitor| monitor.value_key == key)?;
        if self.monitors[index].kind != MonitorKind::List
            || Arc::ptr_eq(&self.monitors[index].list_items, &items)
        {
            return Some((index, false));
        }
        let old_display_len = self.list_display_len(index);
        let old_visible_count =
            self.monitors[index].visible_list_row_count(old_display_len);
        let old_visible_start = self.monitors[index]
            .list_scroll_offset
            .min(self.monitors[index].max_list_scroll_offset(old_display_len));
        self.monitors[index].list_items = items;
        let displayed_len = self.list_display_len(index);
        self.monitors[index].list_scroll_offset = self.monitors[index]
            .list_scroll_offset
            .min(self.monitors[index].max_list_scroll_offset(displayed_len));
        self.monitors[index].list_scroll_remainder = 0.0;
        // update row and scrollbar geometry when list size or the visible range changes.
        let shapes_changed = old_display_len != displayed_len
            || old_visible_count
            != self.monitors[index].visible_list_row_count(displayed_len)
            || old_visible_start
                != self.monitors[index]
                    .list_scroll_offset
                    .min(self.monitors[index].max_list_scroll_offset(displayed_len));
        if self.monitors[index].visibility == MonitorVisibility::Visible {
            if shapes_changed {
                self.mark_gpu_shapes(index);
            }
            self.mark_gpu_text(index);
            Some((index, true))
        } else {
            Some((index, false))
        }
    }

    fn list_scrollbar_geometry(&self, index: usize) -> Option<ListScrollbarGeometry> {
        let monitor = self.monitors.get(index)?;
        if monitor.kind != MonitorKind::List {
            return None;
        }
        let list_len = self.list_display_len(index);
        let track_y = LIST_CONTENT_TOP;
        let track_height = monitor.height - LIST_CONTENT_TOP - LIST_FOOTER_HEIGHT;
        let content_height = list_len as f32 * LIST_ROW_HEIGHT;
        let max_offset = monitor.max_list_scroll_offset(list_len);
        if track_height <= 0.0 || content_height <= track_height || max_offset == 0 {
            return None;
        }
        let thumb_height = (track_height * track_height / content_height)
            .max(LIST_SCROLLBAR_MIN_THUMB_HEIGHT)
            .min(track_height);
        let thumb_travel = track_height - thumb_height;
        let scroll_progress = monitor.list_scroll_offset.min(max_offset) as f32 / max_offset as f32;
        Some(ListScrollbarGeometry {
            track_x: monitor.width - LIST_SCROLLBAR_RIGHT_INSET - LIST_SCROLLBAR_WIDTH,
            track_y,
            track_width: LIST_SCROLLBAR_WIDTH,
            track_height,
            thumb_y: track_y + thumb_travel * scroll_progress,
            thumb_height,
        })
    }

    fn list_scrollbar_is_visible(&self, index: usize) -> bool {
        self.list_scrollbar_hovered == Some(index)
            || self
                .list_scrollbar_drag
                .as_ref()
                .is_some_and(|drag| drag.monitor_index == index)
    }

    pub(crate) fn update_list_scrollbar_hover(&mut self, pointer: Option<(f32, f32)>) -> bool {
        let next = pointer
            .and_then(|(x, y)| self.hit_test(x, y))
            .filter(|&index| self.monitors[index].kind == MonitorKind::List);
        if next == self.list_scrollbar_hovered {
            return false;
        }
        let previous = self.list_scrollbar_hovered;
        self.list_scrollbar_hovered = next;
        let mut visual_change = false;
        if let Some(previous) = previous
            && self.list_scrollbar_geometry(previous).is_some()
        {
            self.mark_gpu_shapes(previous);
            visual_change = true;
        }
        if let Some(next) = next
            && self.list_scrollbar_geometry(next).is_some()
        {
            self.mark_gpu_shapes(next);
            visual_change = true;
        }
        visual_change
    }

    fn set_list_scroll_offset(&mut self, index: usize, offset: usize) -> bool {
        let list_len = self.list_display_len(index);
        let monitor = &mut self.monitors[index];
        let next_offset = offset.min(monitor.max_list_scroll_offset(list_len));
        if next_offset == monitor.list_scroll_offset {
            return false;
        }
        monitor.list_scroll_offset = next_offset;
        monitor.list_scroll_remainder = 0.0;
        // update visible rows and the thumb without touching offscreen list values.
        self.mark_gpu_shapes(index);
        self.mark_gpu_text(index);
        true
    }

    pub(crate) fn scroll_list_at(&mut self, x: f32, y: f32, rows: f32) -> Option<usize> {
        let index = self.hit_test(x, y)?;
        let displayed_len = self.list_display_len(index);
        let monitor = self.monitors.get_mut(index)?;
        if monitor.kind != MonitorKind::List || !rows.is_finite() || rows == 0.0 {
            return None;
        }
        let accumulated = monitor.list_scroll_remainder + rows;
        let whole_rows = accumulated.trunc() as isize;
        monitor.list_scroll_remainder = accumulated - whole_rows as f32;
        if whole_rows == 0 {
            return None;
        }
        let max_offset = monitor.max_list_scroll_offset(displayed_len);
        let offset = (monitor.list_scroll_offset as isize + whole_rows)
            .clamp(0, max_offset as isize) as usize;
        if offset == monitor.list_scroll_offset {
            monitor.list_scroll_remainder = 0.0;
            return None;
        }
        monitor.list_scroll_offset = offset;
        // each visible index can shift its own value field, so refresh both instance sets.
        self.mark_gpu_shapes(index);
        self.mark_gpu_text(index);
        Some(index)
    }

    pub(crate) fn list_editor_is_open(&self) -> bool {
        self.list_editor.is_some()
    }

    pub(crate) fn handle_list_pointer(&mut self, x: f32, y: f32) -> ListPointerResponse {
        let mut events = Vec::new();
        let Some(index) = self.hit_test(x, y) else {
            self.commit_list_editor(&mut events);
            return ListPointerResponse {
                consumed: false,
                resizing: false,
                scrollbar_dragging: false,
                events,
            };
        };
        if self.monitors[index].kind != MonitorKind::List {
            self.commit_list_editor(&mut events);
            return ListPointerResponse {
                consumed: false,
                resizing: false,
                scrollbar_dragging: false,
                events,
            };
        }

        let monitor = &self.monitors[index];
        let local_x = (x - monitor.x) / monitor.scale;
        let local_y = (monitor.y - y) / monitor.scale;
        let width = monitor.width;
        let height = monitor.height;

        if local_y < 24.0 {
            self.commit_list_editor(&mut events);
            return ListPointerResponse {
                consumed: false,
                resizing: false,
                scrollbar_dragging: false,
                events,
            };
        }

        if local_x >= width - LIST_RESIZE_HANDLE_SIZE && local_y >= height - LIST_RESIZE_HANDLE_SIZE
        {
            self.commit_list_editor(&mut events);
            self.list_resize = Some(ListResize {
                monitor_index: index,
                pointer_x: x,
                pointer_y: y,
                width,
                height,
            });
            return ListPointerResponse {
                consumed: true,
                resizing: true,
                scrollbar_dragging: false,
                events,
            };
        }

        if let Some(scrollbar) = self.list_scrollbar_geometry(index)
            && (scrollbar.track_x..=scrollbar.track_x + scrollbar.track_width).contains(&local_x)
            && (scrollbar.track_y..=scrollbar.track_y + scrollbar.track_height).contains(&local_y)
        {
            self.commit_list_editor(&mut events);
            if self.list_scrollbar_hovered != Some(index) {
                if let Some(previous) = self.list_scrollbar_hovered.replace(index) {
                    self.mark_gpu_shapes(previous);
                }
                self.mark_gpu_shapes(index);
            }
            if (scrollbar.thumb_y..=scrollbar.thumb_y + scrollbar.thumb_height)
                .contains(&local_y)
            {
                self.list_scrollbar_drag = Some(ListScrollbarDrag {
                    monitor_index: index,
                    pointer_offset: local_y - scrollbar.thumb_y,
                });
                self.mark_gpu_shapes(index);
                return ListPointerResponse {
                    consumed: true,
                    resizing: false,
                    scrollbar_dragging: true,
                    events,
                };
            }
            let page = self.monitors[index]
                .list_scroll_row_capacity()
                .saturating_sub(1)
                .max(1);
            let current = self.monitors[index].list_scroll_offset;
            let next = if local_y < scrollbar.thumb_y {
                current.saturating_sub(page)
            } else {
                current.saturating_add(page)
            };
            self.set_list_scroll_offset(index, next);
            return ListPointerResponse {
                consumed: true,
                resizing: false,
                scrollbar_dragging: false,
                events,
            };
        }

        if local_y >= height - LIST_FOOTER_HEIGHT
            && (LIST_FOOTER_SIDE_PADDING..LIST_FOOTER_SIDE_PADDING + 22.0).contains(&local_x)
        {
            self.commit_list_editor(&mut events);
            let list_len = self.list_display_len(index);
            let key = self.monitors[index].value_key.clone();
            self.list_editor = Some(ListItemEditor {
                monitor_index: index,
                index: list_len + 1,
                text: String::new(),
                caret: 0,
                select_all: true,
                appended: true,
                dirty: false,
            });
            let capacity = self.monitors[index].list_scroll_row_capacity();
            self.monitors[index].list_scroll_offset = (list_len + 1).saturating_sub(capacity);
            self.mark_gpu_shapes(index);
            self.mark_gpu_text(index);
            events.push(RuntimeInputEvent::ListAppend { key });
            return ListPointerResponse {
                consumed: true,
                resizing: false,
                scrollbar_dragging: false,
                events,
            };
        }

        if local_y >= LIST_CONTENT_TOP && local_y < height - LIST_FOOTER_HEIGHT {
            let monitor = &self.monitors[index];
            let displayed_len = self.list_display_len(index);
            let start = monitor
                .list_scroll_offset
                .min(monitor.max_list_scroll_offset(displayed_len));
            let row = ((local_y - LIST_CONTENT_TOP) / LIST_ROW_HEIGHT).floor() as usize;
            let row_index = start + row + 1;
            // keep pointer hit testing aligned with the dynamically sized value field.
            let value_x = self.list_row_value_x(index, row_index);
            if row_index <= displayed_len && local_x >= value_x && local_x < width - 6.0 {
                if self.list_editor.as_ref().is_some_and(|editor| {
                    editor.monitor_index == index
                        && editor.index == row_index
                        && local_x >= width - 28.0
                }) {
                    self.list_editor = None;
                    let key = self.monitors[index].value_key.clone();
                    events.push(RuntimeInputEvent::ListDeleteItem {
                        key,
                        index: row_index,
                    });
                    self.monitors[index].list_scroll_offset = self.monitors[index]
                        .list_scroll_offset
                        .min(self.monitors[index].max_list_scroll_offset(displayed_len - 1));
                    self.mark_gpu_shapes(index);
                    self.mark_gpu_text(index);
                    return ListPointerResponse {
                        consumed: true,
                        resizing: false,
                        scrollbar_dragging: false,
                        events,
                    };
                }

                if self.list_editor.as_ref().is_some_and(|editor| {
                    editor.monitor_index == index && editor.index == row_index
                }) {
                    let selection_changed = self
                        .list_editor
                        .as_ref()
                        .is_some_and(|editor| editor.select_all);
                    let caret = self.list_caret_from_x(index, local_x, row_index);
                    if let Some(editor) = self.list_editor.as_mut() {
                        editor.caret = caret;
                        editor.select_all = false;
                    }
                    if selection_changed {
                        self.mark_gpu_shapes(index);
                    }
                    self.mark_gpu_text(index);
                    return ListPointerResponse {
                        consumed: true,
                        resizing: false,
                        scrollbar_dragging: false,
                        events,
                    };
                }

                self.commit_list_editor(&mut events);
                let text = self.monitors[index]
                    .list_items
                    .get(row_index - 1)
                    .map(crate::runtime::scratch_string)
                    .unwrap_or_default();
                self.list_editor = Some(ListItemEditor {
                    monitor_index: index,
                    index: row_index,
                    caret: text.len(),
                    text,
                    select_all: true,
                    appended: false,
                    dirty: false,
                });
                self.mark_gpu_text(index);
                return ListPointerResponse {
                    consumed: true,
                    resizing: false,
                    scrollbar_dragging: false,
                    events,
                };
            }
        }

        self.commit_list_editor(&mut events);
        ListPointerResponse {
            consumed: true,
            resizing: false,
            scrollbar_dragging: false,
            events,
        }
    }

    pub(crate) fn edit_list_item(
        &mut self,
        input: ListEditorInput<'_>,
    ) -> (bool, Vec<RuntimeInputEvent>) {
        if matches!(input, ListEditorInput::Commit) {
            let index = self.list_editor.as_ref().map(|editor| editor.monitor_index);
            let mut events = Vec::new();
            self.commit_list_editor(&mut events);
            return (index.is_some(), events);
        }
        if matches!(input, ListEditorInput::Cancel) {
            let editor = self.list_editor.take();
            if let Some(editor) = editor {
                if editor.select_all {
                    self.mark_gpu_shapes(editor.monitor_index);
                }
                let index = editor.monitor_index;
                self.mark_gpu_text(index);
                return (true, Vec::new());
            }
            return (false, Vec::new());
        }
        let Some(editor) = self.list_editor.as_mut() else {
            return (false, Vec::new());
        };
        let index = editor.monitor_index;
        let selection_was_active = editor.select_all;
        let events = Vec::new();
        let changed = match input {
            ListEditorInput::Text(text) if !text.is_empty() => {
                if editor.select_all {
                    editor.text.clear();
                    editor.caret = 0;
                    editor.select_all = false;
                }
                editor.text.insert_str(editor.caret, text);
                editor.caret += text.len();
                editor.dirty = true;
                true
            }
            ListEditorInput::Backspace => {
                if editor.select_all {
                    editor.text.clear();
                    editor.caret = 0;
                    editor.select_all = false;
                    editor.dirty = true;
                    true
                } else if editor.caret > 0 {
                    let previous = previous_char_boundary(&editor.text, editor.caret);
                    editor.text.replace_range(previous..editor.caret, "");
                    editor.caret = previous;
                    editor.dirty = true;
                    true
                } else {
                    false
                }
            }
            ListEditorInput::Delete => {
                if editor.select_all {
                    editor.text.clear();
                    editor.caret = 0;
                    editor.select_all = false;
                    editor.dirty = true;
                    true
                } else if editor.caret < editor.text.len() {
                    let next = next_char_boundary(&editor.text, editor.caret);
                    editor.text.replace_range(editor.caret..next, "");
                    editor.dirty = true;
                    true
                } else {
                    false
                }
            }
            ListEditorInput::Left => {
                editor.caret = if editor.select_all {
                    0
                } else {
                    previous_char_boundary(&editor.text, editor.caret)
                };
                editor.select_all = false;
                true
            }
            ListEditorInput::Right => {
                editor.caret = if editor.select_all {
                    editor.text.len()
                } else {
                    next_char_boundary(&editor.text, editor.caret)
                };
                editor.select_all = false;
                true
            }
            ListEditorInput::Home => {
                editor.caret = 0;
                editor.select_all = false;
                true
            }
            ListEditorInput::End => {
                editor.caret = editor.text.len();
                editor.select_all = false;
                true
            }
            ListEditorInput::Commit | ListEditorInput::Cancel => false,
            ListEditorInput::Text(_) => false,
        };
        if changed {
            if selection_was_active {
                self.mark_gpu_shapes(index);
            }
            self.mark_gpu_text(index);
        }
        (changed, events)
    }

    fn commit_list_editor(&mut self, events: &mut Vec<RuntimeInputEvent>) {
        let Some(editor) = self.list_editor.take() else {
            return;
        };
        if editor.select_all {
            self.mark_gpu_shapes(editor.monitor_index);
        }
        if editor.dirty {
            if let Some(monitor) = self.monitors.get(editor.monitor_index) {
                events.push(RuntimeInputEvent::ListSetItem {
                    key: monitor.value_key.clone(),
                    index: editor.index,
                    value: editor.text,
                });
            }
        }
        self.mark_gpu_text(editor.monitor_index);
    }

    fn list_caret_from_x(&self, monitor_index: usize, local_x: f32, row_index: usize) -> usize {
        let Some(editor) = self.list_editor.as_ref() else {
            return 0;
        };
        if editor.monitor_index != monitor_index || editor.index != row_index {
            return 0;
        }
        // place the caret from the same value-field edge used by row rendering.
        let start_x = self.list_row_value_x(monitor_index, row_index) + LIST_ROW_TEXT_INSET;
        let target_x = (local_x - start_x).max(0.0);
        let scaled = self.regular_font.as_scaled(LIST_ROW_FONT_SIZE);
        let max_width = self.monitors[monitor_index].width - start_x - 28.0;
        let shown = fit_monitor_text(
            &self.regular_font,
            &editor.text,
            max_width,
            LIST_ROW_FONT_SIZE,
        );
        let mut pen_x = 0.0;
        let mut previous = None;
        for (byte_index, character) in shown.char_indices() {
            let glyph = scaled.glyph_id(character);
            if let Some(prior) = previous {
                pen_x += scaled.kern(prior, glyph);
            }
            if target_x < pen_x + scaled.h_advance(glyph) * 0.5 {
                return byte_index;
            }
            pen_x += scaled.h_advance(glyph);
            previous = Some(glyph);
        }
        shown.len().min(editor.text.len())
    }

    // size each value field from its own row index so shorter indices do not waste space.
    pub(super) fn list_row_value_x(&self, monitor_index: usize, row_index: usize) -> f32 {
        let Some(monitor) = self.monitors.get(monitor_index) else {
            return LIST_ROW_VALUE_X;
        };
        let index_text = row_index.to_string();
        let index_width =
            text_width(&self.bold_font, &index_text, LIST_ROW_FONT_SIZE) + LIST_ROW_INDEX_CLEARANCE;
        let available_index_width =
            (monitor.width - LIST_ROW_INDEX_X - 6.0 - LIST_ROW_INDEX_GAP).max(0.0);
        let fitted_index_width = index_width.min(available_index_width);
        (LIST_ROW_INDEX_X + fitted_index_width + LIST_ROW_INDEX_GAP)
            .max(LIST_ROW_VALUE_X)
            .min((monitor.width - 6.0).max(0.0))
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

fn previous_char_boundary(text: &str, caret: usize) -> usize {
    text[..caret.min(text.len())]
        .char_indices()
        .next_back()
        .map_or(0, |(index, _)| index)
}

fn next_char_boundary(text: &str, caret: usize) -> usize {
    let caret = caret.min(text.len());
    text[caret..]
        .char_indices()
        .nth(1)
        .map_or(text.len(), |(offset, _)| caret + offset)
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
    use super::{
        LIST_ROW_FONT_SIZE, LIST_ROW_HEIGHT, ListEditorInput, MonitorDisplayMode, VariableLayer,
        VariableMonitor,
    };
    use crate::runtime::{RuntimeInputEvent, ScratchValue, variable_snapshot_key};
    use std::sync::Arc;

    fn list_layer() -> VariableLayer {
        VariableLayer::new(vec![
            VariableMonitor::new("items", "2", 0.0, 0.0)
                .with_value_key("Stage::items#items-id@shared")
                .with_list_items(vec!["alpha".into(), "beta".into()], 180.0, 150.0),
        ])
        .unwrap()
    }

    fn click_local(
        layer: &mut VariableLayer,
        local_x: f32,
        local_y: f32,
    ) -> super::ListPointerResponse {
        let monitor = &layer.monitors[0];
        let x = monitor.x + local_x * monitor.scale;
        let y = monitor.y - local_y * monitor.scale;
        layer.handle_list_pointer(x, y)
    }

    fn list_with_items(length: usize, height: f32) -> VariableLayer {
        VariableLayer::new(vec![
            VariableMonitor::new("items", "", 0.0, 0.0)
                .with_value_key("Stage::items#items-id@shared")
                .with_list_items(
                    (0..length).map(|index| index.to_string()).collect(),
                    180.0,
                    height,
                ),
        ])
        .unwrap()
    }

    fn stage_point_for_list(layer: &VariableLayer, local_x: f32, local_y: f32) -> (f32, f32) {
        let monitor = &layer.monitors[0];
        (
            monitor.x + local_x * monitor.scale,
            monitor.y - local_y * monitor.scale,
        )
    }

    #[test]
    fn list_plus_appends_one_blank_item_and_opens_it_selected() {
        let mut layer = list_layer();
        let response = click_local(&mut layer, 12.0, 140.0);

        assert!(response.consumed);
        assert!(layer.list_editor_is_open());
        assert_eq!(layer.list_display_len(0), 3);
        assert_eq!(layer.visible_list_rows(0).last(), Some(&(3, String::new())));
        assert!(matches!(
            response.events.as_slice(),
            [RuntimeInputEvent::ListAppend { key }] if key == "Stage::items#items-id@shared"
        ));
        let editor = layer.list_editor.as_ref().unwrap();
        assert_eq!(editor.index, 3);
        assert!(editor.select_all);
        assert!(editor.appended);

        layer.edit_list_item(ListEditorInput::Text("new"));
        let (_, events) = layer.edit_list_item(ListEditorInput::Commit);
        assert!(matches!(
            events.as_slice(),
            [RuntimeInputEvent::ListSetItem { index: 3, value, .. }] if value == "new"
        ));
    }

    #[test]
    fn list_value_editor_selects_all_and_click_positions_caret() {
        let mut layer = list_layer();
        click_local(&mut layer, 35.0, super::LIST_CONTENT_TOP + 11.0);
        assert!(layer.list_editor.as_ref().unwrap().select_all);

        layer.edit_list_item(ListEditorInput::Text("word"));
        let prefix_width = super::text_width(&layer.regular_font, "w", LIST_ROW_FONT_SIZE);
        click_local(
            &mut layer,
            super::LIST_ROW_VALUE_X + super::LIST_ROW_TEXT_INSET + prefix_width,
            super::LIST_CONTENT_TOP + 11.0,
        );
        assert_eq!(layer.list_editor.as_ref().unwrap().caret, 1);
        assert!(!layer.list_editor.as_ref().unwrap().select_all);

        layer.edit_list_item(ListEditorInput::Text("X"));
        let (_, events) = layer.edit_list_item(ListEditorInput::Commit);
        assert!(matches!(
            events.as_slice(),
            [RuntimeInputEvent::ListSetItem { index: 1, value, .. }] if value == "wXord"
        ));
    }

    #[test]
    fn list_active_row_x_deletes_only_that_item() {
        let mut layer = list_layer();
        click_local(&mut layer, 35.0, super::LIST_CONTENT_TOP + 11.0);
        let response = click_local(&mut layer, 165.0, super::LIST_CONTENT_TOP + 11.0);

        assert!(response.consumed);
        assert!(!layer.list_editor_is_open());
        assert!(matches!(
            response.events.as_slice(),
            [RuntimeInputEvent::ListDeleteItem { key, index: 1 }]
                if key == "Stage::items#items-id@shared"
        ));
    }

    #[test]
    fn list_resize_changes_only_available_area_not_typography_or_row_spacing() {
        let mut layer = list_layer();
        let width = layer.monitors[0].width;
        let height = layer.monitors[0].height;
        let scale = layer.monitors[0].scale;
        let row_height = LIST_ROW_HEIGHT;
        let font_size = LIST_ROW_FONT_SIZE;
        let response = click_local(&mut layer, width - 4.0, height - 4.0);
        assert!(response.resizing);

        let monitor = &layer.monitors[0];
        let start_x = monitor.x + (width - 4.0) * scale;
        let start_y = monitor.y - (height - 4.0) * scale;
        assert!(layer.drag_to(start_x + 20.0 * scale, start_y - 18.0 * scale));
        assert!((layer.monitors[0].width - (width + 20.0)).abs() < 0.01);
        assert!((layer.monitors[0].height - (height + 18.0)).abs() < 0.01);
        assert_eq!(LIST_ROW_HEIGHT, row_height);
        assert_eq!(LIST_ROW_FONT_SIZE, font_size);
        assert_eq!(super::LIST_ROW_TEXT_INSET, 6.0);
        assert_eq!(super::LIST_ROW_INDEX_X, 7.0);
    }

    #[test]
    fn list_can_resize_to_stage_bounds_and_move_by_its_title_area() {
        let mut layer = list_layer();
        layer.monitors[0].x = -240.0;
        layer.monitors[0].y = 180.0;
        let scale = layer.monitors[0].scale;
        let width = layer.monitors[0].width;
        let height = layer.monitors[0].height;
        let response = click_local(&mut layer, width - 4.0, height - 4.0);
        assert!(response.resizing);
        let target_x = layer.monitors[0].x + 480.0 - 4.0 * scale;
        let target_y = layer.monitors[0].y - 360.0 + 4.0 * scale;
        assert!(layer.drag_to(target_x, target_y));
        assert!((layer.monitors[0].width * scale - 480.0).abs() < 0.01);
        assert!((layer.monitors[0].height * scale - 360.0).abs() < 0.01);

        let mut movable = list_layer();
        let monitor = &movable.monitors[0];
        let title_x = monitor.x + 20.0 * monitor.scale;
        let title_y = monitor.y - 12.0 * monitor.scale;
        let title_response = movable.handle_list_pointer(title_x, title_y);
        assert!(!title_response.consumed);
        assert!(movable.begin_drag(title_x, title_y));
        assert!(movable.drag_to(title_x + 30.0, title_y - 20.0));
    }

    #[test]
    fn variable_monitor_builds_gpu_instances() {
        let mut layer =
            VariableLayer::new(vec![VariableMonitor::new("my variable", "0", 8.0, 8.0)]).unwrap();
        let (shapes, glyphs) = layer.build_gpu_monitor_instances(0, true, true).unwrap();
        assert!(!shapes.is_empty());
        assert!(!glyphs.is_empty());
    }

    #[test]
    fn list_monitor_uses_list_layout_and_keeps_its_items() {
        let scalar = VariableMonitor::new("score", "3", 8.0, 8.0);
        let monitor = VariableMonitor::new("items", "3", 8.0, 8.0).with_list_items(
            vec!["red".to_owned(), "green".to_owned(), "blue".to_owned()],
            110.0,
            200.0,
        );
        let mut layer = VariableLayer::new(vec![scalar, monitor]).unwrap();
        assert_eq!(layer.monitors[0].scale, super::SCALAR_MONITOR_SCALE);
        assert_eq!(layer.monitors[1].scale, super::LIST_MONITOR_SCALE);
        assert_eq!(layer.monitors[1].display_mode, MonitorDisplayMode::List);
        assert_eq!(layer.monitors[1].category, super::MonitorCategory::Lists);
        assert_eq!(layer.monitors[1].kind, super::MonitorKind::List);
        let (shapes, glyphs) = layer.build_gpu_monitor_instances(1, true, true).unwrap();
        assert_eq!(shapes.len(), 8);
        assert!(glyphs.len() > 10);
        let bounds = layer.monitor_bounds(1);
        assert!(
            layer
                .hit_test((bounds.left + bounds.right) * 0.5 - 240.0, 0.0)
                .is_some()
        );
        assert!(
            layer
                .toggle_compact_at((bounds.left + bounds.right) * 0.5 - 240.0, 0.0)
                .is_none()
        );
    }

    #[test]
    fn list_monitors_draw_above_scalar_monitors() {
        let list = VariableMonitor::new("items", "1", 0.0, 0.0).with_list_items(
            vec!["one".to_owned()],
            110.0,
            100.0,
        );
        let scalar = VariableMonitor::new("score", "1", 0.0, 0.0);
        let layer = VariableLayer::new(vec![list, scalar]).unwrap();

        assert_eq!(
            layer.gpu_monitor_ids_in_draw_order().collect::<Vec<_>>(),
            vec![1, 0]
        );
    }

    #[test]
    fn only_scalar_variable_monitors_cycle_into_slider_mode() {
        let mut scalar =
            VariableLayer::new(vec![VariableMonitor::new("score", "1", 0.0, 0.0)]).unwrap();
        assert!(scalar.toggle_compact_at(1.0, -1.0).is_some());
        assert_eq!(
            scalar.monitors[0].display_mode,
            MonitorDisplayMode::ValueOnly
        );
        assert!(scalar.toggle_compact_at(1.0, -1.0).is_some());
        assert_eq!(scalar.monitors[0].display_mode, MonitorDisplayMode::Slider);

        let reporter = VariableMonitor::new("x position", "0", 0.0, 0.0)
            .with_kind(super::MonitorKind::Reporter)
            .with_category(super::MonitorCategory::Motion);
        let mut reporter = VariableLayer::new(vec![reporter]).unwrap();
        assert!(reporter.toggle_compact_at(1.0, -1.0).is_some());
        assert_eq!(
            reporter.monitors[0].display_mode,
            MonitorDisplayMode::ValueOnly
        );
        assert!(reporter.toggle_compact_at(1.0, -1.0).is_some());
        assert_eq!(
            reporter.monitors[0].display_mode,
            MonitorDisplayMode::Expanded
        );

        let list =
            VariableMonitor::new("items", "0", 0.0, 0.0).with_list_items(Vec::new(), 110.0, 120.0);
        let mut list = VariableLayer::new(vec![list]).unwrap();
        assert!(list.toggle_compact_at(1.0, -1.0).is_none());
        assert_eq!(list.monitors[0].display_mode, MonitorDisplayMode::List);
    }

    #[test]
    fn list_snapshot_refreshes_rows_without_converting_list_to_scalar_readout() {
        let key = "Stage::items#list-id@shared";
        let monitor = VariableMonitor::new("items", "1", 0.0, 0.0)
            .with_value_key(key)
            .with_list_items(vec!["one".to_owned()], 110.0, 120.0);
        let mut layer = VariableLayer::new(vec![monitor]).unwrap();
        let items = Arc::new(vec![
            ScratchValue::Text("one".into()),
            ScratchValue::Text("two".into()),
        ]);
        assert_eq!(
            layer.set_monitor_list(key, Arc::clone(&items)),
            Some((0, true))
        );
        assert!(Arc::ptr_eq(&layer.monitors[0].list_items, &items));
        assert_eq!(layer.monitors[0].display_mode, MonitorDisplayMode::List);
        assert_eq!(layer.take_dirty_gpu_shapes(), vec![0]);
    }

    #[test]
    fn large_list_scroll_keeps_absolute_indices_and_builds_only_visible_gpu_rows() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../aida-ast/tests/src/monitor-tests-sensing.sb3"
        );
        let json = aida_ast::extractor::read_project_json(path).unwrap();
        let project = aida_ast::parser::parse_project(&json).unwrap();
        let source = project
            .targets
            .iter()
            .flat_map(|target| target.lists.values())
            .find(|list| list.items.len() == 200_000)
            .expect("fixture should contain its complete list");
        let values = source
            .items
            .iter()
            .map(|item| match item {
                serde_json::Value::String(value) => ScratchValue::Text(value.clone()),
                serde_json::Value::Number(value) => ScratchValue::Number(value.as_f64().unwrap()),
                serde_json::Value::Bool(value) => ScratchValue::Boolean(*value),
                _ => ScratchValue::Text(item.to_string()),
            })
            .collect::<Vec<_>>();
        let monitor = VariableMonitor::new("large list", "", 0.0, 0.0).with_list_values(
            Arc::new(values),
            220.0,
            196.0,
        );
        let mut layer = VariableLayer::new(vec![monitor]).unwrap();

        let first_rows = layer.visible_list_rows(0);
        assert_eq!(
            first_rows.first(),
            Some(&(1, source.items[0].as_i64().unwrap().to_string()))
        );
        assert_eq!(first_rows.len(), 7);
        let top_scrollbar = layer.list_scrollbar_geometry(0).unwrap();
        assert_eq!(top_scrollbar.thumb_height, super::LIST_SCROLLBAR_MIN_THUMB_HEIGHT);
        assert_eq!(top_scrollbar.thumb_y, top_scrollbar.track_y);
        let (_, first_glyphs) = layer.build_gpu_monitor_instances(0, false, true).unwrap();
        assert!(first_glyphs.len() < 256);

        assert_eq!(layer.scroll_list_at(10.0, -30.0, 250_000.0), Some(0));
        let last_rows = layer.visible_list_rows(0);
        assert_eq!(last_rows.first().unwrap().0, 199_995);
        assert_eq!(
            last_rows.last(),
            Some(&(200_000, source.items[199_999].as_i64().unwrap().to_string()))
        );
        let (_, last_glyphs) = layer.build_gpu_monitor_instances(0, false, true).unwrap();
        assert!(last_glyphs.len() < 256);
        assert_eq!(layer.monitors[0].list_scroll_offset, 199_994);
        let bottom_scrollbar = layer.list_scrollbar_geometry(0).unwrap();
        assert!((bottom_scrollbar.thumb_y + bottom_scrollbar.thumb_height
            - (bottom_scrollbar.track_y + bottom_scrollbar.track_height))
            .abs()
            < 0.01);

        // drag the scrollbar across the full fixture list without traversing its item data.
        assert!(layer.set_list_scroll_offset(0, 0));
        let top_scrollbar = layer.list_scrollbar_geometry(0).unwrap();
        let pointer = stage_point_for_list(
            &layer,
            top_scrollbar.track_x + 1.0,
            top_scrollbar.thumb_y + top_scrollbar.thumb_height * 0.5,
        );
        assert!(layer.handle_list_pointer(pointer.0, pointer.1).scrollbar_dragging);
        let bottom_pointer = stage_point_for_list(
            &layer,
            top_scrollbar.track_x + 1.0,
            top_scrollbar.track_y + top_scrollbar.track_height
                - top_scrollbar.thumb_height * 0.5,
        );
        assert!(layer.drag_to(bottom_pointer.0, bottom_pointer.1));
        assert_eq!(layer.visible_list_rows(0).last().unwrap().0, 200_000);
        let dragged_scrollbar = layer.list_scrollbar_geometry(0).unwrap();
        assert!(
            (dragged_scrollbar.thumb_y + dragged_scrollbar.thumb_height
                - (dragged_scrollbar.track_y + dragged_scrollbar.track_height))
                .abs()
                < 0.01
        );
    }

    #[test]
    fn list_wheel_scroll_accumulates_fractional_rows_and_clamps_to_available_range() {
        let monitor = VariableMonitor::new("items", "", 0.0, 0.0).with_list_items(
            (0..20).map(|index| index.to_string()).collect(),
            160.0,
            196.0,
        );
        let mut layer = VariableLayer::new(vec![monitor]).unwrap();
        assert_eq!(layer.scroll_list_at(10.0, -30.0, 0.5), None);
        assert_eq!(layer.scroll_list_at(10.0, -30.0, 0.5), Some(0));
        assert_eq!(layer.monitors[0].list_scroll_offset, 1);
        assert_eq!(layer.scroll_list_at(10.0, -30.0, 10_000.0), Some(0));
        assert_eq!(layer.monitors[0].list_scroll_offset, 14);
        assert_eq!(layer.scroll_list_at(10.0, -30.0, 3.0), None);
    }

    #[test]
    fn list_scrollbar_only_appears_on_hover_when_content_exceeds_the_viewport() {
        let empty = list_with_items(0, 196.0);
        let fitting = list_with_items(6, 196.0);
        assert_eq!(empty.list_scrollbar_geometry(0), None);
        assert_eq!(fitting.list_scrollbar_geometry(0), None);

        let mut scrollable = list_with_items(7, 196.0);
        let geometry = scrollable.list_scrollbar_geometry(0).unwrap();
        let (hidden_shapes, _) = scrollable
            .build_gpu_monitor_instances(0, true, false)
            .unwrap();
        let track_index = hidden_shapes.len() - 3;
        let thumb_index = hidden_shapes.len() - 2;
        assert_eq!(hidden_shapes[track_index].fill[3], 0.0);
        assert_eq!(hidden_shapes[track_index].fill, [0.0; 4]);
        assert!(track_index < thumb_index);
        assert!(scrollable.update_list_scrollbar_hover(Some(stage_point_for_list(
            &scrollable,
            geometry.track_x + 1.0,
            geometry.track_y + 1.0,
        ))));
        assert!(scrollable.list_scrollbar_is_visible(0));
        let (visible_shapes, _) = scrollable
            .build_gpu_monitor_instances(0, true, false)
            .unwrap();
        assert_eq!(visible_shapes.len(), hidden_shapes.len());
        assert_eq!(visible_shapes[track_index].fill[3], 42.0 / 255.0);
        assert_eq!(visible_shapes[thumb_index].fill[3], 190.0 / 255.0);
        assert!(
            visible_shapes[track_index].fill[0] < visible_shapes[thumb_index].fill[0]
        );
        for index in 0..visible_shapes.len() {
            if index != track_index && index != thumb_index {
                assert_eq!(visible_shapes[index], hidden_shapes[index]);
            }
        }
        assert!(scrollable.update_list_scrollbar_hover(None));
        assert!(!scrollable.list_scrollbar_is_visible(0));
    }

    #[test]
    fn partial_last_list_row_keeps_glyph_spacing_and_clips_at_the_footer() {
        let mut layer = list_with_items(20, 196.0);
        let (_, glyphs) = layer.build_gpu_monitor_instances(0, true, true).unwrap();
        let visible_rows = layer.visible_list_rows(0);
        let last_row = visible_rows.last().unwrap();
        assert_eq!(last_row, &(7, "6".to_owned()));

        let footer_top = layer.monitors[0].height - super::LIST_FOOTER_HEIGHT;
        let partial_row_y = super::LIST_CONTENT_TOP + 6.0 * super::LIST_ROW_HEIGHT;
        let last_row_glyphs = glyphs
            .iter()
            .filter(|glyph| glyph.clip_y[0] == partial_row_y)
            .collect::<Vec<_>>();
        assert!(last_row_glyphs.len() >= 2);
        assert!(
            last_row_glyphs
                .iter()
                .all(|glyph| glyph.clip_y[1] == footer_top)
        );
        assert!(last_row_glyphs
            .iter()
            .any(|glyph| glyph.local_position[0] < super::LIST_ROW_VALUE_X));
        assert!(last_row_glyphs
            .iter()
            .any(|glyph| glyph.local_position[0] >= super::LIST_ROW_VALUE_X));
        let bucket = layer.glyph_atlas_scale_bucket;
        let index_glyph_id = layer.glyph_atlas_entries[&('7', true, bucket)].id;
        let value_glyph_id = layer.glyph_atlas_entries[&('6', false, bucket)].id;
        assert!(last_row_glyphs
            .iter()
            .any(|glyph| glyph.glyph_id == index_glyph_id));
        assert!(last_row_glyphs
            .iter()
            .any(|glyph| glyph.glyph_id == value_glyph_id));
        let previous_index_glyph_id = layer.glyph_atlas_entries[&('6', true, bucket)].id;
        let previous_value_glyph_id = layer.glyph_atlas_entries[&('5', false, bucket)].id;
        let previous_row_y = partial_row_y - super::LIST_ROW_HEIGHT;
        let previous_row_glyphs = glyphs
            .iter()
            .filter(|glyph| glyph.clip_y[0] == previous_row_y)
            .collect::<Vec<_>>();
        let last_index_y = last_row_glyphs
            .iter()
            .find(|glyph| glyph.glyph_id == index_glyph_id)
            .unwrap()
            .local_position[1];
        let previous_index_y = previous_row_glyphs
            .iter()
            .find(|glyph| glyph.glyph_id == previous_index_glyph_id)
            .unwrap()
            .local_position[1];
        let last_value_y = last_row_glyphs
            .iter()
            .find(|glyph| glyph.glyph_id == value_glyph_id)
            .unwrap()
            .local_position[1];
        let previous_value_y = previous_row_glyphs
            .iter()
            .find(|glyph| glyph.glyph_id == previous_value_glyph_id)
            .unwrap()
            .local_position[1];
        assert_eq!(last_index_y - previous_index_y, super::LIST_ROW_HEIGHT);
        assert_eq!(last_value_y - previous_value_y, super::LIST_ROW_HEIGHT);
    }

    #[test]
    fn list_scrollbar_thumb_scales_and_maps_top_middle_and_bottom_offsets() {
        let proportional = list_with_items(20, 196.0)
            .list_scrollbar_geometry(0)
            .unwrap();
        let expected_thumb_height = proportional.track_height * proportional.track_height
            / (20.0 * super::LIST_ROW_HEIGHT);
        assert!((proportional.thumb_height - expected_thumb_height).abs() < 0.01);

        let mut layer = list_with_items(1_000, 196.0);
        let top = layer.list_scrollbar_geometry(0).unwrap();
        assert_eq!(top.thumb_y, top.track_y);
        assert_eq!(top.thumb_height, super::LIST_SCROLLBAR_MIN_THUMB_HEIGHT);

        let max_offset = layer.monitors[0].max_list_scroll_offset(1_000);
        assert!(layer.set_list_scroll_offset(0, max_offset / 2));
        let middle = layer.list_scrollbar_geometry(0).unwrap();
        let travel = middle.track_height - middle.thumb_height;
        assert!((middle.thumb_y - (middle.track_y + travel * 0.5)).abs() < 1.0);

        assert!(layer.set_list_scroll_offset(0, max_offset));
        let bottom = layer.list_scrollbar_geometry(0).unwrap();
        assert!((bottom.thumb_y + bottom.thumb_height
            - (bottom.track_y + bottom.track_height))
            .abs()
            < 0.01);
    }

    #[test]
    fn list_scrollbar_drag_and_track_click_scroll_visible_rows() {
        let mut layer = list_with_items(100, 196.0);
        let geometry = layer.list_scrollbar_geometry(0).unwrap();
        let pointer = stage_point_for_list(
            &layer,
            geometry.track_x + 2.0,
            geometry.thumb_y + geometry.thumb_height * 0.5,
        );
        let response = layer.handle_list_pointer(pointer.0, pointer.1);
        assert!(response.consumed);
        assert!(response.scrollbar_dragging);

        let bottom_pointer_y = geometry.track_y + geometry.track_height
            - geometry.thumb_height * 0.5;
        let bottom_pointer = stage_point_for_list(&layer, geometry.track_x + 2.0, bottom_pointer_y);
        assert!(layer.drag_to(bottom_pointer.0, bottom_pointer.1));
        assert_eq!(layer.monitors[0].list_scroll_offset, 100 - 6);
        assert_eq!(layer.visible_list_rows(0).first().unwrap().0, 95);
        layer.end_drag();

        assert!(layer.set_list_scroll_offset(0, 0));
        let top = layer.list_scrollbar_geometry(0).unwrap();
        let track_click = stage_point_for_list(
            &layer,
            top.track_x + 2.0,
            top.thumb_y + top.thumb_height + 2.0,
        );
        let response = layer.handle_list_pointer(track_click.0, track_click.1);
        assert!(response.consumed);
        assert!(!response.scrollbar_dragging);
        assert!(layer.monitors[0].list_scroll_offset > 0);
    }

    #[test]
    fn list_scrollbar_recalculates_after_list_growth_and_viewport_resize() {
        let mut layer = list_with_items(20, 196.0);
        layer.take_dirty_gpu_shapes();
        let initial = layer.list_scrollbar_geometry(0).unwrap();
        let longer = Arc::new(
            (0..40)
                .map(|index| ScratchValue::Text(index.to_string()))
                .collect(),
        );
        assert_eq!(
            layer.set_monitor_list("Stage::items#items-id@shared", longer),
            Some((0, true))
        );
        assert_eq!(layer.take_dirty_gpu_shapes(), vec![0]);
        let after_growth = layer.list_scrollbar_geometry(0).unwrap();
        assert!(after_growth.thumb_height < initial.thumb_height);

        let monitor = &layer.monitors[0];
        let scale = monitor.scale;
        let start_x = monitor.x + (monitor.width - 4.0) * scale;
        let start_y = monitor.y - (monitor.height - 4.0) * scale;
        assert!(click_local(&mut layer, 176.0, 192.0).resizing);
        assert!(layer.drag_to(start_x, start_y - 44.0 * scale));
        let after_resize = layer.list_scrollbar_geometry(0).unwrap();
        assert!(after_resize.track_height > after_growth.track_height);
        assert!(after_resize.thumb_height > after_growth.thumb_height);
        assert_eq!(layer.take_dirty_gpu_shapes(), vec![0]);
    }

    #[test]
    fn list_changes_refresh_visible_text_without_rebuilding_row_shapes() {
        let initial = Arc::new(
            (0..20)
                .map(|index| ScratchValue::Number(index as f64))
                .collect::<Vec<_>>(),
        );
        let monitor = VariableMonitor::new("items", "", 0.0, 0.0)
            .with_value_key("Stage::items#list-id@shared")
            .with_list_values(Arc::clone(&initial), 160.0, 196.0);
        let mut layer = VariableLayer::new(vec![monitor]).unwrap();
        layer.take_dirty_gpu_shapes();
        layer.take_dirty_gpu_texts();
        let (_, original_glyphs) = layer.build_gpu_monitor_instances(0, false, true).unwrap();

        let mut updated = (*initial).clone();
        updated[19] = ScratchValue::Text("updated tail".into());
        assert_eq!(
            layer.set_monitor_list("Stage::items#list-id@shared", Arc::new(updated)),
            Some((0, true))
        );
        assert!(layer.take_dirty_gpu_shapes().is_empty());
        assert_eq!(layer.take_dirty_gpu_texts(), vec![0]);
        assert_eq!(
            layer.visible_list_rows(0).last(),
            Some(&(7, "6".to_owned()))
        );
        let (_, unchanged_glyphs) = layer.build_gpu_monitor_instances(0, false, true).unwrap();
        assert_eq!(original_glyphs, unchanged_glyphs);

        layer.scroll_list_at(10.0, -30.0, 100.0);
        assert_eq!(
            layer.visible_list_rows(0).last(),
            Some(&(20, "updated tail".to_owned()))
        );
        let (_, updated_glyphs) = layer.build_gpu_monitor_instances(0, false, true).unwrap();
        assert_ne!(original_glyphs, updated_glyphs);
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

        layer.take_dirty_gpu_texts();
        assert_eq!(
            layer.set_monitor_value("timer", "12.3456788".to_owned()),
            None
        );
        assert!(layer.take_dirty_gpu_shapes().is_empty());
        assert!(layer.take_dirty_gpu_texts().is_empty());
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
