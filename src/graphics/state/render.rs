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
}
