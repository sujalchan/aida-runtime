use super::*;

impl GraphicsState {
    pub(crate) fn resize(&mut self, size: PhysicalSize<u32>) {
        if size.width == 0 || size.height == 0 {
            return;
        }
        self.config.width = size.width;
        self.config.height = size.height;
        self.surface.configure(&self.device, &self.config);

        self.viewport = StageViewport::for_window(size.width, size.height);
        self.variable_layer.set_render_scale(
            self.viewport
                .map_or(1.0, |viewport| viewport.width as f32 / SCRATCH_WIDTH as f32),
        );
        self.recreate_stage_textures();
    }

    pub(crate) fn rebuild_viewport_textures(&mut self) {
        self.recreate_stage_textures();
    }

    pub(crate) fn render_with_present_ack(&mut self, presented: impl FnOnce()) -> RenderOutcome {
        let Some(viewport) = self.viewport else {
            return RenderOutcome::Unavailable;
        };
        let size = self.window.inner_size();
        if size.width == 0 || size.height == 0 {
            return RenderOutcome::Unavailable;
        }
        let cpu_frame_started = Instant::now();

        self.poll_gpu_timing();
        self.refresh_frame_time_readout();

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("stage frame encoder"),
            });

        self.sync_variable_gpu_data();
        let timestamp_slot = self
            .gpu_timing
            .as_ref()
            .and_then(|timing| timing.pending_slots.iter().position(|pending| !pending));

        // both offscreen layers are ready before the surface is acquired.
        if self.pen_texture_dirty {
            self.render_pending_pen_operations(&mut encoder);
        }
        let surface_acquire_started = Instant::now();
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
                self.surface.configure(&self.device, &self.config);
                return RenderOutcome::SurfaceRestored;
            }
            wgpu::CurrentSurfaceTexture::Timeout
            | wgpu::CurrentSurfaceTexture::Occluded
            | wgpu::CurrentSurfaceTexture::Validation => return RenderOutcome::Unavailable,
        };
        let surface_acquire_time = surface_acquire_started.elapsed();

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("stage composite pass"),
                timestamp_writes: timestamp_slot.and_then(|_| {
                    self.gpu_timing
                        .as_ref()
                        .map(|timing| wgpu::RenderPassTimestampWrites {
                            query_set: &timing.query_set,
                            beginning_of_pass_write_index: Some(0),
                            end_of_pass_write_index: Some(1),
                        })
                }),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.0,
                            g: 0.0,
                            b: 0.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.composite_pipeline);
            pass.set_viewport(
                viewport.x as f32,
                viewport.y as f32,
                viewport.width as f32,
                viewport.height as f32,
                0.0,
                1.0,
            );
            pass.set_scissor_rect(viewport.x, viewport.y, viewport.width, viewport.height);
            let backdrop_key = self.backdrop_cache_key();
            let backdrop = self
                .backdrop_texture_cache
                .get(&backdrop_key)
                .expect("selected backdrop texture should be cached");
            pass.set_bind_group(0, &backdrop.bind_group, &[]);
            pass.draw(0..3, 0..1);
            pass.set_pipeline(&self.pen_composite_pipeline);
            pass.set_bind_group(0, &self.pen_bind_group, &[]);
            pass.draw(0..3, 0..1);
            pass.set_pipeline(&self.sprite_pipeline);
            for sprite in &self.sprites {
                pass.set_bind_group(
                    0,
                    &sprite.costumes[sprite.current_costume].mips
                        [&sprite.costumes[sprite.current_costume].selected_mip]
                        .texture_bind_group,
                    &[],
                );
                pass.set_bind_group(1, &sprite.transform_bind_group, &[]);
                pass.draw(0..6, 0..1);
            }
            if self.bubble_shape_count > 0 {
                pass.set_pipeline(&self.monitor_instance_pipeline);
                pass.set_bind_group(0, &self.bubble_transform_bind_group, &[]);
                pass.set_vertex_buffer(0, self.bubble_shape_instances.slice(..));
                pass.draw(0..6, 0..self.bubble_shape_count);
            }
            if self.bubble_glyph_count > 0 {
                pass.set_pipeline(&self.glyph_instance_pipeline);
                pass.set_bind_group(0, &self.bubble_glyph_bind_group, &[]);
                pass.set_vertex_buffer(0, self.bubble_glyph_instances.slice(..));
                pass.draw(0..6, 0..self.bubble_glyph_count);
            }
            let shadow_visible = self.dev_mode
                && self.variable_layer.shadow_opacity() > 0.0
                && self.drag_shadow_texture_rect.is_some()
                && (self.variable_layer.drag_index().is_some()
                    || self.fading_shadow_index.is_some());
            let shadow_monitor_id = self
                .variable_layer
                .drag_index()
                .or(self.fading_shadow_index)
                .and_then(|index| self.variable_layer.gpu_monitor_id_at(index));
            for batch in &self.variable_draw_batches {
                if batch.monitor_id.is_some_and(|id| {
                    shadow_precedes_monitor(shadow_visible, shadow_monitor_id, id)
                }) {
                    pass.set_pipeline(&self.composite_pipeline);
                    pass.set_bind_group(0, &self.drag_shadow_bind_group, &[]);
                    pass.draw(0..3, 0..1);
                }
                if batch.shape_count > 0 {
                    pass.set_pipeline(&self.monitor_instance_pipeline);
                    pass.set_bind_group(0, &self.monitor_transform_bind_group, &[]);
                    pass.set_vertex_buffer(0, self.monitor_instances.slice(..));
                    pass.draw(
                        0..6,
                        batch.shape_start..batch.shape_start + batch.shape_count,
                    );
                }
                if batch.glyph_count > 0 {
                    pass.set_pipeline(&self.glyph_instance_pipeline);
                    pass.set_bind_group(0, &self.glyph_bind_group, &[]);
                    pass.set_vertex_buffer(0, self.glyph_instances.slice(..));
                    pass.draw(
                        0..6,
                        batch.glyph_start..batch.glyph_start + batch.glyph_count,
                    );
                }
            }
            if self.readout_vertex_count > 0 {
                pass.set_pipeline(&self.variable_pipeline);
                pass.set_vertex_buffer(0, self.readout_vertices.slice(..));
                pass.draw(0..self.readout_vertex_count, 0..1);
            }
            if self.dev_mode {
                pass.set_pipeline(&self.composite_pipeline);
                pass.set_bind_group(0, &self.ui_bind_group, &[]);
                pass.draw(0..3, 0..1);
            }
        }

        if let (Some(slot), Some(timing)) = (timestamp_slot, self.gpu_timing.as_ref()) {
            encoder.resolve_query_set(&timing.query_set, 0..2, &timing.resolve_buffer, 0);
            encoder.copy_buffer_to_buffer(
                &timing.resolve_buffer,
                0,
                &timing.readback_buffers[slot],
                0,
                16,
            );
        }

        self.queue.submit([encoder.finish()]);
        if self.pen_texture_dirty {
            self.pen_texture_dirty = false;
            self.pen_clear_pending = false;
            self.pending_pen_ops.clear();
            self.pen_stroke_batches.clear();
            self.pen_instance_count = 0;
            self.pen_uploaded_count = 0;
            self.pen_dot_batches.clear();
            self.pen_dot_count = 0;
            self.pen_dot_uploaded_count = 0;
            self.pending_sprite_stamps.clear();
            self.pending_stamp_uploaded_count = 0;
        }
        if let (Some(slot), Some(timing)) = (timestamp_slot, self.gpu_timing.as_mut()) {
            timing.pending_slots[slot] = true;
            let sender = timing.sender.clone();
            timing.readback_buffers[slot]
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |result| {
                    let _ = sender.send((slot, result.is_ok()));
                });
        }
        let waiting_for_first_gpu_sample = self.gpu_frame_time_ms.is_none()
            && self
                .gpu_timing
                .as_ref()
                .is_some_and(|timing| timing.pending_slots.iter().any(|pending| *pending));
        if (self.frametime_mode && self.cpu_frame_time_ms.is_none()) || waiting_for_first_gpu_sample
        {
            self.window.request_redraw();
        }
        if self.frametime_mode {
            self.cpu_frame_time_ms = Some(
                cpu_frame_started
                    .elapsed()
                    .saturating_sub(surface_acquire_time)
                    .as_secs_f32()
                    * 1000.0,
            );
        }
        self.window.pre_present_notify();
        self.queue.present(frame);
        // release the scheduler only after this frame has entered the vsynced surface.
        presented();
        RenderOutcome::Presented
    }

    pub(crate) fn set_hd_pen_mode(&mut self, enabled: bool) {
        if self.hd_pen_mode == enabled {
            return;
        }

        self.hd_pen_mode = enabled;
        self.recreate_stage_textures();
    }

    // draw compatible operations together while splitting at clears to retain script order.
    fn render_pending_pen_operations(&self, encoder: &mut wgpu::CommandEncoder) {
        let mut segment_start = 0;
        let mut clear_segment = self.pen_clear_pending;
        for (index, operation) in self.pending_pen_ops.iter().enumerate() {
            if matches!(operation, PendingPenOp::Clear) {
                if segment_start < index || clear_segment {
                    self.render_pen_segment(encoder, segment_start..index, clear_segment);
                }
                clear_segment = true;
                segment_start = index + 1;
            }
        }
        if segment_start < self.pending_pen_ops.len() || clear_segment {
            self.render_pen_segment(
                encoder,
                segment_start..self.pending_pen_ops.len(),
                clear_segment,
            );
        }
    }

    // render one ordered segment, using clear as the load action after a clear command.
    fn render_pen_segment(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        range: std::ops::Range<usize>,
        clear: bool,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("persistent GPU pen layer"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.pen_texture.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: if clear {
                        wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                    } else {
                        wgpu::LoadOp::Load
                    },
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        for operation in &self.pending_pen_ops[range] {
            match operation {
                // keep point marks in one instanced draw over their compact buffer.
                PendingPenOp::Dots { start, end } if start < end => {
                    pass.set_pipeline(&self.pen_dot_pipeline);
                    pass.set_vertex_buffer(0, self.pen_dots.slice(..));
                    pass.draw(0..6, *start..*end);
                }
                PendingPenOp::Strokes { start, end } if start < end => {
                    pass.set_pipeline(&self.pen_pipeline);
                    pass.set_vertex_buffer(0, self.pen_instances.slice(..));
                    pass.draw(0..6, *start..*end);
                }
                PendingPenOp::Stamp(slot) => {
                    let stamp = &self.pending_sprite_stamps[*slot];
                    let sprite = &self.sprites[stamp.sprite_index];
                    let costume = &sprite.costumes[stamp.costume_index];
                    pass.set_pipeline(&self.stamp_sprite_pipeline);
                    pass.set_bind_group(0, &costume.mips[&stamp.mip_level].texture_bind_group, &[]);
                    pass.set_bind_group(
                        1,
                        &self.stamp_transform_bind_group,
                        &[(*slot as u64 * self.stamp_transform_stride) as u32],
                    );
                    pass.draw(0..6, 0..1);
                }
                PendingPenOp::Dots { .. } | PendingPenOp::Strokes { .. } | PendingPenOp::Clear => {}
            }
        }
    }

    fn recreate_stage_textures(&mut self) {
        let Some(viewport) = self.viewport else {
            return;
        };

        let (stage_width, stage_height) = if self.hd_pen_mode {
            (viewport.width, viewport.height)
        } else {
            (SCRATCH_WIDTH, SCRATCH_HEIGHT)
        };

        self.ensure_backdrop_texture(self.backdrop_index)
            .expect("backdrop should rasterize at the stage's pixel density");

        for sprite_index in 0..self.sprites.len() {
            let costume_index = self.sprites[sprite_index].current_costume;
            self.ensure_sprite_costume_mip(sprite_index, costume_index)
                .expect("sprite costume should rasterize at the stage's pixel density");
        }
        self.prune_unused_costume_mips();
        if self.pen_texture.width != stage_width || self.pen_texture.height != stage_height {
            self.pen_texture =
                RenderTexture::new(&self.device, "pen texture", stage_width, stage_height);
            self.pen_bind_group = layer_bind_group(
                &self.device,
                &self.texture_layout,
                &self.pen_texture,
                &self.pen_texture,
                // keep one scratch pen pixel crisp at every output scale.
                &self.stage_nearest_sampler,
                &self.zero_uv_offset,
            );
            self.rebuild_pen_texture();
        }

        if self.dev_mode
            && (self.ui_texture.width != viewport.width
                || self.ui_texture.height != viewport.height)
        {
            self.ui_texture =
                RenderTexture::new(&self.device, "ui texture", viewport.width, viewport.height);
            self.drag_shadow_texture = RenderTexture::new(
                &self.device,
                "dragged variable shadow texture",
                viewport.width,
                viewport.height,
            );
            self.drag_shadow_texture.clear(&self.queue);
            self.drag_shadow_texture_rect = None;
            self.variable_layer
                .upload_to(&self.queue, &self.ui_texture)
                .expect("variable context menu should rasterize");

            self.ui_bind_group = layer_bind_group(
                &self.device,
                &self.texture_layout,
                &self.ui_texture,
                &self.transparent_texture,
                &self.ui_sampler,
                &self.ui_uv_offset,
            );
            self.drag_shadow_bind_group = layer_bind_group(
                &self.device,
                &self.texture_layout,
                &self.transparent_texture,
                &self.drag_shadow_texture,
                &self.ui_sampler,
                &self.drag_shadow_uv_offset,
            );
            let overlay_index = self
                .variable_layer
                .drag_index()
                .or(self.fading_shadow_index);
            if let Some(index) = overlay_index {
                self.variable_layer
                    .refresh_shadow_tile_for_viewport(index, viewport.width, viewport.height)
                    .expect("variable shadow should rasterize");
                self.update_drag_shadow_position(index)
                    .expect("dragged variable shadow should rasterize");
            }
            self.write_ui_composite_parameters();
        }
    }

    fn backdrop_dimensions(&self) -> (u32, u32) {
        let (width, height) = if self.hd_pen_mode {
            self.viewport
                .map(|viewport| (viewport.width, viewport.height))
                .unwrap_or((SCRATCH_WIDTH, SCRATCH_HEIGHT))
        } else {
            (SCRATCH_WIDTH, SCRATCH_HEIGHT)
        };
        let max_dimension = self.device.limits().max_texture_dimension_2d;
        (width.min(max_dimension), height.min(max_dimension))
    }

    fn backdrop_cache_key(&self) -> (String, u32, u32) {
        let (width, height) = self.backdrop_dimensions();
        let asset_id = self
            .backdrops
            .get(self.backdrop_index)
            .map(|backdrop| backdrop.asset_id.clone())
            .unwrap_or_else(|| "__aida/empty-stage".to_owned());
        (asset_id, width, height)
    }

    pub(super) fn ensure_backdrop_texture(&mut self, backdrop_index: usize) -> Result<(), String> {
        let (width, height) = self.backdrop_dimensions();
        let asset_id = self
            .backdrops
            .get(backdrop_index)
            .map(|backdrop| backdrop.asset_id.clone())
            .unwrap_or_else(|| "__aida/empty-stage".to_owned());
        let key = (asset_id, width, height);
        self.backdrop_cache_clock = self.backdrop_cache_clock.wrapping_add(1);
        if let Some(cached) = self.backdrop_texture_cache.get_mut(&key) {
            cached.last_used = self.backdrop_cache_clock;
            return Ok(());
        }

        let texture = RenderTexture::new(&self.device, "cached backdrop texture", width, height);
        if let Some(backdrop) = self.backdrops.get(backdrop_index) {
            backdrop.sprite.upload_backdrop_to(&self.queue, &texture)?;
        } else {
            let white = vec![255; width as usize * height as usize * 4];
            texture.write_rgba(&self.queue, &white);
        }
        let bind_group = layer_bind_group(
            &self.device,
            &self.texture_layout,
            &texture,
            &texture,
            if self.hd_pen_mode {
                &self.stage_linear_sampler
            } else {
                &self.stage_nearest_sampler
            },
            &self.zero_uv_offset,
        );
        let bytes = width as usize * height as usize * 4;
        self.backdrop_texture_cache.insert(
            key.clone(),
            CachedBackdropTexture {
                _texture: texture,
                bind_group,
                last_used: self.backdrop_cache_clock,
                bytes,
            },
        );
        while self.backdrop_texture_cache.len() > 1
            && self
                .backdrop_texture_cache
                .values()
                .map(|cached| cached.bytes)
                .sum::<usize>()
                > 32 * 1024 * 1024
        {
            let Some(oldest) = self
                .backdrop_texture_cache
                .iter()
                .filter(|(candidate, _)| *candidate != &key)
                .min_by_key(|(_, cached)| cached.last_used)
                .map(|(candidate, _)| candidate.clone())
            else {
                break;
            };
            self.backdrop_texture_cache.remove(&oldest);
        }
        Ok(())
    }

    pub(super) fn ensure_sprite_costume_mip(
        &mut self,
        sprite_index: usize,
        costume_index: usize,
    ) -> Result<(), String> {
        let viewport = self.viewport.ok_or("stage viewport is unavailable")?;
        let mip_level = self.ensure_sprite_costume_mip_for_size(
            sprite_index,
            costume_index,
            self.sprites[sprite_index].transform.scale,
            (viewport.width, viewport.height),
        )?;
        self.sprites[sprite_index].costumes[costume_index].selected_mip = mip_level;
        Ok(())
    }

    // select svg detail from the physical pixels of the surface that will receive the draw.
    pub(super) fn ensure_sprite_costume_mip_for_size(
        &mut self,
        sprite_index: usize,
        costume_index: usize,
        sprite_scale: f32,
        surface_size: (u32, u32),
    ) -> Result<i8, String> {
        let max_texture_dimension = self.device.limits().max_texture_dimension_2d;
        let sprite = &self.sprites[sprite_index];
        let costume = &sprite.costumes[costume_index];
        let use_linear_sampling = costume.sprite.is_vector();
        let asset_id = costume.asset_id.clone();
        let sprite_source = Arc::clone(&costume.sprite);
        let mip_level = costume.sprite.mip_level_for_projected_size(
            sprite_scale,
            surface_size.0,
            surface_size.1,
            costume.logical_size,
            max_texture_dimension,
        );
        if !self.sprites[sprite_index].costumes[costume_index]
            .mips
            .contains_key(&mip_level)
        {
            let key = (asset_id, mip_level);
            let mip = if let Some(mip) = self.shared_costume_mips.get(&key) {
                Arc::clone(mip)
            } else {
                let raster_scale = Sprite::raster_scale_for_mip(mip_level);
                let (width, height) = sprite_source.texture_size(raster_scale);
                let texture =
                    RenderTexture::new(&self.device, "sprite costume mip texture", width, height);
                let opaque_pixels =
                    sprite_source.upload_costume_to(&self.queue, &texture, raster_scale)?;
                let texture_bind_group = layer_bind_group(
                    &self.device,
                    &self.texture_layout,
                    &texture,
                    &texture,
                    if use_linear_sampling {
                        &self.stage_linear_sampler
                    } else {
                        &self.stage_nearest_sampler
                    },
                    &self.zero_uv_offset,
                );
                let mip = Arc::new(SpriteCostumeMip {
                    texture,
                    opaque_pixels,
                    texture_bind_group,
                });
                self.shared_costume_mips.insert(key, Arc::clone(&mip));
                mip
            };
            self.sprites[sprite_index].costumes[costume_index]
                .mips
                .insert(mip_level, mip);
        }
        Ok(mip_level)
    }

    pub(super) fn prune_unused_costume_mips(&mut self) {
        let mut needed = std::collections::HashSet::new();
        for sprite in &self.sprites {
            if let Some(costume) = sprite.costumes.get(sprite.current_costume) {
                needed.insert((costume.asset_id.clone(), costume.selected_mip));
            }
        }
        for stamp in &self.pending_sprite_stamps {
            if let Some(costume) = self.sprites[stamp.sprite_index]
                .costumes
                .get(stamp.costume_index)
            {
                needed.insert((costume.asset_id.clone(), stamp.mip_level));
            }
        }
        self.shared_costume_mips
            .retain(|key, _| needed.contains(key));
        for sprite in &mut self.sprites {
            for costume in &mut sprite.costumes {
                let asset_id = &costume.asset_id;
                costume
                    .mips
                    .retain(|mip, _| needed.contains(&(asset_id.clone(), *mip)));
            }
        }
    }
}
