//! Stable field IDs for joint configuration and prefab overrides.
use super::{FieldDescriptor, FieldKey, FieldValue, ReferencePolicy};
use anyhow::{Result, bail, ensure};

pub const POINT_ENABLED: FieldKey = FieldKey {
    component: 7,
    field: 1,
};
pub const POINT_CONNECTION: FieldKey = FieldKey {
    component: 7,
    field: 2,
};
pub const POINT_ANCHOR: FieldKey = FieldKey {
    component: 7,
    field: 3,
};
pub const POINT_CONNECTED_ANCHOR: FieldKey = FieldKey {
    component: 7,
    field: 4,
};
pub const POINT_CONTACTS_ENABLED: FieldKey = FieldKey {
    component: 7,
    field: 5,
};
pub const HINGE_ENABLED: FieldKey = FieldKey {
    component: 8,
    field: 1,
};
pub const HINGE_CONNECTION: FieldKey = FieldKey {
    component: 8,
    field: 2,
};
pub const HINGE_ANCHOR: FieldKey = FieldKey {
    component: 8,
    field: 3,
};
pub const HINGE_CONNECTED_ANCHOR: FieldKey = FieldKey {
    component: 8,
    field: 4,
};
pub const HINGE_CONTACTS_ENABLED: FieldKey = FieldKey {
    component: 8,
    field: 5,
};
pub const HINGE_ROTATION: FieldKey = FieldKey {
    component: 8,
    field: 8,
};
pub const HINGE_CONNECTED_ROTATION: FieldKey = FieldKey {
    component: 8,
    field: 9,
};
pub const HINGE_LIMITS: FieldKey = FieldKey {
    component: 8,
    field: 10,
};
pub const HINGE_MOTOR: FieldKey = FieldKey {
    component: 8,
    field: 11,
};
pub const SLIDER_ENABLED: FieldKey = FieldKey {
    component: 9,
    field: 1,
};
pub const SLIDER_CONNECTION: FieldKey = FieldKey {
    component: 9,
    field: 2,
};
pub const SLIDER_ANCHOR: FieldKey = FieldKey {
    component: 9,
    field: 3,
};
pub const SLIDER_CONNECTED_ANCHOR: FieldKey = FieldKey {
    component: 9,
    field: 4,
};
pub const SLIDER_CONTACTS_ENABLED: FieldKey = FieldKey {
    component: 9,
    field: 5,
};
pub const SLIDER_ROTATION: FieldKey = FieldKey {
    component: 9,
    field: 8,
};
pub const SLIDER_CONNECTED_ROTATION: FieldKey = FieldKey {
    component: 9,
    field: 9,
};
pub const SLIDER_LIMITS: FieldKey = FieldKey {
    component: 9,
    field: 10,
};
pub const SLIDER_MOTOR: FieldKey = FieldKey {
    component: 9,
    field: 11,
};

fn boolean(value: &FieldValue) -> Result<()> {
    ensure!(matches!(value, FieldValue::Bool(_)), "expected bool");
    Ok(())
}
fn connection(value: &FieldValue) -> Result<()> {
    ensure!(
        matches!(value, FieldValue::JointConnection(_)),
        "expected joint connection"
    );
    Ok(())
}
fn limits(value: &FieldValue, angular: bool) -> Result<()> {
    if let FieldValue::JointLimits(value) = value {
        if let Some(value) = value {
            crate::joints::validate_limits(value, angular)?;
        }
        Ok(())
    } else {
        bail!("expected joint limits")
    }
}
fn motor(value: &FieldValue) -> Result<()> {
    if let FieldValue::JointMotor(value) = value {
        if let Some(value) = value {
            crate::joints::validate_motor(value)?;
        }
        Ok(())
    } else {
        bail!("expected joint motor")
    }
}
macro_rules! read_value {
    ($joint:ident, [$($path:ident),+], copy) => { $joint.$($path).+ };
    ($joint:ident, [$($path:ident),+], clone) => { $joint.$($path).+.clone() };
}
macro_rules! joint_field {
    ($key:ident, $component:ident, $($path:ident).+, $variant:ident, $validate:expr, $references:ident, $read:ident) => {
        FieldDescriptor {
            key: $key,
            path: concat!(stringify!($component), ".", stringify!($($path).+)),
            references: ReferencePolicy::$references,
            read: |entity| entity.$component.as_ref().map(|joint| FieldValue::$variant(read_value!(joint, [$($path),+], $read))),
            write: |entity, value| {
                if let (Some(joint), FieldValue::$variant(value)) = (&mut entity.$component, value) {
                    joint.$($path).+ = value;
                    Ok(())
                } else {
                    bail!("missing joint component or wrong field value")
                }
            },
            validate: $validate,
        }
    };
}
pub(super) static FIELDS: &[FieldDescriptor] = &[
    joint_field!(
        POINT_ENABLED,
        point_joint,
        settings.enabled,
        Bool,
        boolean,
        None,
        copy
    ),
    joint_field!(
        POINT_CONNECTION,
        point_joint,
        settings.connection,
        JointConnection,
        connection,
        Entity,
        clone
    ),
    joint_field!(
        POINT_ANCHOR,
        point_joint,
        anchor,
        Vec3,
        super::vector,
        None,
        copy
    ),
    joint_field!(
        POINT_CONNECTED_ANCHOR,
        point_joint,
        connected_anchor,
        Vec3,
        super::vector,
        None,
        copy
    ),
    joint_field!(
        POINT_CONTACTS_ENABLED,
        point_joint,
        settings.contacts_enabled,
        Bool,
        boolean,
        None,
        copy
    ),
    joint_field!(
        HINGE_ENABLED,
        hinge_joint,
        settings.enabled,
        Bool,
        boolean,
        None,
        copy
    ),
    joint_field!(
        HINGE_CONNECTION,
        hinge_joint,
        settings.connection,
        JointConnection,
        connection,
        Entity,
        clone
    ),
    joint_field!(
        HINGE_ANCHOR,
        hinge_joint,
        frame.anchor,
        Vec3,
        super::vector,
        None,
        copy
    ),
    joint_field!(
        HINGE_CONNECTED_ANCHOR,
        hinge_joint,
        connected_frame.anchor,
        Vec3,
        super::vector,
        None,
        copy
    ),
    joint_field!(
        HINGE_CONTACTS_ENABLED,
        hinge_joint,
        settings.contacts_enabled,
        Bool,
        boolean,
        None,
        copy
    ),
    joint_field!(
        HINGE_ROTATION,
        hinge_joint,
        frame.rotation,
        Quat,
        super::quaternion,
        None,
        copy
    ),
    joint_field!(
        HINGE_CONNECTED_ROTATION,
        hinge_joint,
        connected_frame.rotation,
        Quat,
        super::quaternion,
        None,
        copy
    ),
    joint_field!(
        HINGE_LIMITS,
        hinge_joint,
        limits,
        JointLimits,
        |v| limits(v, true),
        None,
        clone
    ),
    joint_field!(
        HINGE_MOTOR,
        hinge_joint,
        motor,
        JointMotor,
        motor,
        None,
        clone
    ),
    joint_field!(
        SLIDER_ENABLED,
        slider_joint,
        settings.enabled,
        Bool,
        boolean,
        None,
        copy
    ),
    joint_field!(
        SLIDER_CONNECTION,
        slider_joint,
        settings.connection,
        JointConnection,
        connection,
        Entity,
        clone
    ),
    joint_field!(
        SLIDER_ANCHOR,
        slider_joint,
        frame.anchor,
        Vec3,
        super::vector,
        None,
        copy
    ),
    joint_field!(
        SLIDER_CONNECTED_ANCHOR,
        slider_joint,
        connected_frame.anchor,
        Vec3,
        super::vector,
        None,
        copy
    ),
    joint_field!(
        SLIDER_CONTACTS_ENABLED,
        slider_joint,
        settings.contacts_enabled,
        Bool,
        boolean,
        None,
        copy
    ),
    joint_field!(
        SLIDER_ROTATION,
        slider_joint,
        frame.rotation,
        Quat,
        super::quaternion,
        None,
        copy
    ),
    joint_field!(
        SLIDER_CONNECTED_ROTATION,
        slider_joint,
        connected_frame.rotation,
        Quat,
        super::quaternion,
        None,
        copy
    ),
    joint_field!(
        SLIDER_LIMITS,
        slider_joint,
        limits,
        JointLimits,
        |v| limits(v, false),
        None,
        clone
    ),
    joint_field!(
        SLIDER_MOTOR,
        slider_joint,
        motor,
        JointMotor,
        motor,
        None,
        clone
    ),
];
