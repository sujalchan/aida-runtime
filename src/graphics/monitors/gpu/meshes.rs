use super::super::canvas;
use super::super::*;
use super::*;
use crate::graphics::color::linear_rgba;
use crate::graphics::pipelines::{GlyphInstance, MonitorInstance};
use crate::graphics::pipelines::{GlyphMetadata, VariableVertex};
use crate::graphics::text::text_width;
use ab_glyph::{Font, ScaleFont};
use bytemuck::Zeroable;

struct GlyphRun<'a> {
    text: &'a str,
    bold: bool,
    monitor_id: u32,
    start: [f32; 2],
    size: f32,
    color: [u8; 4],
}

struct MonitorInstanceSpec {
    monitor_id: u32,
    rect: [f32; 4],
    radius: f32,
    border_width: f32,
    fill: [u8; 4],
    border: [u8; 4],
}

pub(super) struct RoundedRectMesh {
    pub(super) origin: [f32; 2],
    pub(super) scale: f32,
    pub(super) rect: [f32; 4],
    pub(super) radius: f32,
    pub(super) color: [u8; 4],
}

pub(super) struct TextMeshRun<'a> {
    pub(super) meshes: &'a std::collections::HashMap<(char, bool), Vec<super::super::Triangle>>,
    pub(super) font: &'a ab_glyph::FontArc,
    pub(super) chars: &'a [char],
    pub(super) bold: bool,
    pub(super) size: f32,
    pub(super) origin: [f32; 2],
    pub(super) scale: f32,
    pub(super) start: [f32; 2],
    pub(super) color: [u8; 4],
}

impl VariableLayer {
    pub(crate) fn glyph_atlas_revision(&self) -> u64 {
        self.glyph_atlas.revision()
    }

    pub(crate) fn glyph_atlas_size(&self) -> u32 {
        self.glyph_atlas.size()
    }

    pub(crate) fn glyph_metadata(&self) -> Vec<GlyphMetadata> {
        let atlas_size = self.glyph_atlas.size() as f32;
        let mut metadata =
            vec![GlyphMetadata::zeroed(); self.glyph_atlas.glyph_count().max(1) as usize];
        for entry in self.glyph_atlas.entries() {
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
        self.glyph_atlas.take_updates()
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
        if self.monitors[index].kind == super::super::MonitorKind::List {
            return self.build_gpu_list_monitor_instances(index, build_shapes, build_text);
        }
        let compact =
            self.monitors[index].display_mode == super::super::MonitorDisplayMode::ValueOnly;
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
        let slider = m.display_mode == super::super::MonitorDisplayMode::Slider;
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
                MonitorInstanceSpec {
                    monitor_id: gpu_id,
                    rect: [0.0, 0.0, m.width, m.height],
                    radius: MONITOR_FRAME_RADIUS,
                    border_width: 1.0,
                    fill: MONITOR_FRAME_FILL,
                    border: MONITOR_FRAME_BORDER,
                },
            );
        }
        let vc = monitor_color(m.category);
        if build_shapes && compact {
            push_monitor_instance(
                &mut shapes,
                MonitorInstanceSpec {
                    monitor_id: gpu_id,
                    rect: [value_x, value_y, m.value_width, value_height],
                    radius: value_radius,
                    border_width: 1.0,
                    fill: vc,
                    border: [189, 201, 217, 255],
                },
            );
        } else if build_shapes {
            push_monitor_instance(
                &mut shapes,
                MonitorInstanceSpec {
                    monitor_id: gpu_id,
                    rect: [value_x, value_y, m.value_width, value_height],
                    radius: value_radius,
                    border_width: 1.0,
                    fill: vc,
                    border: MONITOR_FRAME_BORDER,
                },
            );
        }
        if build_text && !compact {
            let baseline = canvas::centered_baseline(
                &self.bold_font,
                18.0,
                if slider { 17.0 } else { m.height * 0.5 },
            );
            self.push_glyph_instances(
                &mut glyphs,
                &name,
                true,
                gpu_id,
                [14.0, baseline],
                [82, 93, 117, 255],
            );
        }
        let value_start = value_x + m.value_width * 0.5 - m.value_text_width * 0.5;
        let baseline =
            canvas::centered_baseline(&self.regular_font, 19.0, value_y + value_height * 0.5);
        if build_text {
            self.push_glyph_instances(
                &mut glyphs,
                &value,
                false,
                gpu_id,
                [value_start, baseline],
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
                MonitorInstanceSpec {
                    monitor_id: gpu_id,
                    rect: [14.0, 36.0, m.width - 28.0, 12.0],
                    radius: 6.0,
                    border_width: 0.0,
                    fill: track_border,
                    border: track_border,
                },
            );
            push_monitor_instance(
                &mut shapes,
                MonitorInstanceSpec {
                    monitor_id: gpu_id,
                    rect: [15.0, 37.0, m.width - 30.0, 10.0],
                    radius: 5.0,
                    border_width: 0.0,
                    fill: track_fill,
                    border: track_fill,
                },
            );
            let thumb_x = 14.0 + (m.width - 28.0) * canvas::monitor_slider_ratio(m);
            let active_width = (thumb_x - 15.0).clamp(0.0, (m.width - 30.0).max(0.0));
            if active_width > 0.0 {
                push_monitor_instance(
                    &mut shapes,
                    MonitorInstanceSpec {
                        monitor_id: gpu_id,
                        rect: [15.0, 37.0, active_width, 10.0],
                        radius: 5.0,
                        border_width: 0.0,
                        fill: scratch_blue,
                        border: scratch_blue,
                    },
                );
            }
            push_monitor_instance(
                &mut shapes,
                MonitorInstanceSpec {
                    monitor_id: gpu_id,
                    rect: [thumb_x - 8.0, 34.0, 16.0, 16.0],
                    radius: 8.0,
                    border_width: 0.0,
                    fill: scratch_blue,
                    border: scratch_blue,
                },
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
        let plus_width = text_width(&self.bold_font, "+", LIST_LENGTH_FONT_SIZE);
        let equals_width = text_width(&self.bold_font, "=", LIST_LENGTH_FONT_SIZE);
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
                let index_max_width =
                    (row_value_x - LIST_ROW_INDEX_X - LIST_ROW_INDEX_GAP).max(0.0);
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
                MonitorInstanceSpec {
                    monitor_id: id,
                    rect: [0.0, 0.0, width, height],
                    radius: MONITOR_FRAME_RADIUS,
                    border_width: 1.0,
                    fill: MONITOR_FRAME_FILL,
                    border: MONITOR_FRAME_BORDER,
                },
            );
            push_monitor_instance(
                &mut shapes,
                MonitorInstanceSpec {
                    monitor_id: id,
                    rect: [1.0, 1.0, width - 2.0, 23.0],
                    radius: MONITOR_FRAME_RADIUS - 1.0,
                    border_width: 0.0,
                    fill: [255, 255, 255, 255],
                    border: [255, 255, 255, 255],
                },
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
                    MonitorInstanceSpec {
                        monitor_id: id,
                        rect: [row_value_x, y, (width - row_value_x - 6.0).max(0.0), 22.0],
                        radius: LIST_ROW_RADIUS,
                        border_width: 1.0,
                        fill: category_color,
                        border: [0, 0, 0, 38],
                    },
                );
                if let Some(editor) = self.list_editor.as_ref().filter(|editor| {
                    editor.monitor_index == index
                        && editor.select_all
                        && editor.index == visible_rows[row].0
                }) {
                    let _ = editor;
                    let selection_width =
                        text_width(&self.regular_font, &shown_items[row], LIST_ROW_FONT_SIZE)
                            .min((width - row_value_x - LIST_ROW_TEXT_INSET - 28.0).max(0.0));
                    if selection_width > 0.0 {
                        push_monitor_instance(
                            &mut shapes,
                            MonitorInstanceSpec {
                                monitor_id: id,
                                rect: [
                                    row_value_x + LIST_ROW_TEXT_INSET,
                                    y + 2.0,
                                    selection_width,
                                    18.0,
                                ],
                                radius: 1.0,
                                border_width: 0.0,
                                fill: [63, 123, 220, 255],
                                border: [63, 123, 220, 255],
                            },
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
            let scrollbar_visible = scrollbar.is_some() && self.list_scrollbar_is_visible(index);
            let track_alpha = if scrollbar_visible { 42 } else { 0 };
            let thumb_alpha = if scrollbar_visible { 190 } else { 0 };
            let track_color = [82, 93, 117, track_alpha];
            let thumb_color = [82, 93, 117, thumb_alpha];
            let track_instance = shapes.len();
            push_monitor_instance(
                &mut shapes,
                MonitorInstanceSpec {
                    monitor_id: id,
                    rect: [track_x, track_y, LIST_SCROLLBAR_WIDTH, track_height],
                    radius: LIST_SCROLLBAR_WIDTH * 0.5,
                    border_width: 0.0,
                    fill: track_color,
                    border: track_color,
                },
            );
            // premultiply these translucent colors for the monitor pipeline's blend mode.
            shapes[track_instance].fill = premultiplied_linear_rgba(track_color);
            shapes[track_instance].border = premultiplied_linear_rgba(track_color);
            let thumb_instance = shapes.len();
            push_monitor_instance(
                &mut shapes,
                MonitorInstanceSpec {
                    monitor_id: id,
                    rect: [
                        track_x + 0.5,
                        thumb_y,
                        LIST_SCROLLBAR_WIDTH - 1.0,
                        thumb_height,
                    ],
                    radius: (LIST_SCROLLBAR_WIDTH - 1.0) * 0.5,
                    border_width: 0.0,
                    fill: thumb_color,
                    border: thumb_color,
                },
            );
            // keep the track first so its edge stays behind the thumb.
            shapes[thumb_instance].fill = premultiplied_linear_rgba(thumb_color);
            shapes[thumb_instance].border = premultiplied_linear_rgba(thumb_color);
            // draw the footer after the rows so it covers the partial bottom row.
            push_monitor_instance(
                &mut shapes,
                MonitorInstanceSpec {
                    monitor_id: id,
                    rect: [1.0, height - LIST_FOOTER_HEIGHT, width - 2.0, 21.0],
                    radius: MONITOR_FRAME_RADIUS - 1.0,
                    border_width: 0.0,
                    fill: [255, 255, 255, 255],
                    border: [255, 255, 255, 255],
                },
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
                canvas::centered_baseline(&self.bold_font, LIST_HEADER_FONT_SIZE, 12.5);
            self.push_glyph_instances_at_size(
                &mut glyphs,
                GlyphRun {
                    text: &shown_name,
                    bold: true,
                    monitor_id: id,
                    start: [
                        centered_glyph_start(
                            &self.bold_font,
                            &shown_name,
                            LIST_HEADER_FONT_SIZE,
                            width,
                        ),
                        header_baseline,
                    ],
                    size: LIST_HEADER_FONT_SIZE,
                    color: [82, 93, 117, 255],
                },
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
                let index_max_width =
                    (row_value_x - LIST_ROW_INDEX_X - LIST_ROW_INDEX_GAP).max(0.0);
                // keep narrow monitors from drawing index glyphs into the value column.
                let shown_index = fit_monitor_text(
                    &self.bold_font,
                    &index_text,
                    index_max_width,
                    LIST_ROW_FONT_SIZE,
                );
                let row_center_y = y + 11.0;
                let baseline =
                    canvas::centered_baseline(&self.bold_font, LIST_ROW_FONT_SIZE, row_center_y);
                self.push_glyph_instances_at_size(
                    &mut glyphs,
                    GlyphRun {
                        text: &shown_index,
                        bold: true,
                        monitor_id: id,
                        start: [LIST_ROW_INDEX_X, baseline],
                        size: LIST_ROW_FONT_SIZE,
                        color: [82, 93, 117, 255],
                    },
                );
                self.push_glyph_instances_at_size(
                    &mut glyphs,
                    GlyphRun {
                        text: item,
                        bold: false,
                        monitor_id: id,
                        start: [
                            row_value_x + LIST_ROW_TEXT_INSET,
                            canvas::centered_baseline(
                                &self.regular_font,
                                LIST_ROW_FONT_SIZE,
                                row_center_y,
                            ),
                        ],
                        size: LIST_ROW_FONT_SIZE,
                        color: [255, 255, 255, 255],
                    },
                );
                if let Some(editor) = self.list_editor.as_ref().filter(|editor| {
                    editor.monitor_index == index && editor.index == visible_rows[row].0
                }) {
                    if !editor.select_all {
                        let prefix = editor.text.get(..editor.caret).unwrap_or_default();
                        let caret_x = row_value_x
                            + LIST_ROW_TEXT_INSET
                            + text_width(&self.regular_font, prefix, LIST_ROW_FONT_SIZE);
                        self.push_glyph_instances_at_size(
                            &mut glyphs,
                            GlyphRun {
                                text: "|",
                                bold: true,
                                monitor_id: id,
                                start: [
                                    caret_x,
                                    canvas::centered_baseline(
                                        &self.regular_font,
                                        LIST_ROW_FONT_SIZE,
                                        row_center_y,
                                    ),
                                ],
                                size: LIST_ROW_FONT_SIZE,
                                color: [82, 93, 117, 255],
                            },
                        );
                    }
                    self.push_glyph_instances_at_size(
                        &mut glyphs,
                        GlyphRun {
                            text: "x",
                            bold: true,
                            monitor_id: id,
                            start: [width - 19.0, baseline],
                            size: LIST_ROW_FONT_SIZE,
                            color: [82, 93, 117, 255],
                        },
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
                    GlyphRun {
                        text: empty,
                        bold: false,
                        monitor_id: id,
                        start: [
                            centered_glyph_start(
                                &self.regular_font,
                                empty,
                                LIST_ROW_FONT_SIZE,
                                width,
                            ),
                            canvas::centered_baseline(
                                &self.regular_font,
                                LIST_ROW_FONT_SIZE,
                                LIST_CONTENT_TOP
                                    + LIST_EMPTY_TOP_PADDING
                                    + LIST_ROW_FONT_SIZE * 0.5,
                            ),
                        ],
                        size: LIST_ROW_FONT_SIZE,
                        color: [82, 93, 117, 255],
                    },
                );
            }
            let footer_y =
                canvas::centered_baseline(&self.bold_font, LIST_LENGTH_FONT_SIZE, height - 11.0);
            self.push_glyph_instances_at_size(
                &mut glyphs,
                GlyphRun {
                    text: "+",
                    bold: true,
                    monitor_id: id,
                    start: [LIST_FOOTER_SIDE_PADDING, footer_y],
                    size: LIST_LENGTH_FONT_SIZE,
                    color: [82, 93, 117, 255],
                },
            );
            let length_width = text_width(&self.bold_font, &shown_length, LIST_LENGTH_FONT_SIZE);
            let equals_x = width - LIST_FOOTER_SIDE_PADDING - equals_width;
            let length_x = (LIST_FOOTER_SIDE_PADDING + plus_width + equals_x - length_width) * 0.5;
            self.push_glyph_instances_at_size(
                &mut glyphs,
                GlyphRun {
                    text: &shown_length,
                    bold: true,
                    monitor_id: id,
                    start: [length_x, footer_y],
                    size: LIST_LENGTH_FONT_SIZE,
                    color: [82, 93, 117, 255],
                },
            );
            self.push_glyph_instances_at_size(
                &mut glyphs,
                GlyphRun {
                    text: "=",
                    bold: true,
                    monitor_id: id,
                    start: [equals_x, footer_y],
                    size: LIST_LENGTH_FONT_SIZE,
                    color: [82, 93, 117, 255],
                },
            );
        }
        Ok((shapes, glyphs))
    }

    pub(in crate::graphics::monitors) fn visible_list_rows(
        &self,
        index: usize,
    ) -> Vec<(usize, String)> {
        let Some(monitor) = self.monitors.get(index) else {
            return Vec::new();
        };
        if monitor.kind != super::super::MonitorKind::List {
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
        start: [f32; 2],
        color: [u8; 4],
    ) {
        let size = if bold { 18.0 } else { 19.0 };
        self.push_glyph_instances_at_size(
            out,
            GlyphRun {
                text,
                bold,
                monitor_id,
                start,
                size,
                color,
            },
        );
    }

    fn push_glyph_instances_at_size(&self, out: &mut Vec<GlyphInstance>, run: GlyphRun<'_>) {
        let font = if run.bold {
            &self.bold_font
        } else {
            &self.regular_font
        };
        let atlas_font_size = if run.bold { 18.0 } else { 19.0 };
        let scaled = font.as_scaled(run.size);
        let mut pen_x = 0.0;
        let mut previous = None;
        for ch in run.text.chars() {
            let gid = scaled.glyph_id(ch);
            if let Some(prev) = previous {
                pen_x += scaled.kern(prev, gid);
            }
            if let Some(e) = self.glyph_atlas.get(ch, run.bold)
                && e.width > 0
                && e.height > 0
            {
                out.push(GlyphInstance {
                    local_position: [run.start[0] + pen_x, run.start[1]],
                    glyph_id: e.id,
                    monitor_id: run.monitor_id,
                    color: linear_rgba(run.color),
                    scale: run.size / atlas_font_size,
                    clip_y: [0.0; 2],
                });
            }
            pen_x += scaled.h_advance(gid);
            previous = Some(gid);
        }
    }

    fn ensure_glyph_atlas(&mut self, ch: char, bold: bool) -> Result<(), String> {
        let font = if bold {
            &self.bold_font
        } else {
            &self.regular_font
        };
        self.glyph_atlas.ensure(font, ch, bold)
    }
}

pub(in crate::graphics::monitors) fn monitor_color(
    category: super::super::MonitorCategory,
) -> [u8; 4] {
    match category {
        super::super::MonitorCategory::Variables => [255, 140, 26, 255],
        super::super::MonitorCategory::Lists => [255, 102, 26, 255],
        super::super::MonitorCategory::Motion => [76, 151, 255, 255],
        super::super::MonitorCategory::Looks => [153, 102, 255, 255],
        super::super::MonitorCategory::Sound => [207, 99, 207, 255],
        super::super::MonitorCategory::Events => [255, 191, 0, 255],
        super::super::MonitorCategory::Control => [255, 171, 25, 255],
        super::super::MonitorCategory::Sensing => [92, 177, 214, 255],
        super::super::MonitorCategory::Operators => [89, 192, 89, 255],
        super::super::MonitorCategory::MyBlocks => [255, 102, 128, 255],
        super::super::MonitorCategory::Extension(color) => [color[0], color[1], color[2], 255],
    }
}

pub(in crate::graphics::monitors) fn fit_monitor_text(
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
        if text_width(font, &candidate, size) > max_width {
            while !fitted.is_empty() {
                let mut with_ellipsis = fitted.clone();
                with_ellipsis.push('…');
                if text_width(font, &with_ellipsis, size) <= max_width {
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

pub(in crate::graphics::monitors) fn centered_glyph_start(
    font: &ab_glyph::FontArc,
    text: &str,
    size: f32,
    container_width: f32,
) -> f32 {
    let Some((left, right)) = glyph_text_x_bounds(font, text, size) else {
        return (container_width - text_width(font, text, size)) * 0.5;
    };
    (container_width - (right - left)) * 0.5 - left
}

pub(in crate::graphics::monitors) fn glyph_text_x_bounds(
    font: &ab_glyph::FontArc,
    text: &str,
    size: f32,
) -> Option<(f32, f32)> {
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

fn push_monitor_instance(out: &mut Vec<MonitorInstance>, spec: MonitorInstanceSpec) {
    let [x, y, width, height] = spec.rect;
    if width <= 0.0 || height <= 0.0 {
        return;
    }
    out.push(MonitorInstance {
        rect: [x, y, width, height],
        fill: linear_rgba(spec.fill),
        border: linear_rgba(spec.border),
        params: [
            spec.radius,
            spec.border_width,
            SCRATCH_WIDTH as f32,
            SCRATCH_HEIGHT as f32,
        ],
        shape: [0.0; 4],
        monitor_id: spec.monitor_id,
        _padding: [0; 3],
    });
}

pub(super) fn premultiplied_linear_rgba(color: [u8; 4]) -> [f32; 4] {
    let mut rgba = linear_rgba(color);
    let alpha = rgba[3];
    for channel in &mut rgba[..3] {
        *channel *= alpha;
    }
    rgba
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

pub(super) fn push_rounded_rect(output: &mut Vec<VariableVertex>, rect: RoundedRectMesh) {
    let [base_x, base_y] = rect.origin;
    let [x, y, width, height] = rect.rect;
    if width <= 0.0 || height <= 0.0 {
        return;
    }
    let radius = rect.radius.max(0.0).min(width * 0.5).min(height * 0.5);
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
    let center_x = base_x + (x + width * 0.5) * rect.scale;
    let center_y = base_y + (y + height * 0.5) * rect.scale;
    for index in 0..points.len() {
        let a = points[index];
        let b = points[(index + 1) % points.len()];
        push_vertex(output, center_x, center_y, rect.color);
        push_vertex(
            output,
            base_x + (x + a.0) * rect.scale,
            base_y + (y + a.1) * rect.scale,
            rect.color,
        );
        push_vertex(
            output,
            base_x + (x + b.0) * rect.scale,
            base_y + (y + b.1) * rect.scale,
            rect.color,
        );
    }
}

pub(super) fn push_text_mesh(output: &mut Vec<VariableVertex>, run: TextMeshRun<'_>) {
    let scaled = run.font.as_scaled(run.size);
    let mut pen_x = 0.0;
    let mut previous = None;
    for character in run.chars {
        let glyph = scaled.glyph_id(*character);
        if let Some(previous) = previous {
            pen_x += scaled.kern(previous, glyph);
        }
        if let Some(mesh) = run.meshes.get(&(*character, run.bold)) {
            for triangle in mesh {
                for point in triangle {
                    push_vertex(
                        output,
                        run.origin[0] + (run.start[0] + pen_x + point.x) * run.scale,
                        run.origin[1] + (run.start[1] - point.y) * run.scale,
                        run.color,
                    );
                }
            }
        }
        pen_x += scaled.h_advance(glyph);
        previous = Some(glyph);
    }
}
