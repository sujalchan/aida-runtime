mod layout;
pub(crate) mod mesh;

use crate::graphics::pipelines::{GlyphInstance, MonitorInstance};
use crate::runtime::BubbleSnapshot;

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
pub(crate) const BUBBLE_FONT_SIZE: f32 = 9.0;
const BUBBLE_MIN_WIDTH: f32 = 56.0;
const BUBBLE_EDGE: f32 = 8.0;
pub(crate) const BUBBLE_BODY_CORNER_RADIUS: f32 = 17.0;
const BUBBLE_DARK: [u8; 4] = [82, 91, 113, 255];
pub(crate) const BUBBLE_BORDER: [u8; 4] = [215, 215, 215, 255];
pub(crate) const BUBBLE_BORDER_WIDTH: f32 = 1.1;
pub(crate) const BUBBLE_SPEECH_TAIL_EXTENT: f32 = 12.5;
pub(crate) const BUBBLE_TAIL_CORNER_CLEARANCE: f32 = 2.0;
