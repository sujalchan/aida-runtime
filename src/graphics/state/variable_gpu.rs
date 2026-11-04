use super::*;

impl GraphicsState {
    pub(super) fn sync_variable_gpu_data(&mut self) {
        let mut shape_ids = self.variable_layer.take_dirty_gpu_shapes();
        let mut text_ids = self.variable_layer.take_dirty_gpu_texts();
        let transform_ids = self.variable_layer.take_dirty_transforms();
        let transforms_changed = !transform_ids.is_empty();
        shape_ids.sort_unstable();
        text_ids.sort_unstable();
        let largest_id = shape_ids
            .iter()
            .chain(&text_ids)
            .chain(&transform_ids)
            .copied()
            .max();
        let mut transforms_grew = false;
        if let Some(id) = largest_id.filter(|id| *id as u64 >= self.gpu_monitor_capacity) {
            self.gpu_monitor_capacity = (id as u64 + 1).next_power_of_two();
            self.monitor_transforms = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("variable monitor transforms"),
                size: self.gpu_monitor_capacity * 16,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.monitor_transform_bind_group =
                self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("monitor transform bindings"),
                    layout: &self.monitor_transform_layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.monitor_transforms.as_entire_binding(),
                    }],
                });
            self.recreate_glyph_bind_group();
            transforms_grew = true;
        }

        let mut update_ids: Vec<_> = shape_ids.iter().chain(&text_ids).copied().collect();
        update_ids.sort_unstable();
        update_ids.dedup();
        let mut changed_shapes = Vec::with_capacity(shape_ids.len());
        let mut changed_glyphs = Vec::with_capacity(text_ids.len());
        for id in update_ids {
            let update_shapes = shape_ids.binary_search(&id).is_ok();
            let update_text = text_ids.binary_search(&id).is_ok();
            match self.variable_layer.build_gpu_monitor_instances_for_id(
                id,
                update_shapes,
                update_text,
            ) {
                Ok((shapes, glyphs)) => {
                    let batch = self.monitor_batches.entry(id).or_default();
                    if update_shapes {
                        changed_shapes.push((id, changed_instance_ranges(&batch.0, &shapes)));
                        batch.0 = shapes;
                    }
                    if update_text {
                        changed_glyphs.push((id, changed_glyph_ranges(&batch.1, &glyphs)));
                        batch.1 = glyphs;
                    }
                }
                Err(error) => eprintln!("failed to update variable monitor {id}: {error}"),
            }
        }

        let required_shape_instances = self
            .monitor_batches
            .values()
            .map(|(shapes, _)| shapes.len() as u64)
            .sum::<u64>();
        let mut shape_buffer_grew = false;
        if required_shape_instances > self.gpu_shape_capacity {
            self.gpu_shape_capacity = required_shape_instances.next_power_of_two();
            self.monitor_instances = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("variable monitor instances"),
                size: self.gpu_shape_capacity * std::mem::size_of::<MonitorInstance>() as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            shape_buffer_grew = true;
        }
        let required_glyph_instances = self
            .monitor_batches
            .values()
            .map(|(_, glyphs)| glyphs.len() as u64)
            .sum::<u64>();
        let mut glyph_buffer_grew = false;
        if required_glyph_instances > self.gpu_glyph_capacity {
            self.gpu_glyph_capacity = required_glyph_instances.next_power_of_two();
            self.glyph_instances = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("variable glyph instances"),
                size: self.gpu_glyph_capacity * std::mem::size_of::<GlyphInstance>() as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            glyph_buffer_grew = true;
        }

        if transforms_grew {
            let transforms = (0..self.gpu_monitor_capacity as u32)
                .map(|id| self.variable_layer.gpu_monitor_transform(id))
                .collect::<Vec<_>>();
            self.queue.write_buffer(
                &self.monitor_transforms,
                0,
                bytemuck::cast_slice(&transforms),
            );
        } else {
            if transforms_changed {
                for id in transform_ids {
                    let transform = self.variable_layer.gpu_monitor_transform(id);
                    self.queue.write_buffer(
                        &self.monitor_transforms,
                        id as u64 * 16,
                        bytemuck::cast_slice(&transform),
                    );
                }
            }
        }
        let atlas_revision = self.variable_layer.glyph_atlas_revision();
        if atlas_revision != self.uploaded_glyph_atlas_revision {
            let atlas_size = self.variable_layer.glyph_atlas_size();
            let mut bindings_changed = false;
            if atlas_size != self.uploaded_glyph_atlas_size {
                self.glyph_atlas_texture = create_glyph_atlas_texture(&self.device, atlas_size);
                self.uploaded_glyph_atlas_size = atlas_size;
                bindings_changed = true;
            }

            let metadata = self.variable_layer.glyph_metadata();
            let metadata_capacity = (metadata.len() as u32).max(1).next_power_of_two();
            if metadata_capacity != self.glyph_metadata_capacity {
                self.glyph_metadata = self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("variable glyph metadata"),
                    size: metadata_capacity as u64 * std::mem::size_of::<GlyphMetadata>() as u64,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                self.glyph_metadata_capacity = metadata_capacity;
                bindings_changed = true;
            }
            if bindings_changed {
                self.recreate_glyph_bind_group();
                self.recreate_bubble_bind_groups();
            }

            for (x, y, width, height, pixels) in self.variable_layer.take_glyph_atlas_updates() {
                self.queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &self.glyph_atlas_texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d { x, y, z: 0 },
                        aspect: wgpu::TextureAspect::All,
                    },
                    &pixels,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(width),
                        rows_per_image: Some(height),
                    },
                    wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                );
            }
            self.queue
                .write_buffer(&self.glyph_metadata, 0, bytemuck::cast_slice(&metadata));
            self.uploaded_glyph_atlas_revision = atlas_revision;
        }

        self.rebuild_variable_draw_batches(
            &changed_shapes,
            &changed_glyphs,
            shape_buffer_grew || glyph_buffer_grew,
            transforms_changed || !changed_shapes.is_empty(),
        );
    }

    fn recreate_glyph_bind_group(&mut self) {
        let atlas_view = self
            .glyph_atlas_texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.glyph_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("variable glyph bindings"),
            layout: &self.glyph_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.monitor_transforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.glyph_metadata.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&atlas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.glyph_atlas_sampler),
                },
            ],
        });
    }

    fn rebuild_variable_draw_batches(
        &mut self,
        changed_shapes: &[(u32, Vec<(usize, usize)>)],
        changed_glyphs: &[(u32, Vec<(usize, usize)>)],
        force_repack: bool,
        force_regroup: bool,
    ) {
        let order = self
            .variable_layer
            .gpu_monitor_ids_in_draw_order()
            .collect::<Vec<_>>();
        let shadow_batch_target = self
            .variable_layer
            .drag_index()
            .or(self.fading_shadow_index)
            .and_then(|index| self.variable_layer.gpu_monitor_id_at(index));
        if !force_repack
            && !force_regroup
            && changed_shapes.is_empty()
            && changed_glyphs.is_empty()
            && order == self.variable_draw_order
            && shadow_batch_target == self.variable_shadow_batch_target
        {
            return;
        }

        let same_order = order == self.variable_draw_order;
        let order_matches = same_order && order.len() == self.monitor_instance_ranges.len();
        let shape_layout_matches = !force_repack
            && order_matches
            && order.iter().all(|id| {
                let Some(range) = self.monitor_instance_ranges.get(id) else {
                    return false;
                };
                let Some((shapes, _)) = self.monitor_batches.get(id) else {
                    return false;
                };
                range.shape_count == shapes.len() as u32
            });
        let glyph_layout_matches = !force_repack
            && order_matches
            && order.iter().all(|id| {
                let Some(range) = self.monitor_instance_ranges.get(id) else {
                    return false;
                };
                let Some((_, glyphs)) = self.monitor_batches.get(id) else {
                    return false;
                };
                range.glyph_count == glyphs.len() as u32
            });
        let repack_shapes = !shape_layout_matches;
        let repack_glyphs = !glyph_layout_matches;

        if force_repack || !same_order || order.len() != self.monitor_instance_ranges.len() {
            self.monitor_instance_ranges.clear();
        }
        if repack_shapes {
            let mut shapes = Vec::new();
            for id in order.iter().copied() {
                let shape_start = shapes.len() as u32;
                if let Some((monitor_shapes, _)) = self.monitor_batches.get(&id) {
                    shapes.extend_from_slice(monitor_shapes);
                    let range = self.monitor_instance_ranges.entry(id).or_default();
                    range.shape_start = shape_start;
                    range.shape_count = monitor_shapes.len() as u32;
                }
            }
            if !shapes.is_empty() {
                self.queue
                    .write_buffer(&self.monitor_instances, 0, bytemuck::cast_slice(&shapes));
            }
        } else {
            for (id, ranges) in changed_shapes {
                if let (Some(range), Some((shapes, _))) = (
                    self.monitor_instance_ranges.get(id),
                    self.monitor_batches.get(id),
                ) {
                    upload_instance_ranges(
                        &self.queue,
                        &self.monitor_instances,
                        range.shape_start,
                        shapes,
                        ranges,
                    );
                }
            }
        }
        if repack_glyphs {
            let mut glyphs = Vec::new();
            for id in order.iter().copied() {
                let glyph_start = glyphs.len() as u32;
                if let Some((_, monitor_glyphs)) = self.monitor_batches.get(&id) {
                    glyphs.extend_from_slice(monitor_glyphs);
                    let range = self.monitor_instance_ranges.entry(id).or_default();
                    range.glyph_start = glyph_start;
                    range.glyph_count = monitor_glyphs.len() as u32;
                }
            }
            if !glyphs.is_empty() {
                self.queue
                    .write_buffer(&self.glyph_instances, 0, bytemuck::cast_slice(&glyphs));
            }
        } else {
            for (id, ranges) in changed_glyphs {
                if let (Some(range), Some((_, glyphs))) = (
                    self.monitor_instance_ranges.get(id),
                    self.monitor_batches.get(id),
                ) {
                    upload_instance_ranges(
                        &self.queue,
                        &self.glyph_instances,
                        range.glyph_start,
                        glyphs,
                        ranges,
                    );
                }
            }
        }

        if repack_shapes
            || repack_glyphs
            || force_regroup
            || order != self.variable_draw_order
            || shadow_batch_target != self.variable_shadow_batch_target
        {
            self.variable_draw_batches = build_variable_draw_batches(
                &order,
                shadow_batch_target,
                &self.variable_layer,
                &self.monitor_instance_ranges,
            );
        }
        self.variable_draw_order = order;
        self.variable_shadow_batch_target = shadow_batch_target;
    }
}

fn upload_instance_ranges<T: bytemuck::Pod>(
    queue: &wgpu::Queue,
    buffer: &wgpu::Buffer,
    base_instance: u32,
    instances: &[T],
    ranges: &[(usize, usize)],
) {
    let stride = std::mem::size_of::<T>() as u64;
    for &(start, end) in ranges {
        if start >= end || end > instances.len() {
            continue;
        }
        queue.write_buffer(
            buffer,
            (base_instance as u64 + start as u64) * stride,
            bytemuck::cast_slice(&instances[start..end]),
        );
    }
}

fn build_variable_draw_batches(
    order: &[u32],
    shadow_batch_target: Option<u32>,
    variable_layer: &VariableLayer,
    monitor_ranges: &HashMap<u32, MonitorInstanceRange>,
) -> Vec<VariableDrawBatch> {
    let mut grouped_ids = Vec::with_capacity(order.len());
    let mut grouped_bounds = Vec::with_capacity(order.len());
    let mut batches = Vec::new();

    for id in order.iter().copied() {
        let Some(bounds) = variable_layer.gpu_monitor_bounds(id) else {
            continue;
        };
        let overlaps_group = grouped_bounds.iter().any(|other: &MonitorBounds| {
            bounds.left - 1.0 < other.right
                && bounds.right + 1.0 > other.left
                && bounds.top - 1.0 < other.bottom
                && bounds.bottom + 1.0 > other.top
        });
        if shadow_batch_target == Some(id) || overlaps_group {
            flush_variable_group(&mut grouped_ids, monitor_ranges, &mut batches, None);
            grouped_bounds.clear();
        }
        grouped_ids.push(id);
        grouped_bounds.push(bounds);
        if shadow_batch_target == Some(id) {
            flush_variable_group(&mut grouped_ids, monitor_ranges, &mut batches, Some(id));
            grouped_bounds.clear();
        }
    }
    flush_variable_group(&mut grouped_ids, monitor_ranges, &mut batches, None);
    batches
}

fn create_glyph_atlas_texture(device: &wgpu::Device, size: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("variable glyph alpha atlas"),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}
