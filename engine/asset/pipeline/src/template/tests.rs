use super::*;
use comet_core::digest;
#[test]
fn templates_match_explicit_program_identity_and_verify_all_snapshot_inputs() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path();
    let app = root.join("comet-app");
    let editor = root.join("editor");
    fs::write(&app, b"app snapshot")?;
    fs::write(&editor, b"editor snapshot")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&app, fs::Permissions::from_mode(0o755))?;
    }
    let identity = RuntimeIdentity::for_build("a".repeat(64), "fixture-target");
    fs::create_dir(root.join("licenses"))?;
    fs::write(root.join("licenses/font.txt"), b"font license")?;
    fs::create_dir(root.join("icons"))?;
    fs::write(root.join("icons/app.icns"), b"app icon")?;
    let manifest = AppTemplateManifest {
        version: 4,
        identity: identity.clone(),
        profile: "release".into(),
        app: "comet-app".into(),
        app_sha256: digest(&fs::read(&app)?),
        editor_sha256: digest(&fs::read(&editor)?),
        app_icon_sha256: digest(b"app icon"),
        notices: std::collections::BTreeMap::from([("font.txt".into(), digest(b"font license"))]),
    };
    let path = root.join("template.json");
    fs::write(&path, serde_json::to_vec(&manifest)?)?;
    let template = AppTemplate::open(&path, &editor, &identity)?;
    for mutate in 0..4 {
        let mut expected = identity.clone();
        match mutate {
            0 => expected.build_id = "b".repeat(64),
            1 => expected.target = "other-target".into(),
            2 => expected.os = "other-os".into(),
            _ => expected.arch = "other-arch".into(),
        }
        assert!(AppTemplate::open(&path, &editor, &expected).is_err());
    }
    fs::write(root.join("icons/app.icns"), b"changed icon")?;
    assert!(template.verify().is_err());
    fs::write(root.join("icons/app.icns"), b"app icon")?;
    fs::write(root.join("licenses/font.txt"), b"changed license")?;
    assert!(template.verify().is_err());
    fs::write(root.join("licenses/font.txt"), b"font license")?;
    template.verify()?;
    for mutate in 0..8 {
        let mut candidate = manifest.clone();
        match mutate {
            0 => candidate.identity.build_id = "b".repeat(64),
            1 => candidate.identity.target = "other-target".into(),
            2 => candidate.identity.os = "other-os".into(),
            3 => candidate.identity.arch = "other-arch".into(),
            4 => candidate.identity.protocols.scene += 1,
            5 => candidate.profile = "dev".into(),
            6 => candidate.app = "../outside".into(),
            _ => candidate.version = 3,
        }
        fs::write(&path, serde_json::to_vec(&candidate)?)?;
        assert!(AppTemplate::open(&path, &editor, &identity).is_err());
    }
    fs::write(&path, serde_json::to_vec(&manifest)?)?;
    fs::write(&editor, b"another editor")?;
    assert!(AppTemplate::open(&path, &editor, &identity).is_err());
    fs::write(&app, b"overwritten cargo binary")?;
    assert!(template.verify().is_err());
    fs::remove_file(&app)?;
    assert!(format!("{:#}", template.verify().unwrap_err()).contains("start.sh"));
    Ok(())
}
