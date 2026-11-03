mod list;
mod motion;
mod operators;
mod pacing;
mod random;
mod scheduler;
mod tick;
mod value;

#[allow(unused_imports)]
pub use list::{SCRATCH_LIST_LIMIT, ScratchList};
#[allow(unused_imports)]
pub use motion::GlideState;
pub use operators::*;
#[allow(unused_imports)]
pub use pacing::{PacingMechanism, PacingMode, SystemPacingMechanism, TickPacer};
#[allow(unused_imports)]
pub use random::{ScratchRandom, scratch_random};
pub use scheduler::{
    Clock, ResumableScript, SchedulerStatus, ScriptScheduler, ScriptStatus, WarpBudget,
};
pub use tick::run_tick;
#[allow(unused_imports)]
pub use value::{
    BubbleKind, BubbleSnapshot, LayerSnapshot, ListSnapshot, ObservableEvent, RuntimeInputEvent,
    RuntimeSnapshot, ScratchInput, ScratchListMetadata, ScratchState, ScratchValue,
    ScratchValueView, ScratchVariableMetadata, VariableSnapshot, list_snapshot_key,
    variable_snapshot_key,
};

#[derive(Clone)]
pub enum PenCommand {
    Clear,
    Stroke {
        start: [f32; 2],
        end: [f32; 2],
        color: [f32; 4],
        diameter: f32,
    },
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

#[allow(dead_code)]
pub struct SpriteSensingCostume {
    pub texture_size: (u32, u32),
    pub logical_size: [f32; 2],
    pub rotation_center: [f32; 2],
    pub opaque_pixels: std::sync::Arc<[u8]>,
}
