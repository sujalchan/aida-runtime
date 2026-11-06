use super::project::{
    LoadedCostumeAsset, ProjectAssetError, load_project_backdrops, load_project_costumes,
    project_monitors, sprite_transform,
};
use super::*;
use crate::runtime::RuntimeSnapshot;
use std::fmt;

#[derive(Debug)]
pub(crate) enum GraphicsInitError {
    Surface(String),
    Adapter(String),
    Device(String),
    UnsupportedSurface,
    ProjectAssets(ProjectAssetError),
    MonitorSetup(String),
    ResourceSetup(String),
}

impl fmt::Display for GraphicsInitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Surface(error) => {
                write!(formatter, "failed to create a rendering surface: {error}")
            }
            Self::Adapter(error) => {
                write!(formatter, "no compatible graphics adapter found: {error}")
            }
            Self::Device(error) => write!(formatter, "failed to create a graphics device: {error}"),
            Self::UnsupportedSurface => {
                write!(
                    formatter,
                    "the selected adapter cannot render to this surface"
                )
            }
            Self::ProjectAssets(error) => {
                write!(formatter, "failed to load project assets: {error}")
            }
            Self::MonitorSetup(error) => {
                write!(formatter, "failed to initialize monitors: {error}")
            }
            Self::ResourceSetup(error) => write!(
                formatter,
                "failed to initialize graphics resources: {error}"
            ),
        }
    }
}

impl std::error::Error for GraphicsInitError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::ProjectAssets(error) => Some(error),
            _ => None,
        }
    }
}

struct SpriteAsset {
    target_name: String,
    target_index: usize,
    layer_order: u32,
    // carry drag mode from the .sb3 target into its rendered drawable.
    draggable: bool,
    costumes: Vec<SpriteCostumeAsset>,
    current_costume: usize,
    transform: SpriteTransform,
}

struct SpriteCostumeAsset {
    sprite: Arc<Sprite>,
    asset_id: String,
    logical_size: [f32; 2],
    rotation_center: [f32; 2],
}

impl GraphicsState {
    pub(crate) async fn new(
        window: Arc<Window>,
        frametime_mode: bool,
        dev_mode: bool,
        project: Arc<aida_ast::parser::ScratchProject>,
        initial_snapshot: RuntimeSnapshot,
        assets_dir: &std::path::Path,
    ) -> Result<Self, GraphicsInitError> {
        let instance = wgpu::Instance::default();
        let surface = instance
            .create_surface(window.clone())
            .map_err(|error| GraphicsInitError::Surface(error.to_string()))?;

        let adapters = instance.enumerate_adapters(wgpu::Backends::all()).await;
        println!("Detected {} graphics adapter(s):", adapters.len());
        for (index, adapter) in adapters.iter().enumerate() {
            print_adapter(index, adapter);
        }

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await
            .map_err(|error| GraphicsInitError::Adapter(error.to_string()))?;

        let selected = adapter.get_info();
        println!(
            "Selected adapter: {} ({:?}, {:?})",
            selected.name, selected.device_type, selected.backend
        );

        let timestamp_queries_supported =
            frametime_mode && adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY);
        let requested_features = if timestamp_queries_supported {
            wgpu::Features::TIMESTAMP_QUERY
        } else {
            wgpu::Features::empty()
        };
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("Symphony wgpu device"),
                required_features: requested_features,
                ..Default::default()
            })
            .await
            .map_err(|error| GraphicsInitError::Device(error.to_string()))?;

        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .ok_or(GraphicsInitError::UnsupportedSurface)?;
        if let Some(format) = surface
            .get_capabilities(&adapter)
            .formats
            .into_iter()
            .find(wgpu::TextureFormat::is_srgb)
        {
            config.format = format;
        }
        // let the runtime's fps pacer own frame timing instead of blocking each update on vblank.
        config.present_mode = wgpu::PresentMode::AutoNoVsync;
        surface.configure(&device, &config);

        let viewport = StageViewport::for_window(size.width, size.height); // calculate the centered 4:3 stage viewport

        let (ui_width, ui_height) = viewport
            .map(|viewport| (viewport.width, viewport.height))
            .unwrap_or((1, 1));

        // the sprite's scratch-space dimensions stay fixed while vector assets
        // rasterize at the current stage pixel density. backdrop and pen follow
        // the native/hd mode.
        let mut asset_sources = HashMap::new();
        let backdrops = load_project_backdrops(&project, assets_dir, &mut asset_sources)
            .map_err(GraphicsInitError::ProjectAssets)?;
        let backdrop_index = project
            .targets
            .iter()
            .find(|target| target.is_stage)
            .map_or(0, |stage| {
                stage.current_costume.min(backdrops.len().saturating_sub(1))
            });
        let mut sprite_targets: Vec<_> = project
            .targets
            .iter()
            .enumerate()
            .filter(|(_, target)| !target.is_stage)
            .collect();
        sprite_targets.sort_by_key(|(_, target)| target.layer_order);
        let mut sprite_assets = Vec::with_capacity(sprite_targets.len());
        for (target_index, target) in sprite_targets {
            let costume_assets = load_project_costumes(target, assets_dir, &mut asset_sources)
                .map_err(GraphicsInitError::ProjectAssets)?;
            let costumes = costume_assets
                .into_iter()
                .map(|costume: LoadedCostumeAsset| SpriteCostumeAsset {
                    sprite: costume.sprite,
                    asset_id: costume.asset_id,
                    logical_size: costume.logical_size,
                    rotation_center: costume.rotation_center,
                })
                .collect::<Vec<_>>();
            let current_costume = target.current_costume.min(costumes.len() - 1);
            let selected = &costumes[current_costume];
            let transform =
                sprite_transform(target, selected.logical_size, selected.rotation_center);
            sprite_assets.push(SpriteAsset {
                target_name: target.name.clone(),
                target_index,
                layer_order: target.layer_order,
                draggable: target.draggable,
                costumes,
                current_costume,
                transform,
            });
        }
        let pen_texture = RenderTexture::new(&device, "pen texture", SCRATCH_WIDTH, SCRATCH_HEIGHT);

        // context menus and drag shadows are development tools. keep only
        // 1x1 fallback textures in normal mode; variable monitors render
        // directly into the stage pass and do not use these layers.
        let overlay_width = if dev_mode { ui_width } else { 1 };
        let overlay_height = if dev_mode { ui_height } else { 1 };
        let ui_texture = RenderTexture::new(&device, "ui texture", overlay_width, overlay_height);
        let drag_shadow_texture = RenderTexture::new(
            &device,
            "dragged variable shadow texture",
            overlay_width,
            overlay_height,
        );
        // this is only a transparent fallback sample for bind groups, so a
        // clamped 1x1 texture is sufficient at every viewport size.
        let transparent_texture = RenderTexture::new(&device, "transparent fallback", 1, 1);
        if dev_mode {
            drag_shadow_texture.clear(&queue);
        }
        transparent_texture.clear(&queue);
        let mut variable_layer = VariableLayer::new(project_monitors(&project))
            .map_err(GraphicsInitError::MonitorSetup)?;
        variable_layer.set_render_scale(
            viewport.map_or(1.0, |viewport| viewport.width as f32 / SCRATCH_WIDTH as f32),
        );
        let glyph_atlas_size = variable_layer.glyph_atlas_size();
        // variable monitors are drawn by the gpu into the final stage pass.
        // the texture remains for the context menu layer only.
        if dev_mode {
            ui_texture.clear(&queue);
        }

        let stage_nearest_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("nearest stage sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let stage_linear_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("linear stage sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let ui_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("linear ui sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let texture_layout = texture_layout(&device);
        let zero_uv_offset = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("zero layer UV offset"),
            contents: bytemuck::cast_slice(&[0.0_f32, 0.0, 0.0, 0.0]),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let ui_uv_offset = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("UI layer UV offset"),
            contents: bytemuck::cast_slice(&[0.0_f32, 0.0, 0.0, 0.0]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let sprite_transform_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("sprite transform layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    // sprite effects are evaluated per pixel in the fragment
                    // shader while the same uniform also drives the vertex transform.
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
        let sprites: Vec<SpriteRenderer> = sprite_assets
            .into_iter()
            .map(|asset| {
                let costumes = asset
                    .costumes
                    .into_iter()
                    .map(|costume| SpriteCostumeRenderer {
                        sprite: costume.sprite,
                        asset_id: costume.asset_id,
                        mips: HashMap::new(),
                        selected_mip: 0,
                        logical_size: costume.logical_size,
                        rotation_center: costume.rotation_center,
                    })
                    .collect();
                let transform_buffer =
                    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("Scratch sprite transform"),
                        contents: bytemuck::bytes_of(&asset.transform),
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    });
                let transform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Scratch sprite transform binding"),
                    layout: &sprite_transform_layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: transform_buffer.as_entire_binding(),
                    }],
                });
                SpriteRenderer {
                    target_name: asset.target_name,
                    target_index: asset.target_index,
                    instance_id: asset.target_index,
                    original: true,
                    layer_order: asset.layer_order,
                    draggable: asset.draggable,
                    costumes,
                    current_costume: asset.current_costume,
                    transform: asset.transform,
                    transform_buffer,
                    transform_bind_group,
                }
            })
            .collect();
        let sprite_indices = sprites
            .iter()
            .enumerate()
            .map(|(index, sprite)| (sprite.target_name.clone(), index))
            .collect();
        let sprite_instance_indices = sprites
            .iter()
            .enumerate()
            .map(|(index, sprite)| ((sprite.target_index, sprite.instance_id), index))
            .collect();
        let drag_shadow_uv_offset = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("drag shadow UV offset"),
            contents: bytemuck::cast_slice(&[2.0_f32, 2.0, 0.0, 0.0]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let pen_bind_group = layer_bind_group(
            &device,
            &texture_layout,
            &pen_texture,
            &pen_texture,
            // preserve scratch-pixel edges when the pen texture is composited.
            &stage_nearest_sampler,
            &zero_uv_offset,
        );
        let ui_bind_group = layer_bind_group(
            &device,
            &texture_layout,
            &ui_texture,
            &transparent_texture,
            &ui_sampler,
            &ui_uv_offset,
        );
        let drag_shadow_bind_group = layer_bind_group(
            &device,
            &texture_layout,
            &transparent_texture,
            &drag_shadow_texture,
            &ui_sampler,
            &drag_shadow_uv_offset,
        );
        let pen_pipeline = pen_pipeline(&device);
        let pen_dot_pipeline = pen_dot_pipeline(&device);
        let pen_composite_pipeline =
            premultiplied_composite_pipeline(&device, config.format, &texture_layout);
        let composite_pipeline = composite_pipeline(&device, config.format, &texture_layout);
        let sprite_render_pipeline = sprite_pipeline(
            &device,
            config.format,
            &texture_layout,
            &sprite_transform_layout,
            "fs_main",
            wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING,
        );
        let stamp_transform_size = std::mem::size_of::<SpriteTransform>() as u64;
        let stamp_transform_alignment = device.limits().min_uniform_buffer_offset_alignment as u64;
        let stamp_transform_stride =
            stamp_transform_size.div_ceil(stamp_transform_alignment) * stamp_transform_alignment;
        let stamp_transform_capacity = 64;
        let stamp_transform_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("pen stamp transform layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: NonZeroU64::new(stamp_transform_size),
                    },
                    count: None,
                }],
            });
        let stamp_transform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("GPU pen stamp transforms"),
            size: stamp_transform_stride * stamp_transform_capacity as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let stamp_transform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("GPU pen stamp transform binding"),
            layout: &stamp_transform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &stamp_transform_buffer,
                    offset: 0,
                    size: NonZeroU64::new(stamp_transform_size),
                }),
            }],
        });
        let stamp_sprite_pipeline = sprite_pipeline(
            &device,
            LAYER_FORMAT,
            &texture_layout,
            &stamp_transform_layout,
            "fs_stamp",
            wgpu::BlendState::ALPHA_BLENDING,
        );
        let variable_pipeline = variable_pipeline(&device, config.format);
        let (monitor_instance_pipeline, monitor_layout) =
            monitor_instance_pipeline(&device, config.format);
        let (glyph_instance_pipeline, glyph_layout) =
            glyph_instance_pipeline(&device, config.format);
        let gpu_monitor_capacity = 8u64;
        let gpu_shape_capacity = 1u64;
        let gpu_glyph_capacity = 1u64;
        let monitor_transforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("variable monitor transforms"),
            size: gpu_monitor_capacity * 16,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let glyph_metadata = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("variable glyph metadata"),
            size: std::mem::size_of::<GlyphMetadata>() as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let monitor_transform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("monitor transform bindings"),
            layout: &monitor_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: monitor_transforms.as_entire_binding(),
            }],
        });
        let monitor_instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("variable monitor instances"),
            size: gpu_shape_capacity * std::mem::size_of::<MonitorInstance>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let glyph_instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("variable glyph instances"),
            size: gpu_glyph_capacity * std::mem::size_of::<GlyphInstance>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let glyph_atlas_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("variable glyph alpha atlas"),
            size: wgpu::Extent3d {
                width: glyph_atlas_size,
                height: glyph_atlas_size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let glyph_atlas_view =
            glyph_atlas_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let glyph_atlas_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("variable glyph atlas sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let glyph_atlas_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("variable glyph atlas bindings"),
            layout: &glyph_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: monitor_transforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: glyph_metadata.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&glyph_atlas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&glyph_atlas_sampler),
                },
            ],
        });
        let bubble_transform_capacity = 16u64;
        let bubble_transforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("speech bubble transforms"),
            size: bubble_transform_capacity * 16,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bubble_transform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("speech bubble shape transforms"),
            layout: &monitor_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: bubble_transforms.as_entire_binding(),
            }],
        });
        let bubble_glyph_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("speech bubble glyph bindings"),
            layout: &glyph_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: bubble_transforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: glyph_metadata.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&glyph_atlas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&glyph_atlas_sampler),
                },
            ],
        });
        let bubble_shape_capacity = 32u64;
        let bubble_shape_instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("speech bubble shape instances"),
            size: bubble_shape_capacity * std::mem::size_of::<MonitorInstance>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bubble_glyph_capacity = 512u64;
        let bubble_glyph_instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("speech bubble glyph instances"),
            size: bubble_glyph_capacity * std::mem::size_of::<GlyphInstance>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let readout_vertices = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("GPU frametime readout vertices"),
            size: 4,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let gpu_timing = timestamp_queries_supported.then(|| create_gpu_timing(&device));

        let pen_instance_capacity = 256;
        let pen_instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("GPU pen stroke instances"),
            size: (pen_instance_capacity * std::mem::size_of::<PenInstance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let pen_dot_capacity = 256;
        let pen_dots = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("GPU pen dot instances"),
            size: (pen_dot_capacity * std::mem::size_of::<PenDotInstance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut state = Self {
            window,
            surface,
            device,
            queue,
            config,
            viewport,
            pen_texture,
            ui_texture,
            drag_shadow_texture,
            transparent_texture,
            pen_bind_group,
            ui_bind_group,
            drag_shadow_bind_group,
            texture_layout,
            zero_uv_offset,
            ui_uv_offset,
            drag_shadow_uv_offset,
            stage_nearest_sampler,
            stage_linear_sampler,
            ui_sampler,
            backdrops,
            backdrop_index,
            backdrop_texture_cache: HashMap::new(),
            backdrop_cache_clock: 0,
            shared_costume_mips: HashMap::new(),
            sprites,
            sprite_indices,
            sprite_instance_indices,
            last_runtime_layers: Vec::new(),
            project_timer_sample_millis: 0,
            project_timer_sampled_at: Instant::now(),
            project_timer_epoch: initial_snapshot.project_timer_epoch,
            sprite_transform_layout,
            variable_layer,
            fading_shadow_index: None,
            fading_shadow_bounds: None,
            drag_shadow_texture_rect: None,
            pen_pipeline,
            pen_dot_pipeline,
            stamp_sprite_pipeline,
            stamp_transform_layout,
            stamp_transform_buffer,
            stamp_transform_bind_group,
            stamp_transform_capacity,
            stamp_transform_stride,
            pending_sprite_stamps: Vec::new(),
            pending_stamp_uploaded_count: 0,
            pen_composite_pipeline,
            composite_pipeline,
            sprite_pipeline: sprite_render_pipeline,
            variable_pipeline,
            monitor_instance_pipeline,
            monitor_transform_layout: monitor_layout,
            monitor_transform_bind_group,
            glyph_instance_pipeline,
            glyph_layout,
            glyph_atlas_sampler,
            glyph_bind_group: glyph_atlas_bind_group,
            monitor_instances,
            gpu_shape_capacity,
            gpu_monitor_capacity,
            glyph_instances,
            gpu_glyph_capacity,
            monitor_transforms,
            glyph_metadata,
            glyph_metadata_capacity: 1,
            monitor_batches: HashMap::new(),
            monitor_instance_ranges: HashMap::new(),
            variable_draw_batches: Vec::new(),
            variable_draw_order: Vec::new(),
            variable_shadow_batch_target: None,
            bubble_shape_instances,
            bubble_shape_capacity,
            bubble_shape_count: 0,
            bubble_transforms,
            bubble_transform_capacity,
            bubble_transform_bind_group,
            bubble_glyph_instances,
            bubble_glyph_capacity,
            bubble_glyph_count: 0,
            bubble_glyph_bind_group,
            glyph_atlas_texture,
            uploaded_glyph_atlas_size: glyph_atlas_size,
            uploaded_glyph_atlas_revision: 0,
            readout_vertices,
            readout_vertex_capacity: 4,
            readout_vertex_count: 0,
            readout_text: String::new(),
            readout_refreshed_at: None,
            gpu_frame_time_ms: None,
            gpu_timing,
            pen_instances,
            pen_instance_capacity,
            pen_stroke_batches: Vec::new(),
            pen_instance_count: 0,
            pen_uploaded_count: 0,
            pen_dots,
            pen_dot_capacity,
            pen_dot_batches: Vec::new(),
            pen_dot_count: 0,
            pen_dot_uploaded_count: 0,
            pending_pen_ops: Vec::new(),
            pen_texture_dirty: true,
            pen_clear_pending: true,
            hd_pen_mode: false,
            dev_mode,
            frametime_mode,
            cpu_frame_time_ms: None,
        };
        for sprite_index in 0..state.sprites.len() {
            let costume_index = state.sprites[sprite_index].current_costume;
            state
                .ensure_sprite_costume_mip(sprite_index, costume_index)
                .map_err(GraphicsInitError::ResourceSetup)?;
        }
        state.prune_unused_costume_mips();
        state
            .ensure_backdrop_texture(state.backdrop_index)
            .map_err(GraphicsInitError::ResourceSetup)?;
        state
            .sync_runtime_snapshot(&initial_snapshot, Vec::new())
            .map_err(GraphicsInitError::ResourceSetup)?;
        Ok(state)
    }
}
fn create_gpu_timing(device: &Device) -> GpuTiming {
    let query_set = device.create_query_set(&wgpu::QuerySetDescriptor {
        label: Some("GPU frame timestamps"),
        ty: wgpu::QueryType::Timestamp,
        count: 2,
    });
    let resolve_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("GPU timestamp resolve buffer"),
        size: 16,
        usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback_buffers = (0..3)
        .map(|_| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("GPU timestamp readback buffer"),
                size: 16,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        })
        .collect();
    let (sender, receiver) = mpsc::channel();
    GpuTiming {
        query_set,
        resolve_buffer,
        readback_buffers,
        pending_slots: vec![false; 3],
        sender,
        receiver,
    }
}

fn print_adapter(index: usize, adapter: &Adapter) {
    let info = adapter.get_info();
    println!(
        "  [{index}] {}\ntype: {:?}\nbackend: {:?}\nvendor: {:#06x}\ndevice: {:#06x}\ndriver: {} ({})",
        info.name,
        info.device_type,
        info.backend,
        info.vendor,
        info.device,
        if info.driver.is_empty() {
            "unknown"
        } else {
            &info.driver
        },
        if info.driver_info.is_empty() {
            "no version information"
        } else {
            &info.driver_info
        }
    );
}
