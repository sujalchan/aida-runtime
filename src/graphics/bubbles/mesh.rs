use ab_glyph::{Font, FontArc, ScaleFont};

use crate::graphics::color::linear_rgba;
use crate::graphics::pipelines::{GlyphInstance, MonitorInstance};
use crate::graphics::text::GlyphAtlas;
use crate::graphics::{SCRATCH_HEIGHT, SCRATCH_WIDTH};
use crate::runtime::BubbleKind;

use super::layout::{self, BubbleLayout};
use super::{
    BUBBLE_BODY_CORNER_RADIUS, BUBBLE_BORDER, BUBBLE_BORDER_WIDTH, BUBBLE_DARK, BUBBLE_FONT_SIZE,
    BUBBLE_LINE_HEIGHT, BUBBLE_PADDING_X, BUBBLE_PADDING_Y, BUBBLE_TAIL_CORNER_CLEARANCE,
    BubbleGpuBatch, BubbleRenderInput,
};

pub(crate) fn build_gpu_instances(
    font: &FontArc,
    glyph_atlas: &mut GlyphAtlas,
    bubbles: &[BubbleRenderInput],
) -> Result<BubbleGpuBatch, String> {
    let mut batch = BubbleGpuBatch::default();
    for (bubble_index, input) in bubbles.iter().enumerate() {
        let BubbleLayout {
            left,
            top,
            width,
            height,
            sprite_x,
            lower_extent,
            lines,
        } = layout::layout_bubble(font, input);
        let bubble_id = bubble_index as u32;
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
                glyph_atlas.ensure(font, ch, false)?;
            }
            let scaled_font = font.as_scaled(BUBBLE_FONT_SIZE);
            let baseline = centered_baseline(
                font,
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
                if let Some(entry) = glyph_atlas.get(ch, false)
                    && entry.width > 0
                    && entry.height > 0
                {
                    batch.glyphs.push(GlyphInstance {
                        local_position: [pen_x, baseline],
                        glyph_id: entry.id,
                        monitor_id: bubble_id,
                        color: linear_rgba(BUBBLE_DARK),
                        scale: BUBBLE_FONT_SIZE / 19.0,
                        clip_y: [0.0; 2],
                    });
                }
                pen_x += scaled_font.h_advance(glyph);
                previous = Some(glyph);
            }
        }
    }
    Ok(batch)
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

fn centered_baseline(font: &FontArc, size: f32, center_y: f32) -> f32 {
    let scaled = font.as_scaled(size);
    center_y + (scaled.ascent() + scaled.descent()) * 0.5
}
