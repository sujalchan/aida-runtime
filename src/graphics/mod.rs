mod pipelines;
pub(crate) mod sprite;
mod state;
mod texture;
mod variables;
mod viewport;

pub(crate) const SCRATCH_WIDTH: u32 = 480;
pub(crate) const SCRATCH_HEIGHT: u32 = 360;

pub(crate) use state::{GraphicsState, RenderOutcome};
#[allow(unused_imports)]
pub(crate) use variables::SliderRangeEditorInput;

pub(crate) fn fence_geometries(
    project: &aida_ast::parser::ScratchProject,
    assets_dir: &std::path::Path,
) -> Vec<(usize, crate::runtime::SpriteFenceGeometry)> {
    project
        .targets
        .iter()
        .enumerate()
        .filter(|(_, target)| !target.is_stage)
        .filter_map(|(target_index, target)| {
            let costume = target.costumes.get(target.current_costume)?;
            let bytes = std::fs::read(assets_dir.join(costume.archive_path()?)).ok()?;
            let sprite = sprite::Sprite::from_bytes(&bytes).ok()?;
            let resolution = costume.bitmap_resolution.unwrap_or(1.0).max(1.0) as f32;
            let logical_size = sprite.logical_size(resolution);
            Some((
                target_index,
                crate::runtime::SpriteFenceGeometry {
                    width: logical_size[0] as f64,
                    height: logical_size[1] as f64,
                    rotation_center_x: costume.rotation_center_x / resolution as f64,
                    rotation_center_y: costume.rotation_center_y / resolution as f64,
                },
            ))
        })
        .collect()
}

#[cfg(test)]
mod fence_geometry_tests {
    use super::fence_geometries;
    use crate::runtime::{ScratchState, SpriteFenceGeometry};

    #[test]
    fn motion_point_svg_costume_provides_costume_aware_fence_bounds() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../aida-ast/tests/src/motion-point-test.sb3");
        let assets = std::env::temp_dir().join(format!("aida-motion-fence-{}", std::process::id()));
        let loaded = aida_ast::load_sb3(fixture, &assets).unwrap();
        let target = &loaded.project.targets[1];
        let geometry = fence_geometries(&loaded.project, &loaded.assets_dir);
        let (_, geometry) = geometry.iter().find(|(index, _)| *index == 1).unwrap();
        assert!(geometry.width > 0.0 && geometry.height > 0.0);
        let costume = &target.costumes[target.current_costume];
        let resolution = costume.bitmap_resolution.unwrap_or(1.0).max(1.0);
        assert_eq!(
            geometry.rotation_center_x,
            costume.rotation_center_x / resolution
        );
        assert_eq!(
            geometry.rotation_center_y,
            costume.rotation_center_y / resolution
        );
        let mut state = ScratchState::new(Vec::new());
        state.register_target(0, "Stage", true);
        state.register_target(1, "Sprite1", false);
        state.set_target_fence_geometry(
            1,
            SpriteFenceGeometry {
                width: geometry.width,
                height: geometry.height,
                rotation_center_x: geometry.rotation_center_x,
                rotation_center_y: geometry.rotation_center_y,
            },
        );
        let sprite = state.original_instance(1).unwrap();
        state.set_current_instance(sprite);
        state.set_xy(500.0, 0.0);
        let inset = (geometry.width.min(geometry.height) / 2.0).floor();
        let fence_x = 240.0 - 15.0_f64.min(inset);
        let expected_x = (fence_x + geometry.rotation_center_x).floor();
        assert_eq!(state.position(sprite), Some((expected_x, 0.0)));
        let _ = std::fs::remove_dir_all(assets);
    }
}
