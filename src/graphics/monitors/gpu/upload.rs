use super::super::super::texture::{RenderTexture, TextureRegion};
use super::super::canvas::{CanvasTransform, blur_shadow_alpha, draw_monitor_shadow};
use super::super::menu::{
    CONTEXT_MENU_ROW_HEIGHT, CONTEXT_MENU_WIDTH, MonitorContextAction, SLIDER_RANGE_EDITOR_HEIGHT,
    SLIDER_RANGE_EDITOR_WIDTH,
};
use super::super::*;
use super::meshes::{RoundedRectMesh, TextMeshRun, push_rounded_rect, push_text_mesh};
use super::*;
use crate::graphics::pipelines::VariableVertex;
use ab_glyph::{Font, ScaleFont};
use wgpu::Queue;

impl VariableLayer {
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
            super::super::canvas::draw_context_menu(
                &mut pixmap,
                &menu,
                &self.glyph_meshes,
                &self.bold_font,
                CanvasTransform {
                    origin: [0.0, 0.0],
                    monitor_scale: 1.0,
                    scale: [scale_x, scale_y],
                    offset: [-(left as f32), -(top as f32)],
                },
            )?;
        } else if let Some(editor) = &self.slider_range_editor {
            super::super::canvas::draw_slider_range_editor(
                &mut pixmap,
                editor,
                &self.glyph_meshes,
                &self.bold_font,
                CanvasTransform {
                    origin: [0.0, 0.0],
                    monitor_scale: 1.0,
                    scale: [scale_x, scale_y],
                    offset: [-(left as f32), -(top as f32)],
                },
            )?;
        }
        target.write_rgba_region(
            queue,
            TextureRegion {
                x: left,
                y: top,
                width,
                height,
            },
            pixmap.data(),
        );
        Ok(())
    }

    pub(in crate::graphics::monitors) fn rasterize_shadow_tile(
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
        draw_monitor_shadow(
            &mut pixmap,
            monitor,
            CanvasTransform {
                origin: [14.0, 14.0],
                monitor_scale: monitor.scale,
                scale: [scale_x, scale_y],
                offset: [0.0, 0.0],
            },
        )?;
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
        let width = crate::graphics::text::text_width(&self.regular_font, text, 19.0) * 0.7 + 16.0;
        let mut vertices = Vec::new();
        push_rounded_rect(
            &mut vertices,
            RoundedRectMesh {
                origin: [0.0, 0.0],
                scale: 1.0,
                rect: [8.0, 8.0, width, 22.0],
                radius: 4.0,
                color: [26, 30, 38, 220],
            },
        );
        push_text_mesh(
            &mut vertices,
            TextMeshRun {
                meshes: &self.glyph_meshes,
                font: &self.regular_font,
                chars: &chars,
                bold: false,
                size: 19.0,
                origin: [0.0, 0.0],
                scale: 0.7,
                start: [16.0, 25.0],
                color: [245, 248, 255, 255],
            },
        );
        Ok(vertices)
    }
}
