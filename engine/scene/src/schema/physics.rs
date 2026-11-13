use super::{FieldDescriptor, FieldKey, FieldValue, ReferencePolicy};
use anyhow::{Result, bail, ensure};

// Component 4: body configuration; component 5: character configuration.
pub const MASS: FieldKey = FieldKey {
    component: 4,
    field: 1,
};
pub const FRICTION: FieldKey = FieldKey {
    component: 4,
    field: 2,
};
pub const MOVE_SPEED: FieldKey = FieldKey {
    component: 5,
    field: 1,
};
pub const JUMP_SPEED: FieldKey = FieldKey {
    component: 5,
    field: 2,
};
pub const CROUCH_SPEED: FieldKey = FieldKey {
    component: 5,
    field: 3,
};
pub const CROUCH_HEIGHT_RATIO: FieldKey = FieldKey {
    component: 5,
    field: 4,
};
pub const BODY_MOTION: FieldKey = FieldKey {
    component: 4,
    field: 3,
};
pub const COLLIDER_SHAPE: FieldKey = FieldKey {
    component: 4,
    field: 4,
};
pub const BODY_SENSOR: FieldKey = FieldKey {
    component: 4,
    field: 5,
};
pub const RESTITUTION: FieldKey = FieldKey {
    component: 4,
    field: 6,
};
pub const COLLISION_LAYER: FieldKey = FieldKey {
    component: 4,
    field: 7,
};
pub const COLLISION_MASK: FieldKey = FieldKey {
    component: 4,
    field: 8,
};
pub const PLAYER_CONTROLLED: FieldKey = FieldKey {
    component: 5,
    field: 5,
};
pub const RUN_MULTIPLIER: FieldKey = FieldKey {
    component: 5,
    field: 6,
};
pub const GRAVITY: FieldKey = FieldKey {
    component: 5,
    field: 7,
};
pub const MAX_SLOPE: FieldKey = FieldKey {
    component: 5,
    field: 8,
};
pub const STEP_HEIGHT: FieldKey = FieldKey {
    component: 5,
    field: 9,
};
pub const SNAP_DISTANCE: FieldKey = FieldKey {
    component: 5,
    field: 10,
};

fn positive(value: &FieldValue) -> Result<()> {
    ensure!(
        matches!(value, FieldValue::Scalar(v) if v.is_finite() && *v > 0.),
        "expected positive finite scalar"
    );
    Ok(())
}
fn boolean(value: &FieldValue) -> Result<()> {
    ensure!(matches!(value, FieldValue::Bool(_)), "expected bool");
    Ok(())
}
fn unsigned(value: &FieldValue) -> Result<()> {
    ensure!(
        matches!(value, FieldValue::Unsigned(_)),
        "expected u32 bit mask"
    );
    Ok(())
}
fn motion(value: &FieldValue) -> Result<()> {
    ensure!(
        matches!(value, FieldValue::BodyMotion(_)),
        "expected body motion"
    );
    Ok(())
}
fn shape(value: &FieldValue) -> Result<()> {
    let FieldValue::ColliderShape(shape) = value else {
        bail!("expected collider shape");
    };
    shape.validate()
}
fn slope(value: &FieldValue) -> Result<()> {
    ensure!(
        matches!(value, FieldValue::Scalar(v) if v.is_finite() && (0. ..90.).contains(v)),
        "expected slope in [0, 90) degrees"
    );
    Ok(())
}
fn crouch_ratio(value: &FieldValue) -> Result<()> {
    ensure!(
        matches!(value, FieldValue::Scalar(v) if v.is_finite() && (0.1..=1.).contains(v)),
        "expected crouch height ratio in [0.1, 1]"
    );
    Ok(())
}
macro_rules! field {
    ($key:ident, $component:ident, $member:ident, $variant:ident, $validate:path, $path:literal, $copy:ident) => {
        FieldDescriptor {
            key: $key,
            path: $path,
            references: ReferencePolicy::None,
            read: |entity| {
                entity
                    .$component
                    .as_ref()
                    .map(|c| FieldValue::$variant(read!(c.$member, $copy)))
            },
            write: |entity, value| {
                if let (Some(c), FieldValue::$variant(value)) = (&mut entity.$component, value) {
                    c.$member = value;
                    Ok(())
                } else {
                    bail!("missing physics component or wrong field value");
                }
            },
            validate: $validate,
        }
    };
}
macro_rules! read {
    ($value:expr, copy) => {
        $value
    };
    ($value:expr, clone) => {
        $value.clone()
    };
}
pub(super) static FIELDS: &[FieldDescriptor] = &[
    field!(MASS, physics, mass, Scalar, positive, "physics.mass", copy),
    field!(
        FRICTION,
        physics,
        friction,
        Scalar,
        super::nonnegative_scalar,
        "physics.friction",
        copy
    ),
    field!(
        BODY_MOTION,
        physics,
        motion,
        BodyMotion,
        motion,
        "physics.motion",
        copy
    ),
    field!(
        COLLIDER_SHAPE,
        physics,
        shape,
        ColliderShape,
        shape,
        "physics.shape",
        clone
    ),
    field!(
        BODY_SENSOR,
        physics,
        sensor,
        Bool,
        boolean,
        "physics.sensor",
        copy
    ),
    field!(
        RESTITUTION,
        physics,
        restitution,
        Scalar,
        super::scalar,
        "physics.restitution",
        copy
    ),
    field!(
        COLLISION_LAYER,
        physics,
        layer,
        Unsigned,
        unsigned,
        "physics.layer",
        copy
    ),
    field!(
        COLLISION_MASK,
        physics,
        mask,
        Unsigned,
        unsigned,
        "physics.mask",
        copy
    ),
    field!(
        MOVE_SPEED,
        character,
        move_speed,
        Scalar,
        positive,
        "character.move_speed",
        copy
    ),
    field!(
        JUMP_SPEED,
        character,
        jump_speed,
        Scalar,
        positive,
        "character.jump_speed",
        copy
    ),
    field!(
        CROUCH_SPEED,
        character,
        crouch_speed,
        Scalar,
        positive,
        "character.crouch_speed",
        copy
    ),
    field!(
        CROUCH_HEIGHT_RATIO,
        character,
        crouch_height_ratio,
        Scalar,
        crouch_ratio,
        "character.crouch_height_ratio",
        copy
    ),
    field!(
        PLAYER_CONTROLLED,
        character,
        player_controlled,
        Bool,
        boolean,
        "character.player_controlled",
        copy
    ),
    field!(
        RUN_MULTIPLIER,
        character,
        run_multiplier,
        Scalar,
        positive,
        "character.run_multiplier",
        copy
    ),
    field!(
        GRAVITY,
        character,
        gravity,
        Scalar,
        positive,
        "character.gravity",
        copy
    ),
    field!(
        MAX_SLOPE,
        character,
        max_slope_degrees,
        Scalar,
        slope,
        "character.max_slope_degrees",
        copy
    ),
    field!(
        STEP_HEIGHT,
        character,
        step_height,
        Scalar,
        super::nonnegative_scalar,
        "character.step_height",
        copy
    ),
    field!(
        SNAP_DISTANCE,
        character,
        snap_distance,
        Scalar,
        super::nonnegative_scalar,
        "character.snap_distance",
        copy
    ),
];
