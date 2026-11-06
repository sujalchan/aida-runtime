use super::*;
use crate::runtime::{RuntimeSnapshot, ScratchValue};

fn same_rendered_layer(
    left: &crate::runtime::LayerSnapshot,
    right: &crate::runtime::LayerSnapshot,
) -> bool {
    if left.target_index != right.target_index
        || left.instance_id != right.instance_id
        || left.original != right.original
        || left.target != right.target
        || left.visible != right.visible
        || left.bubble != right.bubble
    {
        return false;
    }
    if !left.visible && left.bubble.is_none() {
        return true;
    }
    left.x == right.x
        && left.y == right.y
        && left.direction == right.direction
        && left.rotation_style == right.rotation_style
        && left.size == right.size
        && left.costume_index == right.costume_index
}

pub(super) fn rendered_layers_changed(
    previous: &[crate::runtime::LayerSnapshot],
    current: &[crate::runtime::LayerSnapshot],
) -> bool {
    previous.len() != current.len()
        || previous
            .iter()
            .any(|old| !current.iter().any(|new| same_rendered_layer(old, new)))
}

pub(super) fn timer_sample_for_snapshot(
    current_epoch: u64,
    current_millis: u64,
    sampled_at: Instant,
    incoming_epoch: u64,
    incoming_millis: u64,
    now: Instant,
) -> (u64, u64) {
    if incoming_epoch != current_epoch {
        return (incoming_epoch, incoming_millis);
    }
    let estimated = current_millis.saturating_add(
        now.saturating_duration_since(sampled_at)
            .as_millis()
            .min(u64::MAX as u128) as u64,
    );
    (current_epoch, incoming_millis.max(estimated))
}

pub(super) fn apply_runtime_layer_transform(
    transform: &mut super::SpriteTransform,
    layer: &crate::runtime::LayerSnapshot,
) -> bool {
    let position = [layer.x as f32, layer.y as f32];
    let (rendered_direction, flip_x) = layer
        .rotation_style
        .rendered_direction_and_flip(layer.direction);
    let direction = rendered_direction as f32;
    let scale = layer.size as f32 / 100.0;
    let flip_x = flip_x as f32;
    let visible = if layer.visible { 1.0 } else { 0.0 };
    let changed = transform.position != position
        || transform.direction != direction
        || transform.scale != scale
        || transform.flip_x != flip_x
        || transform.visible != visible;
    if changed {
        transform.position = position;
        transform.direction = direction;
        transform.scale = scale;
        transform.flip_x = flip_x;
        transform.visible = visible;
    }
    changed
}

pub(super) fn sprite_monitor_updates(
    variable_layer: &super::VariableLayer,
    layers: &[crate::runtime::LayerSnapshot],
) -> Vec<(String, String)> {
    let mut updates = Vec::new();
    for layer in layers.iter().filter(|layer| layer.original) {
        for (property, value) in [
            ("x", crate::runtime::scratch_motion_coordinate(layer.x)),
            ("y", crate::runtime::scratch_motion_coordinate(layer.y)),
            ("direction", layer.direction),
            ("size", layer.size),
        ] {
            let key = format!("__sprite/{}/{property}", layer.target);
            if variable_layer.has_monitor_value_key(&key) {
                updates.push((key, value.to_string()));
            }
        }
    }
    updates
}

fn is_live_sensing_reporter(key: &str) -> bool {
    [
        "/sensing_current/",
        "/sensing_dayssince2000/",
        "/sensing_mousex/",
        "/sensing_mousey/",
        "/sensing_mousedown/",
    ]
    .iter()
    .any(|category| key.contains(category))
}

impl GraphicsState {
    // feed the renderer display cache from the scheduler's canonical snapshot.
    pub(crate) fn sync_runtime_snapshot(
        &mut self,
        snapshot: &RuntimeSnapshot,
        pen_commands: Vec<crate::runtime::PenCommand>,
    ) -> Result<bool, String> {
        // apply the scheduler's ordered pen operations before presenting this frame.
        // keep the redraw decision after moving the pen batch into renderer-owned storage.
        let has_pen_commands = !pen_commands.is_empty();
        self.write_pen_commands(pen_commands);
        let timer_sampled_at = Instant::now();
        (self.project_timer_epoch, self.project_timer_sample_millis) = timer_sample_for_snapshot(
            self.project_timer_epoch,
            self.project_timer_sample_millis,
            self.project_timer_sampled_at,
            snapshot.project_timer_epoch,
            snapshot.project_timer_millis,
            timer_sampled_at,
        );
        self.project_timer_sampled_at = timer_sampled_at;
        let mut monitor_visibility_changed = false;
        for variable in &snapshot.variables {
            monitor_visibility_changed |= self
                .variable_layer
                .set_monitor_visibility(&variable.key, variable.monitor_visible)
                .is_some();
        }
        for list in &snapshot.lists {
            monitor_visibility_changed |= self
                .variable_layer
                .set_monitor_visibility(&list.key, list.visible)
                .is_some();
        }
        let mut updates = snapshot
            .variables
            .iter()
            .map(|value| (value.key.clone(), scratch_value_text(&value.value)))
            .collect::<Vec<_>>();
        updates.extend(
            snapshot
                .reporters
                .iter()
                .filter(|reporter| !is_live_sensing_reporter(&reporter.key))
                .map(|reporter| (reporter.key.clone(), scratch_value_text(&reporter.value))),
        );
        updates.push((
            "__stage/backdrop/name".to_owned(),
            snapshot.backdrop_name.clone(),
        ));
        updates.push((
            "__stage/backdrop/number".to_owned(),
            (snapshot.backdrop_index + 1).to_string(),
        ));
        let mut lists_changed = false;
        for list in &snapshot.lists {
            lists_changed |= self
                .variable_layer
                .set_monitor_list(&list.key, Arc::clone(&list.items))
                .is_some_and(|(_, visible_changed)| visible_changed);
        }
        updates.extend(sprite_monitor_updates(
            &self.variable_layer,
            &snapshot.layers,
        ));
        let monitors_changed = self.write_changed_monitors(&updates)?;
        // Hidden sprites still publish their real positions in snapshots for
        // motion monitors and later show blocks, but position-only changes do
        // not dirty their GPU transforms or force sprite layer work.
        let layers_changed = rendered_layers_changed(&self.last_runtime_layers, &snapshot.layers);
        let backdrop_index = snapshot
            .backdrop_index
            .min(self.backdrops.len().saturating_sub(1));
        let backdrop_changed = backdrop_index != self.backdrop_index;
        if backdrop_changed {
            self.backdrop_index = backdrop_index;
            self.ensure_backdrop_texture(backdrop_index)?;
        }

        if layers_changed {
            let live_clones = snapshot
                .layers
                .iter()
                .filter(|layer| !layer.original)
                .map(|layer| layer.instance_id)
                .collect::<std::collections::HashSet<_>>();
            self.sprites
                .retain(|sprite| sprite.original || live_clones.contains(&sprite.instance_id));
            self.rebuild_sprite_indices();
            for layer in &snapshot.layers {
                let index = if layer.original {
                    self.sprite_indices.get(&layer.target).copied()
                } else {
                    if !self
                        .sprite_instance_indices
                        .contains_key(&(layer.target_index, layer.instance_id))
                    {
                        self.add_clone_renderer(layer)?;
                    }
                    self.sprite_instance_indices
                        .get(&(layer.target_index, layer.instance_id))
                        .copied()
                };
                if let Some(index) = index {
                    if !layer.visible
                        && layer.bubble.is_none()
                        && self.last_runtime_layers.iter().any(|previous| {
                            previous.target_index == layer.target_index
                                && previous.instance_id == layer.instance_id
                                && !previous.visible
                                && previous.bubble.is_none()
                        })
                    {
                        continue;
                    }
                    let sprite = &mut self.sprites[index];
                    if !sprite.costumes.is_empty() {
                        sprite.current_costume = layer.costume_index.min(sprite.costumes.len() - 1);
                    }
                    if apply_runtime_layer_transform(&mut sprite.transform, layer) {
                        self.queue.write_buffer(
                            &sprite.transform_buffer,
                            0,
                            bytemuck::bytes_of(&sprite.transform),
                        );
                    }
                }
            }
            for sprite_index in 0..self.sprites.len() {
                let current_costume = self.sprites[sprite_index].current_costume;
                self.ensure_sprite_costume_mip(sprite_index, current_costume)?;
            }
            self.prune_unused_costume_mips();
            self.sync_bubble_gpu_data(snapshot)?;
            self.last_runtime_layers.clone_from(&snapshot.layers);
        }
        Ok(monitors_changed
            || has_pen_commands
            || monitor_visibility_changed
            || lists_changed
            || layers_changed
            || backdrop_changed)
    }

    pub(crate) fn has_visible_live_monitor(&self) -> bool {
        self.variable_layer.has_visible_monitor_value_key("timer")
            || [
                "/sensing_current/",
                "/sensing_dayssince2000/",
                "/sensing_mousex/",
                "/sensing_mousey/",
                "/sensing_mousedown/",
            ]
            .iter()
            .any(|category| {
                self.variable_layer
                    .has_visible_monitor_key_containing(category)
            })
    }

    pub(crate) fn refresh_live_monitors(
        &mut self,
        snapshot: &RuntimeSnapshot,
        now: Instant,
        mouse_position: Option<(f64, f64)>,
    ) -> Result<bool, String> {
        let mut updates = Vec::new();
        for reporter in &snapshot.reporters {
            if is_live_sensing_reporter(&reporter.key)
                && self
                    .variable_layer
                    .has_visible_monitor_value_key(&reporter.key)
            {
                let live_mouse_value = mouse_position.and_then(|(x, y)| {
                    if reporter.key.contains("/sensing_mousex/") {
                        Some(x)
                    } else if reporter.key.contains("/sensing_mousey/") {
                        Some(y)
                    } else {
                        None
                    }
                });
                updates.push((
                    reporter.key.clone(),
                    live_mouse_value.map_or_else(
                        || scratch_value_text(&reporter.value),
                        |value| scratch_value_text(&ScratchValue::Number(value)),
                    ),
                ));
            }
        }
        if self.variable_layer.has_visible_monitor_value_key("timer") {
            let elapsed = now.saturating_duration_since(self.project_timer_sampled_at);
            let millis = self
                .project_timer_sample_millis
                .saturating_add(elapsed.as_millis().min(u64::MAX as u128) as u64);
            updates.push(("timer".to_owned(), format!("{:.3}", millis as f64 / 1000.0)));
        }
        self.write_changed_monitors(&updates)
    }

    fn add_clone_renderer(&mut self, layer: &crate::runtime::LayerSnapshot) -> Result<(), String> {
        let Some(original_index) = self.sprite_indices.get(&layer.target).copied() else {
            return Ok(());
        };
        let original = &self.sprites[original_index];
        let draggable = original.draggable;
        let costumes = original.costumes.clone();
        let mut transform = original.transform;
        let current_costume = layer
            .costume_index
            .min(original.costumes.len().saturating_sub(1));
        apply_runtime_layer_transform(&mut transform, layer);
        let transform_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Scratch clone transform"),
                contents: bytemuck::bytes_of(&transform),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });
        let transform_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Scratch clone transform binding"),
            layout: &self.sprite_transform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: transform_buffer.as_entire_binding(),
            }],
        });
        let layer_order = self
            .sprites
            .iter()
            .map(|sprite| sprite.layer_order)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        self.sprites.push(SpriteRenderer {
            target_name: layer.target.clone(),
            target_index: layer.target_index,
            instance_id: layer.instance_id,
            original: false,
            layer_order,
            draggable,
            costumes,
            current_costume,
            transform,
            transform_buffer,
            transform_bind_group,
        });
        self.rebuild_sprite_indices();
        let clone_index = self.sprite_instance_indices[&(layer.target_index, layer.instance_id)];
        for costume_index in 0..self.sprites[clone_index].costumes.len() {
            self.ensure_sprite_costume_mip(clone_index, costume_index)?;
        }
        Ok(())
    }

    fn rebuild_sprite_indices(&mut self) {
        self.sprites.sort_by_key(|sprite| {
            (
                sprite.layer_order,
                sprite.original as u8,
                sprite.instance_id,
            )
        });
        self.sprite_indices.clear();
        self.sprite_instance_indices.clear();
        for (index, sprite) in self.sprites.iter().enumerate() {
            if sprite.original {
                self.sprite_indices
                    .insert(sprite.target_name.clone(), index);
            }
            self.sprite_instance_indices
                .insert((sprite.target_index, sprite.instance_id), index);
        }
    }

    // pick the topmost visible sprite or clone from its current costume alpha mask.
}

fn scratch_value_text(value: &ScratchValue) -> String {
    match value {
        ScratchValue::Number(value) => value.to_string(),
        ScratchValue::Text(value) => value.clone(),
        ScratchValue::Boolean(value) => value.to_string(),
        ScratchValue::Undefined => "undefined".to_owned(),
    }
}
