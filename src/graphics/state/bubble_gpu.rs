use super::*;
use crate::graphics::variables::BubbleRenderInput;
use crate::runtime::RuntimeSnapshot;

impl GraphicsState {
    pub(super) fn sync_bubble_gpu_data(
        &mut self,
        snapshot: &RuntimeSnapshot,
    ) -> Result<(), String> {
        let inputs = snapshot
            .layers
            .iter()
            .filter_map(|layer| {
                let bubble = layer.bubble.clone()?;
                let index = self
                    .sprite_instance_indices
                    .get(&(layer.target_index, layer.instance_id))
                    .copied()?;
                let sprite = &self.sprites[index];
                let costume = &sprite.costumes[sprite.current_costume];
                let scale = sprite.transform.scale;
                Some(BubbleRenderInput {
                    bubble,
                    x: layer.x as f32,
                    y: layer.y as f32,
                    sprite_width: costume.logical_size[0] * scale,
                    sprite_height: costume.logical_size[1] * scale,
                })
            })
            .collect::<Vec<_>>();
        let batch = self.variable_layer.build_gpu_bubble_instances(&inputs)?;
        self.ensure_bubble_capacity(
            batch.transforms.len(),
            batch.shapes.len(),
            batch.glyphs.len(),
        );
        if !batch.transforms.is_empty() {
            self.queue.write_buffer(
                &self.bubble_transforms,
                0,
                bytemuck::cast_slice(&batch.transforms),
            );
        }
        if !batch.shapes.is_empty() {
            self.queue.write_buffer(
                &self.bubble_shape_instances,
                0,
                bytemuck::cast_slice(&batch.shapes),
            );
        }
        if !batch.glyphs.is_empty() {
            self.queue.write_buffer(
                &self.bubble_glyph_instances,
                0,
                bytemuck::cast_slice(&batch.glyphs),
            );
        }
        self.bubble_shape_count = batch.shapes.len() as u32;
        self.bubble_glyph_count = batch.glyphs.len() as u32;
        Ok(())
    }

    fn ensure_bubble_capacity(&mut self, transforms: usize, shapes: usize, glyphs: usize) {
        let required_transforms = transforms.max(1) as u64;
        if required_transforms > self.bubble_transform_capacity {
            self.bubble_transform_capacity = required_transforms.next_power_of_two();
            self.bubble_transforms = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("speech bubble transforms"),
                size: self.bubble_transform_capacity * 16,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.recreate_bubble_bind_groups();
        }
        grow_bubble_buffer(
            &self.device,
            &mut self.bubble_shape_instances,
            &mut self.bubble_shape_capacity,
            shapes,
            std::mem::size_of::<MonitorInstance>() as u64,
            "speech bubble shape instances",
        );
        grow_bubble_buffer(
            &self.device,
            &mut self.bubble_glyph_instances,
            &mut self.bubble_glyph_capacity,
            glyphs,
            std::mem::size_of::<GlyphInstance>() as u64,
            "speech bubble glyph instances",
        );
    }

    pub(super) fn recreate_bubble_bind_groups(&mut self) {
        self.bubble_transform_bind_group =
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("speech bubble shape transforms"),
                layout: &self.monitor_transform_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.bubble_transforms.as_entire_binding(),
                }],
            });
        let atlas_view = self
            .glyph_atlas_texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.bubble_glyph_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("speech bubble glyph bindings"),
            layout: &self.glyph_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.bubble_transforms.as_entire_binding(),
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
}

fn grow_bubble_buffer(
    device: &wgpu::Device,
    buffer: &mut wgpu::Buffer,
    capacity: &mut u64,
    required: usize,
    stride: u64,
    label: &'static str,
) {
    if required as u64 <= *capacity {
        return;
    }
    *capacity = (required as u64).next_power_of_two();
    *buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: *capacity * stride,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
}
