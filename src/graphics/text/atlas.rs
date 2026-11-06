use std::collections::HashMap;

use ab_glyph::{Font, FontArc};

pub(crate) const INITIAL_SIZE: u32 = 128;
pub(crate) const MAX_SIZE: u32 = 2048;
pub(crate) const GLYPH_CAPACITY: u32 = 4096;

#[derive(Clone, Copy)]
pub(crate) struct GlyphAtlasEntry {
    pub(crate) id: u32,
    pub(crate) x: u32,
    pub(crate) y: u32,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) bounds: [f32; 4],
}

/// Owns glyph pixels, packing state, and the regions awaiting GPU upload.
pub(crate) struct GlyphAtlas {
    pixels: Vec<u8>,
    size: u32,
    revision: u64,
    full_dirty: bool,
    entries: HashMap<(char, bool, u32), GlyphAtlasEntry>,
    dirty_regions: Vec<(u32, u32, u32, u32)>,
    scale_bucket: u32,
    count: u32,
    next_x: u32,
    next_y: u32,
    row_height: u32,
}

impl GlyphAtlas {
    pub(crate) fn new() -> Self {
        Self {
            pixels: vec![0; (INITIAL_SIZE * INITIAL_SIZE) as usize],
            size: INITIAL_SIZE,
            revision: 1,
            full_dirty: false,
            entries: HashMap::new(),
            dirty_regions: Vec::new(),
            scale_bucket: 1,
            count: 0,
            next_x: 0,
            next_y: 0,
            row_height: 0,
        }
    }

    pub(crate) fn set_scale(&mut self, bucket: u32) -> bool {
        if self.scale_bucket == bucket {
            return false;
        }
        self.scale_bucket = bucket;
        self.size = INITIAL_SIZE;
        self.pixels = vec![0; (self.size * self.size) as usize];
        self.entries.clear();
        self.dirty_regions.clear();
        self.full_dirty = false;
        self.count = 0;
        self.next_x = 0;
        self.next_y = 0;
        self.row_height = 0;
        self.revision = self.revision.wrapping_add(1);
        true
    }

    pub(crate) fn ensure(
        &mut self,
        font: &FontArc,
        character: char,
        bold: bool,
    ) -> Result<(), String> {
        let bucket = self.scale_bucket;
        if self.entries.contains_key(&(character, bold, bucket)) {
            return Ok(());
        }
        let logical_size = if bold { 18.0 } else { 19.0 };
        let outline = font.outline_glyph(
            font.glyph_id(character)
                .with_scale(logical_size * bucket as f32),
        );
        let bounds = outline.as_ref().map(|glyph| glyph.px_bounds());
        let (width, height) = bounds.map_or((0, 0), |bounds| {
            (
                (bounds.max.x - bounds.min.x).ceil().max(0.0) as u32,
                (bounds.max.y - bounds.min.y).ceil().max(0.0) as u32,
            )
        });
        if self.count >= GLYPH_CAPACITY {
            return Err("variable glyph atlas is full".to_owned());
        }
        let slot = self.count;
        let (cell_x, cell_y) = if width == 0 || height == 0 {
            (0, 0)
        } else {
            if width + 2 > MAX_SIZE || height + 2 > MAX_SIZE {
                return Err(format!(
                    "glyph {character:?} at scale {bucket} exceeds the atlas dimensions"
                ));
            }
            loop {
                let mut x = self.next_x;
                let mut y = self.next_y;
                let mut row_height = self.row_height;
                if x + width + 2 > self.size {
                    x = 0;
                    y += row_height;
                    row_height = 0;
                }
                if y + height + 2 > self.size {
                    self.grow()?;
                    continue;
                }
                self.next_x = x + width + 2;
                self.next_y = y;
                self.row_height = row_height.max(height + 2);
                break (x + 1, y + 1);
            }
        };
        if let Some(outline) = outline {
            let atlas = &mut self.pixels;
            let atlas_size = self.size;
            outline.draw(|x, y, coverage| {
                if x < width && y < height {
                    atlas[((cell_y + y) * atlas_size + cell_x + x) as usize] =
                        (coverage * 255.0).round() as u8;
                }
            });
        }
        if width > 0 && height > 0 {
            self.dirty_regions.push((cell_x, cell_y, width, height));
        }
        self.count += 1;
        self.entries.insert(
            (character, bold, bucket),
            GlyphAtlasEntry {
                id: slot,
                x: cell_x,
                y: cell_y,
                width,
                height,
                // Keep pixel bounds in stage units at each atlas scale tier.
                bounds: bounds.map_or([0.0; 4], |bounds| scaled_px_bounds(bounds, bucket)),
            },
        );
        self.revision = self.revision.wrapping_add(1);
        Ok(())
    }

    pub(crate) fn get(&self, character: char, bold: bool) -> Option<&GlyphAtlasEntry> {
        self.entries.get(&(character, bold, self.scale_bucket))
    }

    pub(crate) fn entries(&self) -> impl Iterator<Item = &GlyphAtlasEntry> {
        self.entries.values()
    }

    pub(crate) fn size(&self) -> u32 {
        self.size
    }

    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    pub(crate) fn glyph_count(&self) -> u32 {
        self.count
    }

    pub(crate) fn take_updates(&mut self) -> Vec<(u32, u32, u32, u32, Vec<u8>)> {
        if self.full_dirty {
            self.full_dirty = false;
            self.dirty_regions.clear();
            return vec![(0, 0, self.size, self.size, self.pixels.clone())];
        }
        self.dirty_regions
            .drain(..)
            .map(|(x, y, width, height)| {
                let mut pixels = Vec::with_capacity((width * height) as usize);
                for row in y..y + height {
                    let start = (row * self.size + x) as usize;
                    pixels.extend_from_slice(&self.pixels[start..start + width as usize]);
                }
                (x, y, width, height, pixels)
            })
            .collect()
    }

    fn grow(&mut self) -> Result<(), String> {
        if self.size >= MAX_SIZE {
            return Err("variable glyph atlas is full".to_owned());
        }
        let old_size = self.size;
        let new_size = (old_size * 2).min(MAX_SIZE);
        let mut pixels = vec![0; (new_size * new_size) as usize];
        for row in 0..old_size as usize {
            let old_start = row * old_size as usize;
            let new_start = row * new_size as usize;
            pixels[new_start..new_start + old_size as usize]
                .copy_from_slice(&self.pixels[old_start..old_start + old_size as usize]);
        }
        self.pixels = pixels;
        self.size = new_size;
        self.full_dirty = true;
        self.revision = self.revision.wrapping_add(1);
        Ok(())
    }
}

pub(crate) fn scaled_px_bounds(bounds: ab_glyph::Rect, scale_bucket: u32) -> [f32; 4] {
    let scale = scale_bucket as f32;
    [
        bounds.min.x / scale,
        bounds.min.y / scale,
        bounds.width() / scale,
        bounds.height() / scale,
    ]
}
