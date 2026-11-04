use super::*;

fn pending_stamp_upload_range(uploaded: usize, total: usize) -> std::ops::Range<usize> {
    uploaded.min(total)..total
}

impl GraphicsState {
    pub(crate) fn write_pen_commands(&mut self, commands: &[PenCommand]) {
        let mut changed = false;
        let stroke_count = commands
            .iter()
            .filter(|command| matches!(command, PenCommand::Stroke { .. }))
            .count();
        self.pen_instance_data.reserve(stroke_count);
        for command in commands {
            match command {
                PenCommand::Clear => {
                    self.pending_pen_ops.push(PendingPenOp::Clear);
                    changed = true;
                }
                PenCommand::Stroke {
                    start,
                    end,
                    color,
                    diameter,
                } if start.iter().chain(end).all(|value| value.is_finite())
                    && color.iter().all(|value| value.is_finite())
                    && diameter.is_finite() =>
                {
                    // keep each scratch pen move as one compact line instance.
                    // the shared gpu pipeline expands the quad and rasterizes
                    // its antialiased coverage; the cpu does not tessellate it.
                    // scratch aligns the 1 px and 3 px pen widths to pixel
                    // centers before drawing its antialiased line geometry.
                    let pixel_offset = if *diameter == 1.0 || *diameter == 3.0 {
                        0.5
                    } else {
                        0.0
                    };
                    let range_start = self.pen_instance_data.len() as u32;
                    self.pen_instance_data.push(PenInstance {
                        start: [start[0] + pixel_offset, start[1] + pixel_offset],
                        end: [end[0] + pixel_offset, end[1] + pixel_offset],
                        color: *color,
                        diameter: diameter.max(1.0),
                    });
                    let end = self.pen_instance_data.len() as u32;
                    match self.pending_pen_ops.last_mut() {
                        Some(PendingPenOp::Strokes {
                            end: previous_end, ..
                        }) if *previous_end == range_start => *previous_end = end,
                        _ => self.pending_pen_ops.push(PendingPenOp::Strokes {
                            start: range_start,
                            end,
                        }),
                    }
                    changed = true;
                }
                PenCommand::Stamp {
                    target_name,
                    position,
                    direction,
                    scale,
                    costume_index,
                    effects_0,
                    effects_1,
                } => {
                    if let Some(sprite_index) = self.sprite_indices.get(target_name).copied() {
                        let sprite = &self.sprites[sprite_index];
                        let costume_index = (*costume_index).min(sprite.costumes.len() - 1);
                        let costume = &sprite.costumes[costume_index];
                        let mut transform = sprite.transform;
                        transform.position = *position;
                        transform.logical_size = costume.logical_size;
                        transform.rotation_center = costume.rotation_center;
                        transform.direction = *direction;
                        transform.scale = *scale;
                        transform.visible = 1.0;
                        transform.effects_0 = *effects_0;
                        transform.effects_1 = *effects_1;
                        sanitize_stamp_effects(&mut transform);
                        let surface_size = (self.pen_texture.width, self.pen_texture.height);
                        let mip_level = self
                            .ensure_sprite_costume_mip_for_size(
                                sprite_index,
                                costume_index,
                                transform.scale,
                                surface_size,
                            )
                            .expect("pen stamp costume should rasterize for the pen surface");
                        let stamp_index = self.pending_sprite_stamps.len();
                        self.pending_sprite_stamps.push(PendingSpriteStamp {
                            sprite_index,
                            costume_index,
                            mip_level,
                            transform,
                        });
                        self.pending_pen_ops.push(PendingPenOp::Stamp(stamp_index));
                        changed = true;
                    }
                }
                PenCommand::Stroke { .. } => {}
            }
        }
        if !self.pending_sprite_stamps.is_empty() {
            self.ensure_stamp_transform_capacity(self.pending_sprite_stamps.len());
            let stride = self.stamp_transform_stride as usize;
            let transform_size = std::mem::size_of::<SpriteTransform>();
            let upload_range = pending_stamp_upload_range(
                self.pending_stamp_uploaded_count,
                self.pending_sprite_stamps.len(),
            );
            let first = upload_range.start;
            if !upload_range.is_empty() {
                let count = upload_range.len();
                let mut packed = vec![0_u8; stride * count];
                for (index, stamp) in self.pending_sprite_stamps[upload_range].iter().enumerate() {
                    let start = index * stride;
                    packed[start..start + transform_size]
                        .copy_from_slice(bytemuck::bytes_of(&stamp.transform));
                }
                self.queue.write_buffer(
                    &self.stamp_transform_buffer,
                    first as u64 * self.stamp_transform_stride,
                    &packed,
                );
                self.pending_stamp_uploaded_count = self.pending_sprite_stamps.len();
            }
        }
        if changed {
            self.upload_pending_pen_instances();
            self.pen_texture_dirty = true;
        }
    }

    fn ensure_stamp_transform_capacity(&mut self, required: usize) {
        if required <= self.stamp_transform_capacity {
            return;
        }
        self.stamp_transform_capacity = required.next_power_of_two();
        self.stamp_transform_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("GPU pen stamp transforms"),
            size: self.stamp_transform_stride * self.stamp_transform_capacity as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.stamp_transform_bind_group =
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("GPU pen stamp transform binding"),
                layout: &self.stamp_transform_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &self.stamp_transform_buffer,
                        offset: 0,
                        size: std::num::NonZeroU64::new(
                            std::mem::size_of::<SpriteTransform>() as u64
                        ),
                    }),
                }],
            });
        // a replacement buffer starts empty, so queued stamp transforms need uploading again.
        self.pending_stamp_uploaded_count = 0;
    }

    fn upload_pending_pen_instances(&mut self) {
        let required = self.pen_instance_data.len();
        if required > self.pen_instance_capacity {
            self.pen_instance_capacity = required.next_power_of_two();
            self.pen_instances = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("GPU pen stroke instances"),
                size: (self.pen_instance_capacity * std::mem::size_of::<PenInstance>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.pen_uploaded_count = 0;
        }

        if self.pen_uploaded_count < required {
            let first = self.pen_uploaded_count;
            self.queue.write_buffer(
                &self.pen_instances,
                (first * std::mem::size_of::<PenInstance>()) as u64,
                bytemuck::cast_slice(&self.pen_instance_data[first..]),
            );
            self.pen_uploaded_count = required;
        }
    }

    pub(super) fn rebuild_pen_texture(&mut self) {
        self.pen_uploaded_count = 0;
        self.pen_clear_pending = true;
        self.pen_texture_dirty = true;
        // retain unpresented operations across texture recreation and keep their barriers.
        self.upload_pending_pen_instances();
    }
}

fn sanitize_stamp_effects(transform: &mut SpriteTransform) {
    for effect in transform
        .effects_0
        .iter_mut()
        .chain(&mut transform.effects_1)
    {
        if !effect.is_finite() {
            *effect = 0.0;
        }
    }
    transform.effects_0[0] = transform.effects_0[0].rem_euclid(200.0);
    transform.effects_0[2] = transform.effects_0[2].rem_euclid(360.0);
    transform.effects_1[1] = transform.effects_1[1].clamp(-100.0, 100.0);
    transform.effects_1[2] = transform.effects_1[2].clamp(0.0, 100.0);
}

#[cfg(test)]
mod tests {
    use super::pending_stamp_upload_range;

    #[test]
    fn stamp_transform_uploads_only_the_new_suffix() {
        assert_eq!(pending_stamp_upload_range(0, 3), 0..3);
        assert_eq!(pending_stamp_upload_range(3, 5), 3..5);
        assert_eq!(pending_stamp_upload_range(5, 5), 5..5);
    }

    #[test]
    fn stamp_transform_upload_restarts_after_pending_queue_reset() {
        assert_eq!(pending_stamp_upload_range(0, 2), 0..2);
    }
}
