use super::*;
use comet_asset_data::product::{AssetKind, Product};
use comet_asset_pipeline::{SourceMeta, prepare};
use comet_core::AssetId;
use comet_scene::{
    Animator, AudioSource, BodyMotion, Character, ColliderShape, EntityRef, PhysicsBody, Script,
    ScriptValue,
};
use std::path::PathBuf;

impl Group {
    pub(super) fn new() -> Result<Self> {
        let (temp, mut editor, mut assets) = fixture()?;
        let model = editor
            .snapshot(&mut assets)?
            .entities
            .iter()
            .find_map(|entity| entity.model.clone())
            .context("renderable fixture model")?;
        let mut parent = EntityData::new("主层级");
        parent.transform = Transform {
            translation: [-1., 1., 0.],
            rotation: Quat::from_euler(EulerRot::XYZ, 0.1, 0.2, 0.3).to_array(),
            scale: [1., 1., 1.],
        };
        parent.model = Some(model.clone());
        parent.model.as_mut().unwrap().surface.roughness = 0.2;
        let mut child = EntityData::new("主实体");
        child.parent = Some(parent.id);
        child.transform = Transform {
            translation: [1., 2., 0.5],
            rotation: Quat::from_euler(EulerRot::XYZ, 0.4, 0.5, 0.6).to_array(),
            scale: [1.5, 1.5, 1.5],
        };
        child.model = Some(model);
        child.model.as_mut().unwrap().surface.roughness = 0.8;
        let ids = [parent.id, child.id];
        editor.document = Document::open(SceneSnapshot {
            entities: vec![parent, child],
            ..Default::default()
        })?;
        editor.selection.replace(ids);
        let ctx = egui::Context::default();
        comet_application::configure_ui(&ctx);
        ctx.options_mut(|options| options.max_passes = 1.try_into().unwrap());
        Ok(Self {
            _temp: temp,
            editor,
            assets,
            ctx,
            ids,
            number: 0,
        })
    }
    pub(super) fn physics(&mut self, character: bool) -> Result<()> {
        let mut source = self.editor.document.source();
        for (index, id) in self.ids.iter().enumerate() {
            let entity = source
                .base
                .entities
                .iter_mut()
                .find(|entity| entity.id == *id)
                .unwrap();
            entity.parent = None;
            entity.physics = Some(PhysicsBody {
                mass: 1. + index as f32,
                friction: 0.2 + index as f32 * 0.3,
                ..Default::default()
            });
            if character {
                entity.transform.rotation = Quat::IDENTITY.to_array();
                let body = entity.physics.as_mut().unwrap();
                body.motion = BodyMotion::Kinematic;
                body.shape = ColliderShape::Capsule {
                    half_height: 0.5,
                    radius: 0.3,
                };
                entity.character = Some(Character {
                    player_controlled: index == 0,
                    move_speed: 3. + index as f32,
                    jump_speed: 6. + index as f32,
                    ..Default::default()
                });
            }
        }
        self.editor.document = Document::open_source(source)?;
        Ok(())
    }
    pub(super) fn audio(&mut self) -> Result<PathBuf> {
        let path = self.editor.project_root().join("assets/common.wav");
        std::fs::copy(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../demo/assets/audio/play_chime.wav"),
            &path,
        )?;
        prepare(self.editor.project_root(), &self.editor.products)?;
        self.assets.refresh()?;
        let clip = SourceMeta::load(&path)?.id;
        let mut source = self.editor.document.source();
        for (index, id) in self.ids.iter().enumerate() {
            source
                .base
                .entities
                .iter_mut()
                .find(|entity| entity.id == *id)
                .unwrap()
                .audio_source = Some(AudioSource {
                gain: 0.25 + index as f32 * 0.5,
                looping: index == 0,
                autoplay: index == 1,
                spatial: index == 0,
                ..AudioSource::new(clip)
            });
        }
        self.editor.document = Document::open_source(source)?;
        Ok(path)
    }
    pub(super) fn mixed_models(&mut self) -> Result<([AssetId; 2], [AssetId; 2])> {
        let products = |kind| -> Result<[AssetId; 2]> {
            self.assets
                .index()
                .products
                .iter()
                .filter_map(|(&id, entry)| (entry.kind == kind).then_some(id))
                .take(2)
                .collect::<Vec<_>>()
                .try_into()
                .map_err(|_| anyhow::anyhow!("two fixture model products"))
        };
        let meshes = products(AssetKind::Mesh)?;
        let materials = products(AssetKind::Material)?;
        let mut source = self.editor.document.source();
        for (index, id) in self.ids.iter().enumerate() {
            let model = source
                .base
                .entities
                .iter_mut()
                .find(|entity| entity.id == *id)
                .unwrap()
                .model
                .as_mut()
                .unwrap();
            model.mesh = meshes[index];
            model.surface.material = materials[index];
        }
        self.editor.document = Document::open_source(source)?;
        Ok((meshes, materials))
    }
    pub(super) fn resource_components(&mut self) -> Result<([AssetId; 2], [AssetId; 2])> {
        let first = self.audio()?;
        let second = self.editor.project_root().join("assets/second.wav");
        std::fs::copy(&first, &second)?;
        let scripts = ["first.rhai", "second.rhai"]
            .map(|name| self.editor.project_root().join("assets").join(name));
        for path in &scripts {
            std::fs::write(path, "fn on_start(ctx) {}")?;
        }
        prepare(self.editor.project_root(), &self.editor.products)?;
        self.assets.refresh()?;
        let clips = [SourceMeta::load(&first)?.id, SourceMeta::load(&second)?.id];
        let scripts = [
            SourceMeta::load(&scripts[0])?.id,
            SourceMeta::load(&scripts[1])?.id,
        ];
        let mut source = self.editor.document.source();
        for (index, id) in self.ids.iter().enumerate() {
            let entity = source
                .base
                .entities
                .iter_mut()
                .find(|entity| entity.id == *id)
                .unwrap();
            entity.audio_source.as_mut().unwrap().clip = clips[index];
            entity.script = Some(Script {
                enabled: index == 0,
                parameters: std::collections::BTreeMap::from([
                    (
                        "owner".into(),
                        ScriptValue::Entity(Some(EntityRef::local(*id))),
                    ),
                    ("number".into(), ScriptValue::Number(index as f64 + 0.125)),
                ]),
                ..Script::new(scripts[index])
            });
        }
        self.editor.document = Document::open_source(source)?;
        Ok((clips, scripts))
    }
}

pub(super) struct AnimationAssets {
    pub(super) prefab: AssetId,
    pub(super) skeleton: AssetId,
    pub(super) clips: [AssetId; 2],
    pub(super) foreign_skeleton: AssetId,
    pub(super) foreign_clip: AssetId,
    pub(super) controllers: [AssetId; 2],
    pub(super) foreign_controller: AssetId,
    pub(super) skins: [AssetId; 2],
    pub(super) foreign_skin: AssetId,
}

impl Group {
    pub(super) fn prepare_animations(&mut self) -> Result<AnimationAssets> {
        let prefab = crate::test_support::prepare_skin_asset(&self.editor)?;
        let path = self
            .editor
            .project_root()
            .join("assets/model/SimpleSkin.gltf");
        let mut data: serde_json::Value = serde_json::from_slice(&std::fs::read(&path)?)?;
        let mut second = data["animations"][0].clone();
        second["name"] = "第二片段".into();
        data["animations"].as_array_mut().unwrap().push(second);
        std::fs::write(&path, serde_json::to_vec_pretty(&data)?)?;
        let mut skin = data["skins"][0].clone();
        skin["name"] = "第二蒙皮".into();
        data["skins"].as_array_mut().unwrap().push(skin);
        let mut node = data["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["skin"] == 0)
            .unwrap()
            .clone();
        node["skin"] = 1.into();
        node["name"] = "第二蒙皮网格".into();
        node["translation"] = serde_json::json!([2., 0., 0.]);
        let index = data["nodes"].as_array().unwrap().len();
        data["nodes"].as_array_mut().unwrap().push(node);
        data["scenes"][0]["nodes"]
            .as_array_mut()
            .unwrap()
            .push(index.into());
        std::fs::write(&path, serde_json::to_vec_pretty(&data)?)?;
        let foreign = path.with_file_name("Foreign.gltf");
        std::fs::write(&foreign, serde_json::to_vec_pretty(&data)?)?;
        prepare(self.editor.project_root(), &self.editor.products)?;
        self.assets.refresh()?;
        let foreign = SourceMeta::load(&foreign)?.id;
        let skeleton = |source| {
            self.assets
                .index()
                .products
                .iter()
                .find_map(|(&id, entry)| {
                    (entry.kind == AssetKind::Skeleton
                        && entry
                            .editor
                            .as_ref()
                            .is_some_and(|metadata| metadata.source == source))
                    .then_some(id)
                })
                .context("fixture skeleton")
        };
        let original = skeleton(prefab)?;
        let other = skeleton(foreign)?;
        let clips = |skeleton| {
            self.assets
                .index()
                .products
                .iter()
                .filter_map(|(&id, entry)| {
                    (entry.kind == AssetKind::AnimationClip
                        && entry.dependencies.contains(&skeleton))
                    .then_some(id)
                })
                .collect::<Vec<_>>()
        };
        let own: [AssetId; 2] = clips(original)
            .try_into()
            .map_err(|_| anyhow::anyhow!("two compatible clips"))?;
        let foreign_clip = clips(other)[0];
        let skin_ids = |skeleton| {
            self.assets
                .index()
                .products
                .iter()
                .filter_map(|(&id, entry)| {
                    (entry.kind == AssetKind::Skin && entry.dependencies.contains(&skeleton))
                        .then_some(id)
                })
                .collect::<Vec<_>>()
        };
        let skins: [AssetId; 2] = skin_ids(original)
            .try_into()
            .map_err(|_| anyhow::anyhow!("two compatible skins"))?;
        let foreign_skin = skin_ids(other)[0];
        let paths = ["first.ani", "second.ani", "foreign.ani"]
            .map(|name| self.editor.project_root().join("assets").join(name));
        for (index, path) in paths.iter().enumerate() {
            let (skeleton, clip) = if index == 2 {
                (other, foreign_clip)
            } else {
                (original, own[index])
            };
            std::fs::write(
                path,
                serde_json::to_vec(&serde_json::json!({
                    "version":1, "skeleton":skeleton, "parameters":{}, "initial":"idle",
                    "states":[{"name":"idle", "looping":true, "motions":[{"threshold":0., "clip":clip}]}], "transitions":[]
                }))?,
            )?;
        }
        prepare(self.editor.project_root(), &self.editor.products)?;
        self.assets.refresh()?;
        Ok(AnimationAssets {
            prefab,
            skeleton: original,
            clips: own,
            foreign_skeleton: other,
            foreign_clip,
            controllers: [
                SourceMeta::load(&paths[0])?.id,
                SourceMeta::load(&paths[1])?.id,
            ],
            foreign_controller: SourceMeta::load(&paths[2])?.id,
            skins,
            foreign_skin,
        })
    }

    pub(super) fn animators(&mut self, clips: &AnimationAssets, foreign: bool) -> Result<()> {
        let mut source = self.editor.document.source();
        for (index, id) in self.ids.iter().enumerate() {
            let skeleton = if foreign && index == 1 {
                clips.foreign_skeleton
            } else {
                clips.skeleton
            };
            let product = self.assets.load(skeleton)?;
            let Product::Skeleton(data) = &product.payload else {
                unreachable!()
            };
            source
                .base
                .entities
                .iter_mut()
                .find(|entity| entity.id == *id)
                .unwrap()
                .animator = Some(Animator {
                clip: if index == 0 {
                    Some(clips.clips[0])
                } else {
                    None
                },
                looping: index == 0,
                speed: index as f32 + 0.5,
                bindings: std::collections::BTreeMap::from([(data.nodes[0].id, None)]),
                ..Animator::new(skeleton)
            });
        }
        self.editor.document = Document::open_source(source)?;
        Ok(())
    }
}
