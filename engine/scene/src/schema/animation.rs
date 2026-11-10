use super::{FieldDescriptor, FieldKey, FieldValue, ReferencePolicy};
use anyhow::{Result, bail, ensure};
pub const ANIMATOR_SKELETON: FieldKey = FieldKey {
    component: 13,
    field: 1,
};
pub const ANIMATOR_CLIP: FieldKey = FieldKey {
    component: 13,
    field: 2,
};
pub const ANIMATOR_BINDINGS: FieldKey = FieldKey {
    component: 13,
    field: 3,
};
pub const ANIMATOR_LOOPING: FieldKey = FieldKey {
    component: 13,
    field: 4,
};
pub const ANIMATOR_SPEED: FieldKey = FieldKey {
    component: 13,
    field: 5,
};
pub const ANIMATOR_CONTROLLER: FieldKey = FieldKey {
    component: 13,
    field: 6,
};
pub const CHARACTER_ANIMATION_CHARACTER: FieldKey = FieldKey {
    component: 15,
    field: 1,
};
pub const CHARACTER_ANIMATION_ANIMATOR: FieldKey = FieldKey {
    component: 15,
    field: 2,
};
pub const SKIN_ASSET: FieldKey = FieldKey {
    component: 14,
    field: 1,
};
pub const SKIN_ANIMATOR: FieldKey = FieldKey {
    component: 14,
    field: 2,
};

fn bindings(value: &FieldValue) -> Result<()> {
    ensure!(
        matches!(value, FieldValue::AnimatorBindings(value) if value.len() <= crate::MAX_ANIMATOR_BINDINGS),
        "expected bounded animator bindings"
    );
    Ok(())
}
macro_rules! field {
    ($key:ident,$component:ident,$member:ident,$variant:ident,$validate:path,$policy:ident,$path:literal,$read:expr) => {
        FieldDescriptor {
            key: $key,
            path: $path,
            references: ReferencePolicy::$policy,
            read: |e| e.$component.as_ref().map($read).map(FieldValue::$variant),
            write: |e, value| {
                if let (Some(c), FieldValue::$variant(value)) = (&mut e.$component, value) {
                    c.$member = value;
                    Ok(())
                } else {
                    bail!("missing animation component or wrong field value")
                }
            },
            validate: $validate,
        }
    };
}
pub(super) static FIELDS: &[FieldDescriptor] = &[
    field!(
        ANIMATOR_CONTROLLER,
        animator,
        controller,
        OptionalAsset,
        optional_asset,
        Asset,
        "animator.controller",
        |c| c.controller
    ),
    field!(
        CHARACTER_ANIMATION_CHARACTER,
        character_animation,
        character,
        Entity,
        super::entity,
        Entity,
        "character_animation.character",
        |c| c.character.clone()
    ),
    field!(
        CHARACTER_ANIMATION_ANIMATOR,
        character_animation,
        animator,
        Entity,
        super::entity,
        Entity,
        "character_animation.animator",
        |c| c.animator.clone()
    ),
    field!(
        ANIMATOR_SKELETON,
        animator,
        skeleton,
        Asset,
        super::asset,
        Asset,
        "animator.skeleton",
        |c| c.skeleton
    ),
    field!(
        ANIMATOR_CLIP,
        animator,
        clip,
        OptionalAsset,
        optional_asset,
        Asset,
        "animator.clip",
        |c| c.clip
    ),
    field!(
        ANIMATOR_BINDINGS,
        animator,
        bindings,
        AnimatorBindings,
        bindings,
        Entity,
        "animator.bindings",
        |c| c.bindings.clone()
    ),
    field!(
        ANIMATOR_LOOPING,
        animator,
        looping,
        Bool,
        boolean,
        None,
        "animator.looping",
        |c| c.looping
    ),
    field!(
        ANIMATOR_SPEED,
        animator,
        speed,
        Scalar,
        super::nonnegative_scalar,
        None,
        "animator.speed",
        |c| c.speed
    ),
    field!(
        SKIN_ASSET,
        skin,
        skin,
        Asset,
        super::asset,
        Asset,
        "skin.skin",
        |c| c.skin
    ),
    field!(
        SKIN_ANIMATOR,
        skin,
        animator,
        Entity,
        super::entity,
        Entity,
        "skin.animator",
        |c| c.animator.clone()
    ),
];
fn optional_asset(value: &FieldValue) -> Result<()> {
    ensure!(
        matches!(value, FieldValue::OptionalAsset(_)),
        "expected optional asset"
    );
    Ok(())
}
fn boolean(value: &FieldValue) -> Result<()> {
    ensure!(matches!(value, FieldValue::Bool(_)), "expected bool");
    Ok(())
}
