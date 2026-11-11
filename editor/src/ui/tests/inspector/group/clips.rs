use super::*;
use comet_asset_data::product::{AssetKind, Product};
use comet_asset_pipeline::{SourceMeta, prepare};
use comet_core::{AssetId, InstanceId};
use comet_scene::{Animator, ContentSource, EntityRef, PrefabInstance, PrefabResolver};

struct Clips {
    prefab: AssetId,
    skeleton: AssetId,
    clips: [AssetId; 2],
    foreign_skeleton: AssetId,
    foreign_clip: AssetId,
}

impl Group {
    fn prepare_clips(&mut self) -> Result<Clips> {
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
        Ok(Clips {
            prefab,
            skeleton: original,
            clips: own,
            foreign_skeleton: other,
            foreign_clip: clips(other)[0],
        })
    }
    fn animators(&mut self, clips: &Clips, foreign: bool) -> Result<()> {
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
                bindings: std::collections::BTreeMap::from([(
                    data.nodes[0].id,
                    Some(EntityRef::local(*id)),
                )]),
                ..Animator::new(skeleton)
            });
        }
        self.editor.document = Document::open_source(source)?;
        Ok(())
    }
    fn select_no_clip(&mut self) -> Result<()> {
        let anchor = self.open_asset_picker("播放片段")?;
        let output = self.frame(vec![]);
        let position = texts(&output)
            .into_iter()
            .find(|(text, point)| text == "不播放片段" && point.y > anchor.y + 3.)
            .context("no clip option")?
            .1;
        self.frame(vec![Event::PointerMoved(position), button(position, true)]);
        self.frame(vec![button(position, false)]);
        Ok(())
    }
}

#[test]
fn common_animation_clip_choices_filter_by_every_skeleton_and_preserve_binding_configuration()
-> Result<()> {
    let mut f = Group::new()?;
    let clips = f.prepare_clips()?;
    f.animators(&clips, false)?;
    let original = f.editor.document.shared_source();
    f.open_asset_picker("播放片段")?;
    let output = f.frame(vec![]);
    let labels = texts(&output);
    assert!(
        labels
            .iter()
            .any(|(text, _)| text == &f.asset_label(clips.clips[0]))
    );
    assert!(
        labels
            .iter()
            .any(|(text, _)| text == &f.asset_label(clips.clips[1]))
    );
    assert!(
        !labels
            .iter()
            .any(|(text, _)| text == &f.asset_label(clips.foreign_clip))
    );
    f.frame(vec![key(egui::Key::Escape, true, Default::default())]);
    f.frame(vec![key(egui::Key::Escape, false, Default::default())]);
    assert_eq!(f.editor.document.shared_source(), original);
    f.select_asset("播放片段", clips.clips[0])?;
    assert_eq!(f.editor.document.history_len(), 1);
    let edited = f.editor.document.shared_source();
    for (before, after) in original.base.entities.iter().zip(&edited.base.entities) {
        let mut expected = before.animator.clone().unwrap();
        expected.clip = Some(clips.clips[0]);
        assert_eq!(after.animator, Some(expected));
    }
    f.select_asset("播放片段", clips.clips[0])?;
    assert_eq!(f.editor.document.history_len(), 1);
    f.select_no_clip()?;
    assert!(
        f.editor
            .document
            .shared_source()
            .base
            .entities
            .iter()
            .all(|entity| entity.animator.as_ref().unwrap().clip.is_none())
    );
    assert_eq!(f.editor.document.history_len(), 2);
    f.editor.document.undo()?;
    f.editor.document.undo()?;
    assert_eq!(f.editor.document.shared_source(), original);
    Ok(())
}

#[test]
fn different_skeletons_allow_only_no_clip_and_reject_incompatible_or_wrong_kind_commands()
-> Result<()> {
    let mut f = Group::new()?;
    let clips = f.prepare_clips()?;
    f.animators(&clips, true)?;
    let original = f.editor.document.shared_source();
    assert!(
        f.editor
            .replace_animation_clips(&f.ids, Some(clips.clips[0]), &mut f.assets)
            .is_err()
    );
    assert!(
        f.editor
            .replace_animation_clips(&f.ids, Some(clips.skeleton), &mut f.assets)
            .is_err()
    );
    assert_eq!(f.editor.document.shared_source(), original);
    assert!(!f.editor.document.is_editing());
    f.open_asset_picker("播放片段")?;
    let output = f.frame(vec![]);
    assert!(
        texts(&output)
            .iter()
            .any(|(text, _)| text == "没有匹配的资产")
    );
    assert!(
        !texts(&output)
            .iter()
            .any(|(text, _)| text == &f.asset_label(clips.clips[0])
                || text == &f.asset_label(clips.foreign_clip))
    );
    f.frame(vec![key(egui::Key::Escape, true, Default::default())]);
    f.frame(vec![key(egui::Key::Escape, false, Default::default())]);
    f.select_no_clip()?;
    assert_eq!(f.editor.document.history_len(), 1);
    assert!(
        f.editor
            .document
            .shared_source()
            .base
            .entities
            .iter()
            .all(|entity| entity.animator.as_ref().unwrap().clip.is_none())
    );
    f.editor.document.undo()?;
    assert_eq!(f.editor.document.shared_source(), original);
    Ok(())
}

#[test]
fn idle_common_asset_menus_use_metadata_without_loading_rig_products() -> Result<()> {
    let mut f = Group::new()?;
    let clips = f.prepare_clips()?;
    f.animators(&clips, false)?;
    let snapshot = f.editor.snapshot(&mut f.assets)?;
    let entities: Vec<_> = f
        .ids
        .iter()
        .map(|id| {
            snapshot
                .entities
                .iter()
                .find(|entity| entity.id == *id)
                .unwrap()
        })
        .collect();
    for id in f
        .assets
        .index()
        .products
        .keys()
        .copied()
        .collect::<Vec<_>>()
    {
        f.assets.evict(id);
    }
    let original = f.editor.document.shared_source();
    for _ in 0..3 {
        let mut output = f.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    Pos2::ZERO,
                    Vec2::new(1280., 800.),
                )),
                ..Default::default()
            },
            |ui| crate::ui::inspector::common::draw(&mut f.editor, ui, &mut f.assets, &entities),
        );
        output.textures_delta.clear();
    }
    assert_eq!(f.assets.resident_count(), 0);
    assert_eq!(f.editor.document.shared_source(), original);
    assert_eq!(f.editor.document.history_len(), 0);
    Ok(())
}

#[test]
fn direct_prefab_clip_choices_keep_source_identity_and_remain_read_only_in_play() -> Result<()> {
    let mut f = Group::new()?;
    let clips = f.prepare_clips()?;
    let prefab = f.assets.prefab(clips.prefab)?;
    let controller = prefab
        .base
        .entities
        .iter()
        .find(|entity| entity.animator.is_some())
        .context("imported animator")?
        .id;
    let current = prefab
        .base
        .entities
        .iter()
        .find(|entity| entity.id == controller)
        .unwrap()
        .animator
        .as_ref()
        .unwrap()
        .clip;
    let replacement = clips
        .clips
        .into_iter()
        .find(|id| Some(*id) != current)
        .unwrap();
    let instances = [InstanceId::new(), InstanceId::new()];
    f.editor.document = Document::open_source(ContentSource {
        instances: instances
            .map(|id| PrefabInstance {
                translation: [0.; 3],
                id,
                prefab: clips.prefab,
                parent: None,
                overrides: vec![],
            })
            .to_vec(),
        ..Default::default()
    })?;
    f.ids = instances.map(|instance| controller.in_instance(instance));
    f.editor.selection.replace(f.ids);
    let original = f.editor.document.shared_source();
    f.select_asset("播放片段", replacement)?;
    let edited = f.editor.document.shared_source();
    for instance in &edited.instances {
        assert_eq!(instance.overrides.len(), 1);
        assert_eq!(instance.overrides[0].entity, controller);
        assert_eq!(
            instance.overrides[0].field,
            comet_scene::schema::ANIMATOR_CLIP
        );
    }
    f.editor.save(&mut f.assets)?;
    assert_eq!(Document::load(&f.editor.source)?.shared_source(), edited);
    assert_eq!(f.assets.prefab(clips.prefab)?, prefab);
    f.editor.document.undo()?;
    assert_eq!(f.editor.document.shared_source(), original);
    f.editor.start(&mut f.assets)?;
    f.editor.selection.replace(f.ids);
    f.open_asset_picker("播放片段")?;
    assert!(
        f.editor
            .replace_animation_clips(&f.ids, None, &mut f.assets)
            .is_err()
    );
    assert_eq!(f.editor.document.shared_source(), original);
    assert_eq!(f.editor.document.history_len(), 0);
    f.editor.stop();
    Ok(())
}

#[test]
#[ignore = "requires a native offscreen GPU; tools/verify/verify.py --gpu runs this test"]
fn headless_common_animation_clip_menu_keeps_bindings_and_authoring_unchanged() -> Result<()> {
    let mut f = Group::new()?;
    let clips = f.prepare_clips()?;
    let prefab = f.assets.prefab(clips.prefab)?;
    let controller = prefab
        .base
        .entities
        .iter()
        .find(|entity| entity.animator.is_some())
        .context("imported animator")?
        .id;
    let instances = [InstanceId::new(), InstanceId::new()];
    f.editor.document = Document::open_source(ContentSource {
        instances: instances
            .iter()
            .enumerate()
            .map(|(index, &id)| PrefabInstance {
                translation: [0.; 3],
                id,
                prefab: clips.prefab,
                parent: None,
                overrides: if index == 0 {
                    vec![]
                } else {
                    vec![comet_scene::FieldOverride {
                        entity: controller,
                        field: comet_scene::schema::ANIMATOR_CLIP,
                        value: comet_scene::schema::FieldValue::OptionalAsset(None),
                    }]
                },
            })
            .collect(),
        ..Default::default()
    })?;
    f.ids = instances.map(|instance| controller.in_instance(instance));
    f.editor.selection.replace(f.ids);
    let output = f.frame(vec![]);
    assert!(
        texts(&output)
            .iter()
            .any(|(text, _)| text == "播放片段（多值）")
    );
    capture_common(
        f,
        "editor-common-animation-clips",
        serde_json::json!({"asset_fields": ["播放片段（多值）"], "source_unchanged": true}),
    )
}
