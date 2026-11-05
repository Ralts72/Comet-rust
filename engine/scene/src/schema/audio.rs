use super::{FieldDescriptor, FieldKey, FieldValue, ReferencePolicy};
use anyhow::{Result, bail, ensure};
pub const AUDIO_CLIP: FieldKey = FieldKey {
    component: 12,
    field: 1,
};
pub const AUDIO_GAIN: FieldKey = FieldKey {
    component: 12,
    field: 2,
};
pub const AUDIO_LOOPING: FieldKey = FieldKey {
    component: 12,
    field: 3,
};
pub const AUDIO_AUTOPLAY: FieldKey = FieldKey {
    component: 12,
    field: 4,
};
pub const AUDIO_SPATIAL: FieldKey = FieldKey {
    component: 12,
    field: 5,
};
pub const AUDIO_MIN_DISTANCE: FieldKey = FieldKey {
    component: 12,
    field: 6,
};
pub const AUDIO_MAX_DISTANCE: FieldKey = FieldKey {
    component: 12,
    field: 7,
};
fn distance(value: &FieldValue) -> Result<()> {
    ensure!(
        matches!(value, FieldValue::Scalar(value) if value.is_finite() && *value >= 0.),
        "audio distance must be finite and nonnegative"
    );
    Ok(())
}
fn gain(value: &FieldValue) -> Result<()> {
    ensure!(
        matches!(value, FieldValue::Scalar(v) if v.is_finite() && (0. ..=1.).contains(v)),
        "audio gain must be finite and in [0, 1]"
    );
    Ok(())
}
fn asset(value: &FieldValue) -> Result<()> {
    ensure!(
        matches!(value, FieldValue::Asset(_)),
        "audio clip requires an asset"
    );
    Ok(())
}
fn boolean(value: &FieldValue) -> Result<()> {
    ensure!(matches!(value, FieldValue::Bool(_)), "expected bool");
    Ok(())
}
macro_rules! field {
    ($key:ident,$member:ident,$variant:ident,$validate:path,$policy:ident,$path:literal) => {
        FieldDescriptor {
            key: $key,
            path: $path,
            references: ReferencePolicy::$policy,
            read: |entity| {
                entity
                    .audio_source
                    .as_ref()
                    .map(|source| FieldValue::$variant(source.$member))
            },
            write: |entity, value| {
                if let (Some(source), FieldValue::$variant(value)) =
                    (&mut entity.audio_source, value)
                {
                    source.$member = value;
                    Ok(())
                } else {
                    bail!("missing audio source or wrong field value")
                }
            },
            validate: $validate,
        }
    };
}
pub(super) static FIELDS: &[FieldDescriptor] = &[
    field!(AUDIO_CLIP, clip, Asset, asset, Asset, "audio_source.clip"),
    field!(AUDIO_GAIN, gain, Scalar, gain, None, "audio_source.gain"),
    field!(
        AUDIO_LOOPING,
        looping,
        Bool,
        boolean,
        None,
        "audio_source.looping"
    ),
    field!(
        AUDIO_AUTOPLAY,
        autoplay,
        Bool,
        boolean,
        None,
        "audio_source.autoplay"
    ),
    field!(
        AUDIO_SPATIAL,
        spatial,
        Bool,
        boolean,
        None,
        "audio_source.spatial"
    ),
    field!(
        AUDIO_MIN_DISTANCE,
        min_distance,
        Scalar,
        distance,
        None,
        "audio_source.min_distance"
    ),
    field!(
        AUDIO_MAX_DISTANCE,
        max_distance,
        Scalar,
        distance,
        None,
        "audio_source.max_distance"
    ),
];
