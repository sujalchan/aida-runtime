use super::*;

impl GraphicsState {
    pub(crate) fn write_pen_commands(&mut self, commands: &[PenCommand]) {
        let mut changed = false;
        let mut draw_start = self
            .pen_draw_range
            .map_or(self.pen_instance_data.len(), |(start, _)| start as usize);
        let stroke_count = commands
            .iter()
            .filter(|command| matches!(command, PenCommand::Stroke { .. }))
            .count();
        self.pen_instance_data.reserve(stroke_count);
        for command in commands {
            match command {
                PenCommand::Clear => {
                    self.pen_instance_data.clear();
                    self.pen_uploaded_count = 0;
                    draw_start = 0;
                    self.pending_sprite_stamps.clear();
                    self.pen_clear_pending = true;
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
                    self.pen_instance_data.push(PenInstance {
                        start: [start[0] + pixel_offset, start[1] + pixel_offset],
                        end: [end[0] + pixel_offset, end[1] + pixel_offset],
                        color: *color,
                        diameter: diameter.max(1.0),
                    });
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
                        self.pending_sprite_stamps.push(PendingSpriteStamp {
                            sprite_index,
                            costume_index,
                            transform,
                        });
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
            let mut packed = vec![0_u8; stride * self.pending_sprite_stamps.len()];
            for (index, stamp) in self.pending_sprite_stamps.iter().enumerate() {
                let start = index * stride;
                packed[start..start + transform_size]
                    .copy_from_slice(bytemuck::bytes_of(&stamp.transform));
            }
            self.queue
                .write_buffer(&self.stamp_transform_buffer, 0, &packed);
        }
        if changed {
            self.upload_pending_pen_instances();
            self.pen_texture_dirty = true;
            let draw_end = self.pen_instance_data.len();
            self.pen_draw_range =
                (draw_start < draw_end).then_some((draw_start as u32, draw_end as u32));
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
        self.upload_pending_pen_instances();
        self.pen_draw_range = (!self.pen_instance_data.is_empty())
            .then_some((0, self.pen_instance_data.len() as u32));
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
