//! Record queries copy bounded values, never lend the product library or access files.
use super::{Call, Context, DataAsset, ScriptResult};
use comet_asset_data::{
    game_data::{DataField, DataValue},
    product::AssetKind,
};
use rhai::{Dynamic, Engine, FLOAT, Map};

fn charge(call: &Call) -> ScriptResult<()> {
    let remaining = call
        .data_budget
        .get()
        .checked_sub(1)
        .ok_or("script data query budget exceeded")?;
    call.data_budget.set(remaining);
    Ok(())
}
fn value(call: &Call, source: &DataValue) -> ScriptResult<Dynamic> {
    charge(call)?;
    Ok(match source {
        DataValue::Bool(v) => Dynamic::from(*v),
        DataValue::Integer(v) => Dynamic::from(*v),
        DataValue::Number(v) => Dynamic::from(*v as FLOAT),
        DataValue::Text(v) => Dynamic::from(v.clone()),
        DataValue::Array(v) => Dynamic::from(
            v.iter()
                .map(|v| value(call, v))
                .collect::<ScriptResult<rhai::Array>>()?,
        ),
        DataValue::Object(v) => Dynamic::from(
            v.iter()
                .map(|(k, v)| Ok((k.clone().into(), value(call, v)?)))
                .collect::<ScriptResult<Map>>()?,
        ),
    })
}
fn table<'a>(
    call: &'a Call,
    asset: &DataAsset,
) -> ScriptResult<&'a comet_asset_data::game_data::GameData> {
    if asset.world != call.world {
        return Err("data asset belongs to a different runtime world".into());
    }
    call.data
        .get(&asset.asset)
        .map(|v| v.as_ref())
        .ok_or_else(|| "data asset is not loaded in this world".into())
}
pub(super) fn register(engine: &mut Engine) {
    engine.register_type_with_name::<DataAsset>("DataAsset");
    engine.register_fn(
        "data_get",
        |context: &mut Context, asset: DataAsset, id: &str| -> ScriptResult<Dynamic> {
            let call = context.0.borrow();
            charge(&call)?;
            let table = table(&call, &asset)?;
            let Some(row) = table.rows.get(id) else {
                return Ok(Dynamic::UNIT);
            };
            let mut result = Map::new();
            for (key, source) in row {
                let item = if matches!(
                    table.fields.get(key),
                    Some(DataField::Asset {
                        asset_type: AssetKind::Data
                    })
                ) {
                    charge(&call)?;
                    let DataValue::Text(id) = source else {
                        return Err("invalid data reference".into());
                    };
                    Dynamic::from(DataAsset {
                        asset: id.parse().map_err(|_| "invalid data identity")?,
                        world: call.world,
                    })
                } else {
                    value(&call, source)?
                };
                result.insert(key.clone().into(), item);
            }
            Ok(Dynamic::from(result))
        },
    );
    engine.register_fn(
        "data_ids",
        |context: &mut Context, asset: DataAsset| -> ScriptResult<rhai::Array> {
            let call = context.0.borrow();
            charge(&call)?;
            table(&call, &asset)?
                .rows
                .keys()
                .map(|id| {
                    charge(&call)?;
                    Ok(Dynamic::from(id.clone()))
                })
                .collect()
        },
    );
}
