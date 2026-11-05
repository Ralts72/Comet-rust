use super::*;
use comet_asset_pipeline::{DEFAULT_SCENE, SourceMeta};
use std::{
    fs,
    time::{Duration, Instant},
};

fn wait(editor: &mut Editor, assets: &mut AssetStore) -> anyhow::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(10);
    while editor.project_load.is_running() {
        editor.poll_project_transition(assets);
        anyhow::ensure!(Instant::now() < deadline, "project load timed out");
        std::thread::sleep(Duration::from_millis(2));
    }
    Ok(())
}
fn open_action(root: &std::path::Path) -> DocumentAction {
    DocumentAction::OpenProject {
        root: root.into(),
        scene: DEFAULT_SCENE.into(),
    }
}

#[test]
fn new_project_confirmation_saves_authoring_and_switches_all_project_state() -> anyhow::Result<()> {
    let (temp, mut editor, mut assets) = crate::test_support::fixture()?;
    let original_root = editor.project_root().to_owned();
    let original_products = editor.products.clone();
    let original_source = editor.source.clone();
    preview(&mut editor, &mut assets)?;
    editor.start(&mut assets)?;
    let original_play = editor.world();
    let authored = editor.document.shared_source();
    editor.translation_snap.enabled = true;
    editor.translation_snap.set_step(0.25)?;
    editor.shadows = false;
    editor.export.template_error = Some("template kept across projects".into());
    let new_root = temp.path().join("new-game");
    editor.request_document_action(DocumentAction::NewProject(new_root.clone()));
    editor.resolve_transition(Choice::Cancel, &mut assets);
    assert!(!new_root.exists());
    assert_eq!(editor.world(), original_play);
    assert_eq!(editor.document.shared_source(), authored);

    editor.request_document_action(DocumentAction::NewProject(new_root.clone()));
    editor.resolve_transition(Choice::Save, &mut assets);
    assert!(editor.is_busy());
    assert_eq!(
        editor.world(),
        original_play,
        "old Play remains alive until acceptance"
    );
    assert_eq!(
        comet_editor_core::Document::load(&original_source)?.shared_source(),
        authored
    );
    assert!(!editor.document.is_dirty());
    assert!(editor.start(&mut assets).is_err());
    assert!(editor.refresh_resources().is_err());
    wait(&mut editor, &mut assets)?;
    assert!(!editor.documents.pending, "{:?}", editor.documents.error);
    assert_eq!(editor.project_root(), new_root.canonicalize()?);
    assert!(editor.play.is_none());
    assert!(editor.selection.is_empty());
    assert_eq!(editor.document.history_len(), 0);
    assert!(editor.document.source().base.entities.is_empty());
    assert!(editor.document.source().instances.is_empty());
    assert!(
        editor
            .products
            .starts_with(original_products.parent().unwrap().join("projects"))
    );
    assert!(!new_root.join("products").exists());
    let entry = SourceMeta::load(&new_root.join("assets").join(DEFAULT_SCENE))?.id;
    assert_eq!(editor.export.entry_scene(), Some(entry));
    assert_eq!(assets.index().products.len(), 1);
    assert_eq!(
        editor
            .sources
            .of_kind(comet_asset_data::product::AssetKind::Scene)
            .count(),
        1
    );
    assert_eq!(
        editor.export.template_error.as_deref(),
        Some("template kept across projects")
    );
    assert!(editor.translation_snap.enabled && !editor.shadows);
    assert_eq!(editor.translation_snap.step(), 0.25);
    editor.request_document_action(open_action(&original_root));
    editor.resolve_transition(Choice::Discard, &mut assets);
    wait(&mut editor, &mut assets)?;
    assert_eq!(editor.project_root(), original_root);
    assert_eq!(editor.products, original_products);
    assert_eq!(editor.document.shared_source(), authored);
    assert_eq!(editor.document.history_len(), 0);
    Ok(())
}

#[test]
fn failed_author_save_does_not_create_a_project_or_stop_play() -> anyhow::Result<()> {
    let (temp, mut editor, mut assets) = crate::test_support::fixture()?;
    preview(&mut editor, &mut assets)?;
    editor.start(&mut assets)?;
    let world = editor.world();
    let authored = editor.document.shared_source();
    let source = editor.source.clone();
    let backup = source.with_extension("backup");
    fs::rename(&source, &backup)?;
    fs::create_dir(&source)?;
    let destination = temp.path().join("must-not-create");
    editor.request_document_action(DocumentAction::NewProject(destination.clone()));
    editor.resolve_transition(Choice::Save, &mut assets);
    assert!(editor.documents.error.is_some());
    assert!(!editor.project_load.is_running());
    assert!(!destination.exists());
    assert_eq!(editor.world(), world);
    assert_eq!(editor.document.shared_source(), authored);
    editor.resolve_transition(Choice::Cancel, &mut assets);
    assert_eq!(editor.world(), world);
    Ok(())
}

#[test]
fn invalid_project_candidates_keep_old_play_resources_and_document_and_can_retry()
-> anyhow::Result<()> {
    let (temp, mut editor, mut assets) = crate::test_support::fixture()?;
    preview(&mut editor, &mut assets)?;
    editor.start(&mut assets)?;
    let world = editor.world();
    let source = editor.source.clone();
    let authored = editor.document.shared_source();
    let revision = assets.revision().to_owned();
    let root = temp.path().join("broken-project");
    comet_asset_pipeline::create_project(&root)?;
    let entry = root.join("assets").join(DEFAULT_SCENE);
    let valid = fs::read(&entry)?;
    fs::write(&entry, "broken scene")?;
    editor.request_document_action(open_action(&root));
    editor.resolve_transition(Choice::Discard, &mut assets);
    wait(&mut editor, &mut assets)?;
    assert!(editor.documents.error.is_some());
    assert_eq!(editor.world(), world);
    assert_eq!(editor.source, source);
    assert_eq!(editor.document.shared_source(), authored);
    assert_eq!(assets.revision(), revision);
    assert!(editor.document.is_dirty());
    fs::write(entry, valid)?;
    editor.resolve_transition(Choice::Discard, &mut assets);
    wait(&mut editor, &mut assets)?;
    assert_eq!(editor.project_root(), root.canonicalize()?);
    assert!(editor.play.is_none());
    assert!(editor.document.source().base.entities.is_empty());
    Ok(())
}

#[test]
fn creation_retry_reuses_published_identity_and_a_later_close_supersedes_the_load()
-> anyhow::Result<()> {
    let (temp, mut editor, mut assets) = crate::test_support::fixture()?;
    let original_root = editor.project_root().to_owned();
    let root = temp.path().join("published-game");
    let locations = crate::project::ProductLocations::new(
        &comet_core::ProjectPaths::open(&original_root)?,
        &editor.products,
    )?;
    let blocked_products = locations.for_root(&root)?;
    fs::create_dir_all(blocked_products.parent().unwrap())?;
    fs::write(&blocked_products, "blocked cache")?;
    editor.request_document_action(DocumentAction::NewProject(root.clone()));
    editor.resolve_transition(Choice::Discard, &mut assets);
    wait(&mut editor, &mut assets)?;
    assert!(root.is_dir());
    assert_eq!(editor.project_root(), original_root);
    assert!(matches!(
        editor.documents.action,
        Some(DocumentAction::OpenProject { .. })
    ));
    let meta = fs::read(root.join("assets").join(format!("{DEFAULT_SCENE}.meta")))?;
    fs::remove_file(blocked_products)?;
    editor.resolve_transition(Choice::Discard, &mut assets);
    assert!(editor.project_load.is_running());
    assert!(!editor.request_close());
    wait(&mut editor, &mut assets)?;
    assert_eq!(
        editor.project_root(),
        original_root,
        "closing must supersede project acceptance"
    );
    assert!(matches!(
        editor.documents.action,
        Some(DocumentAction::Exit)
    ));
    assert_eq!(
        fs::read(root.join("assets").join(format!("{DEFAULT_SCENE}.meta")))?,
        meta
    );
    editor.resolve_transition(Choice::Cancel, &mut assets);
    editor.request_document_action(open_action(&root));
    editor.resolve_transition(Choice::Discard, &mut assets);
    wait(&mut editor, &mut assets)?;
    assert_eq!(editor.project_root(), root.canonicalize()?);
    Ok(())
}

#[test]
fn cancelled_or_stale_project_candidates_cannot_replace_the_current_document() -> anyhow::Result<()>
{
    let (temp, mut editor, mut assets) = crate::test_support::fixture()?;
    let root = temp.path().join("other-game");
    comet_asset_pipeline::create_project(&root)?;
    let original = editor.document.shared_source();
    let original_root = editor.project_root().to_owned();
    let revision = assets.revision().to_owned();
    editor.request_document_action(open_action(&root));
    editor.resolve_transition(Choice::Discard, &mut assets);
    editor.resolve_transition(Choice::Cancel, &mut assets);
    wait(&mut editor, &mut assets)?;
    assert_eq!(editor.document.shared_source(), original);
    assert_eq!(editor.project_root(), original_root);
    assert!(!editor.documents.pending);
    editor.request_document_action(open_action(&root));
    editor.resolve_transition(Choice::Discard, &mut assets);
    editor.document.insert_entity(
        comet_scene::EntityData::new("changed during load"),
        &mut assets,
    )?;
    let changed = editor.document.shared_source();
    wait(&mut editor, &mut assets)?;
    assert!(
        editor
            .documents
            .error
            .as_deref()
            .is_some_and(|error| error.contains("加载期间变化"))
    );
    assert_eq!(editor.document.shared_source(), changed);
    assert_eq!(editor.project_root(), original_root);
    assert_eq!(assets.revision(), revision);
    assert_eq!(editor.document.history_len(), 1);
    Ok(())
}

#[cfg(unix)]
#[test]
fn opening_another_project_cannot_write_products_into_the_current_project_assets()
-> anyhow::Result<()> {
    use std::os::unix::fs::symlink;
    let (temp, mut editor, mut assets) = crate::test_support::fixture()?;
    let initial = comet_core::ProjectPaths::open(editor.project_root())?;
    let locations = crate::project::ProductLocations::new(&initial, &editor.products)?;
    let second = temp.path().join("second-game");
    let third = temp.path().join("third-game");
    comet_asset_pipeline::create_project(&second)?;
    comet_asset_pipeline::create_project(&third)?;
    editor.request_document_action(open_action(&second));
    editor.resolve_transition(Choice::Discard, &mut assets);
    wait(&mut editor, &mut assets)?;
    assert_eq!(editor.project_root(), second.canonicalize()?);
    let cache = locations.for_root(&third)?;
    fs::create_dir_all(cache.parent().unwrap())?;
    let current_assets = second.join("assets");
    let authored = fs::read(current_assets.join(DEFAULT_SCENE))?;
    symlink(&current_assets, &cache)?;
    editor.request_document_action(open_action(&third));
    editor.resolve_transition(Choice::Discard, &mut assets);
    assert!(
        editor
            .documents
            .error
            .as_deref()
            .is_some_and(|error| error.contains("当前项目作者资源"))
    );
    assert!(!editor.project_load.is_running());
    assert_eq!(editor.project_root(), second.canonicalize()?);
    assert_eq!(fs::read(current_assets.join(DEFAULT_SCENE))?, authored);
    assert!(!current_assets.join("index.json").exists());
    assert!(!current_assets.join("products").exists());
    Ok(())
}

#[test]
fn recent_projects_follow_only_accepted_projects_and_keep_the_last_author_file()
-> anyhow::Result<()> {
    let (temp, mut editor, mut assets) = crate::test_support::fixture()?;
    assert!(!editor.recent_projects.is_enabled());
    let preferences = temp.path().join("preferences/recent-projects.json");
    editor.recent_projects = crate::project::RecentProjects::load(preferences.clone())?;
    editor.record_current_project();
    let initial = fs::read(&preferences)?;
    let original_root = editor.project_root().to_owned();
    let original_world = editor.world();
    let next = temp.path().join("next-game");
    comet_asset_pipeline::create_project(&next)?;
    let author = std::path::PathBuf::from("nested/start.scene");
    fs::create_dir(next.join("assets/nested"))?;
    fs::rename(
        next.join("assets/main.scene"),
        next.join("assets").join(&author),
    )?;
    fs::rename(
        next.join("assets/main.scene.meta"),
        next.join("assets/nested/start.scene.meta"),
    )?;
    let action = DocumentAction::OpenProject {
        root: next.clone(),
        scene: author.clone(),
    };
    editor.request_document_action(action.clone());
    editor.resolve_transition(Choice::Cancel, &mut assets);
    assert_eq!(editor.world(), original_world);
    assert_eq!(fs::read(&preferences)?, initial);
    editor.request_document_action(open_action(&next)); // The default file deliberately does not exist.
    editor.resolve_transition(Choice::Discard, &mut assets);
    wait(&mut editor, &mut assets)?;
    assert!(editor.documents.error.is_some());
    assert_eq!(editor.project_root(), original_root);
    assert_eq!(editor.world(), original_world);
    assert_eq!(fs::read(&preferences)?, initial);
    editor.resolve_transition(Choice::Cancel, &mut assets);
    editor.request_document_action(action);
    editor.resolve_transition(Choice::Discard, &mut assets);
    wait(&mut editor, &mut assets)?;
    assert!(!editor.documents.pending);
    assert_eq!(editor.recent_projects.entries().len(), 2);
    assert_eq!(
        editor.recent_projects.entries()[0].root,
        next.canonicalize()?
    );
    assert_eq!(editor.recent_projects.entries()[0].source, author);
    assert_eq!(editor.recent_projects.entries()[1].root, original_root);

    let other = next.join("assets/nested/other.scene");
    fs::copy(&editor.source, &other)?;
    let mut metadata: serde_json::Value =
        serde_json::from_slice(&fs::read(next.join("assets/nested/start.scene.meta"))?)?;
    metadata["id"] = serde_json::to_value(comet_core::AssetId::new())?;
    fs::write(
        next.join("assets/nested/other.scene.meta"),
        serde_json::to_vec(&metadata)?,
    )?;
    editor.request_document_action(DocumentAction::Open("nested/other.scene".into()));
    editor.resolve_transition(Choice::Discard, &mut assets);
    assert!(!editor.documents.pending, "{:?}", editor.documents.error);
    assert_eq!(editor.recent_projects.entries().len(), 2);
    assert_eq!(
        editor.recent_projects.entries()[0].source,
        std::path::Path::new("nested/other.scene")
    );
    let reloaded = crate::project::RecentProjects::load(preferences)?;
    assert_eq!(
        reloaded.entries()[0].source,
        std::path::Path::new("nested/other.scene")
    );
    Ok(())
}

#[test]
fn a_recent_preferences_failure_cannot_undo_or_misreport_a_successful_project_load()
-> anyhow::Result<()> {
    let (temp, mut editor, mut assets) = crate::test_support::fixture()?;
    let folder = temp.path().join("preferences");
    let preferences = folder.join("recent-projects.json");
    editor.recent_projects = crate::project::RecentProjects::load(preferences.clone())?;
    editor.record_current_project();
    let original_root = editor.project_root().to_owned();
    fs::remove_file(&preferences)?;
    fs::remove_dir(&folder)?;
    fs::write(&folder, "not a directory")?;
    let next = temp.path().join("accepted-game");
    comet_asset_pipeline::create_project(&next)?;
    editor.request_document_action(open_action(&next));
    editor.resolve_transition(Choice::Discard, &mut assets);
    wait(&mut editor, &mut assets)?;
    assert!(!editor.documents.pending);
    assert_eq!(editor.project_root(), next.canonicalize()?);
    assert_eq!(
        editor.source,
        next.canonicalize()?.join("assets/main.scene")
    );
    assert!(editor.document.source().base.entities.is_empty());
    assert_eq!(assets.index().products.len(), 1);
    assert!(editor.error.is_none());
    assert!(editor.status.starts_with("已打开项目"));
    let warning = editor
        .recent_warning
        .as_ref()
        .expect("nonfatal preferences diagnostic");
    assert!(warning.len() <= 4096 && warning.contains("最近项目"));
    assert_eq!(editor.recent_projects.entries().len(), 1);
    assert_eq!(editor.recent_projects.entries()[0].root, original_root);
    assert_eq!(fs::read(&folder)?, b"not a directory");
    Ok(())
}
