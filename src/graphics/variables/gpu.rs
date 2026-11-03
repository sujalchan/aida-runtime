use ab_glyph::{Font, ScaleFont};
use bytemuck::Zeroable;
use wgpu::Queue;

use super::super::texture::RenderTexture;
use super::menu::{
    CONTEXT_MENU_ROW_HEIGHT, CONTEXT_MENU_WIDTH, MonitorContextAction, SLIDER_RANGE_EDITOR_HEIGHT,
    SLIDER_RANGE_EDITOR_WIDTH,
};
use super::{GLYPH_ATLAS_MAX_SIZE, GlyphAtlasEntry};
use super::{
    MonitorVisibility, SCRATCH_HEIGHT, SCRATCH_WIDTH, VariableLayer,
    canvas::{blur_shadow_alpha, draw_monitor_shadow},
    tessellate_glyph_outline,
};
use crate::graphics::pipelines::{GlyphInstance, GlyphMetadata, MonitorInstance, VariableVertex};

const SHADOW_BLUR_RADIUS: f32 = 2.5;

impl VariableLayer {
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
        let value_radius = if compact { 7.0 } else { 8.0 };
        if build_shapes && !compact {
            push_monitor_instance(
                &mut shapes,
                gpu_id,
                0.0,
                0.0,
                m.width,
                m.height,
                7.0,
                1.0,
                [232, 242, 255, 255],
                [189, 201, 217, 255],
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
                0.0,
                vc,
                vc,
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
        let font = if bold {
            &self.bold_font
        } else {
            &self.regular_font
        };
        let size = if bold { 18.0 } else { 19.0 };
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
        super::MonitorCategory::Motion => [76, 151, 255, 255],
        super::MonitorCategory::Looks => [153, 102, 255, 255],
        super::MonitorCategory::Sound => [207, 99, 207, 255],
        super::MonitorCategory::Sensing => [92, 177, 214, 255],
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
    use super::{linear_rgba, scaled_px_bounds};

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
}
