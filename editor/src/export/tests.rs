use super::*;
use comet_asset_pipeline::SourceMeta;
use comet_scene::{ContentSource, EntityData, EntityRef, SceneSnapshot, Script};
use std::fs;

fn write_source(path: &Path, id: AssetId, entities: Vec<EntityData>) -> Result<()> {
    let source = ContentSource {
        base: SceneSnapshot {
            entities,
            ..Default::default()
        },
        ..Default::default()
    };
    fs::write(path, source.to_json()?)?;
    fs::write(
        SourceMeta::path(path),
        serde_json::to_vec(&SourceMeta { version: 1, id })?,
    )?;
    Ok(())
}

#[test]
fn bad_selected_or_dependency_scene_scripts_cannot_publish_and_corrected_sources_can_retry()
-> Result<()> {
    let temp = tempfile::tempdir()?;
    let project = temp.path().join("project");
    let sources = project.join("assets");
    fs::create_dir_all(&sources)?;
    let products = temp.path().join("products");
    let (entry, additional, linked, script) = (
        AssetId::new(),
        AssetId::new(),
        AssetId::new(),
        AssetId::new(),
    );
    write_source(&sources.join("main.scene"), entry, vec![])?;
    let mut scripted = EntityData::new("script in another scene");
    scripted.script = Some(Script::new(script));
    let target = scripted.id;
    write_source(
        &sources.join("additional.scene"),
        additional,
        vec![scripted],
    )?;
    let mut link = EntityData::new("explicit external scene link");
    link.links.target = Some(EntityRef::External {
        scene: additional,
        entity: target,
    });
    write_source(&sources.join("linked.scene"), linked, vec![link])?;
    let script_path = sources.join("behavior.rhai");
    fs::write(&script_path, "fn on_start(")?;
    fs::write(
        SourceMeta::path(&script_path),
        serde_json::to_vec(&SourceMeta {
            version: 1,
            id: script,
        })?,
    )?;
    comet_asset_pipeline::prepare(&project, &products)?;
    let mut assets = AssetStore::open(&products)?;
    let app = temp.path().join(if cfg!(windows) {
        "comet-app.exe"
    } else {
        "comet-app"
    });
    fs::write(&app, b"packing fixture; never executed")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&app, fs::Permissions::from_mode(0o755))?;
    }
    for (number, entry_scene, additional_scenes) in [
        (0, entry, vec![additional]),
        (1, additional, vec![]),
        (2, entry, vec![linked]),
    ] {
        let output = temp.path().join(format!("rejected-{number}"));
        let revision = assets.revision().to_owned();
        let result = package_export(
            &mut assets,
            PackageRequest {
                products: &products,
                game_id: comet_core::GameId::new(),
                index_revision: Some(&revision),
                entry_scene,
                additional_scenes: &additional_scenes,
                app: &app,
                runtime: None,
                app_digest: None,
                notices: &[],
                app_icon: None,
                game_icon: None,
                output: &output,
            },
        );
        let error = format!(
            "{:#}",
            result.expect_err("invalid script export must fail before publishing")
        );
        assert!(
            error.contains(&additional.to_string()) && error.contains("脚本校验失败"),
            "{error}"
        );
        assert!(!output.exists());
        assert_eq!(assets.revision(), revision);
    }
    // Candidate validation compiles callback bodies but never executes them during export.
    fs::write(
        &script_path,
        "fn on_start(ctx) { throw \"only runtime executes callbacks\"; }",
    )?;
    comet_asset_pipeline::prepare(&project, &products)?;
    assets.refresh()?;
    let output = temp.path().join("accepted");
    let revision = assets.revision().to_owned();
    let packaged = package_export(
        &mut assets,
        PackageRequest {
            products: &products,
            game_id: comet_core::GameId::new(),
            index_revision: Some(&revision),
            entry_scene: entry,
            additional_scenes: &[linked],
            app: &app,
            runtime: None,
            app_digest: None,
            notices: &[],
            app_icon: None,
            game_icon: None,
            output: &output,
        },
    )?;
    let mut standalone = AssetStore::open(packaged.products)?;
    assert_eq!(standalone.index().products.len(), 4);
    assert!(standalone.index().products.contains_key(&script));
    assert!(standalone.expand_scene(entry)?.entities.is_empty());
    assert_eq!(standalone.expand_scene(additional)?.entities[0].id, target);
    assert_eq!(SourceMeta::load(&script_path)?.id, script);
    assert_eq!(
        SourceMeta::load(&sources.join("additional.scene"))?.id,
        additional
    );
    Ok(())
}

#[test]
fn export_selection_tracks_scenes_independently_of_prefab_authoring() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let project = temp.path().join("project");
    let source = project.join("assets");
    fs::create_dir_all(&source)?;
    let first = AssetId::new();
    let second = AssetId::new();
    let prefab = AssetId::new();
    write_source(&source.join("a.scene"), first, vec![])?;
    write_source(&source.join("b.scene"), second, vec![])?;
    write_source(
        &source.join("native.prefab"),
        prefab,
        vec![EntityData::new("source")],
    )?;
    let products = temp.path().join("products");
    let paths = ProjectPaths::open(&project)?;
    let catalog = || -> Result<SourceCatalog> {
        comet_asset_pipeline::prepare(&project, &products)?;
        let assets = AssetStore::open(&products)?;
        SourceCatalog::read(&paths, assets.index())
    };
    let mut export = ExportController::new(None)?;
    export.retain_scenes(&catalog()?);
    assert_eq!(export.entry_scene(), Some(first));
    export.select_scene(Some(second));
    export.additional.extend([first, second, prefab]);
    export.output = "chosen destination".into();
    export.retain_scenes(&catalog()?);
    assert_eq!(export.entry_scene(), Some(second));
    assert_eq!(export.additional, BTreeSet::from([first]));
    assert_eq!(export.output, "chosen destination");

    // A removed entry falls back to a valid Scene; Prefab products never become entries.
    for name in ["b.scene", "b.scene.meta"] {
        fs::remove_file(source.join(name))?;
    }
    export.retain_scenes(&catalog()?);
    assert_eq!(export.entry_scene(), Some(first));
    assert!(export.additional.is_empty());
    for name in ["a.scene", "a.scene.meta"] {
        fs::remove_file(source.join(name))?;
    }
    export.retain_scenes(&catalog()?);
    assert_eq!(export.entry_scene(), None);
    assert!(export.additional.is_empty());

    write_source(&source.join("new.scene"), first, vec![])?;
    export.retain_scenes(&catalog()?);
    assert_eq!(export.entry_scene(), Some(first));
    export.reset_project(None)?;
    assert_eq!(export.entry_scene(), None);
    assert_ne!(export.output, "chosen destination");
    Ok(())
}

#[test]
fn a_published_package_survives_when_a_changed_prefab_identity_prevents_store_acceptance()
-> Result<()> {
    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };
    let (_temp, editor, mut accepted) = crate::test_support::fixture()?;
    let root = AssetId::new();
    let source = editor.project_root().join("assets/export-source.prefab");
    write_source(&source, root, vec![])?;
    comet_asset_pipeline::prepare(editor.project_root(), &editor.products)?;
    accepted.refresh()?;
    let original_index = accepted.shared_index();
    let candidate = AssetStore::open(&editor.products)?;
    let published = editor.products.parent().unwrap().join("published-game");
    fs::create_dir(&published)?;
    fs::write(published.join("completed"), b"complete package")?;
    let output = PackageOutput {
        executable: published.join("app"),
        products: published.join("products"),
        root: published.clone(),
    };
    let mut export = ExportController::new(editor.export.entry_scene())?;
    let paths = ProjectPaths::open(editor.project_root())?;
    fs::write(
        SourceMeta::path(&source),
        serde_json::to_vec(&SourceMeta {
            version: 1,
            id: AssetId::new(),
        })?,
    )?;
    let (ready, started) = std::sync::mpsc::channel();
    let (release, resume) = std::sync::mpsc::channel();
    let task = Job::spawn("completed-prefab-export", move |sender| {
        sender.send("游戏包已发布，等待接纳".into())?;
        ready.send(())?;
        resume.recv()?;
        Ok((output, candidate))
    })?;
    export.job = Some(ExportJob {
        task,
        run: true,
        paths,
        source,
        prefab_asset: Some(root),
    });
    let ready = started.recv_timeout(Duration::from_secs(5));
    export.poll(&mut accepted);
    let running = export.is_running();
    let progress = export.stage.clone();
    let released = release.send(());
    ready?;
    released?;
    assert!(
        running,
        "progress must retain the pending export and its acceptance identity"
    );
    assert_eq!(progress, "游戏包已发布，等待接纳");
    let deadline = Instant::now() + Duration::from_secs(10);
    while export.is_running() {
        export.poll(&mut accepted);
        ensure!(Instant::now() < deadline, "export completion timed out");
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(Arc::ptr_eq(&original_index, &accepted.shared_index()));
    assert_eq!(export.last_output.as_ref().unwrap().root, published);
    assert_eq!(fs::read(published.join("completed"))?, b"complete package");
    assert_eq!(export.stage, "导出完成，资源未接纳");
    assert!(
        export
            .failure
            .as_ref()
            .is_some_and(|error| error.contains("游戏包已生成") && error.contains("作者身份变化"))
    );
    assert!(export.children.is_empty());
    let logs = export.logs.clone();
    export.poll(&mut accepted);
    assert_eq!(export.logs, logs, "completion must not be delivered twice");
    Ok(())
}

#[test]
fn every_additional_export_scene_requires_complete_animation_bindings() -> Result<()> {
    use comet_scene::PrefabResolver;
    let (_temp, editor, mut assets) = crate::test_support::fixture()?;
    let prefab = crate::test_support::prepare_skin_asset(&editor)?;
    assets.refresh()?;
    let mut broken = assets.prefab(prefab)?;
    for entity in &mut broken.base.entities {
        entity.animator = None;
        if let Some(skin) = &mut entity.skin {
            skin.animator = None;
        }
    }
    let paths = comet_core::ProjectPaths::open(editor.project_root())?;
    let file = comet_asset_pipeline::create_scene(&paths, Path::new("unbound.scene"))?;
    fs::write(&file, broken.to_json()?)?;
    let additional = SourceMeta::load(&file)?.id;
    let entry = SourceMeta::load(&editor.source)?.id;
    comet_asset_pipeline::prepare(editor.project_root(), &editor.products)?;
    assets.refresh()?;
    validate_export_scenes(&mut assets, entry, &[])?;
    let error = validate_export_scenes(&mut assets, entry, &[additional]).unwrap_err();
    assert!(error.is::<comet_scene::UnboundRig>());
    assert!(format!("{error:#}").contains(&additional.to_string()));
    Ok(())
}
