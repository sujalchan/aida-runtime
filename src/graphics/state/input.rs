use super::*;
use crate::graphics::variables::SliderRangeEditorInput;
use crate::runtime::SpriteSensingCostume;
use crate::runtime::{RuntimeSnapshot, ScratchValue};

impl GraphicsState {
    // feed the renderer display cache from the scheduler's canonical snapshot.
    pub(crate) fn sync_runtime_snapshot(
        &mut self,
        snapshot: &RuntimeSnapshot,
    ) -> Result<(), String> {
        let mut updates = snapshot
            .variables
            .iter()
            .map(|value| (value.key.clone(), scratch_value_text(&value.value)))
            .collect::<Vec<_>>();
        for list in &snapshot.lists {
            updates.push((
                list.key.clone(),
                list.items
                    .iter()
                    .map(scratch_value_text)
                    .collect::<Vec<_>>()
                    .join(" "),
            ));
        }
        for layer in &snapshot.layers {
            if layer.original {
                updates.push((format!("__sprite/{}/x", layer.target), layer.x.to_string()));
                updates.push((format!("__sprite/{}/y", layer.target), layer.y.to_string()));
            }
        }
        self.write_changed_monitors(&updates)?;

        let live_clones = snapshot
            .layers
            .iter()
            .filter(|layer| !layer.original)
            .map(|layer| layer.instance_id)
            .collect::<std::collections::HashSet<_>>();
        self.sprites
            .retain(|sprite| sprite.original || live_clones.contains(&sprite.instance_id));
        self.rebuild_sprite_indices();
        for layer in snapshot.layers.iter().filter(|layer| !layer.original) {
            if !self
                .sprite_instance_indices
                .contains_key(&(layer.target_index, layer.instance_id))
            {
                self.add_clone_renderer(layer)?;
            }
            if let Some(index) = self
                .sprite_instance_indices
                .get(&(layer.target_index, layer.instance_id))
                .copied()
            {
                let sprite = &mut self.sprites[index];
                sprite.transform.position = [layer.x as f32, layer.y as f32];
                self.queue.write_buffer(
                    &sprite.transform_buffer,
                    0,
                    bytemuck::bytes_of(&sprite.transform),
                );
            }
        }
        self.sync_bubble_gpu_data(snapshot)?;
        Ok(())
    }

    fn add_clone_renderer(&mut self, layer: &crate::runtime::LayerSnapshot) -> Result<(), String> {
        let Some(original_index) = self.sprite_indices.get(&layer.target).copied() else {
            return Ok(());
        };
        let original = &self.sprites[original_index];
        let mut costumes = Vec::with_capacity(original.costumes.len());
        for costume in &original.costumes {
            let sprite = costume.sprite.clone();
            let texture = RenderTexture::new(
                &self.device,
                "clone costume texture",
                costume.texture.width,
                costume.texture.height,
            );
            let opaque_pixels =
                sprite.upload_costume_to(&self.queue, &texture, costume.raster_scale)?;
            let texture_bind_group = layer_bind_group(
                &self.device,
                &self.texture_layout,
                &texture,
                &texture,
                &self.stage_nearest_sampler,
                &self.zero_uv_offset,
            );
            costumes.push(SpriteCostumeRenderer {
                sprite,
                texture,
                opaque_pixels,
                texture_bind_group,
                logical_size: costume.logical_size,
                rotation_center: costume.rotation_center,
                raster_scale: costume.raster_scale,
            });
        }
        let mut transform = original.transform;
        transform.position = [layer.x as f32, layer.y as f32];
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
            costumes,
            current_costume: original.current_costume,
            transform,
            transform_buffer,
            transform_bind_group,
        });
        self.rebuild_sprite_indices();
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

    pub(crate) fn set_monitor_visibility(&mut self, name: &str, visible: bool) -> bool {
        self.variable_layer
            .set_monitor_visibility(name, visible)
            .is_some()
    }

    pub(crate) fn scratch_sensing_costumes(&self) -> HashMap<String, Vec<SpriteSensingCostume>> {
        self.sprites
            .iter()
            .map(|sprite| {
                let costumes = sprite
                    .costumes
                    .iter()
                    .map(|costume| SpriteSensingCostume {
                        texture_size: (costume.texture.width, costume.texture.height),
                        logical_size: costume.logical_size,
                        rotation_center: costume.rotation_center,
                        opaque_pixels: Arc::from(costume.opaque_pixels.clone()),
                    })
                    .collect();
                (sprite.target_name.clone(), costumes)
            })
            .collect()
    }

    pub(crate) fn scratch_mouse_sprite_updates(
        &self,
        stage_pointer: Option<(f32, f32)>,
    ) -> Vec<(String, String)> {
        self.sprites
            .iter()
            .map(|sprite| {
                let touching = stage_pointer.is_some_and(|(mouse_x, mouse_y)| {
                    let costume = &sprite.costumes[sprite.current_costume];
                    costume_contains_point(
                        &sprite.transform,
                        costume.logical_size,
                        (costume.texture.width, costume.texture.height),
                        &costume.opaque_pixels,
                        (mouse_x, mouse_y),
                    )
                });
                (
                    format!("__sprite/{}/touching_mouse", sprite.target_name),
                    if touching { "1" } else { "0" }.to_owned(),
                )
            })
            .collect()
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
    ) -> Result<bool, String> {
        if !self.dev_mode {
            return Ok(false);
        }
        let Some((_index, _old_bounds, _action)) =
            self.variable_layer.activate_context_menu(pointer)
        else {
            return Ok(false);
        };
        self.variable_layer
            .upload_to(&self.queue, &self.ui_texture)?;
        self.write_ui_composite_parameters();
        Ok(true)
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

    pub(crate) fn read_changed_monitors(&mut self) -> Vec<(String, String)> {
        self.variable_layer.take_changed_manual_values()
    }

    pub(crate) fn write_changed_monitors(
        &mut self,
        updates: &[(String, String)],
    ) -> Result<bool, String> {
        let mut visible_value_changed = false;
        let mut sprite_transform_changed = vec![false; self.sprites.len()];
        for (name, value) in updates {
            if let Some((target_name, property)) = name
                .strip_prefix("__sprite/")
                .and_then(|name| name.rsplit_once('/'))
                && let Ok(value) = value.parse::<f32>()
            {
                if let Some(index) = self.sprite_indices.get(target_name).copied() {
                    let sprite = &mut self.sprites[index];
                    if property == "costume" {
                        let costume_index = (value.max(0.0) as usize) % sprite.costumes.len();
                        sprite.current_costume = costume_index;
                        let costume = &sprite.costumes[costume_index];
                        sprite.transform.logical_size = costume.logical_size;
                        sprite.transform.rotation_center = costume.rotation_center;
                        sprite_transform_changed[index] = true;
                        continue;
                    }
                    let transform = &mut sprite.transform;
                    match property {
                        "x" => transform.position[0] = value,
                        "y" => transform.position[1] = value,
                        "direction" => transform.direction = value.rem_euclid(360.0),
                        "size" => transform.scale = value / 100.0,
                        "visible" => transform.visible = if value != 0.0 { 1.0 } else { 0.0 },
                        // hue is periodic every 200 scratch units; reduce before
                        // converting to f32 so long-running loops retain precision.
                        "effect_color" => transform.effects_0[0] = value.rem_euclid(200.0),
                        "effect_fisheye" => transform.effects_0[1] = value,
                        "effect_whirl" => transform.effects_0[2] = value.rem_euclid(360.0),
                        "effect_pixelate" => transform.effects_0[3] = value,
                        "effect_mosaic" => transform.effects_1[0] = value,
                        "effect_brightness" => transform.effects_1[1] = value,
                        "effect_ghost" => transform.effects_1[2] = value,
                        _ => continue,
                    }
                    sprite_transform_changed[index] = true;
                    continue;
                }
            }
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
        let any_sprite_changed = sprite_transform_changed.iter().any(|changed| *changed);
        for (sprite, changed) in self.sprites.iter().zip(sprite_transform_changed) {
            if changed {
                self.queue.write_buffer(
                    &sprite.transform_buffer,
                    0,
                    bytemuck::bytes_of(&sprite.transform),
                );
            }
        }
        if visible_value_changed && self.dev_mode {
            self.write_ui_composite_parameters();
        }
        Ok(any_sprite_changed)
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

fn scratch_value_text(value: &ScratchValue) -> String {
    match value {
        ScratchValue::Number(value) => value.to_string(),
        ScratchValue::Text(value) => value.clone(),
        ScratchValue::Boolean(value) => value.to_string(),
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
    let local_x = (cosine * dx + sine * dy) / transform.scale;
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
    fn mouse_touch_uses_costume_pixels_even_when_hidden() {
        let transform = SpriteTransform {
            position: [0.0, 0.0],
            logical_size: [2.0, 2.0],
            rotation_center: [1.0, 1.0],
            direction: 90.0,
            scale: 1.0,
            visible: 0.0,
            padding: [0.0; 3],
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
