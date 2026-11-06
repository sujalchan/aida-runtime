use super::*;

impl GraphicsState {
    pub(super) fn recreate_stage_textures(&mut self) {
        let Some(viewport) = self.viewport else {
            return;
        };

        let (stage_width, stage_height) = if self.hd_pen_mode {
            (viewport.width, viewport.height)
        } else {
            (SCRATCH_WIDTH, SCRATCH_HEIGHT)
        };

        self.ensure_backdrop_texture(self.backdrop_index)
            .expect("backdrop should rasterize at the stage's pixel density");

        for sprite_index in 0..self.sprites.len() {
            let costume_index = self.sprites[sprite_index].current_costume;
            self.ensure_sprite_costume_mip(sprite_index, costume_index)
                .expect("sprite costume should rasterize at the stage's pixel density");
        }
        self.prune_unused_costume_mips();
        if self.pen_texture.width != stage_width || self.pen_texture.height != stage_height {
            self.pen_texture =
                RenderTexture::new(&self.device, "pen texture", stage_width, stage_height);
            self.pen_bind_group = layer_bind_group(
                &self.device,
                &self.texture_layout,
                &self.pen_texture,
                &self.pen_texture,
                // keep one scratch pen pixel crisp at every output scale.
                &self.stage_nearest_sampler,
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

    fn backdrop_dimensions(&self) -> (u32, u32) {
        let (width, height) = if self.hd_pen_mode {
            self.viewport
                .map(|viewport| (viewport.width, viewport.height))
                .unwrap_or((SCRATCH_WIDTH, SCRATCH_HEIGHT))
        } else {
            (SCRATCH_WIDTH, SCRATCH_HEIGHT)
        };
        let max_dimension = self.device.limits().max_texture_dimension_2d;
        (width.min(max_dimension), height.min(max_dimension))
    }

    pub(super) fn backdrop_cache_key(&self) -> (String, u32, u32) {
        let (width, height) = self.backdrop_dimensions();
        let asset_id = self
            .backdrops
            .get(self.backdrop_index)
            .map(|backdrop| backdrop.asset_id.clone())
            .unwrap_or_else(|| "__aida/empty-stage".to_owned());
        (asset_id, width, height)
    }

    pub(super) fn ensure_backdrop_texture(&mut self, backdrop_index: usize) -> Result<(), String> {
        let (width, height) = self.backdrop_dimensions();
        let asset_id = self
            .backdrops
            .get(backdrop_index)
            .map(|backdrop| backdrop.asset_id.clone())
            .unwrap_or_else(|| "__aida/empty-stage".to_owned());
        let key = (asset_id, width, height);
        self.backdrop_cache_clock = self.backdrop_cache_clock.wrapping_add(1);
        if let Some(cached) = self.backdrop_texture_cache.get_mut(&key) {
            cached.last_used = self.backdrop_cache_clock;
            return Ok(());
        }

        let texture = RenderTexture::new(&self.device, "cached backdrop texture", width, height);
        if let Some(backdrop) = self.backdrops.get(backdrop_index) {
            backdrop.sprite.upload_backdrop_to(&self.queue, &texture)?;
        } else {
            let white = vec![255; width as usize * height as usize * 4];
            texture.write_rgba(&self.queue, &white);
        }
        let bind_group = layer_bind_group(
            &self.device,
            &self.texture_layout,
            &texture,
            &texture,
            if self.hd_pen_mode {
                &self.stage_linear_sampler
            } else {
                &self.stage_nearest_sampler
            },
            &self.zero_uv_offset,
        );
        let bytes = width as usize * height as usize * 4;
        self.backdrop_texture_cache.insert(
            key.clone(),
            CachedBackdropTexture {
                _texture: texture,
                bind_group,
                last_used: self.backdrop_cache_clock,
                bytes,
            },
        );
        while self.backdrop_texture_cache.len() > 1
            && self
                .backdrop_texture_cache
                .values()
                .map(|cached| cached.bytes)
                .sum::<usize>()
                > 32 * 1024 * 1024
        {
            let Some(oldest) = self
                .backdrop_texture_cache
                .iter()
                .filter(|(candidate, _)| *candidate != &key)
                .min_by_key(|(_, cached)| cached.last_used)
                .map(|(candidate, _)| candidate.clone())
            else {
                break;
            };
            self.backdrop_texture_cache.remove(&oldest);
        }
        Ok(())
    }

    pub(super) fn ensure_sprite_costume_mip(
        &mut self,
        sprite_index: usize,
        costume_index: usize,
    ) -> Result<(), String> {
        let viewport = self.viewport.ok_or("stage viewport is unavailable")?;
        let mip_level = self.ensure_sprite_costume_mip_for_size(
            sprite_index,
            costume_index,
            self.sprites[sprite_index].transform.scale,
            (viewport.width, viewport.height),
        )?;
        self.sprites[sprite_index].costumes[costume_index].selected_mip = mip_level;
        Ok(())
    }

    // select svg detail from the physical pixels of the surface that will receive the draw.
    pub(super) fn ensure_sprite_costume_mip_for_size(
        &mut self,
        sprite_index: usize,
        costume_index: usize,
        sprite_scale: f32,
        surface_size: (u32, u32),
    ) -> Result<i8, String> {
        let max_texture_dimension = self.device.limits().max_texture_dimension_2d;
        let sprite = &self.sprites[sprite_index];
        let costume = &sprite.costumes[costume_index];
        let use_linear_sampling = costume.sprite.is_vector();
        let asset_id = costume.asset_id.clone();
        let sprite_source = Arc::clone(&costume.sprite);
        let mip_level = costume.sprite.mip_level_for_projected_size(
            sprite_scale,
            surface_size.0,
            surface_size.1,
            costume.logical_size,
            max_texture_dimension,
        );
        if !self.sprites[sprite_index].costumes[costume_index]
            .mips
            .contains_key(&mip_level)
        {
            let key = (asset_id, mip_level);
            let mip = if let Some(mip) = self.shared_costume_mips.get(&key) {
                Arc::clone(mip)
            } else {
                let raster_scale = Sprite::raster_scale_for_mip(mip_level);
                let (width, height) = sprite_source.texture_size(raster_scale);
                let texture =
                    RenderTexture::new(&self.device, "sprite costume mip texture", width, height);
                let opaque_pixels =
                    sprite_source.upload_costume_to(&self.queue, &texture, raster_scale)?;
                let texture_bind_group = layer_bind_group(
                    &self.device,
                    &self.texture_layout,
                    &texture,
                    &texture,
                    if use_linear_sampling {
                        &self.stage_linear_sampler
                    } else {
                        &self.stage_nearest_sampler
                    },
                    &self.zero_uv_offset,
                );
                let mip = Arc::new(SpriteCostumeMip {
                    texture,
                    opaque_pixels,
                    texture_bind_group,
                });
                self.shared_costume_mips.insert(key, Arc::clone(&mip));
                mip
            };
            self.sprites[sprite_index].costumes[costume_index]
                .mips
                .insert(mip_level, mip);
        }
        Ok(mip_level)
    }

    pub(super) fn prune_unused_costume_mips(&mut self) {
        let mut needed = std::collections::HashSet::new();
        for sprite in &self.sprites {
            if let Some(costume) = sprite.costumes.get(sprite.current_costume) {
                needed.insert((costume.asset_id.clone(), costume.selected_mip));
            }
        }
        for stamp in &self.pending_sprite_stamps {
            if let Some(costume) = self.sprites[stamp.sprite_index]
                .costumes
                .get(stamp.costume_index)
            {
                needed.insert((costume.asset_id.clone(), stamp.mip_level));
            }
        }
        self.shared_costume_mips
            .retain(|key, _| needed.contains(key));
        for sprite in &mut self.sprites {
            for costume in &mut sprite.costumes {
                let asset_id = &costume.asset_id;
                costume
                    .mips
                    .retain(|mip, _| needed.contains(&(asset_id.clone(), *mip)));
            }
        }
    }
}
