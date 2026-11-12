use anyhow::Result;
use comet_asset_data::package::{
    AppTemplateManifest, PACKAGE_VERSION, PackageManifest, RuntimeIdentity,
};
use comet_core::AssetId;
use std::collections::BTreeMap;

fn identity() -> RuntimeIdentity {
    RuntimeIdentity::for_build("a".repeat(64), "fixture-target")
}

fn package(runtime: Option<RuntimeIdentity>) -> PackageManifest {
    let entry_scene = AssetId::new();
    PackageManifest {
        version: PACKAGE_VERSION,
        game_id: comet_core::GameId::new(),
        entry_scene,
        scenes: vec![entry_scene, AssetId::new()],
        executable: "CometApp".into(),
        runtime,
        game_icon_sha256: Some("b".repeat(64)),
    }
}

fn template(identity: RuntimeIdentity) -> AppTemplateManifest {
    AppTemplateManifest {
        version: 1,
        identity,
        profile: "release".into(),
        app: "CometApp".into(),
        app_sha256: "c".repeat(64),
        editor_sha256: "d".repeat(64),
        app_icon_sha256: "e".repeat(64),
        notices: BTreeMap::from([("THIRD_PARTY_NOTICES.txt".into(), "f".repeat(64))]),
    }
}

#[test]
fn package_structure_and_runtime_compatibility_are_separate_contracts() -> Result<()> {
    let expected = identity();
    expected.validate()?;
    let manifest = package(Some(expected.clone()));
    manifest.validate()?;
    manifest.validate_for(&expected)?;

    for field in ["build", "target", "os", "arch"] {
        let mut foreign = expected.clone();
        match field {
            "build" => foreign.build_id = "b".repeat(64),
            "target" => foreign.target = "other-target".into(),
            "os" => foreign.os = "other-os".into(),
            "arch" => foreign.arch = "other-arch".into(),
            _ => unreachable!(),
        }
        let candidate = package(Some(foreign));
        candidate.validate()?;
        assert!(
            candidate.validate_for(&expected).is_err(),
            "mismatched {field}"
        );
    }
    // The low-level asset CLI may package an external binary with no program identity.
    let external = package(None);
    external.validate()?;
    external.validate_for(&expected)?;
    Ok(())
}

#[test]
fn malformed_identities_and_every_protocol_mismatch_are_rejected_at_the_data_boundary() {
    let mut invalid = vec![];
    for build_id in [String::new(), "a".repeat(63), "g".repeat(64)] {
        let mut candidate = identity();
        candidate.build_id = build_id;
        invalid.push(candidate);
    }
    for field in ["target", "os", "arch"] {
        for value in [
            "",
            " ",
            "two words",
            "line\nbreak",
            "nul\0byte",
            "tab\tvalue",
        ] {
            let mut candidate = identity();
            match field {
                "target" => candidate.target = value.into(),
                "os" => candidate.os = value.into(),
                "arch" => candidate.arch = value.into(),
                _ => unreachable!(),
            }
            invalid.push(candidate);
        }
    }
    for field in ["product", "index", "content", "scene"] {
        let mut candidate = identity();
        match field {
            "product" => candidate.protocols.product += 1,
            "index" => candidate.protocols.index += 1,
            "content" => candidate.protocols.content += 1,
            "scene" => candidate.protocols.scene += 1,
            _ => unreachable!(),
        }
        invalid.push(candidate);
    }
    for candidate in invalid {
        assert!(candidate.validate().is_err(), "{candidate:?}");
        assert!(
            package(Some(candidate.clone())).validate().is_err(),
            "{candidate:?}"
        );
        assert!(
            template(candidate.clone()).validate().is_err(),
            "{candidate:?}"
        );
    }
}

#[test]
fn runtime_identity_free_packages_still_require_a_complete_scene_list_and_safe_file_names()
-> Result<()> {
    let valid = package(None);
    valid.validate()?;
    let mut invalid = vec![];
    let mut candidate = valid.clone();
    candidate.version += 1;
    invalid.push(candidate);
    let mut candidate = valid.clone();
    candidate.scenes.clear();
    invalid.push(candidate);
    let mut candidate = valid.clone();
    candidate.scenes.retain(|id| *id != candidate.entry_scene);
    invalid.push(candidate);
    let mut candidate = valid.clone();
    candidate.scenes.push(candidate.entry_scene);
    invalid.push(candidate);
    for name in [
        "",
        ".",
        "..",
        "../game",
        "game/app",
        "C:\\game.exe",
        "game:app",
    ] {
        let mut candidate = valid.clone();
        candidate.executable = name.into();
        invalid.push(candidate);
    }
    let mut candidate = valid.clone();
    candidate.game_icon_sha256 = Some("invalid digest".into());
    invalid.push(candidate);
    for candidate in invalid {
        assert!(candidate.validate().is_err(), "{candidate:?}");
        assert!(
            candidate.validate_for(&identity()).is_err(),
            "{candidate:?}"
        );
    }
    Ok(())
}

#[test]
fn package_game_identity_is_required_and_old_formats_are_rejected() -> Result<()> {
    let valid = package(Some(identity()));
    let encoded = serde_json::to_value(valid)?;
    for game_id in [
        serde_json::Value::Null,
        serde_json::json!("00000000-0000-0000-0000-000000000000"),
        serde_json::json!("not-a-game-id"),
        serde_json::json!(42),
    ] {
        let mut invalid = encoded.clone();
        invalid["game_id"] = game_id;
        assert!(serde_json::from_value::<PackageManifest>(invalid).is_err());
    }
    let mut missing = encoded.clone();
    missing.as_object_mut().unwrap().remove("game_id");
    assert!(serde_json::from_value::<PackageManifest>(missing).is_err());
    let mut old = encoded;
    old["version"] = 3.into();
    assert!(
        serde_json::from_value::<PackageManifest>(old)?
            .validate()
            .is_err()
    );
    Ok(())
}

#[test]
fn release_templates_validate_their_program_files_independently_of_the_host_identity() -> Result<()>
{
    let mut foreign = identity();
    foreign.build_id = "1".repeat(64);
    foreign.os = "foreign-os".into();
    foreign.arch = "foreign-arch".into();
    let valid = template(foreign);
    valid.validate()?;
    let mut invalid = vec![];
    let mut candidate = valid.clone();
    candidate.profile = "debug".into();
    invalid.push(candidate);
    let mut candidate = valid.clone();
    candidate.version = 3;
    invalid.push(candidate);
    let mut candidate = valid.clone();
    candidate.app = "../CometApp".into();
    invalid.push(candidate);
    for field in ["app", "editor", "icon", "notice"] {
        let mut candidate = valid.clone();
        match field {
            "app" => candidate.app_sha256 = "bad".into(),
            "editor" => candidate.editor_sha256 = "bad".into(),
            "icon" => candidate.app_icon_sha256 = "bad".into(),
            "notice" => {
                candidate.notices.insert("LICENSE.txt".into(), "bad".into());
            }
            _ => unreachable!(),
        }
        invalid.push(candidate);
    }
    let mut candidate = valid.clone();
    candidate
        .notices
        .insert("../LICENSE.txt".into(), "a".repeat(64));
    invalid.push(candidate);
    for candidate in invalid {
        assert!(candidate.validate().is_err(), "{candidate:?}");
    }
    Ok(())
}
