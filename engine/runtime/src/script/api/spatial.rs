//! Queries observe the callback's immutable world, including queued-write isolation.
use super::{Context, Entity, ScriptResult, numbers, vector};
use comet_core::math::Vec3;
use rhai::{Dynamic, Engine, FLOAT, INT};

fn vector_array(values: rhai::Array) -> ScriptResult<[f32; 3]> {
    let values: [Dynamic; 3] = values
        .try_into()
        .map_err(|_| "expected three vector components")?;
    let [x, y, z] = values;
    vector(x, y, z)
}
pub(super) fn register(engine: &mut Engine) {
    engine.register_fn(
        "transform_point",
        |context: &mut Context,
         entity: Entity,
         x: Dynamic,
         y: Dynamic,
         z: Dynamic|
         -> ScriptResult<rhai::Array> {
            let point = vector(x, y, z)?;
            let call = context.0.borrow();
            let id = entity.resolve(&call)?;
            if call.world_matrices.get().is_none() {
                if call.snapshot.entities.len() > 16384 {
                    return Err("world transform query entity budget exceeded".into());
                }
                let matrices =
                    comet_scene::world_transforms(&call.snapshot).map_err(|e| e.to_string())?;
                let _ = call.world_matrices.set(matrices);
            }
            let point =
                call.world_matrices.get().unwrap()[&id].transform_point3(Vec3::from_array(point));
            if !point.is_finite() {
                return Err("world point overflows".into());
            }
            Ok(numbers(point.to_array()))
        },
    );
    engine.register_fn(
        "raycast",
        |context: &mut Context,
         origin: rhai::Array,
         displacement: rhai::Array,
         layers: INT,
         ignore: Entity|
         -> ScriptResult<Dynamic> {
            let origin = vector_array(origin)?;
            let displacement = vector_array(displacement)?;
            let layers =
                u32::try_from(layers).map_err(|_| "ray layers must be a 32-bit unsigned mask")?;
            let call = context.0.borrow();
            let ignore = ignore.resolve(&call)?;
            let remaining = call
                .query_budget
                .get()
                .checked_sub(1)
                .ok_or("physics query phase budget exceeded")?;
            call.query_budget.set(remaining);
            let Some(hit) = call
                .queries
                .raycast(origin, displacement, layers, ignore)
                .map_err(|e| e.to_string())?
            else {
                return Ok(Dynamic::UNIT);
            };
            Ok(rhai::Map::from([
                (
                    "entity".into(),
                    Dynamic::from(Entity::local(hit.entity, call.world)),
                ),
                ("fraction".into(), Dynamic::from(hit.fraction as FLOAT)),
                ("position".into(), numbers(hit.position).into()),
                ("normal".into(), numbers(hit.normal).into()),
            ])
            .into())
        },
    );
}
