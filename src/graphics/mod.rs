mod pipelines;
mod sprite;
mod state;
mod texture;
mod variables;
mod viewport;

pub(crate) const SCRATCH_WIDTH: u32 = 480;
pub(crate) const SCRATCH_HEIGHT: u32 = 360;

pub(crate) use state::{GraphicsState, RenderOutcome};
#[allow(unused_imports)]
pub(crate) use variables::SliderRangeEditorInput;
