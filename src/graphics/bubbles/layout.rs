use ab_glyph::FontArc;

use crate::graphics::text::text_width;
use crate::graphics::{SCRATCH_HEIGHT, SCRATCH_WIDTH};
use crate::runtime::BubbleKind;

use super::{
    BUBBLE_EDGE, BUBBLE_FONT_SIZE, BUBBLE_LINE_HEIGHT, BUBBLE_MAX_WIDTH, BUBBLE_MIN_WIDTH,
    BUBBLE_PADDING_X, BUBBLE_PADDING_Y, BUBBLE_SPEECH_TAIL_EXTENT, BubbleRenderInput,
};

pub(super) struct BubbleLayout {
    pub(super) left: f32,
    pub(super) top: f32,
    pub(super) width: f32,
    pub(super) height: f32,
    pub(super) sprite_x: f32,
    pub(super) lower_extent: f32,
    pub(super) lines: Vec<String>,
}

pub(super) fn layout_bubble(font: &FontArc, input: &BubbleRenderInput) -> BubbleLayout {
    let margin = BUBBLE_EDGE;
    let max_height = SCRATCH_HEIGHT as f32 - margin * 2.0;
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
    let lines = wrap_bubble_text(font, &input.bubble.text, inner_width);
    let height =
        (lines.len().max(1) as f32 * BUBBLE_LINE_HEIGHT + BUBBLE_PADDING_Y * 2.0).min(max_height);
    let text_width = lines
        .iter()
        .map(|line| text_width(font, line, BUBBLE_FONT_SIZE))
        .fold(0.0_f32, f32::max);
    let width = (text_width + BUBBLE_PADDING_X * 2.0).clamp(BUBBLE_MIN_WIDTH, body_max_width);
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
    BubbleLayout {
        left,
        top,
        width,
        height,
        sprite_x,
        lower_extent,
        lines,
    }
}

fn wrap_bubble_text(font: &FontArc, text: &str, max_width: f32) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let mut current = String::new();
        for word in paragraph.split_whitespace() {
            let candidate = if current.is_empty() {
                word.to_owned()
            } else {
                format!("{current} {word}")
            };
            if text_width(font, &candidate, BUBBLE_FONT_SIZE) <= max_width {
                current = candidate;
                continue;
            }
            if !current.is_empty() {
                lines.push(std::mem::take(&mut current));
            }
            if text_width(font, word, BUBBLE_FONT_SIZE) <= max_width {
                current.push_str(word);
                continue;
            }
            for ch in word.chars() {
                let mut candidate = current.clone();
                candidate.push(ch);
                if text_width(font, &candidate, BUBBLE_FONT_SIZE) > max_width && !current.is_empty()
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
