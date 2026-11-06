mod list;
mod motion;
mod operators;
mod pacing;
mod pen;
mod random;
mod scheduler;
mod sound;
mod state;
mod tick;
mod value;

pub use list::{SCRATCH_LIST_LIMIT, ScratchList};
pub use motion::GlideState;
pub use operators::{
    fast_acos_degrees, fast_asin_degrees, fast_atan_degrees, fast_cos, fast_sin, fast_tan,
    scratch_acos_degrees, scratch_add, scratch_and, scratch_asin_degrees, scratch_atan_degrees,
    scratch_bool, scratch_compare, scratch_contains, scratch_cos, scratch_divide, scratch_equals,
    scratch_greater_than, scratch_join, scratch_length, scratch_less_than, scratch_letter,
    scratch_mod, scratch_multiply, scratch_not, scratch_number_operand, scratch_or, scratch_round,
    scratch_sin, scratch_string, scratch_subtract, scratch_tan, scratch_to_number,
};
pub use pacing::{PacingMechanism, PacingMode, SystemPacingMechanism, TickPacer};
pub use pen::PenColorState;
pub use random::{ScratchRandom, scratch_random, scratch_random_constant, with_scratch_random};
pub use scheduler::EdgeHatState;
pub use scheduler::{
    Clock, ResumableScript, SchedulerStatus, ScriptScheduler, ScriptStatus, WarpBudget,
};
pub use sound::{AudioEvent, RealtimeAudio, SoundAssetError, SoundLibrary};
pub(crate) use sound::{SoundParameters, SoundRuntime};
pub use state::{
    BubbleKind, BubbleSnapshot, LayerSnapshot, ListSnapshot, ObservableEvent, ReporterSnapshot,
    RuntimeInputEvent, RuntimeSnapshot, ScratchListMetadata, ScratchRotationStyle,
    ScratchSoundEffect, ScratchState, ScratchVariableMetadata, SpriteFenceGeometry,
    VariableSnapshot, list_snapshot_key, pointer_click_activates, scratch_motion_coordinate,
    variable_snapshot_key,
};
pub use tick::run_tick;
pub use value::{CachedListIndex, ScratchInput, ScratchValue, ScratchValueView};

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
    pub fn as_stroke(self) -> PenStroke {
        PenStroke {
            start: self.position,
            end: self.position,
            color: self.color,
            // Dot alignment uses the clamped diameter; preserve invalid values
            // for the renderer's normal finite-value validation.
            diameter: if self.diameter.is_finite() {
                self.diameter.max(1.0)
            } else {
                self.diameter
            },
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
