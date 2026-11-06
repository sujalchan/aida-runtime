use super::*;
use crate::graphics::variables::SliderRangeEditorInput;
use crate::runtime::{RuntimeInputEvent, RuntimeSnapshot, ScratchValue};
use std::sync::Arc;

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

fn rendered_layers_changed(
    previous: &[crate::runtime::LayerSnapshot],
    current: &[crate::runtime::LayerSnapshot],
) -> bool {
    previous.len() != current.len()
        || previous
            .iter()
            .any(|old| !current.iter().any(|new| same_rendered_layer(old, new)))
}

fn timer_sample_for_snapshot(
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

fn apply_runtime_layer_transform(
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

fn sprite_monitor_updates(
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
    pub(crate) fn pick_sprite_at(&self, stage_point: (f32, f32)) -> Option<(usize, bool)> {
        let candidates = self.sprites.iter().map(|sprite| {
            let hit = sprite.transform.visible > 0.0
                && sprite
                    .costumes
                    .get(sprite.current_costume)
                    .and_then(|costume| {
                        costume.mips.get(&costume.selected_mip).map(|mip| {
                            costume_contains_point(
                                &sprite.transform,
                                costume.logical_size,
                                (mip.texture.width, mip.texture.height),
                                &mip.opaque_pixels,
                                stage_point,
                            )
                        })
                    })
                    .unwrap_or(false);
            (
                sprite.instance_id,
                sprite.transform.visible > 0.0,
                sprite.draggable,
                hit,
            )
        });
        pick_topmost_visible_instance(candidates)
    }

    pub(crate) fn window(&self) -> &Window {
        &self.window
    }

    pub(crate) fn window_to_stage(&self, position: PhysicalPosition<f64>) -> Option<(f32, f32)> {
        let viewport = self.viewport?;
        let x = position.x - viewport.x as f64;
        let y = position.y - viewport.y as f64;
        if x < 0.0 || y < 0.0 || x >= viewport.width as f64 || y >= viewport.height as f64 {
            return None;
        }
        self.window_to_stage_unclamped(position)
    }

    pub(crate) fn window_to_stage_unclamped(
        &self,
        position: PhysicalPosition<f64>,
    ) -> Option<(f32, f32)> {
        let viewport = self.viewport?;
        let x = position.x - viewport.x as f64;
        let y = position.y - viewport.y as f64;
        Some((
            (x * SCRATCH_WIDTH as f64 / viewport.width as f64) as f32 - SCRATCH_WIDTH as f32 * 0.5,
            SCRATCH_HEIGHT as f32 * 0.5
                - (y * SCRATCH_HEIGHT as f64 / viewport.height as f64) as f32,
        ))
    }

    pub(crate) fn window_scroll_pixels_to_list_rows(&self, pixels: f64) -> f32 {
        self.viewport.map_or(0.0, |viewport| {
            (pixels * SCRATCH_HEIGHT as f64 / viewport.height as f64 / 24.0) as f32
        })
    }

    // begin dragging a monitor and update its gpu shadow state
    pub(crate) fn begin_variable_drag(&mut self, x: f32, y: f32) -> Result<bool, String> {
        if !self.dev_mode {
            return Ok(false);
        }
        if self.fading_shadow_index.is_some() && self.variable_layer.hit_test(x, y).is_some() {
            self.finish_fading_shadow()?;
        }
        if !self.variable_layer.begin_drag(x, y) {
            return Ok(false);
        }
        let index = self.variable_layer.drag_index().unwrap();
        self.variable_layer.refresh_shadow_tile_for_viewport(
            index,
            self.ui_texture.width,
            self.ui_texture.height,
        )?;
        self.update_drag_shadow_position(index)?;
        self.write_ui_composite_parameters();
        Ok(true)
    }

    // begin dragging a slider thumb and upload only the affected monitor region
    pub(crate) fn begin_variable_slider(&mut self, x: f32, y: f32) -> Result<bool, String> {
        if !self.variable_layer.begin_slider_drag(x, y) {
            return Ok(false);
        }
        if self.fading_shadow_index.is_some() {
            self.finish_fading_shadow()?;
        }
        if self.variable_layer.slider_drag_index().is_some() {
            self.variable_layer.take_slider_width_changed();
        }
        Ok(true)
    }

    // update slider hover colors and refresh only monitors whose hover changed
    pub(crate) fn update_variable_slider_hover(
        &mut self,
        pointer: Option<(f32, f32)>,
    ) -> Result<bool, String> {
        let Some((previous, current)) = self.variable_layer.update_slider_hover(pointer) else {
            return Ok(false);
        };
        if let Some(index) = previous {
            self.refresh_variable_monitor(index)?;
        }
        if let Some(index) = current.filter(|index| Some(*index) != previous) {
            self.refresh_variable_monitor(index)?;
        }
        Ok(true)
    }

    pub(crate) fn scroll_list_monitor(
        &mut self,
        x: f32,
        y: f32,
        rows: f32,
    ) -> Result<bool, String> {
        if self.variable_layer.scroll_list_at(x, y, rows).is_none() {
            return Ok(false);
        }
        self.write_ui_composite_parameters();
        Ok(true)
    }

    pub(crate) fn update_list_scrollbar_hover(
        &mut self,
        pointer: Option<(f32, f32)>,
    ) -> Result<bool, String> {
        if !self.variable_layer.update_list_scrollbar_hover(pointer) {
            return Ok(false);
        }
        self.write_ui_composite_parameters();
        Ok(true)
    }

    pub(crate) fn handle_list_pointer(
        &mut self,
        x: f32,
        y: f32,
    ) -> Result<ListPointerResponse, String> {
        let response = self.variable_layer.handle_list_pointer(x, y);
        self.write_ui_composite_parameters();
        Ok(response)
    }

    pub(crate) fn list_item_editor_is_open(&self) -> bool {
        self.variable_layer.list_editor_is_open()
    }

    pub(crate) fn edit_list_item(
        &mut self,
        input: ListEditorInput<'_>,
    ) -> (bool, Vec<RuntimeInputEvent>) {
        let result = self.variable_layer.edit_list_item(input);
        if result.0 {
            self.write_ui_composite_parameters();
        }
        result
    }

    // cycle the clicked monitor mode; the gpu geometry is refreshed on draw
    pub(crate) fn toggle_variable_compact(&mut self, x: f32, y: f32) -> Result<bool, String> {
        if !self.dev_mode {
            return Ok(false);
        }
        if self.fading_shadow_index.is_some() && self.variable_layer.hit_test(x, y).is_some() {
            self.finish_fading_shadow()?;
        }
        let Some((_index, _old_bounds)) = self.variable_layer.toggle_compact_at(x, y) else {
            return Ok(false);
        };
        self.write_ui_composite_parameters();
        Ok(true)
    }

    pub(crate) fn variable_monitor_at(&self, x: f32, y: f32) -> bool {
        self.variable_layer.hit_test(x, y).is_some()
    }

    pub(crate) fn context_menu_is_open(&self) -> bool {
        self.dev_mode && self.variable_layer.context_menu_is_open()
    }

    pub(crate) fn open_variable_context_menu(
        &mut self,
        pointer: Option<(f32, f32)>,
    ) -> Result<bool, String> {
        if !self.dev_mode {
            return Ok(false);
        }
        if self.fading_shadow_index.is_some() {
            self.finish_fading_shadow()?;
        }
        let changed = if let Some((x, y)) = pointer {
            self.variable_layer.open_context_menu(x, y)
        } else {
            self.variable_layer.close_context_menu()
        };
        if changed {
            self.variable_layer
                .upload_to(&self.queue, &self.ui_texture)?;
            self.write_ui_composite_parameters();
        }
        Ok(changed)
    }

    pub(crate) fn update_variable_context_menu_hover(
        &mut self,
        pointer: Option<(f32, f32)>,
    ) -> Result<bool, String> {
        if !self.dev_mode {
            return Ok(false);
        }
        if !self.variable_layer.update_context_menu_hover(pointer) {
            return Ok(false);
        }
        self.variable_layer
            .upload_context_menu_to(&self.queue, &self.ui_texture)?;
        self.write_ui_composite_parameters();
        Ok(true)
    }

    pub(crate) fn activate_variable_context_menu(
        &mut self,
        pointer: Option<(f32, f32)>,
    ) -> Result<Option<super::super::variables::ListMonitorFileAction>, String> {
        if !self.dev_mode {
            return Ok(None);
        }
        let Some((index, _old_bounds, action)) = self.variable_layer.activate_context_menu(pointer)
        else {
            return Ok(None);
        };
        let file_action = self.variable_layer.list_file_action(index, action);
        self.variable_layer
            .upload_to(&self.queue, &self.ui_texture)?;
        self.write_ui_composite_parameters();
        Ok(file_action)
    }

    pub(crate) fn edit_variable_slider_range(
        &mut self,
        input: SliderRangeEditorInput<'_>,
    ) -> Result<bool, String> {
        if !self.dev_mode {
            return Ok(false);
        }
        if !self.variable_layer.edit_slider_range(input) {
            return Ok(false);
        }
        self.variable_layer
            .upload_to(&self.queue, &self.ui_texture)?;
        self.write_ui_composite_parameters();
        Ok(true)
    }

    // route pointer movement to slider value updates or whole monitor movement
    pub(crate) fn drag_variable_to(&mut self, x: f32, y: f32) -> Result<bool, String> {
        let slider_drag_index = self.variable_layer.slider_drag_index();
        if !self.variable_layer.drag_to(x, y) {
            return Ok(false);
        }
        if let Some(index) = slider_drag_index {
            if self.variable_layer.take_slider_width_changed() {
                self.variable_layer.invalidate_shadow_tile(index);
                if self.variable_layer.uses_gpu_translation() {
                    self.write_ui_composite_parameters();
                }
            }
        } else {
            self.write_ui_composite_parameters();
        }
        Ok(true)
    }

    pub(crate) fn end_variable_drag(&mut self) -> Result<(), String> {
        if let Some(index) = self.variable_layer.drag_index() {
            self.fading_shadow_index = Some(index);
            self.fading_shadow_bounds = Some(self.variable_layer.visual_monitor_bounds(index));
        }
        self.variable_layer.end_drag();
        self.write_ui_composite_parameters();
        Ok(())
    }

    pub(crate) fn update_variable_shadow(&mut self, now: Instant) -> Result<bool, String> {
        if !self.variable_layer.update_shadow(now) {
            return Ok(false);
        }
        if self.variable_layer.shadow_opacity() <= 0.0 && self.fading_shadow_index.is_some() {
            self.finish_fading_shadow()?;
        }
        self.write_ui_composite_parameters();
        Ok(true)
    }

    fn finish_fading_shadow(&mut self) -> Result<(), String> {
        if let Some((left, top, width, height)) = self.drag_shadow_texture_rect.take() {
            self.drag_shadow_texture
                .clear_region(&self.queue, left, top, width, height);
        }
        self.fading_shadow_index = None;
        self.fading_shadow_bounds = None;
        self.write_ui_composite_parameters();
        Ok(())
    }

    pub(crate) fn read_changed_monitors(&mut self) -> Vec<RuntimeInputEvent> {
        self.variable_layer
            .take_changed_manual_values()
            .into_iter()
            .map(|(key, value)| RuntimeInputEvent::VariableValue { key, value })
            .collect()
    }

    pub(crate) fn write_changed_monitors(
        &mut self,
        updates: &[(String, String)],
    ) -> Result<bool, String> {
        let mut visible_value_changed = false;
        for (name, value) in updates {
            let Some((index, width_changed)) =
                self.variable_layer.set_monitor_value(name, value.clone())
            else {
                // hidden monitors still receive the latest value in their cpu state,
                // but they do not trigger rendering or gpu synchronization.
                continue;
            };
            visible_value_changed = true;
            if width_changed {
                self.variable_layer.invalidate_shadow_tile(index);
            }
            if width_changed
                && (self.variable_layer.drag_index() == Some(index)
                    || self.fading_shadow_index == Some(index))
            {
                self.variable_layer.refresh_shadow_tile_for_viewport(
                    index,
                    self.ui_texture.width,
                    self.ui_texture.height,
                )?;
                self.update_drag_shadow_position(index)?;
            }
            if self.fading_shadow_index == Some(index) {
                self.fading_shadow_bounds = Some(self.variable_layer.visual_monitor_bounds(index));
            }
        }
        if visible_value_changed && self.dev_mode {
            self.write_ui_composite_parameters();
        }
        Ok(visible_value_changed)
    }

    pub(crate) fn variable_shadow_is_animating(&self) -> bool {
        self.variable_layer.shadow_is_animating()
    }

    fn refresh_variable_monitor(&mut self, index: usize) -> Result<(), String> {
        let _ = index;
        Ok(())
    }

    pub(super) fn write_ui_composite_parameters(&self) {
        if !self.dev_mode {
            return;
        }
        let (offset_x, offset_y) = self.variable_layer.composite_offset();
        self.queue.write_buffer(
            &self.ui_uv_offset,
            0,
            bytemuck::cast_slice(&[offset_x, offset_y, 0.0_f32, 0.0_f32]),
        );
        let active_index = self.variable_layer.drag_index();
        let shadow_bounds = active_index
            .map(|index| self.variable_layer.visual_monitor_bounds(index))
            .or(self.fading_shadow_bounds);
        let (shadow_x, shadow_y, opacity) =
            if let (Some(bounds), Some((source_x, source_y, _, _))) =
                (shadow_bounds, self.drag_shadow_texture_rect)
            {
                (
                    (bounds.left - 14.0) / SCRATCH_WIDTH as f32
                        - source_x as f32 / self.drag_shadow_texture.width as f32,
                    (bounds.top - 14.0) / SCRATCH_HEIGHT as f32
                        - source_y as f32 / self.drag_shadow_texture.height as f32,
                    self.variable_layer.shadow_opacity(),
                )
            } else {
                (2.0, 2.0, 0.0)
            };
        self.queue.write_buffer(
            &self.drag_shadow_uv_offset,
            0,
            bytemuck::cast_slice(&[shadow_x, shadow_y, opacity, 0.0_f32]),
        );
    }

    // upload the cached polygon shadow for the dragged monitor.
    pub(super) fn update_drag_shadow_position(&mut self, index: usize) -> Result<(), String> {
        if !self.dev_mode {
            return Ok(());
        }
        if let Some((left, top, width, height)) = self.drag_shadow_texture_rect.take() {
            self.drag_shadow_texture
                .clear_region(&self.queue, left, top, width, height);
        }
        let (width, height) = self.variable_layer.cached_shadow_tile_size(index)?;
        let source_rect = (
            0,
            0,
            width.min(self.drag_shadow_texture.width),
            height.min(self.drag_shadow_texture.height),
        );
        self.variable_layer.upload_cached_tile_at(
            &self.queue,
            &self.drag_shadow_texture,
            index,
            0,
            0,
        )?;
        self.drag_shadow_texture_rect = Some(source_rect);
        Ok(())
    }
}

// preserve back-to-front renderer order while ignoring transparent top layers.
fn pick_topmost_visible_instance(
    candidates: impl DoubleEndedIterator<Item = (usize, bool, bool, bool)>,
) -> Option<(usize, bool)> {
    candidates
        .rev()
        .find_map(|(instance, visible, draggable, hit)| {
            (visible && hit).then_some((instance, draggable))
        })
}

#[cfg(test)]
mod clone_transform_tests {
    use super::{
        SpriteTransform, apply_runtime_layer_transform, rendered_layers_changed,
        sprite_monitor_updates, timer_sample_for_snapshot,
    };
    use crate::graphics::variables::{VariableLayer, VariableMonitor};
    use crate::runtime::LayerSnapshot;

    #[test]
    fn hidden_position_changes_do_not_dirty_rendered_layers() {
        let layer = |x, visible| LayerSnapshot {
            target_index: 1,
            target: "Sprite1".to_owned(),
            instance_id: 1,
            original: true,
            x,
            y: 0.0,
            direction: 90.0,
            rotation_style: crate::runtime::ScratchRotationStyle::AllAround,
            size: 100.0,
            visible,
            costume_index: 0,
            bubble: None,
        };
        assert!(!rendered_layers_changed(
            &[layer(0.0, false)],
            &[layer(50.0, false)]
        ));
        assert!(rendered_layers_changed(
            &[layer(0.0, true)],
            &[layer(0.0, false)]
        ));
        assert!(rendered_layers_changed(
            &[layer(0.0, false)],
            &[layer(50.0, true)]
        ));
    }

    #[test]
    fn clone_drawable_transforms_remain_independent_and_deleted_layers_disappear() {
        let layer = |instance_id, x, y, direction, size, visible| LayerSnapshot {
            target_index: 1,
            target: "Sprite1".to_owned(),
            instance_id,
            original: instance_id == 1,
            x,
            y,
            direction,
            rotation_style: crate::runtime::ScratchRotationStyle::AllAround,
            size,
            visible,
            costume_index: 0,
            bubble: None,
        };
        let original_layer = layer(1, 4.0, 5.0, 90.0, 100.0, true);
        let clone_a = layer(2, -30.0, 20.0, -45.0, 75.0, true);
        let clone_b = layer(3, 40.0, -10.0, 180.0, 150.0, false);
        let mut original_transform: SpriteTransform = bytemuck::Zeroable::zeroed();
        let mut clone_a_transform = original_transform;
        let mut clone_b_transform = original_transform;
        apply_runtime_layer_transform(&mut original_transform, &original_layer);
        apply_runtime_layer_transform(&mut clone_a_transform, &clone_a);
        apply_runtime_layer_transform(&mut clone_b_transform, &clone_b);
        assert_eq!(original_transform.position, [4.0, 5.0]);
        assert_eq!(clone_a_transform.position, [-30.0, 20.0]);
        assert_eq!(clone_a_transform.direction, -45.0);
        assert_eq!(clone_a_transform.scale, 0.75);
        assert_eq!(clone_b_transform.position, [40.0, -10.0]);
        assert_eq!(clone_b_transform.visible, 0.0);

        let live_layers = [&original_layer, &clone_a, &clone_b]
            .into_iter()
            .filter(|layer| layer.visible)
            .count();
        assert_eq!(live_layers, 2);
        let after_delete = [&original_layer, &clone_b]
            .into_iter()
            .filter(|layer| layer.visible)
            .count();
        assert_eq!(after_delete, 1);
    }

    #[test]
    fn size_monitor_updates_follow_original_sprite_snapshot_only() {
        let monitor = VariableMonitor::new("Sprite1: size", "100", 0.0, 0.0)
            .with_value_key("__sprite/Sprite1/size");
        let variable_layer = VariableLayer::new(vec![monitor]).unwrap();
        let layer = |instance_id, original, size| LayerSnapshot {
            target_index: 1,
            target: "Sprite1".to_owned(),
            instance_id,
            original,
            x: 0.0,
            y: 0.0,
            direction: 90.0,
            rotation_style: crate::runtime::ScratchRotationStyle::AllAround,
            size,
            visible: true,
            costume_index: 0,
            bubble: None,
        };

        let updates = sprite_monitor_updates(
            &variable_layer,
            &[layer(1, true, 125.0), layer(2, false, 80.0)],
        );
        assert_eq!(
            updates,
            vec![("__sprite/Sprite1/size".to_owned(), "125".to_owned())]
        );
    }

    // keep motion monitor strings tied to the original target's live state.
    #[test]
    fn motion_monitor_updates_follow_live_target_coordinates() {
        let variable_layer = VariableLayer::new(vec![
            VariableMonitor::new("Sprite1: x position", "0", 0.0, 0.0)
                .with_value_key("__sprite/Sprite1/x"),
            VariableMonitor::new("Sprite1: y position", "0", 0.0, 0.0)
                .with_value_key("__sprite/Sprite1/y"),
            VariableMonitor::new("Sprite1: direction", "90", 0.0, 0.0)
                .with_value_key("__sprite/Sprite1/direction"),
        ])
        .unwrap();
        let layer = |original, x, y, direction| LayerSnapshot {
            target_index: 1,
            target: "Sprite1".to_owned(),
            instance_id: if original { 1 } else { 2 },
            original,
            x,
            y,
            direction,
            rotation_style: crate::runtime::ScratchRotationStyle::AllAround,
            size: 100.0,
            visible: true,
            costume_index: 0,
            bubble: None,
        };
        let updates = sprite_monitor_updates(
            &variable_layer,
            &[
                layer(true, 10.0000000001, -2.0, -45.0),
                layer(false, 99.0, 88.0, 180.0),
            ],
        );
        assert_eq!(
            updates,
            vec![
                ("__sprite/Sprite1/x".to_owned(), "10".to_owned()),
                ("__sprite/Sprite1/y".to_owned(), "-2".to_owned()),
                ("__sprite/Sprite1/direction".to_owned(), "-45".to_owned()),
            ]
        );
    }

    // apply rotation styles to each displayed drawable while retaining stored direction.
    #[test]
    fn renderer_applies_rotation_style_direction_and_flip() {
        let mut transform: SpriteTransform = bytemuck::Zeroable::zeroed();
        let mut layer = LayerSnapshot {
            target_index: 1,
            target: "Sprite1".to_owned(),
            instance_id: 1,
            original: true,
            x: 3.0,
            y: 4.0,
            direction: -15.0,
            rotation_style: crate::runtime::ScratchRotationStyle::LeftRight,
            size: 100.0,
            visible: true,
            costume_index: 0,
            bubble: None,
        };
        apply_runtime_layer_transform(&mut transform, &layer);
        assert_eq!(transform.direction, 90.0);
        assert_eq!(transform.flip_x, -1.0);
        assert_eq!(transform.position, [3.0, 4.0]);

        layer.direction = 15.0;
        assert!(apply_runtime_layer_transform(&mut transform, &layer));
        assert_eq!(transform.direction, 90.0);
        assert_eq!(transform.flip_x, 1.0);

        layer.rotation_style = crate::runtime::ScratchRotationStyle::DontRotate;
        layer.direction = -135.0;
        assert!(!apply_runtime_layer_transform(&mut transform, &layer));
        assert_eq!(transform.direction, 90.0);
        assert_eq!(transform.flip_x, 1.0);

        layer.rotation_style = crate::runtime::ScratchRotationStyle::AllAround;
        assert!(apply_runtime_layer_transform(&mut transform, &layer));
        assert_eq!(transform.direction, -135.0);
    }

    #[test]
    fn stale_snapshot_after_resize_does_not_rewind_timer_but_reset_epoch_does() {
        let start = std::time::Instant::now();
        let later = start + std::time::Duration::from_secs(2);
        assert_eq!(
            timer_sample_for_snapshot(4, 5_000, start, 4, 5_000, later),
            (4, 7_000)
        );
        assert_eq!(
            timer_sample_for_snapshot(4, 7_000, later, 5, 0, later),
            (5, 0)
        );
    }
}

fn scratch_value_text(value: &ScratchValue) -> String {
    match value {
        ScratchValue::Number(value) => value.to_string(),
        ScratchValue::Text(value) => value.clone(),
        ScratchValue::Boolean(value) => value.to_string(),
        ScratchValue::Undefined => "undefined".to_owned(),
    }
}

fn costume_contains_point(
    transform: &SpriteTransform,
    logical_size: [f32; 2],
    texture_size: (u32, u32),
    opaque_pixels: &[u8],
    stage_point: (f32, f32),
) -> bool {
    if transform.scale.abs() < 0.0001 {
        return false;
    }
    let dx = stage_point.0 - transform.position[0];
    let dy = stage_point.1 - transform.position[1];
    let radians = (90.0 - transform.direction).to_radians();
    let cosine = radians.cos();
    let sine = radians.sin();
    let local_x = (cosine * dx + sine * dy) / transform.scale * transform.flip_x;
    let local_y = (-sine * dx + cosine * dy) / transform.scale;
    let u = (local_x + transform.rotation_center[0]) / logical_size[0];
    let v = (transform.rotation_center[1] - local_y) / logical_size[1];
    if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
        return false;
    }
    let pixel_x = (u * texture_size.0 as f32) as usize;
    let pixel_y = (v * texture_size.1 as f32) as usize;
    let index = pixel_y * texture_size.0 as usize + pixel_x;
    opaque_pixels
        .get(index / 8)
        .is_some_and(|byte| byte & (1 << (index % 8)) != 0)
}

#[cfg(test)]
mod mouse_tests {
    use super::*;

    #[test]
    fn pointer_picking_uses_frontmost_opaque_instance_and_its_draggable_flag() {
        let sprites = [(11, true, false, true), (12, true, true, true)];
        assert_eq!(
            pick_topmost_visible_instance(sprites.into_iter()),
            Some((12, true))
        );

        let transparent_top = [(11, true, false, true), (12, true, true, false)];
        assert_eq!(
            pick_topmost_visible_instance(transparent_top.into_iter()),
            Some((11, false))
        );
        assert_eq!(
            pick_topmost_visible_instance([(11, false, false, true)].into_iter()),
            None
        );
    }

    #[test]
    fn original_and_clone_hit_regions_keep_their_instance_identity() {
        let parent_left_hit = [(1, true, false, true), (2, true, false, false)];
        let clone_right_hit = [(1, true, false, false), (2, true, false, true)];
        assert_eq!(
            pick_topmost_visible_instance(parent_left_hit.into_iter()),
            Some((1, false))
        );
        assert_eq!(
            pick_topmost_visible_instance(clone_right_hit.into_iter()),
            Some((2, false))
        );
    }

    #[test]
    fn mouse_touch_uses_costume_pixels_even_when_hidden() {
        let transform = SpriteTransform {
            position: [0.0, 0.0],
            logical_size: [2.0, 2.0],
            rotation_center: [1.0, 1.0],
            direction: 90.0,
            scale: 1.0,
            visible: 0.0,
            flip_x: 1.0,
            padding: [0.0; 2],
            effects_0: [0.0; 4],
            effects_1: [0.0; 4],
        };
        // only the top-left pixel of a 2x2 costume has nonzero alpha.
        assert!(costume_contains_point(
            &transform,
            [2.0, 2.0],
            (2, 2),
            &[0b0001],
            (-0.5, 0.5),
        ));
        assert!(!costume_contains_point(
            &transform,
            [2.0, 2.0],
            (2, 2),
            &[0b0001],
            (0.5, 0.5),
        ));
    }
}
