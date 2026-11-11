use super::*;
use comet_asset_pipeline::{SourceMeta, prepare};
use comet_core::InstanceId;
use comet_scene::{
    ContentSource, FieldOverride, PrefabInstance,
    schema::{self, FieldKey, FieldValue},
};

impl Group {
    fn prefab_overrides(&mut self) -> Result<std::path::PathBuf> {
        let mut node = self
            .editor
            .document
            .shared_source()
            .base
            .entities
            .iter()
            .find(|entity| entity.id == self.ids[1])
            .unwrap()
            .clone();
        node.parent = None;
        let node_id = node.id;
        let other = EntityData::new("未选中的 Prefab 实体");
        let other_id = other.id;
        let path = self
            .editor
            .project_root()
            .join("assets/group-overrides.prefab");
        std::fs::write(
            &path,
            ContentSource {
                base: SceneSnapshot {
                    entities: vec![node, other],
                    ..Default::default()
                },
                ..Default::default()
            }
            .to_json()?,
        )?;
        prepare(self.editor.project_root(), &self.editor.products)?;
        self.assets.refresh()?;
        let instances = [InstanceId::new(), InstanceId::new()];
        self.editor.document = Document::open_source(ContentSource {
            instances: instances
                .iter()
                .enumerate()
                .map(|(index, &id)| PrefabInstance {
                    translation: [0.; 3],
                    id,
                    prefab: SourceMeta::load(&path).unwrap().id,
                    parent: None,
                    overrides: vec![
                        FieldOverride {
                            entity: node_id,
                            field: schema::TRANSLATION,
                            value: FieldValue::Vec3([index as f32, 1., 0.]),
                        },
                        FieldOverride {
                            entity: node_id,
                            field: schema::ROUGHNESS,
                            value: FieldValue::Scalar(if index == 0 { 0.2 } else { 0.6 }),
                        },
                        FieldOverride {
                            entity: other_id,
                            field: schema::SCALE,
                            value: FieldValue::Vec3([2.; 3]),
                        },
                    ],
                })
                .collect(),
            ..Default::default()
        })?;
        self.ids = instances.map(|instance| node_id.in_instance(instance));
        self.editor.selection.replace(self.ids);
        Ok(path)
    }
    fn click_override_button(&mut self, field: Option<FieldKey>) -> Result<()> {
        self.frame(vec![]);
        let output = self.frame(vec![]);
        let header = texts(&output)
            .into_iter()
            .find(|(text, pos)| pos.x > 970. && text.starts_with("实例覆盖 ("))
            .context("group override section")?
            .1;
        // Open it only when the controls are currently collapsed.
        if !texts(&output)
            .iter()
            .any(|(text, _)| text == "恢复全部继承")
        {
            self.frame(vec![Event::PointerMoved(header), button(header, true)]);
            self.frame(vec![button(header, false)]);
            for _ in 0..12 {
                self.frame(vec![]);
            }
        }
        let output = self.frame(vec![]);
        let values = texts(&output);
        let position = if field.is_some() {
            let row = values
                .iter()
                .find(|(text, pos)| pos.x > 970. && text.starts_with("网格渲染 · 粗糙度"))
                .context("group override field")?
                .1;
            values
                .iter()
                .find(|(text, pos)| {
                    text == "恢复继承" && (pos.y - row.y).abs() < 3. && pos.x > row.x
                })
                .context("group field restore button")?
                .1
        } else {
            values
                .iter()
                .find(|(text, _)| text == "恢复全部继承")
                .context("group restore all button")?
                .1
        };
        self.frame(vec![Event::PointerMoved(position), button(position, true)]);
        self.frame(vec![button(position, false)]);
        Ok(())
    }
}

#[test]
fn grouped_override_widgets_restore_fields_and_all_selected_nodes_with_one_undo_each() -> Result<()>
{
    let mut f = Group::new()?;
    let path = f.prefab_overrides()?;
    let bytes = std::fs::read(&path)?;
    let metadata = std::fs::read(SourceMeta::path(&path))?;
    let original = f.editor.document.shared_source();
    f.click_override_button(Some(schema::ROUGHNESS))?;
    assert_eq!(f.editor.document.history_len(), 1);
    assert!(
        f.editor
            .document
            .shared_source()
            .instances
            .iter()
            .all(|instance| instance.overrides.len() == 2
                && !instance
                    .overrides
                    .iter()
                    .any(|value| value.field == schema::ROUGHNESS))
    );
    f.click_override_button(None)?;
    assert_eq!(f.editor.document.history_len(), 2);
    let edited = f.editor.document.shared_source();
    assert!(
        edited
            .instances
            .iter()
            .all(|instance| instance.overrides.len() == 1
                && instance.overrides[0].field == schema::SCALE)
    );
    f.editor.save(&mut f.assets)?;
    assert_eq!(Document::load(&f.editor.source)?.shared_source(), edited);
    assert_eq!(std::fs::read(&path)?, bytes);
    assert_eq!(std::fs::read(SourceMeta::path(&path))?, metadata);
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
fn grouped_override_commands_reject_mixed_or_stale_targets_and_play_hides_authoring_controls()
-> Result<()> {
    let mut f = Group::new()?;
    f.prefab_overrides()?;
    let mut source = f.editor.document.source();
    let author = EntityData::new("作者实体");
    let author_id = author.id;
    source.base.entities.push(author);
    f.editor.document = Document::open_source(source)?;
    let original = f.editor.document.shared_source();
    assert!(
        f.editor
            .revert_prefab_fields(&[f.ids[0], author_id], None, &mut f.assets)
            .is_err()
    );
    assert!(
        f.editor
            .revert_prefab_fields(&[f.ids[0], EntityId::new()], None, &mut f.assets)
            .is_err()
    );
    assert_eq!(f.editor.document.shared_source(), original);
    assert_eq!(f.editor.document.history_len(), 0);
    f.editor.start(&mut f.assets)?;
    f.editor.selection.replace(f.ids);
    let output = f.frame(vec![]);
    assert!(
        !texts(&output)
            .iter()
            .any(|(text, _)| text.starts_with("实例覆盖"))
    );
    assert!(
        f.editor
            .revert_prefab_fields(&f.ids, None, &mut f.assets)
            .is_err()
    );
    assert_eq!(f.editor.document.shared_source(), original);
    f.editor.stop();
    Ok(())
}
