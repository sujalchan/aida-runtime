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

    pub(crate) fn render_with_submission_ack(&mut self, submitted: impl FnOnce()) -> RenderOutcome {
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
        if self.backdrop_dirty {
            clear_texture(&mut encoder, &self.backdrop_texture, wgpu::Color::WHITE);
        }
        if self.pen_texture_dirty {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("persistent GPU pen layer"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.pen_texture.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: if self.pen_clear_pending {
                            wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                        } else {
                            wgpu::LoadOp::Load
                        },
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            if let Some((start, end)) = self.pen_draw_range {
                if start < end {
                    pass.set_pipeline(&self.pen_pipeline);
                    pass.set_vertex_buffer(0, self.pen_instances.slice(..));
                    pass.draw(0..6, start..end);
                }
            }
            for (slot, stamp) in self.pending_sprite_stamps.iter().enumerate() {
                let sprite = &self.sprites[stamp.sprite_index];
                let costume = &sprite.costumes[stamp.costume_index];
                pass.set_pipeline(&self.stamp_sprite_pipeline);
                pass.set_bind_group(
                    0,
                    &costume.mips[&costume.selected_mip].texture_bind_group,
                    &[],
                );
                pass.set_bind_group(
                    1,
                    &self.stamp_transform_bind_group,
                    &[(slot as u64 * self.stamp_transform_stride) as u32],
                );
                pass.draw(0..6, 0..1);
            }
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
            pass.set_bind_group(0, &self.backdrop_bind_group, &[]);
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
        submitted();
        if self.pen_texture_dirty {
            self.pen_texture_dirty = false;
            self.pen_clear_pending = false;
            self.pen_draw_range = None;
            // the texture now contains these strokes. keeping the instance
            // history would grow cpu and gpu memory without limit in turbo.
            self.pen_instance_data.clear();
            self.pen_uploaded_count = 0;
            self.pending_sprite_stamps.clear();
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
        self.backdrop_dirty = false;
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
        RenderOutcome::Presented
    }

    pub(crate) fn set_hd_pen_mode(&mut self, enabled: bool) {
        if self.hd_pen_mode == enabled {
            return;
        }

        self.hd_pen_mode = enabled;
        self.recreate_stage_textures();
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

        if self.backdrop_texture.width != stage_width
            || self.backdrop_texture.height != stage_height
        {
            self.backdrop_texture =
                RenderTexture::new(&self.device, "backdrop texture", stage_width, stage_height);
            self.backdrop_bind_group = layer_bind_group(
                &self.device,
                &self.texture_layout,
                &self.backdrop_texture,
                &self.backdrop_texture,
                if self.hd_pen_mode {
                    &self.stage_linear_sampler
                } else {
                    &self.stage_nearest_sampler
                },
                &self.zero_uv_offset,
            );
            if let Some(backdrop) = &self.backdrop_sprite {
                backdrop
                    .upload_backdrop_to(&self.queue, &self.backdrop_texture)
                    .expect("backdrop should rasterize at the stage's pixel density");
                self.backdrop_dirty = false;
            } else {
                self.backdrop_dirty = true;
            }
        }

        for sprite_index in 0..self.sprites.len() {
            let costume_count = self.sprites[sprite_index].costumes.len();
            for costume_index in 0..costume_count {
                self.ensure_sprite_costume_mip(sprite_index, costume_index)
                    .expect("sprite costume should rasterize at the stage's pixel density");
            }
        }
        if self.pen_texture.width != stage_width || self.pen_texture.height != stage_height {
            self.pen_texture =
                RenderTexture::new(&self.device, "pen texture", stage_width, stage_height);
            self.pen_bind_group = layer_bind_group(
                &self.device,
                &self.texture_layout,
                &self.pen_texture,
                &self.pen_texture,
                if self.hd_pen_mode {
                    &self.stage_linear_sampler
                } else {
                    &self.stage_nearest_sampler
                },
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

    pub(super) fn ensure_sprite_costume_mip(
        &mut self,
        sprite_index: usize,
        costume_index: usize,
    ) -> Result<(), String> {
        let viewport = self.viewport.ok_or("stage viewport is unavailable")?;
        let max_texture_dimension = self.device.limits().max_texture_dimension_2d;
        let sprite = &self.sprites[sprite_index];
        let costume = &sprite.costumes[costume_index];
        let mip_level = costume.sprite.mip_level_for_projected_size(
            sprite.transform.scale,
            viewport.width,
            viewport.height,
            costume.logical_size,
            max_texture_dimension,
        );
        let costume = &mut self.sprites[sprite_index].costumes[costume_index];
        if !costume.mips.contains_key(&mip_level) {
            let raster_scale = Sprite::raster_scale_for_mip(mip_level);
            let (width, height) = costume.sprite.texture_size(raster_scale);
            let texture =
                RenderTexture::new(&self.device, "sprite costume mip texture", width, height);
            let opaque_pixels =
                costume
                    .sprite
                    .upload_costume_to(&self.queue, &texture, raster_scale)?;
            let texture_bind_group = layer_bind_group(
                &self.device,
                &self.texture_layout,
                &texture,
                &texture,
                &self.stage_nearest_sampler,
                &self.zero_uv_offset,
            );
            costume.mips.insert(
                mip_level,
                SpriteCostumeMip {
                    texture,
                    opaque_pixels,
                    texture_bind_group,
                },
            );
        }
        costume.selected_mip = mip_level;
        Ok(())
    }
}
