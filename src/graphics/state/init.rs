use super::*;
use crate::runtime::{RuntimeSnapshot, list_snapshot_key, variable_snapshot_key};

struct SpriteAsset {
    target_name: String,
    target_index: usize,
    layer_order: u32,
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
    sources: &mut HashMap<String, Arc<Sprite>>,
) -> Result<Vec<(Arc<Sprite>, String, [f32; 2], [f32; 2])>, String> {
    if target.costumes.is_empty() {
        let asset_id = "__aida/default-costume1.svg".to_owned();
        let sprite = sources
            .entry(asset_id.clone())
            .or_insert_with(|| {
                Sprite::from_bytes(include_bytes!("../../../resources/costume1.svg"))
                    .map(Arc::new)
                    .expect("built-in costume should parse")
            })
            .clone();
        let (size, center) = sprite_costume_metrics(target, 0, &sprite);
        return Ok(vec![(sprite, asset_id, size, center)]);
    }
    target
        .costumes
        .iter()
        .enumerate()
        .map(|(index, costume)| {
            let asset_id = if costume.md5ext.is_empty() {
                format!("{}::{}", target.name, costume.name)
            } else {
                costume.md5ext.clone()
            };
            let sprite = if let Some(sprite) = sources.get(&asset_id) {
                Arc::clone(sprite)
            } else {
                let bytes = std::fs::read(assets_dir.join(&costume.md5ext)).map_err(|error| {
                    format!("failed to read costume `{}`: {error}", costume.name)
                })?;
                let sprite = Arc::new(Sprite::from_bytes(&bytes).map_err(|error| {
                    format!(
                        "failed to load costume `{}` on sprite `{}`: {error}",
                        costume.name, target.name
                    )
                })?);
                sources.insert(asset_id.clone(), Arc::clone(&sprite));
                sprite
            };
            let (size, center) = sprite_costume_metrics(target, index, &sprite);
            Ok((sprite, asset_id, size, center))
        })
        .collect()
}

fn load_project_backdrops(
    project: &aida_ast::parser::ScratchProject,
    assets_dir: &std::path::Path,
    sources: &mut HashMap<String, Arc<Sprite>>,
) -> Result<Vec<BackdropAsset>, String> {
    let Some(stage) = project.targets.iter().find(|target| target.is_stage) else {
        return Ok(Vec::new());
    };
    stage
        .costumes
        .iter()
        .map(|costume| {
            let asset_id = costume.md5ext.clone();
            let sprite = if let Some(sprite) = sources.get(&asset_id) {
                Arc::clone(sprite)
            } else {
                let bytes = std::fs::read(assets_dir.join(&costume.md5ext)).map_err(|error| {
                    format!("failed to read backdrop `{}`: {error}", costume.name)
                })?;
                let sprite = Arc::new(Sprite::from_bytes(&bytes).map_err(|error| {
                    format!("failed to load backdrop `{}`: {error}", costume.name)
                })?);
                sources.insert(asset_id.clone(), Arc::clone(&sprite));
                sprite
            };
            Ok(BackdropAsset { asset_id, sprite })
        })
        .collect()
}

fn project_monitors(project: &aida_ast::parser::ScratchProject) -> Vec<VariableMonitor> {
    use super::MonitorKind;

    project
        .monitors
        .iter()
        .map(|monitor| {
            let id = monitor.id.as_str();
            let target = project.targets.iter().find(|target| {
                monitor
                    .sprite_name
                    .as_deref()
                    .map_or(target.is_stage, |sprite| target.name == sprite)
                    && (target.variables.contains_key(id) || target.lists.contains_key(id))
            });
            let variable = target.and_then(|target| target.variables.get(id));
            let list = target.and_then(|target| target.lists.get(id));
            let fallback_name = monitor
                .params
                .get("VARIABLE")
                .or_else(|| monitor.params.get("LIST"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("reporter");
            let is_list = monitor.opcode == "data_listcontents" || list.is_some();
            let kind = if is_list {
                MonitorKind::List
            } else if monitor.opcode == "data_variable" || variable.is_some() {
                MonitorKind::ScalarVariable
            } else {
                MonitorKind::Reporter
            };
            let base_name = variable
                .map(|variable| variable.name.as_str())
                .or_else(|| list.map(|list| list.name.as_str()))
                .unwrap_or(fallback_name);
            let name = if kind == MonitorKind::ScalarVariable
                && target.is_some_and(|target| !target.is_stage)
            {
                format!(
                    "{}: {}",
                    target.map_or("", |target| target.name.as_str()),
                    base_name
                )
            } else if kind == MonitorKind::ScalarVariable || kind == MonitorKind::List {
                base_name.to_owned()
            } else {
                monitor_label(&monitor.opcode, &monitor.params)
            };
            let value_key = if let Some(target) = target {
                let target_index = project
                    .targets
                    .iter()
                    .position(|item| std::ptr::eq(item, target))
                    .unwrap_or(0);
                if let Some(variable) = variable {
                    variable_snapshot_key(
                        &target.name,
                        &variable.name,
                        id,
                        (!target.is_stage).then_some(target_index),
                    )
                } else if let Some(list) = list {
                    list_snapshot_key(
                        &target.name,
                        &list.name,
                        id,
                        (!target.is_stage).then_some(target_index),
                    )
                } else {
                    format!("__reporter/{}/{}", monitor.opcode, monitor.id)
                }
            } else {
                match monitor.opcode.as_str() {
                    "sensing_timer" => "timer".to_owned(),
                    "sensing_mousex" => "__input/mouse_x".to_owned(),
                    "sensing_mousey" => "__input/mouse_y".to_owned(),
                    "looks_backdropnumbername" => match monitor
                        .params
                        .get("NUMBER_NAME")
                        .and_then(serde_json::Value::as_str)
                    {
                        Some("number") => "__stage/backdrop/number".to_owned(),
                        _ => "__stage/backdrop/name".to_owned(),
                    },
                    _ => format!("__reporter/{}/{}", monitor.opcode, monitor.id),
                }
            };
            let value = monitor
                .value
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| monitor.value.to_string());
            let mode = match monitor.mode.as_str() {
                "large" => MonitorDisplayMode::ValueOnly,
                "slider" => MonitorDisplayMode::Slider,
                "list" => MonitorDisplayMode::List,
                _ => MonitorDisplayMode::Expanded,
            };
            let category = monitor_category(&monitor.opcode, kind);
            let mut result = VariableMonitor::new(
                name,
                value,
                monitor.x as f32 - SCRATCH_WIDTH as f32 * 0.5,
                SCRATCH_HEIGHT as f32 * 0.5 - monitor.y as f32,
            )
            .with_display_mode(mode)
            .with_category(category)
            .with_kind(kind)
            .with_value_key(value_key)
            .with_visibility(monitor.visible)
            .with_slider_range(monitor.slider_min, monitor.slider_max);
            if let Some(list) = list {
                result = result.with_list_items(
                    list.items.iter().map(project_value_text).collect(),
                    if monitor.width > 0.0 {
                        monitor.width as f32
                    } else {
                        110.0
                    },
                    if monitor.height > 0.0 {
                        monitor.height as f32
                    } else {
                        200.0
                    },
                );
            }
            result
        })
        .collect()
}

#[cfg(test)]
mod monitor_project_tests {
    use super::{monitor_category, monitor_label, project_monitors};
    use crate::graphics::state::{MonitorCategory, MonitorDisplayMode, MonitorKind};

    #[test]
    fn project_monitors_keep_owner_scope_category_and_list_mode() {
        let project: aida_ast::parser::ScratchProject = serde_json::from_str(
            r#"{
                "targets": [
                    {"name":"Stage","isStage":true,"variables":{"global-id":["score",7]},"lists":{"stage-list":["stage items",["a","b"]]}},
                    {"name":"Sprite1","isStage":false,"variables":{"local-id":["score",5]}},
                    {"name":"Sprite2","isStage":false,"variables":{"local-id-2":["score",9]}}
                ],
                "monitors": [
                    {"id":"local-id","mode":"default","opcode":"data_variable","params":{"VARIABLE":"score"},"spriteName":"Sprite1","value":5,"x":10,"y":20,"visible":true},
                    {"id":"local-id-2","mode":"default","opcode":"data_variable","params":{"VARIABLE":"score"},"spriteName":"Sprite2","value":9,"x":10,"y":60,"visible":true},
                    {"id":"global-id","mode":"default","opcode":"data_variable","params":{"VARIABLE":"score"},"spriteName":null,"value":7,"x":10,"y":100,"visible":true},
                    {"id":"stage-list","mode":"list","opcode":"data_listcontents","params":{"LIST":"stage items"},"spriteName":null,"value":2,"width":120,"height":150,"visible":true},
                    {"id":"plus","mode":"large","opcode":"operator_add","params":{},"spriteName":null,"value":3,"visible":true}
                ]
            }"#,
        )
        .unwrap();
        let monitors = project_monitors(&project);
        assert_eq!(monitors[0].name, "Sprite1: score");
        assert_eq!(monitors[1].name, "Sprite2: score");
        assert_eq!(monitors[2].name, "score");
        assert_ne!(monitors[0].value_key, monitors[1].value_key);
        assert_eq!(monitors[3].kind, MonitorKind::List);
        assert_eq!(monitors[3].category, MonitorCategory::Lists);
        assert_eq!(monitors[3].display_mode, MonitorDisplayMode::List);
        assert_eq!(monitors[3].width, 120.0);
        assert_eq!(monitors[3].height, 150.0);
        assert_eq!(monitors[4].kind, MonitorKind::Reporter);
        assert_eq!(monitors[4].category, MonitorCategory::Operators);
        assert_eq!(monitors[4].display_mode, MonitorDisplayMode::ValueOnly);
    }

    #[test]
    fn reporter_categories_cover_scratch_core_groups_and_extensions() {
        let cases = [
            ("motion_xposition", MonitorCategory::Motion),
            ("looks_size", MonitorCategory::Looks),
            ("sound_volume", MonitorCategory::Sound),
            ("event_example", MonitorCategory::Events),
            ("control_example", MonitorCategory::Control),
            ("sensing_timer", MonitorCategory::Sensing),
            ("operator_add", MonitorCategory::Operators),
            ("procedures_reporter", MonitorCategory::MyBlocks),
            ("pen_reporter", MonitorCategory::Extension([15, 189, 140])),
        ];
        for (opcode, expected) in cases {
            assert_eq!(monitor_category(opcode, MonitorKind::Reporter), expected);
        }
    }

    #[test]
    fn backdrop_monitor_label_includes_number_or_name_selector() {
        let number = std::collections::HashMap::from([(
            "NUMBER_NAME".to_owned(),
            serde_json::Value::String("number".to_owned()),
        )]);
        let name = std::collections::HashMap::from([(
            "NUMBER_NAME".to_owned(),
            serde_json::Value::String("name".to_owned()),
        )]);
        assert_eq!(
            monitor_label("looks_backdropnumbername", &number),
            "backdrop number"
        );
        assert_eq!(
            monitor_label("looks_backdropnumbername", &name),
            "backdrop name"
        );

        let project: aida_ast::parser::ScratchProject = serde_json::from_str(
            r#"{"targets":[{"name":"Stage","isStage":true}],"monitors":[
                {"id":"backdrop-number","mode":"default","opcode":"looks_backdropnumbername","params":{"NUMBER_NAME":"number"},"spriteName":null,"value":0,"visible":true},
                {"id":"backdrop-name","mode":"default","opcode":"looks_backdropnumbername","params":{"NUMBER_NAME":"name"},"spriteName":null,"value":"backdrop1","visible":true}
            ]}"#,
        )
        .unwrap();
        let monitors = project_monitors(&project);
        assert_eq!(monitors[0].value_key, "__stage/backdrop/number");
        assert_eq!(monitors[1].value_key, "__stage/backdrop/name");
    }
}

fn monitor_category(opcode: &str, kind: super::MonitorKind) -> super::MonitorCategory {
    use super::MonitorCategory;
    if kind == super::MonitorKind::List {
        return MonitorCategory::Lists;
    }
    if kind == super::MonitorKind::ScalarVariable {
        return MonitorCategory::Variables;
    }
    match opcode.split('_').next().unwrap_or("") {
        "motion" => MonitorCategory::Motion,
        "looks" => MonitorCategory::Looks,
        "sound" => MonitorCategory::Sound,
        "event" => MonitorCategory::Events,
        "control" => MonitorCategory::Control,
        "sensing" => MonitorCategory::Sensing,
        "operator" => MonitorCategory::Operators,
        "procedures" => MonitorCategory::MyBlocks,
        "pen" => MonitorCategory::Extension([15, 189, 140]),
        _ => MonitorCategory::Extension([15, 189, 140]),
    }
}

fn monitor_label(
    opcode: &str,
    params: &std::collections::HashMap<String, serde_json::Value>,
) -> String {
    if opcode == "looks_backdropnumbername" || opcode == "looks_costumenumbername" {
        let subject = if opcode == "looks_backdropnumbername" {
            "backdrop"
        } else {
            "costume"
        };
        let detail = match params
            .get("NUMBER_NAME")
            .and_then(serde_json::Value::as_str)
        {
            Some("number") => "number",
            Some("name") => "name",
            _ => return subject.to_owned(),
        };
        return format!("{subject} {detail}");
    }
    let label = match opcode {
        "motion_xposition" => "x position",
        "motion_yposition" => "y position",
        "motion_direction" => "direction",
        "looks_size" => "size",
        "sound_volume" => "volume",
        "sensing_answer" => "answer",
        "sensing_loudness" => "loudness",
        "sensing_timer" => "timer",
        "sensing_current" => "current",
        "sensing_mousex" => "mouse x",
        "sensing_mousey" => "mouse y",
        "sensing_mousedown" => "mouse down?",
        "sensing_keypressed" => "key pressed?",
        "sensing_username" => "username",
        "sensing_dayssince2000" => "days since 2000",
        _ => {
            return opcode
                .split_once('_')
                .map_or_else(|| opcode.to_owned(), |(_, rest)| rest.replace('_', " "));
        }
    };
    if opcode == "sensing_current" {
        if let Some(value) = params.get("CURRENTMENU") {
            return format!("current {value}");
        }
    }
    label.to_owned()
}

fn project_value_text(value: &serde_json::Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string())
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
        let mut asset_sources = HashMap::new();
        let backdrops = load_project_backdrops(&project, assets_dir, &mut asset_sources)?;
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
            let costume_assets = load_project_costumes(target, assets_dir, &mut asset_sources)?;
            let costumes = costume_assets
                .into_iter()
                .map(|(sprite, asset_id, logical_size, rotation_center)| {
                    Ok::<_, String>(SpriteCostumeAsset {
                        sprite,
                        asset_id,
                        logical_size,
                        rotation_center,
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
            dev_mode,
            frametime_mode,
            cpu_frame_time_ms: None,
        };
        for sprite_index in 0..state.sprites.len() {
            let costume_index = state.sprites[sprite_index].current_costume;
            state.ensure_sprite_costume_mip(sprite_index, costume_index)?;
        }
        state.prune_unused_costume_mips();
        state.ensure_backdrop_texture(state.backdrop_index)?;
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
