use resvg::{tiny_skia, usvg};
use std::fmt;
use wgpu::Queue;

use super::texture::RenderTexture;
use super::{SCRATCH_HEIGHT, SCRATCH_WIDTH};

#[cfg(test)]
const SPRITE_CENTER_X_STAGE_PX: f32 = 0.0;
const SCRATCH_MAX_TEXTURE_DIMENSION: u32 = 2048;
const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

#[derive(Debug)]
pub(crate) enum SpriteDecodeError {
    Png(String),
    Svg(String),
}

impl fmt::Display for SpriteDecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Png(error) => write!(formatter, "failed to decode sprite PNG: {error}"),
            Self::Svg(error) => write!(formatter, "failed to parse sprite SVG: {error}"),
        }
    }
}

impl std::error::Error for SpriteDecodeError {}

#[derive(Clone)]
pub(crate) struct Sprite {
    source: SpriteSource,
}

#[derive(Clone)]
enum SpriteSource {
    Svg(Box<usvg::Tree>),
    Png(tiny_skia::Pixmap),
    Empty,
}

impl Sprite {
    pub(crate) fn from_bytes(bytes: &[u8]) -> Result<Self, SpriteDecodeError> {
        if bytes.starts_with(PNG_SIGNATURE) {
            let pixmap = tiny_skia::Pixmap::decode_png(bytes)
                .map_err(|error| SpriteDecodeError::Png(error.to_string()))?;
            return Ok(Self {
                source: SpriteSource::Png(pixmap),
            });
        }

        let tree = match usvg::Tree::from_data(bytes, &usvg::Options::default()) {
            Ok(tree) => tree,
            Err(usvg::Error::InvalidSize) if is_zero_sized_svg(bytes) => {
                return Ok(Self {
                    source: SpriteSource::Empty,
                });
            }
            Err(error) => return Err(SpriteDecodeError::Svg(error.to_string())),
        };
        Ok(Self {
            source: SpriteSource::Svg(Box::new(tree)),
        })
    }

    pub(crate) fn is_vector(&self) -> bool {
        matches!(&self.source, SpriteSource::Svg(_))
    }

    pub(crate) fn mip_level_for_projected_size(
        &self,
        sprite_scale: f32,
        viewport_width: u32,
        viewport_height: u32,
        logical_size: [f32; 2],
        max_texture_dimension: u32,
    ) -> i8 {
        if !self.is_vector() {
            return 0;
        }
        let (source_width, source_height) = self.source_size();
        // viewport dimensions are physical pixels, while logical_size and the
        // sprite transform use Scratch stage units. This ratio therefore also
        // accounts for bitmap resolution without applying window DPI twice.
        let projected_x = sprite_scale.abs() * viewport_width as f32 / SCRATCH_WIDTH as f32
            * logical_size[0]
            / source_width;
        let projected_y = sprite_scale.abs() * viewport_height as f32 / SCRATCH_HEIGHT as f32
            * logical_size[1]
            / source_height;
        let requested_scale = projected_x.max(projected_y).max(f32::MIN_POSITIVE);
        let (source_width, source_height) = self.source_size();
        let texture_limit = max_texture_dimension.min(SCRATCH_MAX_TEXTURE_DIMENSION) as f32;
        let maximum_scale = (texture_limit / source_width)
            .min(texture_limit / source_height)
            .max(f32::MIN_POSITIVE);
        let maximum_level = maximum_scale.log2().floor() as i32;
        let requested_level = requested_scale.log2().ceil() as i32;
        // scratch uses mip level 8 for 1x and supports levels below it down to
        // zero. If an unusually large SVG needs more downscaling to fit the
        // device limit, the texture limit takes precedence.
        requested_level.clamp((-8).min(maximum_level), maximum_level) as i8
    }

    pub(crate) fn raster_scale_for_mip(level: i8) -> f32 {
        2.0_f32.powi(level as i32)
    }

    pub(crate) fn logical_size(&self, bitmap_resolution: f32) -> [f32; 2] {
        if matches!(&self.source, SpriteSource::Empty) {
            return [0.0, 0.0];
        }
        let (width, height) = self.source_size();
        [
            width / bitmap_resolution.max(1.0),
            height / bitmap_resolution.max(1.0),
        ]
    }

    // derive the same opaque-pixel hull used by scratch's tight drawable bounds.
    pub(crate) fn opaque_bounds_hull(&self, bitmap_resolution: f64) -> Vec<[f64; 2]> {
        let (source_width, source_height) = self.source_size();
        let (width, height) = match &self.source {
            // scratch's SVG canvas dimensions truncate fractional natural sizes at 1x.
            SpriteSource::Svg(_) => (
                source_width.floor().max(1.0) as u32,
                source_height.floor().max(1.0) as u32,
            ),
            SpriteSource::Png(source) => (source.width(), source.height()),
            SpriteSource::Empty => return Vec::new(),
        };
        let Some(mut pixmap) = tiny_skia::Pixmap::new(width, height) else {
            return Vec::new();
        };
        match &self.source {
            SpriteSource::Svg(tree) => {
                resvg::render(tree, tiny_skia::Transform::identity(), &mut pixmap.as_mut());
            }
            SpriteSource::Png(source) => pixmap.draw_pixmap(
                0,
                0,
                source.as_ref(),
                &tiny_skia::PixmapPaint::default(),
                tiny_skia::Transform::identity(),
                None,
            ),
            SpriteSource::Empty => return Vec::new(),
        }

        let logical_scale = bitmap_resolution.max(1.0);
        let logical_width = source_width as f64 / logical_scale;
        let logical_height = source_height as f64 / logical_scale;
        let row_bytes = width as usize * 4;
        let mut edge_points = Vec::with_capacity(logical_height.ceil() as usize * 2);
        for sample_y in 0..logical_height.ceil() as usize {
            let texture_y =
                (sample_y as f64 / logical_height * (height - 1) as f64).floor() as usize;
            let mut first = None;
            let mut last = 0;
            for sample_x in 0..logical_width.ceil() as usize {
                let texture_x =
                    (sample_x as f64 / logical_width * (width - 1) as f64).floor() as usize;
                let touches = (texture_y..=texture_y + 1).any(|y| {
                    (texture_x..=texture_x + 1).any(|x| {
                        x < width as usize
                            && y < height as usize
                            && pixmap.data()[y * row_bytes + x * 4 + 3] != 0
                    })
                });
                if touches {
                    first.get_or_insert(sample_x);
                    last = sample_x;
                }
            }
            if let Some(first) = first {
                // scratch's hull samples use logical integer coordinates, not texel centers.
                edge_points.push([first as f64, sample_y as f64]);
                if last != first {
                    edge_points.push([last as f64, sample_y as f64]);
                }
            }
        }
        convex_hull(edge_points)
    }

    pub(crate) fn texture_size(&self, raster_scale: f32) -> (u32, u32) {
        let (width, height) = self.source_size();
        let scale = if self.is_vector() {
            raster_scale.max(f32::MIN_POSITIVE)
        } else {
            1.0
        };
        (
            (width * scale).ceil().max(1.0) as u32,
            (height * scale).ceil().max(1.0) as u32,
        )
    }

    pub(crate) fn upload_costume_to(
        &self,
        queue: &Queue,
        target: &RenderTexture,
        raster_scale: f32,
    ) -> Result<Vec<u8>, String> {
        let (width, height) = self.texture_size(raster_scale);
        if target.width != width || target.height != height {
            return Err("sprite costume texture dimensions do not match the costume".to_owned());
        }
        let mut pixmap = tiny_skia::Pixmap::new(width, height)
            .ok_or_else(|| "failed to allocate the sprite costume raster surface".to_owned())?;
        match &self.source {
            SpriteSource::Svg(tree) => {
                let (source_width, source_height) = self.source_size();
                let transform = tiny_skia::Transform::from_row(
                    width as f32 / source_width,
                    0.0,
                    0.0,
                    height as f32 / source_height,
                    0.0,
                    0.0,
                );
                resvg::render(tree, transform, &mut pixmap.as_mut());
            }
            SpriteSource::Png(source) => pixmap.draw_pixmap(
                0,
                0,
                source.as_ref(),
                &tiny_skia::PixmapPaint::default(),
                tiny_skia::Transform::identity(),
                None,
            ),
            SpriteSource::Empty => {}
        }
        let mut opaque_pixels = vec![0_u8; (width as usize * height as usize).div_ceil(8)];
        for (index, pixel) in pixmap.data().chunks_exact(4).enumerate() {
            if pixel[3] != 0 {
                opaque_pixels[index / 8] |= 1 << (index % 8);
            }
        }
        target.write_rgba(queue, pixmap.data());
        Ok(opaque_pixels)
    }

    pub(crate) fn upload_backdrop_to(
        &self,
        queue: &Queue,
        target: &RenderTexture,
    ) -> Result<(), String> {
        let width = target.width;
        let height = target.height;
        let mut pixmap = tiny_skia::Pixmap::new(width, height)
            .ok_or_else(|| "failed to allocate the backdrop raster surface".to_owned())?;
        pixmap.fill(tiny_skia::Color::WHITE);

        let (source_width, source_height) = self.source_size();
        let transform = tiny_skia::Transform::from_row(
            width as f32 / source_width,
            0.0,
            0.0,
            height as f32 / source_height,
            0.0,
            0.0,
        );
        match &self.source {
            SpriteSource::Svg(tree) => resvg::render(tree, transform, &mut pixmap.as_mut()),
            SpriteSource::Png(source) => pixmap.draw_pixmap(
                0,
                0,
                source.as_ref(),
                &tiny_skia::PixmapPaint::default(),
                transform,
                None,
            ),
            SpriteSource::Empty => {}
        }
        target.write_rgba(queue, pixmap.data());
        Ok(())
    }

    fn source_size(&self) -> (f32, f32) {
        match &self.source {
            SpriteSource::Svg(tree) => (tree.size().width(), tree.size().height()),
            SpriteSource::Png(pixmap) => (pixmap.width() as f32, pixmap.height() as f32),
            SpriteSource::Empty => (1.0, 1.0),
        }
    }

    #[cfg(test)]
    fn rasterize(&self, width: u32, height: u32) -> Result<tiny_skia::Pixmap, String> {
        let mut pixmap = tiny_skia::Pixmap::new(width, height)
            .ok_or_else(|| "failed to allocate the sprite raster surface".to_owned())?;

        let scale =
            (width as f32 / SCRATCH_WIDTH as f32).min(height as f32 / SCRATCH_HEIGHT as f32);
        let (sprite_width, sprite_height) = match &self.source {
            SpriteSource::Svg(tree) => (tree.size().width(), tree.size().height()),
            SpriteSource::Png(pixmap) => (pixmap.width() as f32, pixmap.height() as f32),
            SpriteSource::Empty => (1.0, 1.0),
        };
        let x = (width as f32 - sprite_width * scale) * 0.5 + SPRITE_CENTER_X_STAGE_PX * scale;
        let y = (height as f32 - sprite_height * scale) * 0.5;
        let transform = tiny_skia::Transform::from_row(scale, 0.0, 0.0, scale, x, y);

        match &self.source {
            SpriteSource::Svg(tree) => resvg::render(tree, transform, &mut pixmap.as_mut()),
            SpriteSource::Png(source) => pixmap.draw_pixmap(
                0,
                0,
                source.as_ref(),
                &tiny_skia::PixmapPaint {
                    quality: tiny_skia::FilterQuality::Nearest,
                    ..Default::default()
                },
                transform,
                None,
            ),
            SpriteSource::Empty => {}
        }
        Ok(pixmap)
    }
}

// use a monotone-chain hull over scanline extremes instead of retaining every opaque pixel.
fn convex_hull(mut points: Vec<[f64; 2]>) -> Vec<[f64; 2]> {
    points.sort_by(|left, right| {
        left[0]
            .total_cmp(&right[0])
            .then_with(|| left[1].total_cmp(&right[1]))
    });
    points.dedup();
    if points.len() <= 2 {
        return points;
    }

    let cross = |origin: [f64; 2], first: [f64; 2], second: [f64; 2]| {
        (first[0] - origin[0]) * (second[1] - origin[1])
            - (first[1] - origin[1]) * (second[0] - origin[0])
    };
    let mut hull = Vec::with_capacity(points.len() * 2);
    for point in &points {
        while hull.len() >= 2 && cross(hull[hull.len() - 2], hull[hull.len() - 1], *point) <= 0.0 {
            hull.pop();
        }
        hull.push(*point);
    }
    let lower_len = hull.len();
    for point in points.iter().rev().skip(1) {
        while hull.len() > lower_len
            && cross(hull[hull.len() - 2], hull[hull.len() - 1], *point) <= 0.0
        {
            hull.pop();
        }
        hull.push(*point);
    }
    hull.pop();
    hull
}

fn is_zero_sized_svg(bytes: &[u8]) -> bool {
    let Ok(xml) = std::str::from_utf8(bytes) else {
        return false;
    };
    let Ok(document) = roxmltree::Document::parse(xml) else {
        return false;
    };
    let root = document.root_element();
    if root.tag_name().name() != "svg" {
        return false;
    }
    ["width", "height"].into_iter().any(|name| {
        root.attribute(name)
            .and_then(|value| value.trim().trim_end_matches("px").parse::<f64>().ok())
            == Some(0.0)
    })
}

#[cfg(test)]
mod tests {
    use super::Sprite;

    #[test]
    fn embedded_costume_rasterizes_with_visible_pixels() {
        let sprite = Sprite::from_bytes(include_bytes!("../../resources/costume1.svg")).unwrap();
        let pixmap = sprite.rasterize(480, 360).unwrap();

        assert!(pixmap.data().chunks_exact(4).any(|pixel| pixel[3] != 0));
        assert_eq!(&pixmap.data()[0..4], &[0, 0, 0, 0]);
    }

    #[test]
    fn zero_sized_svg_costume_is_transparent() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="0" height="0" viewBox="0 0 0 0"><g/></svg>"#;
        let sprite = Sprite::from_bytes(svg).unwrap();
        assert_eq!(sprite.logical_size(1.0), [0.0, 0.0]);
        assert!(
            sprite
                .rasterize(480, 360)
                .unwrap()
                .data()
                .iter()
                .all(|byte| *byte == 0)
        );
        assert!(Sprite::from_bytes(b"<svg width='0' height='0'>").is_err());
    }

    #[test]
    fn svg_mip_selection_tracks_projected_physical_size_and_caps_to_gpu_limit() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="50" viewBox="0 0 100 50"><rect width="100" height="50"/></svg>"#;
        let sprite = Sprite::from_bytes(svg).unwrap();
        let logical_size = sprite.logical_size(1.0);

        assert_eq!(
            sprite.mip_level_for_projected_size(1.0, 480, 360, logical_size, 4096),
            0
        );
        assert_eq!(
            sprite.mip_level_for_projected_size(2.0, 480, 360, logical_size, 4096),
            1
        );
        assert_eq!(
            sprite.mip_level_for_projected_size(1.0, 960, 720, logical_size, 4096),
            1
        );
        assert_eq!(
            sprite.mip_level_for_projected_size(0.1, 480, 360, logical_size, 4096),
            -3
        );
        assert_eq!(
            sprite.mip_level_for_projected_size(20.0, 480, 360, logical_size, 200),
            1
        );
        assert_eq!(sprite.logical_size(1.0), logical_size);
        assert_eq!(
            sprite.texture_size(Sprite::raster_scale_for_mip(1)),
            (200, 100)
        );
    }

    #[test]
    fn hd_pen_stamp_mip_uses_pen_surface_pixels_instead_of_standard_pen_resolution() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="50" viewBox="0 0 100 50"><rect width="100" height="50"/></svg>"#;
        let sprite = Sprite::from_bytes(svg).unwrap();
        let logical_size = sprite.logical_size(1.0);

        let standard_pen_mip =
            sprite.mip_level_for_projected_size(1.0, 480, 360, logical_size, 4096);
        let hd_pen_mip = sprite.mip_level_for_projected_size(1.0, 1920, 1440, logical_size, 4096);

        assert_eq!(standard_pen_mip, 0);
        assert_eq!(hd_pen_mip, 2);
    }

    #[test]
    fn png_costume_dimensions_do_not_follow_svg_mip_selection() {
        let pixmap = tiny_skia::Pixmap::new(3, 2).unwrap();
        let sprite = Sprite::from_bytes(&pixmap.encode_png().unwrap()).unwrap();

        assert_eq!(
            sprite.mip_level_for_projected_size(8.0, 3840, 2880, [3.0, 2.0], 4096),
            0
        );
        assert_eq!(sprite.texture_size(1.0), (3, 2));
        assert_eq!(sprite.texture_size(256.0), (3, 2));
    }
}
