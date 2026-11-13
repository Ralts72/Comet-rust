use super::*;
use comet_asset_data::product::{AssetKind, ProductIndex};
use comet_scene::{EntityData, SceneSnapshot};

/// Shared eligibility for menus and commands; reference scope is checked separately.
pub(crate) fn accepts_binding(
    field: FieldKey,
    owner: &EntityData,
    target: &EntityData,
    snapshot: &SceneSnapshot,
    index: &ProductIndex,
) -> bool {
    match field {
        schema::NAVIGATION_TARGET => owner.id != target.id,
        schema::POINT_CONNECTION | schema::HINGE_CONNECTION | schema::SLIDER_CONNECTION => {
            owner.id != target.id
                && target.parent.is_none()
                && target.character.is_none()
                && target.physics.as_ref().is_some_and(|body| {
                    body.motion == comet_scene::BodyMotion::Dynamic
                        || owner
                            .physics
                            .as_ref()
                            .is_some_and(|owner| owner.motion == comet_scene::BodyMotion::Dynamic)
                })
        }
        schema::SKIN_ANIMATOR => owner.skin.as_ref().is_some_and(|skin| {
            target.animator.as_ref().is_some_and(|animator| {
                index.products.get(&skin.skin).is_some_and(|entry| {
                    entry.kind == AssetKind::Skin && entry.dependencies.contains(&animator.skeleton)
                })
            })
        }),
        schema::CHARACTER_ANIMATION_CHARACTER => {
            owner.parent == Some(target.id) && target.character.is_some()
        }
        schema::CHARACTER_ANIMATION_ANIMATOR => {
            if target.animator.is_none() {
                return false;
            }
            let mut ancestor = Some(target.id);
            while let Some(id) = ancestor {
                if id == owner.id {
                    return true;
                }
                ancestor = snapshot
                    .entities
                    .binary_search_by_key(&id, |entity| entity.id)
                    .ok()
                    .and_then(|index| snapshot.entities[index].parent);
            }
            false
        }
        _ => false,
    }
}

impl Editor {
    pub(crate) fn replace_entity_bindings(
        &mut self,
        ids: &[EntityId],
        field: FieldKey,
        target: Option<EntityId>,
        assets: &mut AssetStore,
    ) -> Result<()> {
        ensure!(
            matches!(
                field,
                schema::NAVIGATION_TARGET
                    | schema::SKIN_ANIMATOR
                    | schema::CHARACTER_ANIMATION_CHARACTER
                    | schema::CHARACTER_ANIMATION_ANIMATOR
            ),
            "此字段不支持实体绑定"
        );
        self.preview_common_fields(
            ids,
            &[(field, FieldValue::Entity(target.map(EntityRef::local)))],
            assets,
            |owner, snapshot, assets| {
                if let Some(id) = target {
                    let target = snapshot
                        .entities
                        .binary_search_by_key(&id, |e| e.id)
                        .ok()
                        .map(|index| &snapshot.entities[index])
                        .context("绑定目标已不存在")?;
                    ensure!(
                        accepts_binding(field, owner, target, snapshot, assets.index()),
                        "目标实体不符合此绑定的组件、层级或骨架要求"
                    );
                }
                if field == schema::SKIN_ANIMATOR {
                    crate::animation::validate_configuration(owner, snapshot, assets)?;
                }
                Ok(())
            },
        )
    }
}
