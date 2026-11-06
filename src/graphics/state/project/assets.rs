use super::super::*;
use crate::graphics::sprite::{Sprite, SpriteDecodeError};
use crate::runtime::ScratchRotationStyle;
use std::{fmt, io, path::PathBuf, sync::Arc};

#[derive(Debug)]
pub(crate) enum ProjectAssetError {
    MissingPath {
        kind: &'static str,
        owner: String,
        name: String,
    },
    Read {
        kind: &'static str,
        name: String,
        path: PathBuf,
        source: io::Error,
    },
    Decode {
        kind: &'static str,
        owner: String,
        name: String,
        source: SpriteDecodeError,
    },
}

impl fmt::Display for ProjectAssetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingPath { kind, owner, name } => {
                write!(formatter, "{kind} `{name}` on `{owner}` has no asset path")
            }
            Self::Read {
                kind,
                name,
                path,
                source,
            } => write!(
                formatter,
                "failed to read {kind} `{name}` from `{}`: {source}",
                path.display()
            ),
            Self::Decode {
                kind,
                owner,
                name,
                source,
            } => write!(
                formatter,
                "failed to load {kind} `{name}` on `{owner}`: {source}"
            ),
        }
    }
}

impl std::error::Error for ProjectAssetError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read { source, .. } => Some(source),
            Self::Decode { source, .. } => Some(source),
            Self::MissingPath { .. } => None,
        }
    }
}

pub(in crate::graphics::state) struct LoadedCostumeAsset {
    pub(in crate::graphics::state) sprite: Arc<Sprite>,
    pub(in crate::graphics::state) asset_id: String,
    pub(in crate::graphics::state) logical_size: [f32; 2],
    pub(in crate::graphics::state) rotation_center: [f32; 2],
}

fn sprite_costume_metrics(
    target: &aida_ast::parser::ScratchTarget,
    costume_index: usize,
    sprite: &Sprite,
) -> ([f32; 2], [f32; 2]) {
    let costume = target.costumes.get(costume_index);
    let raw_resolution = costume
        .and_then(|costume| costume.bitmap_resolution)
        .unwrap_or(1.0);
    let resolution = if raw_resolution == 0.0 {
        2.0
    } else if raw_resolution.is_finite() && raw_resolution > 0.0 {
        raw_resolution
    } else {
        1.0
    } as f32;
    let logical_size = costume
        .and_then(|costume| costume.logical_size)
        .filter(|[width, height]| {
            width.is_finite() && height.is_finite() && *width >= 0.0 && *height >= 0.0
        })
        .map(|[width, height]| [width as f32, height as f32])
        .unwrap_or_else(|| sprite.logical_size(resolution));
    let default_x = logical_size[0] * 0.5;
    let default_y = logical_size[1] * 0.5;
    (
        logical_size,
        [
            costume.map_or(default_x, |costume| {
                costume.rotation_center_x as f32 / resolution
            }),
            costume.map_or(default_y, |costume| {
                costume.rotation_center_y as f32 / resolution
            }),
        ],
    )
}

pub(in crate::graphics::state) fn sprite_transform(
    target: &aida_ast::parser::ScratchTarget,
    logical_size: [f32; 2],
    rotation_center: [f32; 2],
) -> SpriteTransform {
    let style = ScratchRotationStyle::from_scratch(&target.rotation_style).unwrap_or_default();
    let (direction, flip_x) = style.rendered_direction_and_flip(target.direction);
    SpriteTransform {
        position: [target.x as f32, target.y as f32],
        logical_size,
        rotation_center,
        direction: direction as f32,
        scale: target.size as f32 / 100.0,
        visible: if target.visible { 1.0 } else { 0.0 },
        flip_x: flip_x as f32,
        padding: [0.0; 2],
        effects_0: [0.0; 4],
        effects_1: [0.0; 4],
    }
}

pub(in crate::graphics::state) fn load_project_costumes(
    target: &aida_ast::parser::ScratchTarget,
    assets_dir: &std::path::Path,
    sources: &mut HashMap<String, Arc<Sprite>>,
) -> Result<Vec<LoadedCostumeAsset>, ProjectAssetError> {
    if target.costumes.is_empty() {
        let asset_id = "__aida/default-costume1.svg".to_owned();
        let sprite = sources
            .entry(asset_id.clone())
            .or_insert_with(|| {
                Sprite::from_bytes(include_bytes!("../../../../resources/costume1.svg"))
                    .map(Arc::new)
                    .expect("built-in costume should parse")
            })
            .clone();
        let (size, center) = sprite_costume_metrics(target, 0, &sprite);
        return Ok(vec![LoadedCostumeAsset {
            sprite,
            asset_id,
            logical_size: size,
            rotation_center: center,
        }]);
    }
    target
        .costumes
        .iter()
        .enumerate()
        .map(|(index, costume)| {
            let asset_id = costume.identity().to_owned();
            let sprite = if let Some(sprite) = sources.get(&asset_id) {
                Arc::clone(sprite)
            } else {
                let archive_path =
                    costume
                        .archive_path()
                        .ok_or_else(|| ProjectAssetError::MissingPath {
                            kind: "costume",
                            owner: target.name.clone(),
                            name: costume.name.clone(),
                        })?;
                let path = assets_dir.join(archive_path);
                let bytes = std::fs::read(&path).map_err(|source| ProjectAssetError::Read {
                    kind: "costume",
                    name: costume.name.clone(),
                    path,
                    source,
                })?;
                let sprite = Arc::new(Sprite::from_bytes(&bytes).map_err(|source| {
                    ProjectAssetError::Decode {
                        kind: "costume",
                        owner: target.name.clone(),
                        name: costume.name.clone(),
                        source,
                    }
                })?);
                sources.insert(asset_id.clone(), Arc::clone(&sprite));
                sprite
            };
            let (size, center) = sprite_costume_metrics(target, index, &sprite);
            Ok(LoadedCostumeAsset {
                sprite,
                asset_id,
                logical_size: size,
                rotation_center: center,
            })
        })
        .collect()
}

pub(in crate::graphics::state) fn load_project_backdrops(
    project: &aida_ast::parser::ScratchProject,
    assets_dir: &std::path::Path,
    sources: &mut HashMap<String, Arc<Sprite>>,
) -> Result<Vec<BackdropAsset>, ProjectAssetError> {
    let Some(stage) = project.targets.iter().find(|target| target.is_stage) else {
        return Ok(Vec::new());
    };
    stage
        .costumes
        .iter()
        .map(|costume| {
            let asset_id = costume.identity().to_owned();
            let sprite = if let Some(sprite) = sources.get(&asset_id) {
                Arc::clone(sprite)
            } else {
                let archive_path =
                    costume
                        .archive_path()
                        .ok_or_else(|| ProjectAssetError::MissingPath {
                            kind: "backdrop",
                            owner: stage.name.clone(),
                            name: costume.name.clone(),
                        })?;
                let path = assets_dir.join(archive_path);
                let bytes = std::fs::read(&path).map_err(|source| ProjectAssetError::Read {
                    kind: "backdrop",
                    name: costume.name.clone(),
                    path,
                    source,
                })?;
                let sprite = Arc::new(Sprite::from_bytes(&bytes).map_err(|source| {
                    ProjectAssetError::Decode {
                        kind: "backdrop",
                        owner: stage.name.clone(),
                        name: costume.name.clone(),
                        source,
                    }
                })?);
                sources.insert(asset_id.clone(), Arc::clone(&sprite));
                sprite
            };
            Ok(BackdropAsset { asset_id, sprite })
        })
        .collect()
}
