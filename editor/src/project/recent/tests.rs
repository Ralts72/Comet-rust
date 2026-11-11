use super::*;
use std::fs;

fn project(base: &Path, name: &str, sources: &[&str]) -> Result<ProjectPaths> {
    let root = base.join(name);
    fs::create_dir_all(root.join("assets"))?;
    let paths = ProjectPaths::open(root)?;
    let bytes = comet_scene::ContentSource::default().to_json()?;
    for source in sources {
        let path = paths.assets().join(source);
        fs::create_dir_all(path.parent().unwrap())?;
        fs::write(path, &bytes)?;
    }
    Ok(paths)
}
fn environment(os: &str, values: &[(&str, OsString)]) -> Result<PathBuf> {
    user_path(os, |name| {
        values
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.clone())
    })
}

#[test]
fn platform_paths_are_editor_specific_absolute_and_never_fall_back_to_the_working_directory()
-> Result<()> {
    let temp = tempfile::tempdir()?;
    let base = temp.path().as_os_str().to_owned();
    for (os, key, suffix) in [
        (
            "macos",
            "HOME",
            "Library/Application Support/Comet/editor/recent-projects.json",
        ),
        ("windows", "APPDATA", "Comet/editor/recent-projects.json"),
        (
            "linux",
            "HOME",
            ".local/state/comet/editor/recent-projects.json",
        ),
        (
            "linux",
            "XDG_STATE_HOME",
            "comet/editor/recent-projects.json",
        ),
    ] {
        assert_eq!(
            environment(os, &[(key, base.clone())])?,
            temp.path().join(suffix)
        );
    }
    assert_eq!(
        environment(
            "linux",
            &[("HOME", base.clone()), ("XDG_STATE_HOME", OsString::new())]
        )?,
        temp.path()
            .join(".local/state/comet/editor/recent-projects.json")
    );
    for (os, key) in [("macos", "HOME"), ("windows", "APPDATA"), ("linux", "HOME")] {
        assert!(environment(os, &[]).is_err());
        for value in [OsString::new(), OsString::from("relative")] {
            assert!(environment(os, &[(key, value)]).is_err());
        }
    }
    assert!(
        environment(
            "linux",
            &[("HOME", base), ("XDG_STATE_HOME", "relative".into())]
        )
        .is_err()
    );
    assert!(environment("unknown", &[]).is_err());
    assert!(RecentProjects::load("relative-recent.json".into()).is_err());
    Ok(())
}

#[test]
fn disabled_state_does_no_io_and_loading_a_missing_file_does_not_create_it() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let paths = project(temp.path(), "project", &["main.scene"])?;
    let source = paths.assets().join("main.scene");
    let storage = temp.path().join("state/recent-projects.json");
    let enabled = RecentProjects::load(storage.clone())?;
    assert!(enabled.is_enabled());
    assert!(enabled.entries().is_empty());
    assert!(!storage.parent().unwrap().exists());
    fs::remove_dir_all(paths.root())?;
    let mut disabled = RecentProjects::default();
    assert!(!disabled.is_enabled());
    disabled.record(&paths, &source)?;
    disabled.record(&paths, Path::new("../unaccepted.scene"))?;
    disabled.remove(paths.root())?;
    assert!(disabled.entries().is_empty());
    assert!(!storage.exists());
    Ok(())
}

#[test]
fn accepted_roots_are_unique_most_recent_and_remember_the_last_scene_or_prefab() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let first = project(temp.path(), "first", &["main.scene", "nested/unit.prefab"])?;
    let second = project(temp.path(), "second", &["custom.scene"])?;
    let storage = temp.path().join("state/recent-projects.json");
    let mut recent = RecentProjects::load(storage.clone())?;
    let source = first.assets().join("main.scene");
    let original = fs::read(&source)?;
    recent.record(&first, &source)?;
    recent.record(&second, &second.assets().join("custom.scene"))?;
    recent.record(&first, &first.assets().join("nested/unit.prefab"))?;
    recent.record(&first, &first.assets().join("nested/unit.prefab"))?;
    assert_eq!(recent.entries().len(), 2);
    assert_eq!(
        recent.entries()[0],
        RecentProject {
            root: first.root().to_owned(),
            source: "nested/unit.prefab".into()
        }
    );
    assert_eq!(recent.entries()[1].root, second.root());
    assert_eq!(RecentProjects::load(storage)?.entries(), recent.entries());
    assert_eq!(fs::read(source)?, original);
    assert!(!first.assets().join("main.scene.meta").exists());
    Ok(())
}

#[test]
fn the_entry_budget_evicts_old_projects_and_deleted_history_can_be_removed_without_visiting_it()
-> Result<()> {
    let temp = tempfile::tempdir()?;
    let storage = temp.path().join("recent-projects.json");
    let mut recent = RecentProjects::load(storage.clone())?;
    let mut roots = Vec::new();
    for index in 0..MAX_ENTRIES + 2 {
        let paths = project(temp.path(), &format!("project-{index}"), &["entry.scene"])?;
        recent.record(&paths, &paths.assets().join("entry.scene"))?;
        roots.push(paths.root().to_owned());
    }
    assert_eq!(recent.entries().len(), MAX_ENTRIES);
    assert_eq!(recent.entries()[0].root, roots[MAX_ENTRIES + 1]);
    assert_eq!(recent.entries()[MAX_ENTRIES - 1].root, roots[2]);
    fs::remove_dir_all(&roots[MAX_ENTRIES + 1])?;
    recent = RecentProjects::load(storage.clone())?;
    assert_eq!(recent.entries()[0].root, roots[MAX_ENTRIES + 1]);
    recent.remove(&roots[MAX_ENTRIES + 1])?;
    assert_eq!(recent.entries().len(), MAX_ENTRIES - 1);
    assert_eq!(RecentProjects::load(storage)?.entries(), recent.entries());
    Ok(())
}

#[test]
fn malformed_or_oversized_configuration_is_rejected_without_repairing_its_bytes() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let storage = temp.path().join("recent-projects.json");
    let root = temp.path().join("missing-project");
    let good = json!({"version":VERSION,"projects":[{"root":root,"source":"main.scene"}]});
    let mut cases = Vec::new();
    let mut value = good.clone();
    value["version"] = 2.into();
    cases.push(value);
    let mut value = good.clone();
    value["extra"] = true.into();
    cases.push(value);
    let mut value = good.clone();
    value["projects"][0]["extra"] = true.into();
    cases.push(value);
    let mut value = good.clone();
    value["projects"][0]["root"] = "relative".into();
    cases.push(value);
    let mut value = good.clone();
    value["projects"][0]["root"] = json!(temp.path().join("one/../two"));
    cases.push(value);
    for source in [
        "../outside.scene",
        "/absolute.scene",
        "wrong.gltf",
        "nul\0.scene",
        "",
    ] {
        let mut value = good.clone();
        value["projects"][0]["source"] = source.into();
        cases.push(value);
    }
    let mut value = good.clone();
    value["projects"]
        .as_array_mut()
        .unwrap()
        .push(good["projects"][0].clone());
    cases.push(value);
    let mut value = good;
    value["projects"] = json!(vec![value["projects"][0].clone(); MAX_ENTRIES + 1]);
    cases.push(value);
    for value in cases {
        let bytes = serde_json::to_vec(&value)?;
        fs::write(&storage, &bytes)?;
        assert!(RecentProjects::load(storage.clone()).is_err());
        assert_eq!(fs::read(&storage)?, bytes);
    }
    let mut exact = serde_json::to_vec(&json!({"version":VERSION,"projects":[]}))?;
    exact.resize(MAX_FILE_BYTES, b' ');
    fs::write(&storage, &exact)?;
    assert!(RecentProjects::load(storage.clone())?.entries().is_empty());
    exact.push(b' ');
    fs::write(&storage, &exact)?;
    assert!(
        format!("{:#}", RecentProjects::load(storage.clone()).err().unwrap()).contains("exceeds")
    );
    assert_eq!(fs::read(storage)?, exact);
    Ok(())
}

#[test]
fn record_revalidates_real_author_paths_and_rejected_sources_keep_the_saved_history() -> Result<()>
{
    let temp = tempfile::tempdir()?;
    let paths = project(temp.path(), "project", &["main.scene", "wrong.gltf"])?;
    let storage = temp.path().join("recent-projects.json");
    let mut recent = RecentProjects::load(storage.clone())?;
    recent.record(&paths, &paths.assets().join("main.scene"))?;
    let entries = recent.entries().to_vec();
    let bytes = fs::read(&storage)?;
    let outside = temp.path().join("outside.scene");
    fs::write(&outside, b"outside")?;
    for source in [
        outside,
        paths.assets().join("wrong.gltf"),
        paths.assets().join("missing.scene"),
        paths.assets().to_owned(),
        "main.scene".into(),
    ] {
        assert!(recent.record(&paths, &source).is_err());
        assert_eq!(recent.entries(), entries);
        assert_eq!(fs::read(&storage)?, bytes);
    }
    Ok(())
}

#[test]
fn write_or_encoding_failures_preserve_memory_and_the_last_complete_file() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let first = project(temp.path(), "first", &["main.scene"])?;
    let second = project(temp.path(), "second", &["main.scene"])?;
    let parent = temp.path().join("state");
    let storage = parent.join("recent-projects.json");
    let mut recent = RecentProjects::load(storage.clone())?;
    recent.record(&first, &first.assets().join("main.scene"))?;
    let entries = recent.entries().to_vec();
    let bytes = fs::read(&storage)?;
    let saved = temp.path().join("saved-state");
    fs::rename(&parent, &saved)?;
    fs::write(&parent, b"blocked parent")?;
    assert!(
        recent
            .record(&second, &second.assets().join("main.scene"))
            .is_err()
    );
    assert!(recent.remove(first.root()).is_err());
    assert_eq!(recent.entries(), entries);
    assert_eq!(fs::read(saved.join("recent-projects.json"))?, bytes);
    fs::remove_file(&parent)?;
    fs::rename(saved, &parent)?;
    assert!(
        recent
            .publish(vec![RecentProject {
                root: temp.path().join("x".repeat(MAX_FILE_BYTES)),
                source: "main.scene".into()
            }])
            .is_err()
    );
    assert_eq!(recent.entries(), entries);
    assert_eq!(fs::read(storage)?, bytes);
    Ok(())
}

#[cfg(unix)]
#[test]
fn canonical_aliases_deduplicate_and_storage_links_are_not_overwritten() -> Result<()> {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir()?;
    let paths = project(temp.path(), "project", &["main.scene"])?;
    let alias = temp.path().join("alias");
    symlink(paths.root(), &alias)?;
    let alias_paths = ProjectPaths::open(&alias)?;
    let storage = temp.path().join("recent-projects.json");
    let mut recent = RecentProjects::load(storage.clone())?;
    recent.record(&paths, &paths.assets().join("main.scene"))?;
    recent.record(&alias_paths, &alias.join("assets/main.scene"))?;
    assert_eq!(recent.entries().len(), 1);
    let bytes = fs::read(&storage)?;
    let target = temp.path().join("preserved.json");
    fs::rename(&storage, &target)?;
    symlink(&target, &storage)?;
    assert!(RecentProjects::load(storage.clone()).is_err());
    assert!(recent.remove(paths.root()).is_err());
    assert_eq!(recent.entries().len(), 1);
    assert_eq!(fs::read(target)?, bytes);
    assert!(fs::symlink_metadata(storage)?.is_symlink());
    Ok(())
}
