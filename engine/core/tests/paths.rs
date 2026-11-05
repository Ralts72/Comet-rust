use anyhow::Result;
use comet_core::{DirectoryCandidate, ProjectPaths};
use std::fs;

#[test]
fn project_paths_check_existing_and_new_destinations_by_canonical_components() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let project = temp.path().join("project");
    let assets = project.join("assets");
    fs::create_dir_all(assets.join("scenes"))?;
    fs::create_dir_all(project.join("assets-extra"))?;
    fs::write(assets.join("main.scene"), b"inside")?;
    fs::write(project.join("outside.scene"), b"outside")?;
    let paths = ProjectPaths::open(&project)?;
    assert_eq!(
        paths.resolve_asset_path("scenes/../main.scene")?,
        assets.join("main.scene").canonicalize()?
    );
    assert_eq!(
        paths.resolve_asset_path("new/folder/scene.scene")?,
        assets.canonicalize()?.join("new/folder/scene.scene")
    );
    for path in ["../outside.scene", "../assets-extra/new.scene", "", "."] {
        assert!(paths.resolve_asset_path(path).is_err(), "{path}");
    }
    assert!(
        paths
            .resolve_asset_path(project.join("outside.scene"))
            .is_err()
    );
    let products = project.join("cache/products");
    assert!(
        paths
            .export_destination(assets.join("game"), &products)
            .is_err()
    );
    assert!(paths.export_destination(&project, &products).is_err());
    assert!(
        paths
            .export_destination(products.join("game"), &products)
            .is_err()
    );
    assert!(
        paths
            .export_destination(temp.path().join("output"), &products)
            .is_ok()
    );
    Ok(())
}
#[cfg(unix)]
#[test]
fn symlink_escape_is_rechecked_for_reads_and_saves() -> Result<()> {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir()?;
    let project = temp.path().join("project");
    let assets = project.join("assets");
    let outside = temp.path().join("outside");
    fs::create_dir_all(assets.join("folder"))?;
    fs::create_dir(&outside)?;
    fs::write(outside.join("external.scene"), b"outside")?;
    fs::write(assets.join("main.scene"), b"inside")?;
    let paths = ProjectPaths::open(&project)?;
    symlink(&outside, assets.join("escape"))?;
    symlink(outside.join("external.scene"), assets.join("bad.scene"))?;
    assert!(paths.resolve_asset_path("escape/external.scene").is_err());
    assert!(paths.resolve_asset_path("escape/new.scene").is_err());
    assert!(paths.resolve_asset_path("bad.scene").is_err());
    let save = paths.resolve_asset_path("folder/new.scene")?;
    fs::remove_dir(assets.join("folder"))?;
    symlink(&outside, assets.join("folder"))?;
    assert!(paths.resolve_asset_path(save).is_err());
    symlink(assets.join("main.scene"), assets.join("internal.scene"))?;
    assert_eq!(
        paths.resolve_asset_path("internal.scene")?,
        assets.join("main.scene").canonicalize()?
    );
    Ok(())
}
#[cfg(unix)]
#[test]
fn the_assets_directory_itself_must_stay_inside_the_project() -> Result<()> {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir()?;
    let project = temp.path().join("project");
    let outside = temp.path().join("outside");
    fs::create_dir(&project)?;
    fs::create_dir(&outside)?;
    fs::write(outside.join("main.scene"), b"outside")?;
    symlink(&outside, project.join("assets"))?;
    assert!(ProjectPaths::open(&project).is_err());
    assert_eq!(fs::read(outside.join("main.scene"))?, b"outside");
    fs::remove_file(project.join("assets"))?;
    symlink(&project, project.join("assets"))?;
    assert!(ProjectPaths::open(&project).is_err());
    Ok(())
}
#[cfg(unix)]
#[test]
fn project_and_internal_assets_aliases_work_but_retargeted_assets_are_rejected() -> Result<()> {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir()?;
    let project = temp.path().join("project");
    let sources = project.join("source-data");
    let alias = temp.path().join("project-alias");
    let outside = temp.path().join("outside");
    fs::create_dir_all(&sources)?;
    fs::create_dir(&outside)?;
    fs::write(sources.join("main.scene"), b"inside")?;
    symlink(&sources, project.join("assets"))?;
    symlink(&project, &alias)?;
    let paths = ProjectPaths::open(&alias)?;
    assert_eq!(paths.root(), project.canonicalize()?);
    assert_eq!(paths.assets(), sources.canonicalize()?);
    assert_eq!(
        paths.resolve_asset_path("main.scene")?,
        sources.join("main.scene").canonicalize()?
    );
    let saved = paths.resolve_asset_path("new.scene")?;
    fs::remove_file(project.join("assets"))?;
    symlink(&outside, project.join("assets"))?;
    assert!(paths.resolve_asset_path("main.scene").is_err());
    assert!(paths.resolve_asset_path(saved).is_err());
    assert!(
        paths
            .export_destination(temp.path().join("export"), &project.join("products"))
            .is_err()
    );
    assert_eq!(fs::read(sources.join("main.scene"))?, b"inside");
    assert!(fs::read_dir(&outside)?.next().is_none());
    Ok(())
}
#[test]
fn directory_publication_is_atomic_exclusive_and_only_cleans_its_own_candidate() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let output = temp.path().join("game");
    {
        let candidate = DirectoryCandidate::new(&output)?;
        fs::write(candidate.path().join("partial"), b"incomplete")?;
        assert!(!output.exists());
    }
    assert!(!output.exists());
    let first = DirectoryCandidate::new(&output)?;
    let second = DirectoryCandidate::new(&output)?;
    fs::write(first.path().join("manifest"), b"complete")?;
    first.publish()?;
    assert!(second.publish().is_err());
    assert_eq!(fs::read(output.join("manifest"))?, b"complete");
    assert!(DirectoryCandidate::new(&output).is_err());
    let another = temp.path().join("appeared");
    let candidate = DirectoryCandidate::new(&another)?;
    fs::create_dir(&another)?;
    assert!(candidate.publish().is_err());
    assert!(another.is_dir());
    Ok(())
}
