use super::*;
use crate::runtime::{RuntimeSnapshot, variable_snapshot_key};

struct SpriteAsset {
    target_name: String,
    target_index: usize,
    layer_order: u32,
    costumes: Vec<SpriteCostumeAsset>,
    current_costume: usize,
    transform: SpriteTransform,
}

struct SpriteCostumeAsset {
    sprite: Sprite,
    texture: RenderTexture,
    opaque_pixels: Vec<u8>,
    logical_size: [f32; 2],
    rotation_center: [f32; 2],
    raster_scale: f32,
}

fn sprite_costume_metrics(
    target: &aida_ast::parser::ScratchTarget,
    costume_index: usize,
    sprite: &Sprite,
) -> ([f32; 2], [f32; 2]) {
    let costume = target.costumes.get(costume_index);
    let resolution = costume
        .and_then(|costume| costume.bitmap_resolution)
        .unwrap_or(1.0)
        .max(1.0) as f32;
    let logical_size = sprite.logical_size(resolution);
    let default_x = logical_size[0] * 0.5;
    let default_y = logical_size[1] * 0.5;
    (
        logical_size,
        [
            costume.map_or(default_x, |costume| {
                costume.rotation_center_x as f32 / resolution
            }),
            costume.map_or(default_y, |costume| {
                costume.rotation_center_y as f32 / resolution
            }),
        ],
    )
}

fn sprite_transform(
    target: &aida_ast::parser::ScratchTarget,
    logical_size: [f32; 2],
    rotation_center: [f32; 2],
) -> SpriteTransform {
    SpriteTransform {
        position: [target.x as f32, target.y as f32],
        logical_size,
        rotation_center,
        direction: target.direction as f32,
        scale: target.size as f32 / 100.0,
        visible: if target.visible { 1.0 } else { 0.0 },
        padding: [0.0; 3],
        effects_0: [0.0; 4],
        effects_1: [0.0; 4],
    }
}

fn load_project_costumes(
    target: &aida_ast::parser::ScratchTarget,
    assets_dir: &std::path::Path,
) -> Result<Vec<(Sprite, [f32; 2], [f32; 2])>, String> {
    if target.costumes.is_empty() {
        let sprite = Sprite::from_bytes(include_bytes!("../../../resources/costume1.svg"))?;
        let (size, center) = sprite_costume_metrics(target, 0, &sprite);
        return Ok(vec![(sprite, size, center)]);
    }
    target
        .costumes
        .iter()
        .enumerate()
        .map(|(index, costume)| {
            let bytes = std::fs::read(assets_dir.join(&costume.md5ext))
                .map_err(|error| format!("failed to read costume `{}`: {error}", costume.name))?;
            let sprite = Sprite::from_bytes(&bytes).map_err(|error| {
                format!(
                    "failed to load costume `{}` on sprite `{}`: {error}",
                    costume.name, target.name
                )
            })?;
            let (size, center) = sprite_costume_metrics(target, index, &sprite);
            Ok((sprite, size, center))
        })
        .collect()
}

fn load_project_backdrop(
    project: &aida_ast::parser::ScratchProject,
    assets_dir: &std::path::Path,
) -> Result<Option<Sprite>, String> {
    let Some(stage) = project.targets.iter().find(|target| target.is_stage) else {
        return Ok(None);
    };
    let Some(costume) = stage.costumes.get(stage.current_costume) else {
        return Ok(None);
    };
    let bytes = std::fs::read(assets_dir.join(&costume.md5ext))
        .map_err(|error| format!("failed to read backdrop `{}`: {error}", costume.name))?;
    Sprite::from_bytes(&bytes)
        .map(Some)
        .map_err(|error| format!("failed to load backdrop `{}`: {error}", costume.name))
}

fn project_monitors(project: &aida_ast::parser::ScratchProject) -> Vec<VariableMonitor> {
    use aida_ast::parser::ScratchMonitor;

    let monitor_details = |monitor: &ScratchMonitor| match monitor.opcode.as_str() {
        "sensing_timer" => ("timer".to_owned(), "timer".to_owned()),
        "sensing_mousex" => ("mouse x".to_owned(), "__input/mouse_x".to_owned()),
        "sensing_mousey" => ("mouse y".to_owned(), "__input/mouse_y".to_owned()),
        _ => {
            let name = monitor
                .params
                .get("VARIABLE")
                .and_then(serde_json::Value::as_str)
                .or_else(|| {
                    monitor
                        .params
                        .get("LIST")
                        .and_then(serde_json::Value::as_str)
                })
                .unwrap_or("variable")
                .to_owned();
            let id = monitor.id.as_str();
            let target = project.targets.iter().find(|target| {
                monitor
                    .sprite_name
                    .as_deref()
                    .map_or(target.is_stage, |sprite| target.name == sprite)
                    && (target.variables.contains_key(id) || target.lists.contains_key(id))
            });
            let key = target.map_or(name.clone(), |target| {
                let variable_name = target
                    .variables
                    .get(id)
                    .map(|variable| variable.name.as_str())
                    .or_else(|| target.lists.get(id).map(|list| list.name.as_str()))
                    .unwrap_or(&name);
                variable_snapshot_key(
                    &target.name,
                    variable_name,
                    id,
                    (!target.is_stage).then_some(
                        project
                            .targets
                            .iter()
                            .position(|candidate| std::ptr::eq(candidate, target))
                            .unwrap_or(0),
                    ),
                )
            });
            (name, key)
        }
    };
    project
        .monitors
        .iter()
        .map(|monitor| {
            let (name, value_key) = monitor_details(monitor);
            let value = monitor
                .value
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| monitor.value.to_string());
            let mode = match monitor.mode.as_str() {
                "large" => MonitorDisplayMode::ValueOnly,
                "slider" => MonitorDisplayMode::Slider,
                _ => MonitorDisplayMode::Expanded,
            };
            let category = if monitor.opcode.starts_with("sensing_") {
                MonitorCategory::Sensing
            } else {
                MonitorCategory::Variables
            };
            VariableMonitor::new(
                name,
                value,
                monitor.x as f32 - SCRATCH_WIDTH as f32 * 0.5,
                SCRATCH_HEIGHT as f32 * 0.5 - monitor.y as f32,
            )
            .with_display_mode(mode)
            .with_category(category)
            .with_value_key(value_key)
            .with_visibility(monitor.visible)
            .with_slider_range(monitor.slider_min, monitor.slider_max)
        })
        .collect()
}

impl GraphicsState {
    pub(crate) async fn new(
        window: Arc<Window>,
        frametime_mode: bool,
        dev_mode: bool,
        project: Arc<aida_ast::parser::ScratchProject>,
        initial_snapshot: RuntimeSnapshot,
        assets_dir: &std::path::Path,
    ) -> Result<Self, String> {
        let instance = wgpu::Instance::default();
        let surface = instance
            .create_surface(window.clone())
            .map_err(|error| format!("failed to create a rendering surface: {error}"))?;

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
            .map_err(|error| format!("no compatible graphics adapter found: {error}"))?;

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
            .map_err(|error| format!("failed to create a graphics device: {error}"))?;

        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .ok_or_else(|| "the selected adapter cannot render to this surface".to_owned())?;
        if let Some(format) = surface
            .get_capabilities(&adapter)
            .formats
            .into_iter()
            .find(wgpu::TextureFormat::is_srgb)
        {
            config.format = format;
        }
        config.present_mode = wgpu::PresentMode::AutoVsync;
        surface.configure(&device, &config);

        let viewport = StageViewport::for_window(size.width, size.height); // calculate the centered 4:3 stage viewport

        let (ui_width, ui_height) = viewport
            .map(|viewport| (viewport.width, viewport.height))
            .unwrap_or((1, 1));

        // the sprite's scratch-space dimensions stay fixed while vector assets
        // rasterize at the current stage pixel density. backdrop and pen follow
        // the native/hd mode.
        let backdrop_texture =
            RenderTexture::new(&device, "backdrop texture", SCRATCH_WIDTH, SCRATCH_HEIGHT);
        let backdrop_sprite = load_project_backdrop(&project, assets_dir)?;
        if let Some(backdrop) = &backdrop_sprite {
            backdrop.upload_backdrop_to(&queue, &backdrop_texture)?;
        }
        let mut sprite_targets: Vec<_> = project
            .targets
            .iter()
            .enumerate()
            .filter(|(_, target)| !target.is_stage)
            .collect();
        sprite_targets.sort_by_key(|(_, target)| target.layer_order);
        let mut sprite_assets = Vec::with_capacity(sprite_targets.len());
        for (target_index, target) in sprite_targets {
            let costume_assets = load_project_costumes(target, assets_dir)?;
            let costumes = costume_assets
                .into_iter()
                .map(|(sprite, logical_size, rotation_center)| {
                    let viewport_width = viewport.map_or(SCRATCH_WIDTH, |viewport| viewport.width);
                    let viewport_height =
                        viewport.map_or(SCRATCH_HEIGHT, |viewport| viewport.height);
                    let mip_level = sprite.mip_level_for_projected_size(
                        target.size as f32 / 100.0,
                        viewport_width,
                        viewport_height,
                        logical_size,
                        device.limits().max_texture_dimension_2d,
                    );
                    let raster_scale = Sprite::raster_scale_for_mip(mip_level);
                    let (width, height) = sprite.texture_size(raster_scale);
                    let texture =
                        RenderTexture::new(&device, "sprite costume texture", width, height);
                    let opaque_pixels = sprite.upload_costume_to(&queue, &texture, raster_scale)?;
                    Ok(SpriteCostumeAsset {
                        sprite,
                        texture,
                        opaque_pixels,
                        logical_size,
                        rotation_center,
                        raster_scale,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            let current_costume = target.current_costume.min(costumes.len() - 1);
            let selected = &costumes[current_costume];
            let transform =
                sprite_transform(target, selected.logical_size, selected.rotation_center);
            sprite_assets.push(SpriteAsset {
                target_name: target.name.clone(),
                target_index,
                layer_order: target.layer_order,
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
        let mut variable_layer = VariableLayer::new(project_monitors(&project))?;
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
            label: Some("linear HD stage sampler"),
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
                    .map(|costume| {
                        let texture_bind_group = layer_bind_group(
                            &device,
                            &texture_layout,
                            &costume.texture,
                            &costume.texture,
                            &stage_nearest_sampler,
                            &zero_uv_offset,
                        );
                        SpriteCostumeRenderer {
                            sprite: costume.sprite,
                            mips: HashMap::from([(
                                costume.raster_scale.log2().round() as i8,
                                SpriteCostumeMip {
                                    texture: costume.texture,
                                    opaque_pixels: costume.opaque_pixels,
                                    texture_bind_group,
                                },
                            )]),
                            selected_mip: costume.raster_scale.log2().round() as i8,
                            logical_size: costume.logical_size,
                            rotation_center: costume.rotation_center,
                        }
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
        let backdrop_bind_group = layer_bind_group(
            &device,
            &texture_layout,
            &backdrop_texture,
            &backdrop_texture,
            &stage_nearest_sampler,
            &zero_uv_offset,
        );
        let pen_bind_group = layer_bind_group(
            &device,
            &texture_layout,
            &pen_texture,
            &pen_texture,
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
            size: gpu_monitor_capacity
                * SHAPES_PER_MONITOR
                * std::mem::size_of::<MonitorInstance>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let glyph_instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("variable glyph instances"),
            size: gpu_monitor_capacity
                * GLYPHS_PER_MONITOR
                * std::mem::size_of::<GlyphInstance>() as u64,
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
        let backdrop_dirty = backdrop_sprite.is_none();
        let mut state = Self {
            window,
            surface,
            device,
            queue,
            config,
            viewport,
            backdrop_texture,
            pen_texture,
            ui_texture,
            drag_shadow_texture,
            transparent_texture,
            backdrop_bind_group,
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
            backdrop_sprite,
            sprites,
            sprite_indices,
            sprite_instance_indices,
            sprite_transform_layout,
            variable_layer,
            fading_shadow_index: None,
            fading_shadow_bounds: None,
            drag_shadow_texture_rect: None,
            pen_pipeline,
            stamp_sprite_pipeline,
            stamp_transform_layout,
            stamp_transform_buffer,
            stamp_transform_bind_group,
            stamp_transform_capacity,
            stamp_transform_stride,
            pending_sprite_stamps: Vec::new(),
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
            gpu_monitor_capacity,
            glyph_instances,
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
            pen_instance_data: Vec::new(),
            pen_uploaded_count: 0,
            pen_draw_range: None,
            pen_texture_dirty: true,
            pen_clear_pending: true,
            hd_pen_mode: false,
            backdrop_dirty,
            dev_mode,
            frametime_mode,
            cpu_frame_time_ms: None,
        };
        state.sync_runtime_snapshot(&initial_snapshot)?;
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
