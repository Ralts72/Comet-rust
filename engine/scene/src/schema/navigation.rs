use super::{FieldDescriptor, FieldKey, FieldValue, ReferencePolicy};
use anyhow::{Result, bail, ensure};

pub const NAVIGATION_DATA: FieldKey = FieldKey {
    component: 10,
    field: 1,
};
pub const NAVIGATION_HALF_EXTENTS: FieldKey = FieldKey {
    component: 10,
    field: 2,
};
pub const NAVIGATION_CELL_SIZE: FieldKey = FieldKey {
    component: 10,
    field: 3,
};
pub const NAVIGATION_AGENT_RADIUS: FieldKey = FieldKey {
    component: 10,
    field: 4,
};
pub const NAVIGATION_AGENT_HEIGHT: FieldKey = FieldKey {
    component: 10,
    field: 5,
};
pub const NAVIGATION_MAX_CLIMB: FieldKey = FieldKey {
    component: 10,
    field: 6,
};
pub const NAVIGATION_MAX_SLOPE: FieldKey = FieldKey {
    component: 10,
    field: 7,
};
pub const NAVIGATION_ENABLED: FieldKey = FieldKey {
    component: 11,
    field: 1,
};
pub const NAVIGATION_TARGET: FieldKey = FieldKey {
    component: 11,
    field: 2,
};
pub const NAVIGATION_ARRIVAL_DISTANCE: FieldKey = FieldKey {
    component: 11,
    field: 3,
};

fn positive(value: &FieldValue) -> Result<()> {
    ensure!(
        matches!(value, FieldValue::Scalar(v) if v.is_finite() && *v > 0.),
        "expected positive finite scalar"
    );
    Ok(())
}
fn slope(value: &FieldValue) -> Result<()> {
    ensure!(
        matches!(value, FieldValue::Scalar(v) if v.is_finite() && (0. ..90.).contains(v)),
        "expected slope in [0, 90) degrees"
    );
    Ok(())
}
fn half_extents(value: &FieldValue) -> Result<()> {
    ensure!(
        matches!(value, FieldValue::Vec3(v) if v.iter().all(|v| v.is_finite() && *v > 0.)),
        "expected positive finite extents"
    );
    Ok(())
}
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
macro_rules! field {
    ($key:ident, $component:ident, $member:ident, $variant:ident, $validate:path, $policy:ident, $path:literal, $copy:ident) => {
        FieldDescriptor {
            key: $key,
            path: $path,
            references: ReferencePolicy::$policy,
            read: |e| {
                e.$component
                    .as_ref()
                    .map(|c| FieldValue::$variant(read!(c.$member, $copy)))
            },
            write: |e, value| {
                if let (Some(c), FieldValue::$variant(v)) = (&mut e.$component, value) {
                    c.$member = v;
                    Ok(())
                } else {
                    bail!("missing navigation component or wrong field value")
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
    field!(
        NAVIGATION_DATA,
        navigation_region,
        data,
        OptionalAsset,
        optional_asset,
        Asset,
        "navigation_region.data",
        copy
    ),
    field!(
        NAVIGATION_HALF_EXTENTS,
        navigation_region,
        half_extents,
        Vec3,
        half_extents,
        None,
        "navigation_region.half_extents",
        copy
    ),
    field!(
        NAVIGATION_CELL_SIZE,
        navigation_region,
        cell_size,
        Scalar,
        positive,
        None,
        "navigation_region.cell_size",
        copy
    ),
    field!(
        NAVIGATION_AGENT_RADIUS,
        navigation_region,
        agent_radius,
        Scalar,
        positive,
        None,
        "navigation_region.agent_radius",
        copy
    ),
    field!(
        NAVIGATION_AGENT_HEIGHT,
        navigation_region,
        agent_height,
        Scalar,
        positive,
        None,
        "navigation_region.agent_height",
        copy
    ),
    field!(
        NAVIGATION_MAX_CLIMB,
        navigation_region,
        max_climb,
        Scalar,
        super::nonnegative_scalar,
        None,
        "navigation_region.max_climb",
        copy
    ),
    field!(
        NAVIGATION_MAX_SLOPE,
        navigation_region,
        max_slope_degrees,
        Scalar,
        slope,
        None,
        "navigation_region.max_slope_degrees",
        copy
    ),
    field!(
        NAVIGATION_ENABLED,
        navigation_agent,
        enabled,
        Bool,
        boolean,
        None,
        "navigation_agent.enabled",
        copy
    ),
    field!(
        NAVIGATION_TARGET,
        navigation_agent,
        target,
        Entity,
        super::entity,
        Entity,
        "navigation_agent.target",
        clone
    ),
    field!(
        NAVIGATION_ARRIVAL_DISTANCE,
        navigation_agent,
        arrival_distance,
        Scalar,
        positive,
        None,
        "navigation_agent.arrival_distance",
        copy
    ),
];
