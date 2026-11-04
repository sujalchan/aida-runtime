mod bubble_gpu;
mod init;
mod input;
mod pen;
mod profiling;
mod render;
mod variable_gpu;

use std::collections::HashMap;
use std::num::NonZeroU64;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;
use wgpu::{Adapter, Device, Queue, Surface, SurfaceConfiguration};
use winit::{
    dpi::{PhysicalPosition, PhysicalSize},
    window::Window,
};

use super::pipelines::{
    GlyphInstance, GlyphMetadata, MonitorInstance, composite_pipeline, glyph_instance_pipeline,
    layer_bind_group, monitor_instance_pipeline, pen_pipeline, premultiplied_composite_pipeline,
    sprite_pipeline, texture_layout, variable_pipeline,
};
use super::sprite::Sprite;
use super::texture::{LAYER_FORMAT, RenderTexture};
use super::variables::{
    ListEditorInput, ListPointerResponse, MonitorBounds, MonitorCategory, MonitorDisplayMode,
    MonitorKind, VariableLayer, VariableMonitor,
};
use super::viewport::StageViewport;
use super::{SCRATCH_HEIGHT, SCRATCH_WIDTH};
use crate::runtime::PenCommand;

pub(crate) enum RenderOutcome {
    Presented,
    SurfaceRestored,
    Unavailable,
}

pub(crate) struct GraphicsState {
    window: Arc<Window>,
    surface: Surface<'static>,
    device: Device,
    queue: Queue,
    config: SurfaceConfiguration,
    viewport: Option<StageViewport>,
    pen_texture: RenderTexture,
    ui_texture: RenderTexture,
    drag_shadow_texture: RenderTexture,
    transparent_texture: RenderTexture,
    pen_bind_group: wgpu::BindGroup,
    ui_bind_group: wgpu::BindGroup,
    drag_shadow_bind_group: wgpu::BindGroup,
    texture_layout: wgpu::BindGroupLayout,
    zero_uv_offset: wgpu::Buffer,
    ui_uv_offset: wgpu::Buffer,
    drag_shadow_uv_offset: wgpu::Buffer,
    stage_nearest_sampler: wgpu::Sampler,
    stage_linear_sampler: wgpu::Sampler,
    ui_sampler: wgpu::Sampler,
    backdrops: Vec<BackdropAsset>,
    backdrop_index: usize,
    backdrop_texture_cache: HashMap<(String, u32, u32), CachedBackdropTexture>,
    backdrop_cache_clock: u64,
    shared_costume_mips: HashMap<(String, i8), Arc<SpriteCostumeMip>>,
    sprites: Vec<SpriteRenderer>,
    sprite_indices: HashMap<String, usize>,
    sprite_instance_indices: HashMap<(usize, usize), usize>,
    last_runtime_layers: Vec<crate::runtime::LayerSnapshot>,
    project_timer_sample_millis: u64,
    project_timer_sampled_at: Instant,
    project_timer_epoch: u64,
    variable_layer: VariableLayer,
    fading_shadow_index: Option<usize>,
    fading_shadow_bounds: Option<MonitorBounds>,
    drag_shadow_texture_rect: Option<(u32, u32, u32, u32)>,
    pen_pipeline: wgpu::RenderPipeline,
    stamp_sprite_pipeline: wgpu::RenderPipeline,
    stamp_transform_layout: wgpu::BindGroupLayout,
    stamp_transform_buffer: wgpu::Buffer,
    stamp_transform_bind_group: wgpu::BindGroup,
    stamp_transform_capacity: usize,
    stamp_transform_stride: u64,
    pending_sprite_stamps: Vec<PendingSpriteStamp>,
    // keep appended stamp transforms from being uploaded again each update.
    pending_stamp_uploaded_count: usize,
    pen_composite_pipeline: wgpu::RenderPipeline,
    composite_pipeline: wgpu::RenderPipeline,
    sprite_pipeline: wgpu::RenderPipeline,
    sprite_transform_layout: wgpu::BindGroupLayout,
    variable_pipeline: wgpu::RenderPipeline,
    monitor_instance_pipeline: wgpu::RenderPipeline,
    monitor_transform_layout: wgpu::BindGroupLayout,
    monitor_transform_bind_group: wgpu::BindGroup,
    glyph_instance_pipeline: wgpu::RenderPipeline,
    glyph_layout: wgpu::BindGroupLayout,
    glyph_atlas_sampler: wgpu::Sampler,
    glyph_bind_group: wgpu::BindGroup,
    monitor_instances: wgpu::Buffer,
    gpu_shape_capacity: u64,
    gpu_monitor_capacity: u64,
    glyph_instances: wgpu::Buffer,
    gpu_glyph_capacity: u64,
    monitor_transforms: wgpu::Buffer,
    glyph_metadata: wgpu::Buffer,
    glyph_metadata_capacity: u32,
    monitor_batches: HashMap<u32, (Vec<MonitorInstance>, Vec<GlyphInstance>)>,
    monitor_instance_ranges: HashMap<u32, MonitorInstanceRange>,
    variable_draw_batches: Vec<VariableDrawBatch>,
    variable_draw_order: Vec<u32>,
    variable_shadow_batch_target: Option<u32>,
    bubble_shape_instances: wgpu::Buffer,
    bubble_shape_capacity: u64,
    bubble_shape_count: u32,
    bubble_transforms: wgpu::Buffer,
    bubble_transform_capacity: u64,
    bubble_transform_bind_group: wgpu::BindGroup,
    bubble_glyph_instances: wgpu::Buffer,
    bubble_glyph_capacity: u64,
    bubble_glyph_count: u32,
    bubble_glyph_bind_group: wgpu::BindGroup,
    glyph_atlas_texture: wgpu::Texture,
    uploaded_glyph_atlas_size: u32,
    uploaded_glyph_atlas_revision: u64,
    readout_vertices: wgpu::Buffer,
    readout_vertex_capacity: u64,
    readout_vertex_count: u32,
    readout_text: String,
    readout_refreshed_at: Option<Instant>,
    gpu_frame_time_ms: Option<f32>,
    gpu_timing: Option<GpuTiming>,
    pen_instances: wgpu::Buffer,
    pen_instance_capacity: usize,
    pen_instance_data: Vec<PenInstance>,
    pen_uploaded_count: usize,
    pending_pen_ops: Vec<PendingPenOp>,
    pen_texture_dirty: bool,
    pen_clear_pending: bool,
    hd_pen_mode: bool,
    dev_mode: bool,
    frametime_mode: bool,
    cpu_frame_time_ms: Option<f32>,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PenInstance {
    start: [f32; 2],
    end: [f32; 2],
    color: [f32; 4],
    diameter: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SpriteTransform {
    position: [f32; 2],
    logical_size: [f32; 2],
    rotation_center: [f32; 2],
    direction: f32,
    scale: f32,
    visible: f32,
    padding: [f32; 3],
    effects_0: [f32; 4],
    effects_1: [f32; 4],
}

struct SpriteCostumeMip {
    texture: RenderTexture,
    opaque_pixels: Vec<u8>,
    texture_bind_group: wgpu::BindGroup,
}

struct BackdropAsset {
    asset_id: String,
    sprite: Arc<Sprite>,
}

struct CachedBackdropTexture {
    _texture: RenderTexture,
    bind_group: wgpu::BindGroup,
    last_used: u64,
    bytes: usize,
}

#[derive(Clone)]
struct SpriteCostumeRenderer {
    sprite: Arc<Sprite>,
    asset_id: String,
    mips: HashMap<i8, Arc<SpriteCostumeMip>>,
    selected_mip: i8,
    logical_size: [f32; 2],
    rotation_center: [f32; 2],
}

struct SpriteRenderer {
    target_name: String,
    target_index: usize,
    instance_id: usize,
    original: bool,
    layer_order: u32,
    costumes: Vec<SpriteCostumeRenderer>,
    current_costume: usize,
    transform: SpriteTransform,
    transform_buffer: wgpu::Buffer,
    transform_bind_group: wgpu::BindGroup,
}

struct PendingSpriteStamp {
    sprite_index: usize,
    costume_index: usize,
    // preserve the pen-surface mip chosen for this captured stamp.
    mip_level: i8,
    transform: SpriteTransform,
}

// preserve the ordering barriers between pen strokes, stamps, and clear operations.
enum PendingPenOp {
    Strokes { start: u32, end: u32 },
    Stamp(usize),
    Clear,
}

struct GpuTiming {
    query_set: wgpu::QuerySet,
    resolve_buffer: wgpu::Buffer,
    readback_buffers: Vec<wgpu::Buffer>,
    pending_slots: Vec<bool>,
    sender: Sender<(usize, bool)>,
    receiver: Receiver<(usize, bool)>,
}

#[derive(Clone, Copy, Debug, Default)]
struct VariableDrawBatch {
    monitor_id: Option<u32>,
    shape_start: u32,
    shape_count: u32,
    glyph_start: u32,
    glyph_count: u32,
}

fn changed_glyph_ranges(old: &[GlyphInstance], new: &[GlyphInstance]) -> Vec<(usize, usize)> {
    changed_instance_ranges(old, new)
}

fn changed_instance_ranges<T: bytemuck::Pod>(old: &[T], new: &[T]) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut start = None;
    for index in 0..new.len() {
        let unchanged = old.get(index).is_some_and(|previous| {
            bytemuck::bytes_of(previous) == bytemuck::bytes_of(&new[index])
        });
        if !unchanged && start.is_none() {
            start = Some(index);
        }
        if unchanged && start.is_some() {
            ranges.push((start.take().unwrap(), index));
        }
    }
    if let Some(start) = start {
        ranges.push((start, new.len()));
    }
    ranges
}

#[derive(Clone, Copy, Debug, Default)]
struct MonitorInstanceRange {
    shape_start: u32,
    shape_count: u32,
    glyph_start: u32,
    glyph_count: u32,
}

fn shadow_precedes_monitor(
    shadow_visible: bool,
    shadow_target: Option<u32>,
    monitor_id: u32,
) -> bool {
    shadow_visible && shadow_target == Some(monitor_id)
}

fn flush_variable_group(
    group_ids: &mut Vec<u32>,
    monitor_ranges: &HashMap<u32, MonitorInstanceRange>,
    batches: &mut Vec<VariableDrawBatch>,
    monitor_id: Option<u32>,
) {
    if group_ids.is_empty() {
        return;
    }
    let first_range = monitor_ranges
        .get(&group_ids[0])
        .copied()
        .unwrap_or_default();
    let shape_count = group_ids
        .iter()
        .filter_map(|id| monitor_ranges.get(id))
        .map(|range| range.shape_count)
        .sum();
    let glyph_count = group_ids
        .iter()
        .filter_map(|id| monitor_ranges.get(id))
        .map(|range| range.glyph_count)
        .sum();
    group_ids.clear();
    batches.push(VariableDrawBatch {
        monitor_id,
        shape_start: first_range.shape_start,
        shape_count,
        glyph_start: first_range.glyph_start,
        glyph_count,
    });
}

#[cfg(test)]
mod gpu_variable_tests {
    use super::changed_glyph_ranges;
    use crate::graphics::pipelines::{GlyphInstance, GlyphMetadata, MonitorInstance};

    fn glyph(glyph_id: u32) -> GlyphInstance {
        GlyphInstance {
            local_position: [1.0, 2.0],
            glyph_id,
            monitor_id: 3,
            color: [1.0; 4],
            scale: 1.0,
            clip_y: [0.0; 2],
        }
    }

    #[test]
    fn same_length_text_change_updates_only_changed_glyph_range() {
        let old = [glyph(1), glyph(2), glyph(3), glyph(4)];
        let new = [glyph(1), glyph(2), glyph(8), glyph(4)];
        assert_eq!(changed_glyph_ranges(&old, &new), vec![(2, 3)]);
    }

    #[test]
    fn appended_text_updates_only_the_new_tail() {
        let old = [glyph(1), glyph(2)];
        let new = [glyph(1), glyph(2), glyph(3)];
        assert_eq!(changed_glyph_ranges(&old, &new), vec![(2, 3)]);
    }

    #[test]
    fn gpu_instance_layouts_match_shader_storage() {
        assert_eq!(std::mem::size_of::<MonitorInstance>(), 96);
        assert_eq!(std::mem::size_of::<GlyphInstance>(), 44);
        assert_eq!(std::mem::size_of::<GlyphMetadata>(), 32);
    }

    #[test]
    fn drag_shadow_is_inserted_immediately_before_its_monitor() {
        assert!(super::shadow_precedes_monitor(true, Some(7), 7));
        assert!(!super::shadow_precedes_monitor(true, Some(7), 3));
        assert!(!super::shadow_precedes_monitor(false, Some(7), 7));
    }
}
