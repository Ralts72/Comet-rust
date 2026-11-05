use super::*;
use std::sync::Arc;

fn extraction_failure(editor: &mut Editor, assets: &mut AssetStore) -> anyhow::Error {
    let snapshot = editor.snapshot(assets).unwrap();
    match comet_application::extract(&snapshot, assets, &[]) {
        Err(error) => error,
        Ok(_) => panic!("the GPU-unusable transform must fail extraction"),
    }
}

#[test]
fn author_view_failure_preserves_the_live_transaction_and_rejects_pending_picks() -> Result<()> {
    let (temp, mut editor, mut assets) = fixture()?;
    let original = editor.document.shared_source();
    let snapshot = editor.snapshot(&mut assets)?;
    let id = snapshot
        .entities
        .iter()
        .find(|entity| entity.name == "Amber cube")
        .unwrap()
        .id;
    let other = snapshot
        .entities
        .iter()
        .find(|entity| entity.id != id)
        .unwrap()
        .id;
    let bytes = std::fs::read(temp.path().join("project/assets/main.scene"))?;
    editor
        .selection
        .choose(Some(id), crate::selection::SelectMode::Replace);
    editor.begin_scale(&mut assets)?;
    editor.preview_scale(comet_core::math::Vec3::splat(1e-40), &mut assets)?;
    let preview = editor.document.shared_source();
    let revision = editor.document.revision();
    let history = editor.document.history_len();
    editor.last_snapshot = editor.shared_snapshot(&mut assets)?;
    editor.target_generation = 7;
    let token = ViewToken {
        world: editor.world(),
        view: 1,
        target_generation: 7,
    };
    editor.picking.request(
        7,
        editor.world(),
        7,
        editor.selection.generation(),
        crate::selection::SelectMode::Replace,
    );
    let error = extraction_failure(&mut editor, &mut assets);
    let fallback = editor.view_error(error)?;
    assert!(fallback.snapshot.entities.is_empty());
    assert_eq!(fallback.world, editor.document.world_id());
    assert!(fallback.pick.is_none() && fallback.selected.is_empty() && !fallback.shadows);
    assert!(fallback.viewport.width() > 0. && fallback.viewport.height() > 0.);
    assert!(fallback.camera.view_projection(1.).is_finite());
    assert!(Arc::ptr_eq(&preview, &editor.document.shared_source()));
    assert_eq!(editor.document.revision(), revision);
    assert_eq!(editor.document.history_len(), history);
    assert!(
        editor.document.is_editing() && editor.document.is_dirty() && editor.drag_origin.is_some()
    );
    assert_eq!(editor.selection.active(), Some(id));
    assert_eq!(
        std::fs::read(temp.path().join("project/assets/main.scene"))?,
        bytes
    );
    editor.feedback(&FrameFeedback {
        number: 7,
        token,
        presented: true,
        backend: "test".into(),
        worker_thread: "test".into(),
        draw_count: 1,
        gpu_ms: 0.,
        pick: Some(PickFeedback {
            token,
            entities: vec![other],
        }),
    });
    assert_eq!(
        editor.selection.active(),
        Some(id),
        "the failed view's pending pick must be retired"
    );
    editor.cancel_transform()?;
    assert_eq!(editor.document.shared_source(), original);
    assert!(!editor.document.is_dirty());
    assert!(
        !comet_application::extract(&editor.snapshot(&mut assets)?, &mut assets, &[])?.is_empty()
    );
    Ok(())
}

#[test]
fn play_view_failure_stops_only_the_runtime_and_keeps_author_history_and_file() -> Result<()> {
    let (temp, mut editor, mut assets) = fixture()?;
    let author = editor.document.shared_source();
    let history = editor.document.history_len();
    let bytes = std::fs::read(temp.path().join("project/assets/main.scene"))?;
    editor.start(&mut assets)?;
    let runtime_world = editor.world();
    let snapshot = editor.snapshot(&mut assets)?;
    let id = snapshot
        .entities
        .iter()
        .find(|entity| entity.name == "Amber cube")
        .unwrap()
        .id;
    editor
        .play
        .as_mut()
        .unwrap()
        .set_field(&[id], schema::SCALE, FieldValue::Vec3([1e-40; 3]))?;
    let error = extraction_failure(&mut editor, &mut assets);
    let fallback = editor.view_error(error)?;
    assert!(editor.play.is_none());
    assert_ne!(fallback.world, runtime_world);
    assert_eq!(fallback.world, editor.document.world_id());
    assert!(fallback.snapshot.entities.is_empty() && fallback.pick.is_none());
    assert!(Arc::ptr_eq(&author, &editor.document.shared_source()));
    assert_eq!(editor.document.history_len(), history);
    assert!(!editor.document.is_dirty());
    assert_eq!(
        std::fs::read(temp.path().join("project/assets/main.scene"))?,
        bytes
    );
    assert!(
        editor
            .error
            .as_deref()
            .is_some_and(|message| message.contains("场景暂时无法显示"))
    );
    assert!(editor.report()["error"].is_string());
    assert!(
        !comet_application::extract(&editor.snapshot(&mut assets)?, &mut assets, &[])?.is_empty()
    );
    editor.start(&mut assets)?;
    assert_ne!(editor.world(), runtime_world);
    Ok(())
}

#[test]
#[ignore = "requires a native offscreen GPU; tools/verify/verify.py --gpu runs this test"]
fn headless_view_recovery_delivers_ui_and_returns_to_authoring() -> Result<()> {
    let (temp, mut editor, mut assets) = fixture()?;
    let file = temp.path().join("project/assets/main.scene");
    let author_bytes = std::fs::read(&file)?;
    let author_world = editor.document.world_id();
    let history = editor.document.history_len();
    editor.start(&mut assets)?;
    let snapshot = editor.snapshot(&mut assets)?;
    let id = snapshot
        .entities
        .iter()
        .find(|entity| entity.name == "Amber cube")
        .unwrap()
        .id;
    editor
        .play
        .as_mut()
        .unwrap()
        .set_field(&[id], schema::SCALE, FieldValue::Vec3([1e-40; 3]))?;
    let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("target/verification");
    std::fs::create_dir_all(&output)?;
    let report = output.join("editor-view-recovery.json");
    let mut options = comet_application::HostOptions::new("content recovery test");
    options.headless = true;
    options.smoke = true;
    options.capture = Some(output.join("editor-view-recovery.png"));
    options.report = Some(report.clone());
    comet_application::run(options, assets, editor)?;
    let report: serde_json::Value = serde_json::from_slice(&std::fs::read(report)?)?;
    assert_eq!(report["submitted_frames"], 8);
    assert_eq!(report["application"]["play_active"], false);
    assert_eq!(report["application"]["dirty"], false);
    assert_eq!(report["application"]["history"], history);
    assert!(
        report["application"]["error"]
            .as_str()
            .is_some_and(|message| message.contains("场景暂时无法显示"))
    );
    let frames = report["frames"].as_array().unwrap();
    assert_eq!(frames.first().unwrap()["draw_count"], 0);
    assert!(frames.last().unwrap()["draw_count"].as_u64().unwrap() > 0);
    assert!(
        frames
            .iter()
            .all(|frame| frame["token"]["world"] == author_world.to_string())
    );
    assert_eq!(std::fs::read(file)?, author_bytes);
    Ok(())
}
