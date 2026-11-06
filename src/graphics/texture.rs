use wgpu::{Device, Queue, Texture, TextureView};

pub(crate) const LAYER_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

#[derive(Clone, Copy)]
pub(crate) struct TextureRegion {
    pub(crate) x: u32,
    pub(crate) y: u32,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

pub(crate) struct RenderTexture {
    _texture: Texture,
    pub(crate) view: TextureView,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

impl RenderTexture {
    pub(crate) fn new(device: &Device, label: &'static str, width: u32, height: u32) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: LAYER_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            _texture: texture,
            view,
            width,
            height,
        }
    }

    pub(crate) fn write_rgba(&self, queue: &Queue, pixels: &[u8]) {
        assert_eq!(pixels.len(), (self.width * self.height * 4) as usize);
        let pixels = unpremultiply_rgba(pixels);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self._texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(self.width * 4),
                rows_per_image: Some(self.height),
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
    }

    pub(crate) fn clear(&self, queue: &Queue) {
        let pixels = vec![0; (self.width * self.height * 4) as usize];
        self.write_straight_rgba_region(
            queue,
            TextureRegion {
                x: 0,
                y: 0,
                width: self.width,
                height: self.height,
            },
            &pixels,
            self.width * 4,
        );
    }

    pub(crate) fn clear_region(&self, queue: &Queue, x: u32, y: u32, width: u32, height: u32) {
        let pixels = vec![0; (width * height * 4) as usize];
        self.write_straight_rgba_region(
            queue,
            TextureRegion {
                x,
                y,
                width,
                height,
            },
            &pixels,
            width * 4,
        );
    }

    pub(crate) fn write_rgba_region(&self, queue: &Queue, region: TextureRegion, pixels: &[u8]) {
        assert!(region.x + region.width <= self.width);
        assert!(region.y + region.height <= self.height);
        assert_eq!(pixels.len(), (region.width * region.height * 4) as usize);
        let pixels = unpremultiply_rgba(pixels);
        self.write_straight_rgba_region(queue, region, &pixels, region.width * 4);
    }

    pub(crate) fn write_straight_rgba_region(
        &self,
        queue: &Queue,
        region: TextureRegion,
        pixels: &[u8],
        bytes_per_row: u32,
    ) {
        let TextureRegion {
            x,
            y,
            width,
            height,
        } = region;
        if width == 0 || height == 0 {
            return;
        }
        assert!(x + width <= self.width);
        assert!(y + height <= self.height);
        assert!(pixels.len() >= ((height - 1) * bytes_per_row + width * 4) as usize);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self._texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
    }
}

// pixmaps use premultiplied rgba while srgb textures store straight color channels
pub(crate) fn unpremultiply_rgba(pixels: &[u8]) -> Vec<u8> {
    let mut straight = pixels.to_vec();
    for pixel in straight.chunks_exact_mut(4) {
        let alpha = u32::from(pixel[3]);
        if alpha == 0 {
            pixel[..3].fill(0);
            continue;
        }
        for channel in &mut pixel[..3] {
            *channel = (((u32::from(*channel) * 255 + alpha / 2) / alpha).min(255)) as u8;
        }
    }
    straight
}
