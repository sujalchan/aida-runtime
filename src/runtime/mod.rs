mod list;
mod motion;
mod operators;
mod pacing;
mod pen;
mod random;
mod scheduler;
mod sound;
mod tick;
mod value;

#[allow(unused_imports)]
pub use list::{SCRATCH_LIST_LIMIT, ScratchList};
#[allow(unused_imports)]
pub use motion::GlideState;
pub use operators::*;
#[allow(unused_imports)]
pub use pacing::{PacingMechanism, PacingMode, SystemPacingMechanism, TickPacer};
pub use pen::PenColorState;
#[allow(unused_imports)]
pub use random::{ScratchRandom, scratch_random, scratch_random_constant, with_scratch_random};
#[allow(unused_imports)]
pub use scheduler::EdgeHatState;
pub use scheduler::{
    Clock, ResumableScript, SchedulerStatus, ScriptScheduler, ScriptStatus, WarpBudget,
};
pub use sound::{AudioEvent, RealtimeAudio, SoundLibrary};
pub(crate) use sound::{SoundParameters, SoundRuntime};
pub use tick::run_tick;
#[allow(unused_imports)]
pub use value::pointer_click_activates;
pub use value::scratch_motion_coordinate;
#[allow(unused_imports)]
pub use value::{
    BubbleKind, BubbleSnapshot, CachedListIndex, LayerSnapshot, ListSnapshot, ObservableEvent, ReporterSnapshot,
    RuntimeInputEvent, RuntimeSnapshot, ScratchInput, ScratchListMetadata, ScratchRotationStyle,
    ScratchSoundEffect, ScratchState, ScratchValue, ScratchValueView, ScratchVariableMetadata,
    SpriteFenceGeometry, VariableSnapshot, list_snapshot_key, variable_snapshot_key,
};

#[derive(Clone, Debug, PartialEq)]
pub enum PenCommand {
    Clear,
    Dots(Vec<PenDot>),
    Strokes(Vec<PenStroke>),
    // An ordered run of points and strokes, with points encoded as zero-length strokes.
    Mixed(Vec<PenStroke>),
    Stamp {
        target_name: String,
        position: [f32; 2],
        direction: f32,
        scale: f32,
        costume_index: usize,
        effects_0: [f32; 4],
        effects_1: [f32; 4],
    },
}

// store pen-down points without duplicating their center as a line endpoint.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PenDot {
    pub position: [f32; 2],
    pub color: [f32; 4],
    pub diameter: f32,
    pub pixel_offset: f32,
}

impl PenDot {
    pub(crate) fn as_stroke(self) -> PenStroke {
        PenStroke {
            start: self.position,
            end: self.position,
            color: self.color,
            // Dot alignment uses the clamped diameter; preserve invalid values
            // for the renderer's normal finite-value validation.
            diameter: if self.diameter.is_finite() { self.diameter.max(1.0) } else { self.diameter },
            pixel_offset: self.pixel_offset,
        }
    }
}

// encode fixed pen dropdown choices once in generated code instead of matching strings per block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PenColorParameter {
    Color,
    Saturation,
    Brightness,
    Transparency,
    Unsupported,
}

impl From<&str> for PenColorParameter {
    fn from(value: &str) -> Self {
        match value {
            "color" => Self::Color,
            "saturation" => Self::Saturation,
            "brightness" => Self::Brightness,
            "transparency" => Self::Transparency,
            _ => Self::Unsupported,
        }
    }
}

impl From<String> for PenColorParameter {
    fn from(value: String) -> Self {
        Self::from(value.as_str())
    }
}

// keep adjacent pen marks in one compact buffer so every script path can batch gpu uploads.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PenStroke {
    pub start: [f32; 2],
    pub end: [f32; 2],
    pub color: [f32; 4],
    pub diameter: f32,
    // offset thin marks by half a physical pen-texture pixel at upload time.
    pub pixel_offset: f32,
}

#[allow(dead_code)]
pub struct SpriteSensingCostume {
    pub texture_size: (u32, u32),
    pub logical_size: [f32; 2],
    pub rotation_center: [f32; 2],
    pub opaque_pixels: std::sync::Arc<[u8]>,
}
