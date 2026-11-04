use ab_glyph::{Font, ScaleFont};
use bytemuck::Zeroable;
use wgpu::Queue;

use super::super::texture::RenderTexture;
use super::menu::{
    CONTEXT_MENU_ROW_HEIGHT, CONTEXT_MENU_WIDTH, MonitorContextAction, SLIDER_RANGE_EDITOR_HEIGHT,
    SLIDER_RANGE_EDITOR_WIDTH,
};
use super::{
    GLYPH_ATLAS_MAX_SIZE, GlyphAtlasEntry, LIST_CONTENT_TOP, LIST_EMPTY_TOP_PADDING,
    LIST_FOOTER_HEIGHT, LIST_FOOTER_SIDE_PADDING, LIST_HEADER_FONT_SIZE,
    LIST_HEADER_HORIZONTAL_PADDING, LIST_LENGTH_FONT_SIZE, LIST_ROW_FONT_SIZE,
    LIST_ROW_HEIGHT, LIST_ROW_INDEX_GAP, LIST_ROW_INDEX_X, LIST_ROW_TEXT_INSET,
    LIST_SCROLLBAR_RIGHT_INSET, LIST_SCROLLBAR_WIDTH,
};
use super::{
    MonitorVisibility, SCRATCH_HEIGHT, SCRATCH_WIDTH, VariableLayer,
    canvas::{blur_shadow_alpha, draw_monitor_shadow},
    tessellate_glyph_outline,
};
use crate::graphics::pipelines::{GlyphInstance, GlyphMetadata, MonitorInstance, VariableVertex};
use crate::runtime::{BubbleKind, BubbleSnapshot};

const SHADOW_BLUR_RADIUS: f32 = 2.5;
const MONITOR_FRAME_RADIUS: f32 = 7.0;
const MONITOR_VALUE_RADIUS: f32 = 8.0;
const LIST_ROW_RADIUS: f32 = 4.0;
const MONITOR_FRAME_FILL: [u8; 4] = [229, 240, 255, 255];
const MONITOR_FRAME_BORDER: [u8; 4] = [0, 0, 0, 38];
const MONITOR_SHAPE_TOP_ROUNDED: f32 = 3.0;
const MONITOR_SHAPE_BOTTOM_ROUNDED: f32 = 4.0;

pub(crate) struct BubbleRenderInput {
    pub(crate) bubble: BubbleSnapshot,
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) sprite_width: f32,
    pub(crate) sprite_height: f32,
}

#[derive(Default)]
pub(crate) struct BubbleGpuBatch {
    pub(crate) transforms: Vec<[f32; 4]>,
    pub(crate) shapes: Vec<MonitorInstance>,
    pub(crate) glyphs: Vec<GlyphInstance>,
}

const BUBBLE_MAX_WIDTH: f32 = 210.0;
const BUBBLE_PADDING_X: f32 = 10.0;
const BUBBLE_PADDING_Y: f32 = 8.0;
const BUBBLE_LINE_HEIGHT: f32 = 10.5;
const BUBBLE_FONT_SIZE: f32 = 9.0;
const BUBBLE_MIN_WIDTH: f32 = 56.0;
const BUBBLE_EDGE: f32 = 8.0;
const BUBBLE_BODY_CORNER_RADIUS: f32 = 17.0;
const BUBBLE_DARK: [u8; 4] = [82, 91, 113, 255];
const BUBBLE_BORDER: [u8; 4] = [215, 215, 215, 255];
const BUBBLE_BORDER_WIDTH: f32 = 1.1;
const BUBBLE_SPEECH_TAIL_EXTENT: f32 = 12.5;
const BUBBLE_TAIL_CORNER_CLEARANCE: f32 = 2.0;

impl VariableLayer {
    pub(crate) fn build_gpu_bubble_instances(
        &mut self,
        bubbles: &[BubbleRenderInput],
    ) -> Result<BubbleGpuBatch, String> {
        let mut batch = BubbleGpuBatch::default();
        let margin = BUBBLE_EDGE;
        let max_height = SCRATCH_HEIGHT as f32 - margin * 2.0;
        for input in bubbles {
            let sprite_x = SCRATCH_WIDTH as f32 * 0.5 + input.x;
            let sprite_y = SCRATCH_HEIGHT as f32 * 0.5 - input.y;
            let sprite_side_offset = input.sprite_width * 0.38 + 2.0;
            let right_x = sprite_x + sprite_side_offset;
            let left_edge = sprite_x - sprite_side_offset;
            let right_room = SCRATCH_WIDTH as f32 - margin - right_x;
            let left_room = left_edge - margin;
            let body_on_right = right_room >= BUBBLE_MIN_WIDTH || right_room >= left_room;
            let body_max_width = BUBBLE_MAX_WIDTH
                .min(if body_on_right { right_room } else { left_room })
                .max(BUBBLE_MIN_WIDTH);
            let inner_width = body_max_width - BUBBLE_PADDING_X * 2.0;
            let lines = wrap_bubble_text(&self.regular_font, &input.bubble.text, inner_width);
            let height = (lines.len().max(1) as f32 * BUBBLE_LINE_HEIGHT + BUBBLE_PADDING_Y * 2.0)
                .min(max_height);
            let text_width = lines
                .iter()
                .map(|line| super::text_width(&self.regular_font, line, BUBBLE_FONT_SIZE))
                .fold(0.0_f32, f32::max);
            let width =
                (text_width + BUBBLE_PADDING_X * 2.0).clamp(BUBBLE_MIN_WIDTH, body_max_width);

            let desired_x = if body_on_right {
                right_x
            } else {
                left_edge - width
            };
            let left = desired_x.clamp(margin, SCRATCH_WIDTH as f32 - margin - width);
            let sprite_top = sprite_y - input.sprite_height * 0.5;
            let desired_top = sprite_top - height + 8.0;
            let lower_extent = if input.bubble.kind == BubbleKind::Say {
                BUBBLE_SPEECH_TAIL_EXTENT
            } else {
                0.0
            };
            let top = desired_top.clamp(
                margin,
                SCRATCH_HEIGHT as f32 - margin - height - lower_extent,
            );
            let bubble_id = batch.transforms.len() as u32;
            batch.transforms.push([left, top, 1.0, 0.0]);
            let fill = linear_rgba([255, 255, 255, 255]);
            let border = linear_rgba(BUBBLE_BORDER);
            if input.bubble.kind == BubbleKind::Think {
                let thought_radius = 4.6_f32;
                let safe_offset = BUBBLE_BODY_CORNER_RADIUS + thought_radius;
                let anchor_x = (sprite_x - left).clamp(safe_offset, width - safe_offset);
                let thought_base = [left + anchor_x, top + height];
                let direction_x = if sprite_x < left + width * 0.5 {
                    -1.0
                } else {
                    1.0
                };
                for (center, radius) in [
                    (thought_base, thought_radius),
                    (
                        [thought_base[0] + direction_x * 7.5, thought_base[1] + 4.0],
                        3.4,
                    ),
                    (
                        [thought_base[0] + direction_x * 13.0, thought_base[1] + 8.0],
                        2.4,
                    ),
                ] {
                    push_bubble_shape(
                        &mut batch.shapes,
                        [
                            center[0] - left - radius,
                            center[1] - top - radius,
                            radius * 2.0,
                            radius * 2.0,
                        ],
                        fill,
                        border,
                        radius,
                        bubble_id,
                    );
                }
            }

            push_bubble_shape(
                &mut batch.shapes,
                [0.0, 0.0, width, height + lower_extent],
                fill,
                border,
                17.0,
                bubble_id,
            );
            if input.bubble.kind == BubbleKind::Say {
                let body = batch.shapes.last_mut().expect("speech bubble body");
                let safe_offset = BUBBLE_BODY_CORNER_RADIUS + BUBBLE_TAIL_CORNER_CLEARANCE;
                let anchor_x = (sprite_x - left).clamp(safe_offset, width - safe_offset);
                let direction = if sprite_x < left + width * 0.5 {
                    1.0
                } else {
                    2.0
                };
                body.shape = [width, height, direction, anchor_x];
            }

            for (line_index, line) in lines.iter().enumerate() {
                for ch in line.chars() {
                    self.ensure_glyph_atlas(ch, false)?;
                }
                let scaled_font = self.regular_font.as_scaled(BUBBLE_FONT_SIZE);
                let baseline = super::canvas::centered_baseline(
                    &self.regular_font,
                    BUBBLE_FONT_SIZE,
                    BUBBLE_PADDING_Y
                        + line_index as f32 * BUBBLE_LINE_HEIGHT
                        + BUBBLE_LINE_HEIGHT * 0.5,
                );
                let mut pen_x = BUBBLE_PADDING_X;
                let mut previous = None;
                for ch in line.chars() {
                    let glyph = scaled_font.glyph_id(ch);
                    if let Some(previous) = previous {
                        pen_x += scaled_font.kern(previous, glyph);
                    }
                    if let Some(entry) =
                        self.glyph_atlas_entries
                            .get(&(ch, false, self.glyph_atlas_scale_bucket))
                    {
                        if entry.width > 0 && entry.height > 0 {
                            batch.glyphs.push(GlyphInstance {
                                local_position: [pen_x, baseline],
                                glyph_id: entry.id,
                                monitor_id: bubble_id,
                                color: linear_rgba(BUBBLE_DARK),
                                scale: BUBBLE_FONT_SIZE / 19.0,
                                clip_y: [0.0; 2],
                            });
                        }
                    }
                    pen_x += scaled_font.h_advance(glyph);
                    previous = Some(glyph);
                }
            }
        }
        Ok(batch)
    }

    pub(crate) fn glyph_atlas_revision(&self) -> u64 {
        self.glyph_atlas_revision
    }

    pub(crate) fn glyph_atlas_size(&self) -> u32 {
        self.glyph_atlas_size
    }

    pub(crate) fn glyph_metadata(&self) -> Vec<GlyphMetadata> {
        let mut metadata = vec![GlyphMetadata::zeroed(); self.glyph_atlas_count.max(1) as usize];
        for entry in self.glyph_atlas_entries.values() {
            let atlas_size = self.glyph_atlas_size as f32;
            metadata[entry.id as usize] = GlyphMetadata {
                uv: [
                    (entry.x as f32 + 0.5) / atlas_size,
                    (entry.y as f32 + 0.5) / atlas_size,
                    entry.width.saturating_sub(1) as f32 / atlas_size,
                    entry.height.saturating_sub(1) as f32 / atlas_size,
                ],
                bounds: entry.bounds,
            };
        }
        metadata
    }

    pub(crate) fn take_glyph_atlas_updates(&mut self) -> Vec<(u32, u32, u32, u32, Vec<u8>)> {
        if self.glyph_atlas_full_dirty {
            self.glyph_atlas_full_dirty = false;
            self.glyph_atlas_dirty_regions.clear();
            return vec![(
                0,
                0,
                self.glyph_atlas_size,
                self.glyph_atlas_size,
                self.glyph_atlas.clone(),
            )];
        }
        self.glyph_atlas_dirty_regions
            .drain(..)
            .map(|(x, y, width, height)| {
                let mut pixels = Vec::with_capacity((width * height) as usize);
                for row in y..y + height {
                    let start = (row * self.glyph_atlas_size + x) as usize;
                    pixels.extend_from_slice(&self.glyph_atlas[start..start + width as usize]);
                }
                (x, y, width, height, pixels)
            })
            .collect()
    }

    pub(crate) fn build_gpu_monitor_instances(
        &mut self,
        index: usize,
        build_shapes: bool,
        build_text: bool,
    ) -> Result<(Vec<MonitorInstance>, Vec<GlyphInstance>), String> {
        let mut shapes = Vec::new();
        let mut glyphs = Vec::new();
        if self.monitors[index].visibility == MonitorVisibility::Hidden {
            return Ok((shapes, glyphs));
        }
        let name = self.monitors[index].name.clone();
        let value = self.monitors[index].value.clone();
        if self.monitors[index].kind == super::MonitorKind::List {
            return self.build_gpu_list_monitor_instances(index, build_shapes, build_text);
        }
        let compact = self.monitors[index].display_mode == super::MonitorDisplayMode::ValueOnly;
        if build_text && !compact {
            for ch in name.chars() {
                self.ensure_glyph_atlas(ch, true)?;
            }
        }
        if build_text {
            for ch in value.chars() {
                self.ensure_glyph_atlas(ch, false)?;
            }
        }

        let m = &self.monitors[index];
        let gpu_id = m.gpu_id;
        let slider = m.display_mode == super::MonitorDisplayMode::Slider;
        let value_x = if compact {
            0.0
        } else {
            14.0 + m.label_width + 14.0
        };
        let value_y = if compact { 0.0 } else { 4.0 };
        let value_height = if compact { m.height } else { 26.0 };
        let value_radius = if compact { 7.0 } else { MONITOR_VALUE_RADIUS };
        if build_shapes && !compact {
            push_monitor_instance(
                &mut shapes,
                gpu_id,
                0.0,
                0.0,
                m.width,
                m.height,
                MONITOR_FRAME_RADIUS,
                1.0,
                MONITOR_FRAME_FILL,
                MONITOR_FRAME_BORDER,
            );
        }
        let vc = monitor_color(m.category);
        if build_shapes && compact {
            push_monitor_instance(
                &mut shapes,
                gpu_id,
                value_x,
                value_y,
                m.value_width,
                value_height,
                value_radius,
                1.0,
                vc,
                [189, 201, 217, 255],
            );
        } else if build_shapes {
            push_monitor_instance(
                &mut shapes,
                gpu_id,
                value_x,
                value_y,
                m.value_width,
                value_height,
                value_radius,
                1.0,
                vc,
                MONITOR_FRAME_BORDER,
            );
        }
        if build_text && !compact {
            let baseline = super::canvas::centered_baseline(
                &self.bold_font,
                18.0,
                if slider { 17.0 } else { m.height * 0.5 },
            );
            self.push_glyph_instances(
                &mut glyphs,
                &name,
                true,
                gpu_id,
                14.0,
                baseline,
                m.label_width,
                [82, 93, 117, 255],
            );
        }
        let value_start = value_x + m.value_width * 0.5 - m.value_text_width * 0.5;
        let baseline = super::canvas::centered_baseline(
            &self.regular_font,
            19.0,
            value_y + value_height * 0.5,
        );
        if build_text {
            self.push_glyph_instances(
                &mut glyphs,
                &value,
                false,
                gpu_id,
                value_start,
                baseline,
                m.value_text_width,
                [255, 255, 255, 255],
            );
        }
        if build_shapes && slider {
            let hovered = self.slider_hovered == Some(index) && self.slider_dragging != Some(index);
            let thumb_active =
                self.slider_thumb_hovered == Some(index) || self.slider_dragging == Some(index);
            let track_border = if hovered {
                [189, 201, 217, 255]
            } else {
                [217, 227, 242, 255]
            };
            let track_fill = [233, 241, 252, 255];
            let scratch_blue = if thumb_active {
                [51, 115, 204, 255]
            } else {
                [76, 151, 255, 255]
            };
            push_monitor_instance(
                &mut shapes,
                gpu_id,
                14.0,
                36.0,
                m.width - 28.0,
                12.0,
                6.0,
                0.0,
                track_border,
                track_border,
            );
            push_monitor_instance(
                &mut shapes,
                gpu_id,
                15.0,
                37.0,
                m.width - 30.0,
                10.0,
                5.0,
                0.0,
                track_fill,
                track_fill,
            );
            let thumb_x = 14.0 + (m.width - 28.0) * super::canvas::monitor_slider_ratio(m);
            let active_width = (thumb_x - 15.0).clamp(0.0, (m.width - 30.0).max(0.0));
            if active_width > 0.0 {
                push_monitor_instance(
                    &mut shapes,
                    gpu_id,
                    15.0,
                    37.0,
                    active_width,
                    10.0,
                    5.0,
                    0.0,
                    scratch_blue,
                    scratch_blue,
                );
            }
            push_monitor_instance(
                &mut shapes,
                gpu_id,
                thumb_x - 8.0,
                34.0,
                16.0,
                16.0,
                8.0,
                0.0,
                scratch_blue,
                scratch_blue,
            );
        }
        Ok((shapes, glyphs))
    }

    fn build_gpu_list_monitor_instances(
        &mut self,
        index: usize,
        build_shapes: bool,
        build_text: bool,
    ) -> Result<(Vec<MonitorInstance>, Vec<GlyphInstance>), String> {
        let monitor = &self.monitors[index];
        let id = monitor.gpu_id;
        let name = monitor.name.clone();
        let item_count = self.list_display_len(index);
        let width = monitor.width;
        let height = monitor.height;
        let category_color = monitor_color(monitor.category);
        let visible_rows = self.visible_list_rows(index);
        let visible_count = visible_rows.len();
        let scrollbar = self.list_scrollbar_geometry(index);
        let shown_items = visible_rows
            .iter()
            .map(|(row_index, item)| {
                let editing = self.list_editor.as_ref().is_some_and(|editor| {
                    editor.monitor_index == index && editor.index == *row_index
                });
                // reserve the remaining row width for the value, leaving room for edit controls.
                let editor_reserve = if editing { 34.0 } else { 18.0 };
                let row_value_x = self.list_row_value_x(index, *row_index);
                let max_width = width - row_value_x - editor_reserve;
                fit_monitor_text(&self.regular_font, item, max_width, LIST_ROW_FONT_SIZE)
            })
            .collect::<Vec<_>>();
        let shown_name = fit_monitor_text(
            &self.bold_font,
            &name,
            width - LIST_HEADER_HORIZONTAL_PADDING * 2.0,
            LIST_HEADER_FONT_SIZE,
        );
        let length = format!("length {item_count}");
        let plus_width = super::text_width(&self.bold_font, "+", LIST_LENGTH_FONT_SIZE);
        let equals_width = super::text_width(&self.bold_font, "=", LIST_LENGTH_FONT_SIZE);
        let footer_text_width =
            (width - LIST_FOOTER_SIDE_PADDING * 2.0 - plus_width - equals_width).max(1.0);
        let shown_length = fit_monitor_text(
            &self.bold_font,
            &length,
            footer_text_width,
            LIST_LENGTH_FONT_SIZE,
        );
        if build_text {
            for ch in shown_name.chars() {
                self.ensure_glyph_atlas(ch, true)?;
            }
            for ch in shown_length.chars().chain(['+', '=']) {
                self.ensure_glyph_atlas(ch, true)?;
            }
            if item_count == 0 {
                for ch in "(empty)".chars() {
                    self.ensure_glyph_atlas(ch, false)?;
                }
            }
            for (row, item) in shown_items.iter().enumerate() {
                let row_value_x = self.list_row_value_x(index, visible_rows[row].0);
                let index_max_width = (row_value_x - LIST_ROW_INDEX_X - LIST_ROW_INDEX_GAP)
                    .max(0.0);
                let index_text = visible_rows[row].0.to_string();
                let shown_index = fit_monitor_text(
                    &self.bold_font,
                    &index_text,
                    index_max_width,
                    LIST_ROW_FONT_SIZE,
                );
                // prepare every glyph used by the fitted label, including a possible ellipsis.
                for ch in shown_index.chars() {
                    self.ensure_glyph_atlas(ch, true)?;
                }
                for ch in item.chars() {
                    self.ensure_glyph_atlas(ch, false)?;
                }
                if self.list_editor.as_ref().is_some_and(|editor| {
                    editor.monitor_index == index && editor.index == visible_rows[row].0
                }) {
                    self.ensure_glyph_atlas('x', true)?;
                    self.ensure_glyph_atlas('|', true)?;
                }
            }
        }

        let mut shapes = Vec::new();
        let mut glyphs = Vec::new();
        if build_shapes {
            push_monitor_instance(
                &mut shapes,
                id,
                0.0,
                0.0,
                width,
                height,
                MONITOR_FRAME_RADIUS,
                1.0,
                MONITOR_FRAME_FILL,
                MONITOR_FRAME_BORDER,
            );
            push_monitor_instance(
                &mut shapes,
                id,
                1.0,
                1.0,
                width - 2.0,
                23.0,
                MONITOR_FRAME_RADIUS - 1.0,
                0.0,
                [255, 255, 255, 255],
                [255, 255, 255, 255],
            );
            shapes.last_mut().unwrap().shape = [
                width - 2.0,
                23.0,
                MONITOR_SHAPE_TOP_ROUNDED,
                MONITOR_FRAME_RADIUS - 1.0,
            ];
            for row in 0..visible_count {
                let y = LIST_CONTENT_TOP + row as f32 * 24.0;
                if y >= height - LIST_FOOTER_HEIGHT {
                    break;
                }
                let row_value_x = self.list_row_value_x(index, visible_rows[row].0);
                // let this row's index determine how much width the value field receives.
                push_monitor_instance(
                    &mut shapes,
                    id,
                    row_value_x,
                    y,
                    (width - row_value_x - 6.0).max(0.0),
                    22.0,
                    LIST_ROW_RADIUS,
                    1.0,
                    category_color,
                    [0, 0, 0, 38],
                );
                if let Some(editor) = self.list_editor.as_ref().filter(|editor| {
                    editor.monitor_index == index
                        && editor.select_all
                        && editor.index == visible_rows[row].0
                }) {
                    let _ = editor;
                    let selection_width = super::text_width(
                        &self.regular_font,
                        &shown_items[row],
                        LIST_ROW_FONT_SIZE,
                    )
                    .min((width - row_value_x - LIST_ROW_TEXT_INSET - 28.0).max(0.0));
                    if selection_width > 0.0 {
                        push_monitor_instance(
                            &mut shapes,
                            id,
                            row_value_x + LIST_ROW_TEXT_INSET,
                            y + 2.0,
                            selection_width,
                            18.0,
                            1.0,
                            0.0,
                            [63, 123, 220, 255],
                            [63, 123, 220, 255],
                        );
                    }
                }
            }
            // keep two reusable instances so hover only updates scrollbar data, not batch layout.
            let track_y = LIST_CONTENT_TOP;
            let track_height = (height - LIST_CONTENT_TOP - LIST_FOOTER_HEIGHT).max(1.0);
            let track_x = width - LIST_SCROLLBAR_RIGHT_INSET - LIST_SCROLLBAR_WIDTH;
            let thumb_height = scrollbar.map_or(track_height, |bar| bar.thumb_height);
            let thumb_y = scrollbar.map_or(track_y, |bar| bar.thumb_y);
            let scrollbar_visible =
                scrollbar.is_some() && self.list_scrollbar_is_visible(index);
            let track_alpha = if scrollbar_visible { 42 } else { 0 };
            let thumb_alpha = if scrollbar_visible { 190 } else { 0 };
            let track_color = [82, 93, 117, track_alpha];
            let thumb_color = [82, 93, 117, thumb_alpha];
            let track_instance = shapes.len();
            push_monitor_instance(
                &mut shapes,
                id,
                track_x,
                track_y,
                LIST_SCROLLBAR_WIDTH,
                track_height,
                LIST_SCROLLBAR_WIDTH * 0.5,
                0.0,
                track_color,
                track_color,
            );
            // premultiply these translucent colors for the monitor pipeline's blend mode.
            shapes[track_instance].fill = premultiplied_linear_rgba(track_color);
            shapes[track_instance].border = premultiplied_linear_rgba(track_color);
            let thumb_instance = shapes.len();
            push_monitor_instance(
                &mut shapes,
                id,
                track_x + 0.5,
                thumb_y,
                LIST_SCROLLBAR_WIDTH - 1.0,
                thumb_height,
                (LIST_SCROLLBAR_WIDTH - 1.0) * 0.5,
                0.0,
                thumb_color,
                thumb_color,
            );
            // keep the track first so its edge stays behind the thumb.
            shapes[thumb_instance].fill = premultiplied_linear_rgba(thumb_color);
            shapes[thumb_instance].border = premultiplied_linear_rgba(thumb_color);
            // draw the footer after the rows so it covers the partial bottom row.
            push_monitor_instance(
                &mut shapes,
                id,
                1.0,
                height - LIST_FOOTER_HEIGHT,
                width - 2.0,
                21.0,
                MONITOR_FRAME_RADIUS - 1.0,
                0.0,
                [255, 255, 255, 255],
                [255, 255, 255, 255],
            );
            shapes.last_mut().unwrap().shape = [
                width - 2.0,
                21.0,
                MONITOR_SHAPE_BOTTOM_ROUNDED,
                MONITOR_FRAME_RADIUS - 1.0,
            ];
        }
        if build_text {
            let header_baseline =
                super::canvas::centered_baseline(&self.bold_font, LIST_HEADER_FONT_SIZE, 12.5);
            self.push_glyph_instances_at_size(
                &mut glyphs,
                &shown_name,
                true,
                id,
                centered_glyph_start(&self.bold_font, &shown_name, LIST_HEADER_FONT_SIZE, width),
                header_baseline,
                width,
                LIST_HEADER_FONT_SIZE,
                [82, 93, 117, 255],
            );
            for (row, item) in shown_items.iter().enumerate() {
                let y = LIST_CONTENT_TOP + row as f32 * 24.0;
                let footer_top = height - LIST_FOOTER_HEIGHT;
                let visible_height = (footer_top - y).clamp(0.0, LIST_ROW_HEIGHT);
                if visible_height <= 0.0 {
                    break;
                }
                let row_glyph_start = glyphs.len();
                let index_text = visible_rows[row].0.to_string();
                let row_value_x = self.list_row_value_x(index, visible_rows[row].0);
                let index_max_width = (row_value_x - LIST_ROW_INDEX_X - LIST_ROW_INDEX_GAP)
                    .max(0.0);
                // keep narrow monitors from drawing index glyphs into the value column.
                let shown_index = fit_monitor_text(
                    &self.bold_font,
                    &index_text,
                    index_max_width,
                    LIST_ROW_FONT_SIZE,
                );
                let row_center_y = y + 11.0;
                let baseline = super::canvas::centered_baseline(
                    &self.bold_font,
                    LIST_ROW_FONT_SIZE,
                    row_center_y,
                );
                self.push_glyph_instances_at_size(
                    &mut glyphs,
                    &shown_index,
                    true,
                    id,
                    LIST_ROW_INDEX_X,
                    baseline,
                    index_max_width,
                    LIST_ROW_FONT_SIZE,
                    [82, 93, 117, 255],
                );
                self.push_glyph_instances_at_size(
                    &mut glyphs,
                    item,
                    false,
                    id,
                    row_value_x + LIST_ROW_TEXT_INSET,
                    super::canvas::centered_baseline(
                        &self.regular_font,
                        LIST_ROW_FONT_SIZE,
                        row_center_y,
                    ),
                    width - row_value_x - LIST_ROW_TEXT_INSET - 12.0,
                    LIST_ROW_FONT_SIZE,
                    [255, 255, 255, 255],
                );
                if let Some(editor) = self.list_editor.as_ref().filter(|editor| {
                    editor.monitor_index == index && editor.index == visible_rows[row].0
                }) {
                    if !editor.select_all {
                        let prefix = editor.text.get(..editor.caret).unwrap_or_default();
                        let caret_x = row_value_x
                            + LIST_ROW_TEXT_INSET
                            + super::text_width(&self.regular_font, prefix, LIST_ROW_FONT_SIZE);
                        self.push_glyph_instances_at_size(
                            &mut glyphs,
                            "|",
                            true,
                            id,
                            caret_x,
                            super::canvas::centered_baseline(
                                &self.regular_font,
                                LIST_ROW_FONT_SIZE,
                                row_center_y,
                            ),
                            4.0,
                            LIST_ROW_FONT_SIZE,
                            [82, 93, 117, 255],
                        );
                    }
                    self.push_glyph_instances_at_size(
                        &mut glyphs,
                        "x",
                        true,
                        id,
                        width - 19.0,
                        baseline,
                        12.0,
                        LIST_ROW_FONT_SIZE,
                        [82, 93, 117, 255],
                    );
                }
                // clip the partial row at the footer after keeping its text in the visible slice.
                for glyph in &mut glyphs[row_glyph_start..] {
                    glyph.clip_y = [y, footer_top];
                }
            }
            if item_count == 0 {
                let empty = "(empty)";
                self.push_glyph_instances_at_size(
                    &mut glyphs,
                    empty,
                    false,
                    id,
                    centered_glyph_start(&self.regular_font, empty, LIST_ROW_FONT_SIZE, width),
                    super::canvas::centered_baseline(
                        &self.regular_font,
                        LIST_ROW_FONT_SIZE,
                        LIST_CONTENT_TOP + LIST_EMPTY_TOP_PADDING + LIST_ROW_FONT_SIZE * 0.5,
                    ),
                    width,
                    LIST_ROW_FONT_SIZE,
                    [82, 93, 117, 255],
                );
            }
            let footer_y = super::canvas::centered_baseline(
                &self.bold_font,
                LIST_LENGTH_FONT_SIZE,
                height - 11.0,
            );
            self.push_glyph_instances_at_size(
                &mut glyphs,
                "+",
                true,
                id,
                LIST_FOOTER_SIDE_PADDING,
                footer_y,
                plus_width,
                LIST_LENGTH_FONT_SIZE,
                [82, 93, 117, 255],
            );
            let length_width =
                super::text_width(&self.bold_font, &shown_length, LIST_LENGTH_FONT_SIZE);
            let equals_x = width - LIST_FOOTER_SIDE_PADDING - equals_width;
            let length_x = (LIST_FOOTER_SIDE_PADDING + plus_width + equals_x - length_width) * 0.5;
            self.push_glyph_instances_at_size(
                &mut glyphs,
                &shown_length,
                true,
                id,
                length_x,
                footer_y,
                footer_text_width,
                LIST_LENGTH_FONT_SIZE,
                [82, 93, 117, 255],
            );
            self.push_glyph_instances_at_size(
                &mut glyphs,
                "=",
                true,
                id,
                equals_x,
                footer_y,
                equals_width,
                LIST_LENGTH_FONT_SIZE,
                [82, 93, 117, 255],
            );
        }
        Ok((shapes, glyphs))
    }

    pub(super) fn visible_list_rows(&self, index: usize) -> Vec<(usize, String)> {
        let Some(monitor) = self.monitors.get(index) else {
            return Vec::new();
        };
        if monitor.kind != super::MonitorKind::List {
            return Vec::new();
        }
        let display_len = self.list_display_len(index);
        let start = monitor
            .list_scroll_offset
            .min(monitor.max_list_scroll_offset(display_len));
        let end = display_len.min(start + monitor.list_row_capacity());
        (start..end)
            .map(|list_index| {
                let value = self
                    .list_editor
                    .as_ref()
                    .filter(|editor| {
                        editor.monitor_index == index && editor.index == list_index + 1
                    })
                    .map(|editor| editor.text.clone())
                    .or_else(|| {
                        monitor
                            .list_items
                            .get(list_index)
                            .map(crate::runtime::scratch_string)
                    })
                    .unwrap_or_default();
                (list_index + 1, value)
            })
            .collect()
    }

    pub(crate) fn build_gpu_monitor_instances_for_id(
        &mut self,
        gpu_id: u32,
        build_shapes: bool,
        build_text: bool,
    ) -> Result<(Vec<MonitorInstance>, Vec<GlyphInstance>), String> {
        let Some(index) = self
            .monitors
            .iter()
            .position(|monitor| monitor.gpu_id == gpu_id)
        else {
            return Ok((Vec::new(), Vec::new()));
        };
        self.build_gpu_monitor_instances(index, build_shapes, build_text)
    }

    fn push_glyph_instances(
        &self,
        out: &mut Vec<GlyphInstance>,
        text: &str,
        bold: bool,
        monitor_id: u32,
        start_x: f32,
        baseline: f32,
        _max_width: f32,
        color: [u8; 4],
    ) {
        let size = if bold { 18.0 } else { 19.0 };
        self.push_glyph_instances_at_size(
            out, text, bold, monitor_id, start_x, baseline, _max_width, size, color,
        );
    }

    fn push_glyph_instances_at_size(
        &self,
        out: &mut Vec<GlyphInstance>,
        text: &str,
        bold: bool,
        monitor_id: u32,
        start_x: f32,
        baseline: f32,
        _max_width: f32,
        size: f32,
        color: [u8; 4],
    ) {
        let font = if bold {
            &self.bold_font
        } else {
            &self.regular_font
        };
        let atlas_font_size = if bold { 18.0 } else { 19.0 };
        let scaled = font.as_scaled(size);
        let mut pen_x = 0.0;
        let mut previous = None;
        for ch in text.chars() {
            let gid = scaled.glyph_id(ch);
            if let Some(prev) = previous {
                pen_x += scaled.kern(prev, gid);
            }
            if let Some(e) =
                self.glyph_atlas_entries
                    .get(&(ch, bold, self.glyph_atlas_scale_bucket))
            {
                if e.width > 0 && e.height > 0 {
                    out.push(GlyphInstance {
                        local_position: [start_x + pen_x, baseline],
                        glyph_id: e.id,
                        monitor_id,
                        color: linear_rgba(color),
                        scale: size / atlas_font_size,
                        clip_y: [0.0; 2],
                    });
                }
            }
            pen_x += scaled.h_advance(gid);
            previous = Some(gid);
        }
    }

    fn ensure_glyph_atlas(&mut self, ch: char, bold: bool) -> Result<(), String> {
        let bucket = self.glyph_atlas_scale_bucket;
        if self.glyph_atlas_entries.contains_key(&(ch, bold, bucket)) {
            return Ok(());
        }
        let font = if bold {
            self.bold_font.clone()
        } else {
            self.regular_font.clone()
        };
        let logical_size = if bold { 18.0 } else { 19.0 };
        let outline =
            font.outline_glyph(font.glyph_id(ch).with_scale(logical_size * bucket as f32));
        let bounds = outline.as_ref().map(|glyph| glyph.px_bounds());
        let (width, height) = bounds.map_or((0, 0), |bounds| {
            (
                (bounds.max.x - bounds.min.x).ceil().max(0.0) as u32,
                (bounds.max.y - bounds.min.y).ceil().max(0.0) as u32,
            )
        });
        if self.glyph_atlas_count >= super::GLYPH_ATLAS_GLYPH_CAPACITY {
            return Err("variable glyph atlas is full".to_owned());
        }
        let slot = self.glyph_atlas_count;
        let (cell_x, cell_y) = if width == 0 || height == 0 {
            (0, 0)
        } else {
            if width + 2 > GLYPH_ATLAS_MAX_SIZE || height + 2 > GLYPH_ATLAS_MAX_SIZE {
                return Err(format!(
                    "glyph {ch:?} at scale {bucket} exceeds the atlas dimensions"
                ));
            }
            loop {
                let mut x = self.glyph_atlas_next_x;
                let mut y = self.glyph_atlas_next_y;
                let mut row_height = self.glyph_atlas_row_height;
                if x + width + 2 > self.glyph_atlas_size {
                    x = 0;
                    y += row_height;
                    row_height = 0;
                }
                if y + height + 2 > self.glyph_atlas_size {
                    self.grow_glyph_atlas()?;
                    continue;
                }
                self.glyph_atlas_next_x = x + width + 2;
                self.glyph_atlas_next_y = y;
                self.glyph_atlas_row_height = row_height.max(height + 2);
                break (x + 1, y + 1);
            }
        };
        if let Some(outline) = outline {
            let atlas = &mut self.glyph_atlas;
            let atlas_size = self.glyph_atlas_size;
            outline.draw(|x, y, coverage| {
                if x < width && y < height {
                    atlas[((cell_y + y) * atlas_size + cell_x + x) as usize] =
                        (coverage * 255.0).round() as u8;
                }
            });
        }
        if width > 0 && height > 0 {
            self.glyph_atlas_dirty_regions
                .push((cell_x, cell_y, width, height));
        }
        self.glyph_atlas_count += 1;
        self.glyph_atlas_entries.insert(
            (ch, bold, bucket),
            GlyphAtlasEntry {
                id: slot,
                x: cell_x,
                y: cell_y,
                width,
                height,
                // `px_bounds` and `draw()` use the same top-left pixel
                // convention. scale their coordinates back to stage units so
                // the atlas crop stays correctly oriented and bearing
                // quantization is limited to 1/bucket of a stage pixel.
                bounds: bounds.map_or([0.0; 4], |bounds| scaled_px_bounds(bounds, bucket)),
            },
        );
        self.glyph_atlas_revision = self.glyph_atlas_revision.wrapping_add(1);
        Ok(())
    }

    fn grow_glyph_atlas(&mut self) -> Result<(), String> {
        if self.glyph_atlas_size >= GLYPH_ATLAS_MAX_SIZE {
            return Err("variable glyph atlas is full".to_owned());
        }
        let old_size = self.glyph_atlas_size;
        let new_size = (old_size * 2).min(GLYPH_ATLAS_MAX_SIZE);
        let mut pixels = vec![0; (new_size * new_size) as usize];
        for row in 0..old_size as usize {
            let old_start = row * old_size as usize;
            let new_start = row * new_size as usize;
            pixels[new_start..new_start + old_size as usize]
                .copy_from_slice(&self.glyph_atlas[old_start..old_start + old_size as usize]);
        }
        self.glyph_atlas = pixels;
        self.glyph_atlas_size = new_size;
        self.glyph_atlas_full_dirty = true;
        self.glyph_atlas_revision = self.glyph_atlas_revision.wrapping_add(1);
        Ok(())
    }

    pub(crate) fn upload_to(
        &mut self,
        queue: &Queue,
        target: &RenderTexture,
    ) -> Result<(), String> {
        target.clear(queue);
        self.upload_context_menu_to(queue, target)
    }

    // rerasterize one monitor region for smooth hover and slider updates
    pub(crate) fn upload_context_menu_to(
        &mut self,
        queue: &Queue,
        target: &RenderTexture,
    ) -> Result<(), String> {
        let (left_stage, top_stage, width_stage, height_stage) =
            if let Some(menu) = self.context_menu {
                (
                    menu.left,
                    menu.top,
                    CONTEXT_MENU_WIDTH,
                    CONTEXT_MENU_ROW_HEIGHT * menu.options.len() as f32,
                )
            } else if let Some(editor) = &self.slider_range_editor {
                (
                    editor.left,
                    editor.top,
                    SLIDER_RANGE_EDITOR_WIDTH,
                    SLIDER_RANGE_EDITOR_HEIGHT,
                )
            } else {
                return Ok(());
            };
        let scale_x = target.width as f32 / SCRATCH_WIDTH as f32;
        let scale_y = target.height as f32 / SCRATCH_HEIGHT as f32;
        let left = (left_stage * scale_x).floor().max(0.0) as u32;
        let top = (top_stage * scale_y).floor().max(0.0) as u32;
        let right = ((left_stage + width_stage) * scale_x)
            .ceil()
            .min(target.width as f32) as u32;
        let bottom = ((top_stage + height_stage) * scale_y)
            .ceil()
            .min(target.height as f32) as u32;
        let width = right.saturating_sub(left);
        let height = bottom.saturating_sub(top);
        if width == 0 || height == 0 {
            return Ok(());
        }
        let displayed_text = if let Some(menu) = self.context_menu {
            menu.options
                .iter()
                .map(|option| match option {
                    MonitorContextAction::NormalReadout => "normal readout",
                    MonitorContextAction::LargeReadout => "large readout",
                    MonitorContextAction::Slider => "slider",
                    MonitorContextAction::ChangeSliderRange => "change slider range",
                    MonitorContextAction::ImportList => "import",
                    MonitorContextAction::ExportList => "export",
                    MonitorContextAction::Hide => "hide",
                    MonitorContextAction::Dismiss => "",
                })
                .collect::<String>()
        } else {
            let editor = self.slider_range_editor.as_ref().unwrap();
            format!(
                "min: {} max: {} tab switch enter apply esc cancel",
                editor.min_text, editor.max_text
            )
        };
        for character in displayed_text.chars() {
            self.ensure_glyph_mesh(character, true)?;
        }
        let mut pixmap = tiny_skia::Pixmap::new(width, height)
            .ok_or_else(|| "failed to allocate the variable context menu surface".to_owned())?;
        if let Some(menu) = self.context_menu {
            super::canvas::draw_context_menu(
                &mut pixmap,
                &menu,
                &self.glyph_meshes,
                &self.bold_font,
                scale_x,
                scale_y,
                -(left as f32),
                -(top as f32),
            )?;
        } else if let Some(editor) = &self.slider_range_editor {
            super::canvas::draw_slider_range_editor(
                &mut pixmap,
                editor,
                &self.glyph_meshes,
                &self.bold_font,
                scale_x,
                scale_y,
                -(left as f32),
                -(top as f32),
            )?;
        }
        target.write_rgba_region(queue, left, top, width, height, pixmap.data());
        Ok(())
    }

    pub(super) fn rasterize_shadow_tile(
        &self,
        width: u32,
        height: u32,
        scale_x: f32,
        scale_y: f32,
        index: usize,
    ) -> Result<tiny_skia::Pixmap, String> {
        let mut pixmap = tiny_skia::Pixmap::new(width, height)
            .ok_or_else(|| "failed to allocate the variable shadow surface".to_owned())?;
        let monitor = &self.monitors[index];
        draw_monitor_shadow(&mut pixmap, monitor, 14.0, 14.0, scale_x, scale_y, 0.0, 0.0)?;
        let blur_radius = (SHADOW_BLUR_RADIUS * monitor.scale * (scale_x + scale_y) * 0.5)
            .ceil()
            .max(1.0) as usize;
        blur_shadow_alpha(&mut pixmap, blur_radius);
        Ok(pixmap)
    }

    pub(super) fn ensure_glyph_mesh(&mut self, character: char, bold: bool) -> Result<(), String> {
        if self.glyph_meshes.contains_key(&(character, bold)) {
            return Ok(());
        }
        let font = if bold {
            &self.bold_font
        } else {
            &self.regular_font
        };
        let font_size = if bold { 18.0 } else { 19.0 };
        let font_scale = font.as_scaled(font_size).scale_factor();
        let glyph = font.glyph_id(character);
        let mesh = font
            .outline(glyph)
            .map(|outline| {
                tessellate_glyph_outline(
                    &outline.curves,
                    font_scale.horizontal,
                    font_scale.vertical,
                )
            })
            .transpose()?
            .unwrap_or_default();
        self.glyph_meshes.insert((character, bold), mesh);
        Ok(())
    }

    pub(crate) fn gpu_readout_vertices(
        &mut self,
        text: &str,
    ) -> Result<Vec<VariableVertex>, String> {
        let chars = text.chars().collect::<Vec<_>>();
        for character in &chars {
            self.ensure_glyph_mesh(*character, false)?;
        }
        let width = super::text_width(&self.regular_font, text, 19.0) * 0.7 + 16.0;
        let mut vertices = Vec::new();
        push_rounded_rect(
            &mut vertices,
            0.0,
            0.0,
            1.0,
            8.0,
            8.0,
            width,
            22.0,
            4.0,
            [26, 30, 38, 220],
        );
        push_text_mesh(
            &mut vertices,
            &self.glyph_meshes,
            &self.regular_font,
            &chars,
            false,
            19.0,
            0.0,
            0.0,
            0.7,
            16.0,
            25.0,
            width - 16.0,
            [245, 248, 255, 255],
        );
        Ok(vertices)
    }
}

fn scaled_px_bounds(bounds: ab_glyph::Rect, scale_bucket: u32) -> [f32; 4] {
    let scale = scale_bucket as f32;
    [
        bounds.min.x / scale,
        bounds.min.y / scale,
        bounds.width() / scale,
        bounds.height() / scale,
    ]
}

fn monitor_color(category: super::MonitorCategory) -> [u8; 4] {
    match category {
        super::MonitorCategory::Variables => [255, 140, 26, 255],
        super::MonitorCategory::Lists => [255, 102, 26, 255],
        super::MonitorCategory::Motion => [76, 151, 255, 255],
        super::MonitorCategory::Looks => [153, 102, 255, 255],
        super::MonitorCategory::Sound => [207, 99, 207, 255],
        super::MonitorCategory::Events => [255, 191, 0, 255],
        super::MonitorCategory::Control => [255, 171, 25, 255],
        super::MonitorCategory::Sensing => [92, 177, 214, 255],
        super::MonitorCategory::Operators => [89, 192, 89, 255],
        super::MonitorCategory::MyBlocks => [255, 102, 128, 255],
        super::MonitorCategory::Extension(color) => [color[0], color[1], color[2], 255],
    }
}

pub(super) fn fit_monitor_text(
    font: &ab_glyph::FontArc,
    text: &str,
    max_width: f32,
    size: f32,
) -> String {
    let max_width = max_width.max(1.0);
    let mut fitted = String::new();
    for ch in text.chars() {
        let mut candidate = fitted.clone();
        candidate.push(ch);
        if super::text_width(font, &candidate, size) > max_width {
            while !fitted.is_empty() {
                let mut with_ellipsis = fitted.clone();
                with_ellipsis.push('…');
                if super::text_width(font, &with_ellipsis, size) <= max_width {
                    fitted.push('…');
                    break;
                }
                fitted.pop();
            }
            break;
        }
        fitted.push(ch);
    }
    fitted
}

fn centered_glyph_start(
    font: &ab_glyph::FontArc,
    text: &str,
    size: f32,
    container_width: f32,
) -> f32 {
    let Some((left, right)) = glyph_text_x_bounds(font, text, size) else {
        return (container_width - super::text_width(font, text, size)) * 0.5;
    };
    (container_width - (right - left)) * 0.5 - left
}

fn glyph_text_x_bounds(font: &ab_glyph::FontArc, text: &str, size: f32) -> Option<(f32, f32)> {
    let scaled = font.as_scaled(size);
    let mut pen_x = 0.0;
    let mut previous = None;
    let mut left = f32::INFINITY;
    let mut right = f32::NEG_INFINITY;
    for ch in text.chars() {
        let glyph_id = scaled.glyph_id(ch);
        if let Some(previous) = previous {
            pen_x += scaled.kern(previous, glyph_id);
        }
        let mut glyph = font.glyph_id(ch).with_scale(size);
        glyph.position = ab_glyph::point(pen_x, 0.0);
        if let Some(outline) = font.outline_glyph(glyph) {
            let bounds = outline.px_bounds();
            left = left.min(bounds.min.x);
            right = right.max(bounds.max.x);
        }
        pen_x += scaled.h_advance(glyph_id);
        previous = Some(glyph_id);
    }
    (left.is_finite() && right.is_finite()).then_some((left, right))
}

#[cfg(test)]
mod monitor_color_tests {
    use super::monitor_color;
    use crate::graphics::variables::MonitorCategory;

    #[test]
    fn default_monitor_category_colors_match_scratch_palette() {
        let categories = [
            (MonitorCategory::Motion, [76, 151, 255, 255]),
            (MonitorCategory::Looks, [153, 102, 255, 255]),
            (MonitorCategory::Sound, [207, 99, 207, 255]),
            (MonitorCategory::Events, [255, 191, 0, 255]),
            (MonitorCategory::Control, [255, 171, 25, 255]),
            (MonitorCategory::Sensing, [92, 177, 214, 255]),
            (MonitorCategory::Operators, [89, 192, 89, 255]),
            (MonitorCategory::Variables, [255, 140, 26, 255]),
            (MonitorCategory::Lists, [255, 102, 26, 255]),
            (MonitorCategory::MyBlocks, [255, 102, 128, 255]),
            (
                MonitorCategory::Extension([15, 189, 140]),
                [15, 189, 140, 255],
            ),
        ];
        for (category, expected) in categories {
            assert_eq!(monitor_color(category), expected);
        }
    }
}

fn push_monitor_instance(
    out: &mut Vec<MonitorInstance>,
    monitor_id: u32,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    radius: f32,
    border_width: f32,
    fill: [u8; 4],
    border: [u8; 4],
) {
    if width <= 0.0 || height <= 0.0 {
        return;
    }
    out.push(MonitorInstance {
        rect: [x, y, width, height],
        fill: linear_rgba(fill),
        border: linear_rgba(border),
        params: [
            radius,
            border_width,
            SCRATCH_WIDTH as f32,
            SCRATCH_HEIGHT as f32,
        ],
        shape: [0.0; 4],
        monitor_id,
        _padding: [0; 3],
    });
}

fn linear_rgba(color: [u8; 4]) -> [f32; 4] {
    let linear = |channel: u8| {
        let srgb = channel as f32 / 255.0;
        if srgb <= 0.04045 {
            srgb / 12.92
        } else {
            ((srgb + 0.055) / 1.055).powf(2.4)
        }
    };
    [
        linear(color[0]),
        linear(color[1]),
        linear(color[2]),
        color[3] as f32 / 255.0,
    ]
}

fn premultiplied_linear_rgba(color: [u8; 4]) -> [f32; 4] {
    let mut rgba = linear_rgba(color);
    let alpha = rgba[3];
    for channel in &mut rgba[..3] {
        *channel *= alpha;
    }
    rgba
}

fn wrap_bubble_text(font: &ab_glyph::FontArc, text: &str, max_width: f32) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let mut current = String::new();
        for word in paragraph.split_whitespace() {
            let candidate = if current.is_empty() {
                word.to_owned()
            } else {
                format!("{current} {word}")
            };
            if super::text_width(font, &candidate, BUBBLE_FONT_SIZE) <= max_width {
                current = candidate;
                continue;
            }
            if !current.is_empty() {
                lines.push(std::mem::take(&mut current));
            }
            if super::text_width(font, word, BUBBLE_FONT_SIZE) <= max_width {
                current.push_str(word);
                continue;
            }
            for ch in word.chars() {
                let mut candidate = current.clone();
                candidate.push(ch);
                if super::text_width(font, &candidate, BUBBLE_FONT_SIZE) > max_width
                    && !current.is_empty()
                {
                    lines.push(std::mem::take(&mut current));
                }
                current.push(ch);
            }
        }
        if !current.is_empty() || paragraph.is_empty() {
            lines.push(current);
        }
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn push_bubble_shape(
    shapes: &mut Vec<MonitorInstance>,
    rect: [f32; 4],
    fill: [f32; 4],
    border: [f32; 4],
    radius: f32,
    bubble_id: u32,
) {
    shapes.push(MonitorInstance {
        rect,
        fill,
        border,
        params: [
            radius,
            BUBBLE_BORDER_WIDTH,
            SCRATCH_WIDTH as f32,
            SCRATCH_HEIGHT as f32,
        ],
        shape: [0.0; 4],
        monitor_id: bubble_id,
        _padding: [0; 3],
    });
}

fn push_vertex(output: &mut Vec<VariableVertex>, stage_x: f32, stage_y: f32, color: [u8; 4]) {
    output.push(VariableVertex {
        position: [
            2.0 * stage_x / SCRATCH_WIDTH as f32 - 1.0,
            1.0 - 2.0 * stage_y / SCRATCH_HEIGHT as f32,
        ],
        color: color.map(|channel| channel as f32 / 255.0),
    });
}

fn push_rounded_rect(
    output: &mut Vec<VariableVertex>,
    base_x: f32,
    base_y: f32,
    scale: f32,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    radius: f32,
    color: [u8; 4],
) {
    if width <= 0.0 || height <= 0.0 {
        return;
    }
    let radius = radius.max(0.0).min(width * 0.5).min(height * 0.5);
    let centers = [
        (width - radius, radius, -90.0_f32),
        (width - radius, height - radius, 0.0),
        (radius, height - radius, 90.0),
        (radius, radius, 180.0),
    ];
    let mut points = Vec::with_capacity(20);
    for (cx, cy, start) in centers {
        for step in 0..=4 {
            let angle = (start + step as f32 * 90.0 / 4.0).to_radians();
            points.push((cx + radius * angle.cos(), cy + radius * angle.sin()));
        }
    }
    let center_x = base_x + (x + width * 0.5) * scale;
    let center_y = base_y + (y + height * 0.5) * scale;
    for index in 0..points.len() {
        let a = points[index];
        let b = points[(index + 1) % points.len()];
        push_vertex(output, center_x, center_y, color);
        push_vertex(
            output,
            base_x + (x + a.0) * scale,
            base_y + (y + a.1) * scale,
            color,
        );
        push_vertex(
            output,
            base_x + (x + b.0) * scale,
            base_y + (y + b.1) * scale,
            color,
        );
    }
}

fn push_text_mesh(
    output: &mut Vec<VariableVertex>,
    meshes: &std::collections::HashMap<(char, bool), Vec<super::Triangle>>,
    font: &ab_glyph::FontArc,
    chars: &[char],
    bold: bool,
    size: f32,
    base_x: f32,
    base_y: f32,
    monitor_scale: f32,
    start_x: f32,
    baseline_y: f32,
    _text_width: f32,
    color: [u8; 4],
) {
    let scaled = font.as_scaled(size);
    let mut pen_x = 0.0;
    let mut previous = None;
    for character in chars {
        let glyph = scaled.glyph_id(*character);
        if let Some(previous) = previous {
            pen_x += scaled.kern(previous, glyph);
        }
        if let Some(mesh) = meshes.get(&(*character, bold)) {
            for triangle in mesh {
                for point in triangle {
                    push_vertex(
                        output,
                        base_x + (start_x + pen_x + point.x) * monitor_scale,
                        base_y + (baseline_y - point.y) * monitor_scale,
                        color,
                    );
                }
            }
        }
        pen_x += scaled.h_advance(glyph);
        previous = Some(glyph);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BUBBLE_BODY_CORNER_RADIUS, BUBBLE_BORDER, BUBBLE_BORDER_WIDTH, BUBBLE_FONT_SIZE,
        BUBBLE_SPEECH_TAIL_EXTENT, BUBBLE_TAIL_CORNER_CLEARANCE, BubbleRenderInput,
        LIST_CONTENT_TOP, LIST_EMPTY_TOP_PADDING, LIST_FOOTER_SIDE_PADDING, LIST_HEADER_FONT_SIZE,
        LIST_HEADER_HORIZONTAL_PADDING, LIST_LENGTH_FONT_SIZE, LIST_ROW_FONT_SIZE, LIST_ROW_RADIUS,
        MONITOR_FRAME_RADIUS, MONITOR_SHAPE_BOTTOM_ROUNDED, MONITOR_SHAPE_TOP_ROUNDED,
        MONITOR_VALUE_RADIUS, SCRATCH_WIDTH, VariableLayer, linear_rgba, scaled_px_bounds,
    };
    use crate::graphics::variables::{LIST_ROW_VALUE_X, MonitorKind, VariableMonitor};
    use crate::runtime::{BubbleKind, BubbleSnapshot};

    #[test]
    fn list_monitor_frame_matches_scalar_border_and_keeps_rounded_value_rows() {
        let scalar = VariableMonitor::new("score", "5", 0.0, 0.0);
        let list = VariableMonitor::new("items", "", 0.0, 0.0)
            .with_kind(MonitorKind::List)
            .with_list_items(vec!["one".to_owned()], 120.0, 100.0);
        let mut layer = VariableLayer::new(vec![scalar, list]).unwrap();
        let scalar_frame = layer
            .build_gpu_monitor_instances_for_id(0, true, false)
            .unwrap()
            .0[0];
        let scalar_value = layer
            .build_gpu_monitor_instances_for_id(0, true, false)
            .unwrap()
            .0[1];
        let list_frame = layer
            .build_gpu_monitor_instances_for_id(1, true, false)
            .unwrap()
            .0[0];

        assert_eq!(list_frame.params[0], scalar_frame.params[0]);
        assert_eq!(list_frame.params[1], scalar_frame.params[1]);
        assert_eq!(list_frame.fill, scalar_frame.fill);
        assert_eq!(scalar_frame.fill, linear_rgba([229, 240, 255, 255]));
        assert_eq!(list_frame.border, scalar_frame.border);
        assert_eq!(scalar_value.params[1], 1.0);
        assert_eq!(scalar_value.border, linear_rgba([0, 0, 0, 38]));
        assert_eq!(scalar_value.params[0], MONITOR_VALUE_RADIUS);
        let list_shapes = layer
            .build_gpu_monitor_instances_for_id(1, true, false)
            .unwrap()
            .0;
        assert_eq!(list_shapes[1].params[0], MONITOR_FRAME_RADIUS - 1.0);
        assert_eq!(list_shapes[1].shape[2], MONITOR_SHAPE_TOP_ROUNDED);
        assert_eq!(list_shapes.last().unwrap().params[0], MONITOR_FRAME_RADIUS - 1.0);
        assert_eq!(list_shapes.last().unwrap().shape[2], MONITOR_SHAPE_BOTTOM_ROUNDED);
        assert_eq!(list_shapes[2].params[0], LIST_ROW_RADIUS);
        assert_eq!(list_shapes[2].rect[0], LIST_ROW_VALUE_X);
        assert_eq!(list_shapes[2].rect[2], 120.0 - LIST_ROW_VALUE_X - 6.0);

        let header = super::fit_monitor_text(
            &layer.bold_font,
            "a very long list name that must be clipped",
            64.0 - LIST_HEADER_HORIZONTAL_PADDING * 2.0,
            LIST_HEADER_FONT_SIZE,
        );
        let plus_width =
            crate::graphics::variables::text_width(&layer.bold_font, "+", LIST_LENGTH_FONT_SIZE);
        let equals_width =
            crate::graphics::variables::text_width(&layer.bold_font, "=", LIST_LENGTH_FONT_SIZE);
        let footer_text_width = 64.0 - LIST_FOOTER_SIDE_PADDING * 2.0 - plus_width - equals_width;
        let length = super::fit_monitor_text(
            &layer.bold_font,
            "length 123456789",
            footer_text_width,
            LIST_LENGTH_FONT_SIZE,
        );
        assert!(
            crate::graphics::variables::text_width(
                &layer.bold_font,
                &header,
                LIST_HEADER_FONT_SIZE
            ) <= 40.0
        );
        assert!(
            crate::graphics::variables::text_width(
                &layer.bold_font,
                &length,
                LIST_LENGTH_FONT_SIZE
            ) <= footer_text_width
        );
        let title_ink =
            super::glyph_text_x_bounds(&layer.bold_font, "items", LIST_HEADER_FONT_SIZE).unwrap();
        let title_start =
            super::centered_glyph_start(&layer.bold_font, "items", LIST_HEADER_FONT_SIZE, 120.0);
        assert!(((title_start + (title_ink.0 + title_ink.1) * 0.5) - 60.0).abs() < 0.01);
        let (_, list_glyphs) = layer
            .build_gpu_monitor_instances_for_id(1, false, true)
            .unwrap();
        assert!((list_glyphs[0].scale - 14.0 / 18.0).abs() < f32::EPSILON);
        assert!((list_glyphs[5].scale - LIST_ROW_FONT_SIZE / 18.0).abs() < f32::EPSILON);
        assert!((list_glyphs[6].scale - LIST_ROW_FONT_SIZE / 19.0).abs() < f32::EPSILON);
        assert!((list_glyphs.last().unwrap().scale - 2.0 / 3.0).abs() < f32::EPSILON);

        let empty_list = VariableMonitor::new("items", "", 0.0, 0.0)
            .with_kind(MonitorKind::List)
            .with_list_items(Vec::new(), 120.0, 100.0);
        let mut empty_layer = VariableLayer::new(vec![empty_list]).unwrap();
        let (_, empty_glyphs) = empty_layer
            .build_gpu_monitor_instances_for_id(0, false, true)
            .unwrap();
        let empty_text_start = super::centered_glyph_start(
            &empty_layer.regular_font,
            "(empty)",
            LIST_ROW_FONT_SIZE,
            120.0,
        );
        let empty_text_center =
            LIST_CONTENT_TOP + LIST_EMPTY_TOP_PADDING + LIST_ROW_FONT_SIZE * 0.5;
        let empty_text_baseline = crate::graphics::variables::canvas::centered_baseline(
            &empty_layer.regular_font,
            LIST_ROW_FONT_SIZE,
            empty_text_center,
        );
        assert!((empty_glyphs[5].local_position[0] - empty_text_start).abs() < f32::EPSILON);
        assert!((empty_glyphs[5].local_position[1] - empty_text_baseline).abs() < f32::EPSILON);
    }

    #[test]
    fn full_stage_list_monitor_builds_unpadded_gpu_instances_for_all_visible_rows() {
        let long_item = "1234567890".repeat(10);
        let items = vec![long_item; 200];
        let monitor = VariableMonitor::new("large list", "", 0.0, 0.0)
            .with_kind(MonitorKind::List)
            .with_list_items(
                items,
                SCRATCH_WIDTH as f32 / 0.65,
                super::SCRATCH_HEIGHT as f32 / 0.65,
            );
        let mut layer = VariableLayer::new(vec![monitor]).unwrap();
        let (shapes, glyphs) = layer
            .build_gpu_monitor_instances_for_id(0, true, true)
            .unwrap();

        assert!(shapes.len() > 16);
        assert!(glyphs.len() > 256);
        assert!(glyphs.len() < 4096);
    }

    #[test]
    fn scaled_pixel_bounds_keep_top_left_glyph_orientation() {
        let bounds = scaled_px_bounds(
            ab_glyph::Rect {
                min: ab_glyph::point(1.0, -27.0),
                max: ab_glyph::point(21.0, 3.0),
            },
            4,
        );
        assert_eq!(bounds, [0.25, -6.75, 5.0, 7.5]);
    }

    #[test]
    fn monitor_color_bytes_are_converted_from_srgb_before_surface_output() {
        let color = linear_rgba([128, 64, 255, 128]);
        assert!((color[0] - 0.21586).abs() < 0.0001);
        assert!((color[1] - 0.05127).abs() < 0.0001);
        assert_eq!(color[2], 1.0);
        assert!((color[3] - 128.0 / 255.0).abs() < f32::EPSILON);
    }

    #[test]
    fn bubble_layout_wraps_clamps_and_keeps_each_bubble_instanced() {
        let mut layer = VariableLayer::new(Vec::new()).unwrap();
        let long_text = "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.";
        let batch = layer
            .build_gpu_bubble_instances(&[
                BubbleRenderInput {
                    bubble: BubbleSnapshot {
                        kind: BubbleKind::Say,
                        text: long_text.to_owned(),
                    },
                    x: -100.0,
                    y: 170.0,
                    sprite_width: 80.0,
                    sprite_height: 100.0,
                },
                BubbleRenderInput {
                    bubble: BubbleSnapshot {
                        kind: BubbleKind::Think,
                        text: "Hmm...".to_owned(),
                    },
                    x: 40.0,
                    y: 0.0,
                    sprite_width: 80.0,
                    sprite_height: 100.0,
                },
            ])
            .unwrap();

        assert_eq!(batch.transforms.len(), 2);
        assert!(batch.transforms[0][1] >= 8.0);
        let say_body = batch.shapes[0];
        assert_eq!(say_body.params[0], 17.0);
        assert!((say_body.params[1] - BUBBLE_BORDER_WIDTH).abs() < f32::EPSILON);
        assert_eq!(say_body.border, linear_rgba(BUBBLE_BORDER));
        assert_eq!(say_body.shape[2], 1.0);
        assert_eq!(
            say_body.rect[3],
            say_body.shape[1] + BUBBLE_SPEECH_TAIL_EXTENT
        );
        let say_min_anchor = BUBBLE_BODY_CORNER_RADIUS + BUBBLE_TAIL_CORNER_CLEARANCE;
        let expected_say_anchor = (SCRATCH_WIDTH as f32 * 0.5 - 100.0 - batch.transforms[0][0])
            .clamp(say_min_anchor, say_body.shape[0] - say_min_anchor);
        assert!((say_body.shape[3] - expected_say_anchor).abs() < 0.01);
        assert_eq!(batch.shapes.len(), 5);
        assert!(batch.shapes[0].rect[3] > 100.0);
        assert!(say_body.shape[1] <= 344.0);
        assert!(
            batch.shapes[1..4]
                .iter()
                .all(|circle| circle.rect[0].abs() < 100.0 && circle.rect[1].abs() < 400.0)
        );
        assert_eq!(batch.shapes[4].monitor_id, 1);
        assert_eq!(batch.shapes[4].rect[0..2], [0.0, 0.0]);
        let thought_body = batch.shapes[4].rect;
        let largest_circle = batch.shapes[1].rect;
        let thought_min_anchor = BUBBLE_BODY_CORNER_RADIUS + 4.6;
        let expected_thought_anchor = (SCRATCH_WIDTH as f32 * 0.5 + 40.0 - batch.transforms[1][0])
            .clamp(thought_min_anchor, thought_body[2] - thought_min_anchor);
        assert!(
            (largest_circle[0] + largest_circle[2] * 0.5 - expected_thought_anchor).abs() < 0.01
        );
        assert!((largest_circle[1] + largest_circle[3] * 0.5 - thought_body[3]).abs() < 0.01);
        let middle_circle = batch.shapes[2].rect;
        let smallest_circle = batch.shapes[3].rect;
        assert!(middle_circle[0] < largest_circle[0]);
        assert!(smallest_circle[0] < middle_circle[0]);
        assert!(middle_circle[1] > largest_circle[1]);
        assert!(smallest_circle[1] > middle_circle[1]);
        let large_center_x = largest_circle[0] + largest_circle[2] * 0.5;
        let middle_center_x = middle_circle[0] + middle_circle[2] * 0.5;
        let small_center_x = smallest_circle[0] + smallest_circle[2] * 0.5;
        let circle_distance = |a: [f32; 4], b: [f32; 4]| {
            let dx = (a[0] + a[2] * 0.5) - (b[0] + b[2] * 0.5);
            let dy = (a[1] + a[3] * 0.5) - (b[1] + b[3] * 0.5);
            (dx * dx + dy * dy).sqrt()
        };
        let large_middle_gap = circle_distance(largest_circle, middle_circle)
            - (largest_circle[2] + middle_circle[2]) * 0.5;
        let middle_small_gap = circle_distance(middle_circle, smallest_circle)
            - (middle_circle[2] + smallest_circle[2]) * 0.5;
        assert!(large_middle_gap >= 0.0);
        assert!(middle_small_gap >= 0.0);
        assert!((large_middle_gap - middle_small_gap).abs() < 1.0);
        assert!(middle_center_x - small_center_x < large_center_x - middle_center_x);
        assert!(batch.glyphs.iter().any(|glyph| glyph.monitor_id == 0));
        assert!(batch.glyphs.iter().any(|glyph| glyph.monitor_id == 1));
        assert!((batch.glyphs[0].scale - BUBBLE_FONT_SIZE / 19.0).abs() < f32::EPSILON);
    }

    #[test]
    fn speech_tail_mirrors_toward_the_owning_sprite() {
        let mut layer = VariableLayer::new(Vec::new()).unwrap();
        let batch = layer
            .build_gpu_bubble_instances(&[
                BubbleRenderInput {
                    bubble: BubbleSnapshot {
                        kind: BubbleKind::Say,
                        text: "left".to_owned(),
                    },
                    x: -100.0,
                    y: 0.0,
                    sprite_width: 80.0,
                    sprite_height: 100.0,
                },
                BubbleRenderInput {
                    bubble: BubbleSnapshot {
                        kind: BubbleKind::Say,
                        text: "right".to_owned(),
                    },
                    x: 160.0,
                    y: 0.0,
                    sprite_width: 80.0,
                    sprite_height: 100.0,
                },
            ])
            .unwrap();

        assert_eq!(batch.shapes.len(), 2);
        assert_eq!(batch.shapes[0].shape[2], 1.0);
        assert_eq!(batch.shapes[1].shape[2], 2.0);
        for index in 0..2 {
            assert!(
                (batch.transforms[index][1] + batch.shapes[index].shape[1] - 138.0).abs() < 0.01
            );
        }
    }
}
