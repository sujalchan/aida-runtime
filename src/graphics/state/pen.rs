use super::*;

fn pending_stamp_upload_range(uploaded: usize, total: usize) -> std::ops::Range<usize> {
    uploaded.min(total)..total
}

fn pen_pixel_center_offset(diameter: f32, texture_width: u32) -> f32 {
    if diameter == 1.0 || diameter == 3.0 {
        0.5 * super::super::SCRATCH_WIDTH as f32 / texture_width.max(1) as f32
    } else {
        0.0
    }
}

impl GraphicsState {
    pub(crate) fn write_pen_commands(&mut self, commands: Vec<PenCommand>) {
        let mut changed = false;
        for command in commands {
            match command {
                PenCommand::Clear => {
                    self.pending_pen_ops.push(PendingPenOp::Clear);
                    changed = true;
                }
                PenCommand::Dots(mut dots) => {
                    let texture_width = self.pen_texture.width;
                    dots.retain_mut(|dot| {
                        if !dot
                            .position
                            .iter()
                            .chain(&dot.color)
                            .chain(std::iter::once(&dot.diameter))
                            .all(|value| value.is_finite())
                        {
                            return false;
                        }
                        // align thin marks by half a physical texture pixel at standard and hd scales.
                        dot.diameter = dot.diameter.max(1.0);
                        dot.pixel_offset = pen_pixel_center_offset(dot.diameter, texture_width);
                        true
                    });
                    if !dots.is_empty() {
                        let range_start = self.pen_dot_count as u32;
                        self.pen_dot_count += dots.len();
                        let end = self.pen_dot_count as u32;
                        self.pen_dot_batches.push(PendingPenDotBatch {
                            start: range_start as usize,
                            dots,
                        });
                        append_dot_range(&mut self.pending_pen_ops, range_start, end);
                        changed = true;
                    }
                }
                PenCommand::Strokes(mut strokes) => {
                    // keep each scratch pen move as one compact line instance.
                    // the shared gpu pipeline expands the quad and rasterizes
                    // its antialiased coverage; the cpu does not tessellate it.
                    strokes.retain_mut(|stroke| {
                        if !stroke
                            .start
                            .iter()
                            .chain(&stroke.end)
                            .chain(&stroke.color)
                            .chain(std::iter::once(&stroke.diameter))
                            .all(|value| value.is_finite())
                        {
                            return false;
                        }
                        // align thin marks by half a physical texture pixel at standard and hd scales.
                        stroke.pixel_offset =
                            pen_pixel_center_offset(stroke.diameter, self.pen_texture.width);
                        stroke.diameter = stroke.diameter.max(1.0);
                        true
                    });
                    if !strokes.is_empty() {
                        let range_start = self.pen_instance_count as u32;
                        self.pen_instance_count += strokes.len();
                        let end = self.pen_instance_count as u32;
                        self.pen_stroke_batches.push(PendingPenStrokeBatch {
                            start: range_start as usize,
                            strokes,
                        });
                        append_stroke_range(&mut self.pending_pen_ops, range_start, end);
                        changed = true;
                    }
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
                    if let Some(sprite_index) = self.sprite_indices.get(&target_name).copied() {
                        let sprite = &self.sprites[sprite_index];
                        let costume_index = costume_index.min(sprite.costumes.len() - 1);
                        let costume = &sprite.costumes[costume_index];
                        let mut transform = sprite.transform;
                        transform.position = position;
                        transform.logical_size = costume.logical_size;
                        transform.rotation_center = costume.rotation_center;
                        transform.direction = direction;
                        transform.scale = scale;
                        transform.visible = 1.0;
                        transform.effects_0 = effects_0;
                        transform.effects_1 = effects_1;
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
            self.upload_pending_pen_dots();
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
        let required = self.pen_instance_count;
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
            for batch in &self.pen_stroke_batches {
                let batch_end = batch.start + batch.strokes.len();
                if batch_end <= first {
                    continue;
                }
                let batch_offset = first.saturating_sub(batch.start);
                let strokes = &batch.strokes[batch_offset..];
                if strokes.is_empty() {
                    continue;
                }
                let write_offset = batch.start + batch_offset;
                self.queue.write_buffer(
                    &self.pen_instances,
                    (write_offset * std::mem::size_of::<PenInstance>()) as u64,
                    bytemuck::cast_slice(strokes),
                );
            }
            self.pen_uploaded_count = required;
        }
    }

    fn upload_pending_pen_dots(&mut self) {
        let required = self.pen_dot_count;
        if required > self.pen_dot_capacity {
            self.pen_dot_capacity = required.next_power_of_two();
            self.pen_dots = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("GPU pen dot instances"),
                size: (self.pen_dot_capacity * std::mem::size_of::<PenDotInstance>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.pen_dot_uploaded_count = 0;
        }

        if self.pen_dot_uploaded_count < required {
            let first = self.pen_dot_uploaded_count;
            for batch in &self.pen_dot_batches {
                let batch_end = batch.start + batch.dots.len();
                if batch_end <= first {
                    continue;
                }
                let batch_offset = first.saturating_sub(batch.start);
                let dots = &batch.dots[batch_offset..];
                if dots.is_empty() {
                    continue;
                }
                let write_offset = batch.start + batch_offset;
                self.queue.write_buffer(
                    &self.pen_dots,
                    (write_offset * std::mem::size_of::<PenDotInstance>()) as u64,
                    bytemuck::cast_slice(dots),
                );
            }
            self.pen_dot_uploaded_count = required;
        }
    }

    pub(super) fn rebuild_pen_texture(&mut self) {
        self.pen_uploaded_count = 0;
        self.pen_clear_pending = true;
        self.pen_texture_dirty = true;
        // resize-dependent offsets must be refreshed before pending marks are replayed.
        let texture_width = self.pen_texture.width;
        for batch in &mut self.pen_stroke_batches {
            for stroke in &mut batch.strokes {
                stroke.pixel_offset = pen_pixel_center_offset(stroke.diameter, texture_width);
            }
        }
        for batch in &mut self.pen_dot_batches {
            for dot in &mut batch.dots {
                dot.pixel_offset = pen_pixel_center_offset(dot.diameter, texture_width);
            }
        }
        // retain unpresented operations across texture recreation and keep their barriers.
        self.upload_pending_pen_instances();
        self.upload_pending_pen_dots();
    }
}

fn append_dot_range(operations: &mut Vec<PendingPenOp>, start: u32, end: u32) {
    // combine adjacent dot batches while preserving clear, stroke, and stamp ordering.
    match operations.last_mut() {
        Some(PendingPenOp::Dots {
            end: previous_end, ..
        }) if *previous_end == start => *previous_end = end,
        _ => operations.push(PendingPenOp::Dots { start, end }),
    }
}

fn append_stroke_range(operations: &mut Vec<PendingPenOp>, start: u32, end: u32) {
    // combine adjacent gpu ranges while leaving clear and stamp barriers intact.
    match operations.last_mut() {
        Some(PendingPenOp::Strokes {
            end: previous_end, ..
        }) if *previous_end == start => *previous_end = end,
        _ => operations.push(PendingPenOp::Strokes { start, end }),
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
    use super::{
        PendingPenOp, append_dot_range, append_stroke_range, pen_pixel_center_offset,
        pending_stamp_upload_range,
    };

    #[test]
    fn thin_pen_marks_align_to_physical_pixels_at_standard_and_hd_density() {
        assert_eq!(pen_pixel_center_offset(1.0, 480), 0.5);
        assert_eq!(pen_pixel_center_offset(3.0, 480), 0.5);
        assert_eq!(pen_pixel_center_offset(1.0, 960), 0.25);
        assert_eq!(pen_pixel_center_offset(2.0, 960), 0.0);
    }

    #[test]
    fn adjacent_stroke_batches_share_a_draw_range_without_crossing_barriers() {
        let mut operations = Vec::new();
        append_stroke_range(&mut operations, 0, 2);
        append_stroke_range(&mut operations, 2, 5);
        operations.push(PendingPenOp::Clear);
        append_stroke_range(&mut operations, 5, 8);

        assert!(matches!(
            operations.as_slice(),
            [
                PendingPenOp::Strokes { start: 0, end: 5 },
                PendingPenOp::Clear,
                PendingPenOp::Strokes { start: 5, end: 8 }
            ]
        ));
    }

    #[test]
    fn adjacent_dot_batches_share_a_draw_range_without_crossing_other_operations() {
        let mut operations = Vec::new();
        append_dot_range(&mut operations, 0, 3);
        append_dot_range(&mut operations, 3, 7);
        operations.push(PendingPenOp::Strokes { start: 0, end: 1 });
        append_dot_range(&mut operations, 7, 9);
        operations.push(PendingPenOp::Clear);
        append_dot_range(&mut operations, 9, 10);

        assert!(matches!(
            operations.as_slice(),
            [
                PendingPenOp::Dots { start: 0, end: 7 },
                PendingPenOp::Strokes { start: 0, end: 1 },
                PendingPenOp::Dots { start: 7, end: 9 },
                PendingPenOp::Clear,
                PendingPenOp::Dots { start: 9, end: 10 }
            ]
        ));
    }

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
