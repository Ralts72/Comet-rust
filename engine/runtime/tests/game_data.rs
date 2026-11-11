use anyhow::Result;
use comet_asset_data::game_data::{DataLibrary, GameData};
use comet_core::AssetId;
use comet_runtime::{FIXED_DELTA, PlaySession, RuntimeResources};
use comet_scene::{EntityData, SceneSnapshot, Script, ScriptValue};
use serde_json::json;
use std::sync::Arc;
fn start(source: &str, data: DataLibrary, id: AssetId) -> Result<PlaySession> {
    let asset = AssetId::new();
    let mut entity = EntityData::new("data consumer");
    let mut script = Script::new(asset);
    script
        .parameters
        .insert("stats".into(), ScriptValue::Data(Some(id)));
    entity.script = Some(script);
    PlaySession::start_with_resources(
        SceneSnapshot {
            entities: vec![entity],
            ..Default::default()
        },
        RuntimeResources {
            data,
            scripts: [(asset, source.into())].into(),
            ..Default::default()
        },
    )
}
fn table() -> Result<GameData> {
    Ok(serde_json::from_value(
        json!({"version":1,"fields":{"hp":{"type":"integer"},"rules":{"type":"json"}},"rows":{
            "z":{"id":"z","hp":10,"rules":{"values":[1,2]}},"a":{"id":"a","hp":20,"rules":{"values":[3,4]}}
        }}),
    )?)
}
#[test]
fn copied_records_nested_data_and_sorted_ids_remain_independent_across_pause_and_restart()
-> Result<()> {
    let (id, child) = (AssetId::new(), AssetId::new());
    let library: DataLibrary=[(child,Arc::new(table()?)),(id,Arc::new(serde_json::from_value(json!({"version":1,"fields":{"next":{"type":"asset","asset_type":"data"}},"rows":{"one":{"id":"one","next":child}}}))?))].into();
    let mut play = start(
        r#"
        fn on_start(ctx) {
            this.data = ctx.data_get(ctx.param("stats"), "one").next;
            let ids = ctx.data_ids(this.data);
            if ids != ["a", "z"] { throw "IDs not sorted"; }
            let first = ctx.data_get(this.data,"a");
            first.hp = 999; first.rules.values[0] = 900;
            let second = ctx.data_get(this.data,"a");
            if second.hp != 20 || second.rules.values[0] != 3 { throw "shared data mutated"; }
            if ctx.data_get(this.data,"missing") != () { throw "missing row defaulted"; }
            ctx.set_hud("hp=" + second.hp);
            this.hp = second.hp;
        }
        fn on_fixed_update(ctx, dt) { this.hp -= 1; ctx.set_hud("hp=" + this.hp); }
        fn on_stop(ctx) { ctx.log("base=" + ctx.data_get(this.data,"a").hp); }
    "#,
        library.clone(),
        id,
    )?;
    assert_eq!(play.hud_frame().unwrap().labels[0].text.as_ref(), "hp=20");
    play.set_paused(true);
    play.advance(FIXED_DELTA)?;
    assert_eq!(play.hud_frame().unwrap().labels[0].text.as_ref(), "hp=20");
    play.single_step()?;
    assert_eq!(play.hud_frame().unwrap().labels[0].text.as_ref(), "hp=19");
    play.restart()?;
    assert_eq!(play.hud_frame().unwrap().labels[0].text.as_ref(), "hp=20");
    assert_eq!(
        library[&child].rows["a"]["hp"],
        comet_asset_data::game_data::DataValue::Integer(20)
    );
    play.stop();
    assert!(
        play.script_diagnostics().iter().all(|d| !d.failed),
        "{:?}",
        play.script_diagnostics()
    );
    assert!(
        play.script_diagnostics()
            .iter()
            .any(|d| d.message == "base=20")
    );
    Ok(())
}
#[test]
fn unavailable_data_and_query_quota_fail_without_committing_a_callback_prefix() -> Result<()> {
    let id = AssetId::new();
    assert!(start("fn on_start(ctx) {}", Default::default(), id).is_err());
    let data: GameData = serde_json::from_value(
        json!({"version":1,"fields":{"values":{"type":"json"}},"rows":{"one":{"id":"one","values":vec![1;1000]}}}),
    )?;
    let mut play = start(
        r#"
        fn on_fixed_update(ctx, dt) {
            ctx.translate_local(ctx.entity(),1,0,0);ctx.set_hud("must not publish");ctx.log("must not publish");
            for i in 0..20 { let row=ctx.data_get(ctx.param("stats"),"one"); }
        }
    "#,
        [(id, Arc::new(data))].into(),
        id,
    )?;
    let before = play.shared_snapshot();
    play.advance(FIXED_DELTA)?;
    assert!(Arc::ptr_eq(&before, &play.shared_snapshot()));
    assert!(play.hud_frame().is_none());
    assert_eq!(play.script_diagnostics().len(), 1);
    assert!(play.script_diagnostics()[0].failed);
    assert!(
        play.script_diagnostics()[0]
            .message
            .contains("data query budget"),
        "{:?}",
        play.script_diagnostics()
    );
    Ok(())
}
