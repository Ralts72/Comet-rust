use super::*;
use comet_asset_pipeline::{SourceMeta, prepare};
use comet_core::{AssetId, InstanceId};
use comet_scene::{
    ContentSource, FieldOverride, PrefabInstance,
    schema::{self, FieldValue},
};
use std::sync::Arc;

#[test]
fn common_audio_and_script_choices_unify_primary_values_without_changing_configuration()
-> Result<()> {
    let mut f = Group::new()?;
    let (clips, scripts) = f.resource_components()?;
    let original = f.editor.document.shared_source();
    let source_bytes = std::fs::read(&f.editor.source)?;
    f.open_asset_picker("音频片段")?;
    f.frame(vec![key(egui::Key::Escape, true, Default::default())]);
    f.frame(vec![key(egui::Key::Escape, false, Default::default())]);
    assert_eq!(f.editor.document.shared_source(), original);
    f.select_asset("音频片段", clips[1])?;
    assert_eq!(f.editor.document.history_len(), 1);
    let accepted = f.editor.document.shared_source();
    f.select_asset("音频片段", clips[1])?;
    assert!(Arc::ptr_eq(&accepted, &f.editor.document.shared_source()));
    f.select_asset("脚本", scripts[1])?;
    let edited = f.editor.document.shared_source();
    assert_eq!(f.editor.document.history_len(), 2);
    for (before, after) in original.base.entities.iter().zip(&edited.base.entities) {
        let mut expected_audio = before.audio_source.clone().unwrap();
        expected_audio.clip = clips[1];
        assert_eq!(after.audio_source, Some(expected_audio));
        let mut expected_script = before.script.clone().unwrap();
        expected_script.asset = scripts[1];
        assert_eq!(after.script, Some(expected_script));
        assert_eq!(after.transform, before.transform);
        assert_eq!(after.model, before.model);
    }
    assert_eq!(std::fs::read(&f.editor.source)?, source_bytes);
    f.editor.save(&mut f.assets)?;
    assert_eq!(Document::load(&f.editor.source)?.shared_source(), edited);
    f.editor.document.undo()?;
    f.editor.document.undo()?;
    assert_eq!(f.editor.document.shared_source(), original);
    f.editor.document.redo()?;
    f.editor.document.redo()?;
    assert_eq!(f.editor.document.shared_source(), edited);
    assert!(!f.editor.document.is_dirty());
    Ok(())
}

#[test]
fn common_resource_validation_rejects_wrong_types_and_keeps_the_last_accepted_preview() -> Result<()>
{
    let mut f = Group::new()?;
    let (clips, scripts) = f.resource_components()?;
    let original = f.editor.document.shared_source();
    assert!(
        f.editor
            .replace_audio_clips(&f.ids, scripts[0], &mut f.assets)
            .is_err()
    );
    assert!(
        f.editor
            .replace_script_assets(&f.ids, clips[0], &mut f.assets)
            .is_err()
    );
    assert!(
        f.editor
            .replace_audio_clips(&f.ids, AssetId::new(), &mut f.assets)
            .is_err()
    );
    assert!(Arc::ptr_eq(&original, &f.editor.document.shared_source()));
    assert!(!f.editor.document.is_editing());
    f.editor
        .replace_audio_clips(&f.ids, clips[0], &mut f.assets)?;
    let accepted = f.editor.document.shared_source();
    assert!(
        f.editor
            .replace_script_assets(&[f.ids[0], EntityId::new()], scripts[0], &mut f.assets)
            .is_err()
    );
    assert!(Arc::ptr_eq(&accepted, &f.editor.document.shared_source()));
    assert!(f.editor.document.is_editing());
    f.editor.cancel()?;
    assert_eq!(f.editor.document.shared_source(), original);
    let mut source = f.editor.document.source();
    source
        .base
        .entities
        .iter_mut()
        .find(|entity| entity.id == f.ids[1])
        .unwrap()
        .script = None;
    f.editor.document = Document::open_source(source)?;
    let before = f.editor.document.shared_source();
    assert!(
        f.editor
            .replace_script_assets(&f.ids, scripts[0], &mut f.assets)
            .is_err()
    );
    assert_eq!(f.editor.document.shared_source(), before);
    let output = f.frame(vec![]);
    assert!(
        !texts(&output)
            .iter()
            .any(|(text, pos)| pos.x > 970. && (text == "脚本" || text == "脚本（多值）"))
    );
    assert_eq!(f.editor.document.history_len(), 0);
    Ok(())
}

#[test]
fn common_resource_prefab_overrides_preserve_lexical_parameter_references_and_source_metadata()
-> Result<()> {
    let mut f = Group::new()?;
    let (clips, scripts) = f.resource_components()?;
    let mut source = f.editor.document.source();
    let mut node = source
        .base
        .entities
        .iter()
        .find(|entity| entity.id == f.ids[1])
        .unwrap()
        .clone();
    node.parent = None;
    let node_id = node.id;
    let parameters = node.script.as_ref().unwrap().parameters.clone();
    let path = f
        .editor
        .project_root()
        .join("assets/common-resources.prefab");
    std::fs::write(
        &path,
        ContentSource {
            base: SceneSnapshot {
                entities: vec![node],
                ..Default::default()
            },
            ..Default::default()
        }
        .to_json()?,
    )?;
    prepare(f.editor.project_root(), &f.editor.products)?;
    f.assets.refresh()?;
    let bytes = std::fs::read(&path)?;
    let metadata = std::fs::read(SourceMeta::path(&path))?;
    let pin = FieldOverride {
        entity: node_id,
        field: schema::SCRIPT_PARAMETERS,
        value: FieldValue::ScriptParameters(parameters),
    };
    let instance = InstanceId::new();
    source.base.entities.retain(|entity| entity.id == f.ids[0]);
    source.instances.push(PrefabInstance {
        id: instance,
        prefab: SourceMeta::load(&path)?.id,
        parent: None,
        overrides: vec![pin.clone()],
    });
    f.editor.document = Document::open_source(source)?;
    f.ids[1] = node_id.in_instance(instance);
    f.editor.selection.replace(f.ids);
    let original = f.editor.document.shared_source();
    f.select_asset("音频片段", clips[0])?;
    f.select_asset("脚本", scripts[0])?;
    let edited = f.editor.document.shared_source();
    let overrides = &edited.instances[0].overrides;
    assert_eq!(overrides.len(), 3);
    assert!(overrides.contains(&pin));
    assert!(overrides.iter().all(|value| value.entity == node_id));
    f.editor.save(&mut f.assets)?;
    assert_eq!(Document::load(&f.editor.source)?.shared_source(), edited);
    assert_eq!(std::fs::read(&path)?, bytes);
    assert_eq!(std::fs::read(SourceMeta::path(&path))?, metadata);
    f.editor.document.undo()?;
    f.editor.document.undo()?;
    assert_eq!(f.editor.document.shared_source(), original);
    Ok(())
}

#[test]
fn common_resource_menus_and_commands_remain_read_only_in_play() -> Result<()> {
    let mut f = Group::new()?;
    let (clips, scripts) = f.resource_components()?;
    let original = f.editor.document.shared_source();
    f.editor.start(&mut f.assets)?;
    f.editor.selection.replace(f.ids);
    f.open_asset_picker("音频片段")?;
    f.open_asset_picker("脚本")?;
    assert!(
        f.editor
            .replace_audio_clips(&f.ids, clips[1], &mut f.assets)
            .is_err()
    );
    assert!(
        f.editor
            .replace_script_assets(&f.ids, scripts[1], &mut f.assets)
            .is_err()
    );
    assert_eq!(f.editor.document.shared_source(), original);
    assert_eq!(f.editor.document.history_len(), 0);
    f.editor.stop();
    Ok(())
}
