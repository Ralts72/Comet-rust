use super::*;
use comet_asset_data::product::AssetKind;
use comet_asset_pipeline::{SourceMeta, prepare};
use comet_core::AssetId;
use comet_scene::{
    AudioSource, BodyMotion, Character, ColliderShape, EntityRef, PhysicsBody, Script, ScriptValue,
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
