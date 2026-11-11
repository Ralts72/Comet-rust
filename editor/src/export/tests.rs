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
    // ResourceCandidate validation compiles callback bodies but never executes them during export.
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
fn a_published_package_survives_rejected_document_candidates_without_partial_store_or_ui_updates()
-> Result<()> {
    use crate::resources::{DocumentVersion, ResourceCandidate};
    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };
    for rejection in ["identity", "document", "origin"] {
        let (_temp, mut editor, mut accepted) = crate::test_support::fixture()?;
        let root = AssetId::new();
        let source = editor.project_root().join("assets/export-source.prefab");
        write_source(&source, root, vec![])?;
        editor.switch_document(source.clone(), &mut accepted)?;
        let original_index = accepted.shared_index();
        let original_snapshot = editor.snapshot(&mut accepted)?;
        let candidate = ResourceCandidate {
            document: DocumentVersion::capture(&editor.document),
            assets: AssetStore::open(&editor.products)?,
            sources: SourceCatalog::read(
                &ProjectPaths::open(editor.project_root())?,
                accepted.index(),
            )?,
            snapshot: Arc::new(original_snapshot.clone()),
        };
        let published = editor.products.parent().unwrap().join("published-game");
        fs::create_dir(&published)?;
        fs::write(published.join("completed"), b"complete package")?;
        let output = PackageOutput {
            executable: published.join("app"),
            products: published.join("products"),
            root: published.clone(),
        };
        let paths = ProjectPaths::open(editor.project_root())?;
        let mut origin = source.clone();
        match rejection {
            "identity" => fs::write(
                SourceMeta::path(&source),
                serde_json::to_vec(&SourceMeta {
                    version: 1,
                    id: AssetId::new(),
                })?,
            )?,
            "document" => {
                editor
                    .document
                    .insert_entity(EntityData::new("changed after export"), &mut accepted)?;
            }
            "origin" => {
                origin = editor.project_root().join("assets/another.prefab");
            }
            _ => unreachable!(),
        }
        let authored = editor.document.shared_source();
        let snapshot = editor.snapshot(&mut accepted)?;
        let choices = editor
            .sources
            .of_kind(AssetKind::Prefab)
            .map(|source| source.id)
            .collect::<Vec<_>>();
        let (ready, started) = std::sync::mpsc::channel();
        let (release, resume) = std::sync::mpsc::channel();
        let task = Job::spawn("completed-export", move |sender| {
            sender.send("prepared".into())?;
            ready.send(())?;
            resume.recv()?;
            Ok((output, candidate))
        })?;
        editor.export.job = Some(ExportJob {
            task,
            run: true,
            paths,
            source: origin,
        });
        let ready = started.recv_timeout(Duration::from_secs(5));
        editor.poll_export(&mut accepted);
        let running = editor.export.is_running();
        let progress = editor.export.stage.clone();
        let released = release.send(());
        ready?;
        released?;
        assert!(running);
        assert_eq!(progress, "prepared");
        let deadline = Instant::now() + Duration::from_secs(10);
        while editor.export.is_running() {
            editor.poll_export(&mut accepted);
            ensure!(Instant::now() < deadline, "export completion timed out");
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(Arc::ptr_eq(&original_index, &accepted.shared_index()));
        assert!(Arc::ptr_eq(&authored, &editor.document.shared_source()));
        assert_eq!(editor.snapshot(&mut accepted)?, snapshot);
        assert_eq!(
            editor
                .sources
                .of_kind(AssetKind::Prefab)
                .map(|source| source.id)
                .collect::<Vec<_>>(),
            choices
        );
        assert_eq!(editor.export.last_output.as_ref().unwrap().root, published);
        assert_eq!(fs::read(published.join("completed"))?, b"complete package");
        assert_eq!(editor.export.stage, "导出完成，资源未接纳");
        assert!(
            editor
                .export
                .failure
                .as_ref()
                .is_some_and(|error| error.contains("游戏包已生成"))
        );
        assert!(editor.export.children.is_empty());
        let logs = editor.export.logs.clone();
        editor.poll_export(&mut accepted);
        assert_eq!(editor.export.logs, logs);
    }
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

#[test]
fn nested_scene_data_budget_rejects_publication_and_repaired_sources_can_retry() -> Result<()> {
    use comet_asset_data::MAX_SCENE_DATA_ASSETS;
    use comet_scene::ScriptValue;
    use serde_json::json;
    use std::sync::Arc;
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
    let tables: Vec<_> = (0..=MAX_SCENE_DATA_ASSETS)
        .map(|_| AssetId::new())
        .collect();
    let table = |next: Option<AssetId>| match next {
        Some(id) => {
            json!({"fields":{"next":{"type":"asset","asset_type":"data"}},"rows":[{"id":"link","next":id}]})
        }
        None => json!({"fields":{},"rows":[{"id":"leaf"}]}),
    };
    for (index, id) in tables.iter().enumerate() {
        let path = sources.join(format!("table-{index}.data.json"));
        fs::write(
            &path,
            serde_json::to_vec(&table(tables.get(index + 1).copied()))?,
        )?;
        fs::write(
            SourceMeta::path(&path),
            serde_json::to_vec(&SourceMeta {
                version: 1,
                id: *id,
            })?,
        )?;
    }
    let script_path = sources.join("behavior.rhai");
    fs::write(
        &script_path,
        "fn on_start(ctx) { throw \"export must not run callbacks\"; }",
    )?;
    fs::write(
        SourceMeta::path(&script_path),
        serde_json::to_vec(&SourceMeta {
            version: 1,
            id: script,
        })?,
    )?;
    let mut actor = EntityData::new("data owner in dependency scene");
    let mut binding = Script::new(script);
    binding
        .parameters
        .insert("stats".into(), ScriptValue::Data(Some(tables[0])));
    actor.script = Some(binding);
    let actor_id = actor.id;
    write_source(&sources.join("main.scene"), entry, vec![])?;
    write_source(&sources.join("additional.scene"), additional, vec![actor])?;
    let mut link = EntityData::new("link to data scene");
    link.links.target = Some(EntityRef::External {
        scene: additional,
        entity: actor_id,
    });
    write_source(&sources.join("linked.scene"), linked, vec![link])?;
    comet_asset_pipeline::prepare(&project, &products)?;
    let mut assets = AssetStore::open(&products)?;
    let mut disabled = assets.expand_scene(additional)?;
    disabled.entities[0].script.as_mut().unwrap().enabled = false;
    let resources = assets.scene_resources(&disabled)?;
    assert!(resources.data.is_empty() && resources.scripts.is_empty());
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
    let output = temp.path().join("game");
    let publish = |assets: &mut AssetStore| {
        let revision = assets.revision().to_owned();
        package_export(
            assets,
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
        )
    };
    let accepted = assets.shared_index();
    let author = fs::read(sources.join("additional.scene"))?;
    let error = format!("{:#}", publish(&mut assets).unwrap_err());
    assert!(
        error.contains(&additional.to_string()) && error.contains("runtime data asset budget"),
        "{error}"
    );
    assert!(!output.exists());
    assert!(Arc::ptr_eq(&accepted, &assets.shared_index()));
    assert_eq!(fs::read(sources.join("additional.scene"))?, author);
    // Skip one link: the same entry now has exactly the supported reachable data set.
    fs::write(
        sources.join("table-0.data.json"),
        serde_json::to_vec(&table(Some(tables[2])))?,
    )?;
    comet_asset_pipeline::prepare(&project, &products)?;
    assets.refresh()?;
    let packaged = publish(&mut assets)?;
    let mut portable = AssetStore::open(packaged.products)?;
    let snapshot = portable.expand_scene(additional)?;
    assert_eq!(
        portable.scene_resources(&snapshot)?.data.len(),
        MAX_SCENE_DATA_ASSETS
    );
    Ok(())
}
