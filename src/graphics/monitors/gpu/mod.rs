const SHADOW_BLUR_RADIUS: f32 = 2.5;
const MONITOR_FRAME_RADIUS: f32 = 7.0;
const MONITOR_VALUE_RADIUS: f32 = 8.0;
const LIST_ROW_RADIUS: f32 = 4.0;
const MONITOR_FRAME_FILL: [u8; 4] = [229, 240, 255, 255];
const MONITOR_FRAME_BORDER: [u8; 4] = [0, 0, 0, 38];
const MONITOR_SHAPE_TOP_ROUNDED: f32 = 3.0;
const MONITOR_SHAPE_BOTTOM_ROUNDED: f32 = 4.0;

mod meshes;
#[cfg(test)]
mod tests;
mod upload;

pub(super) use meshes::fit_monitor_text;
#[cfg(test)]
pub(super) use meshes::{centered_glyph_start, glyph_text_x_bounds, monitor_color};
