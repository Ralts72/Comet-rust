use super::*;
use comet_core::{AssetId, EntityId, ProgressEntry, ProgressValue, ProgressValues};

struct Temporary(PathBuf);
impl Temporary {
    fn new() -> Result<Self> {
        let path = std::env::temp_dir().join(format!("comet-game-data-{}", GameId::new()));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }
}
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn checkpoint(game: GameId, value: bool) -> Result<Checkpoint> {
    Checkpoint::new(
        game,
        AssetId::new(),
        ProgressValues {
            entries: vec![ProgressEntry {
                scene: AssetId::new(),
                owner: EntityId::new(),
                key: "collected".into(),
                value: ProgressValue::Bool(value),
            }],
        },
    )
}

#[test]
fn memory_slot_is_game_scoped_and_rejects_invalid_replacements() -> Result<()> {
    let game = GameId::new();
    let mut slot = Store::memory(game);
    assert!(slot.load_checkpoint()?.is_none());
    let first = checkpoint(game, true)?;
    slot.save_checkpoint(&first)?;
    assert_eq!(
        slot.load_checkpoint()?.unwrap().to_bytes()?,
        first.to_bytes()?
    );
    assert!(
        slot.save_checkpoint(&checkpoint(GameId::new(), false)?)
            .is_err()
    );
    let mut invalid = checkpoint(game, false)?;
    invalid.version = 0;
    assert!(slot.save_checkpoint(&invalid).is_err());
    assert_eq!(
        slot.load_checkpoint()?.unwrap().to_bytes()?,
        first.to_bytes()?
    );
    assert!(Store::memory(game).load_checkpoint()?.is_none());
    assert!(slot.load_settings()?.is_none());
    let settings = Settings {
        master_volume: 0.25,
        shadows: false,
    };
    slot.save_settings(&settings)?;
    assert_eq!(slot.load_settings()?, Some(settings));
    assert_eq!(
        slot.load_checkpoint()?.unwrap().to_bytes()?,
        first.to_bytes()?
    );
    let invalid = Settings {
        master_volume: f32::NAN,
        ..settings
    };
    assert!(slot.save_settings(&invalid).is_err());
    assert_eq!(slot.load_settings()?, Some(settings));
    assert_eq!(
        slot.load_checkpoint()?.unwrap().to_bytes()?,
        first.to_bytes()?
    );
    slot.save_checkpoint(&checkpoint(game, false)?)?;
    assert_eq!(slot.load_settings()?, Some(settings));
    assert!(Store::memory(game).load_settings()?.is_none());
    Ok(())
}

#[test]
fn explicit_directory_reopens_the_same_game_slot_without_creating_on_load() -> Result<()> {
    let temp = Temporary::new()?;
    let root = temp.0.join(" 存档 数据 ");
    let game = GameId::new();
    let mut slot = Store::directory(game, root.clone())?;
    assert!(slot.load_checkpoint()?.is_none());
    assert!(slot.load_settings()?.is_none());
    assert!(!root.exists());
    let first = checkpoint(game, true)?;
    slot.save_checkpoint(&first)?;
    let path = root.join(game.to_string()).join("checkpoint.json");
    assert_eq!(fs::read(&path)?, first.to_bytes()?);
    assert_eq!(
        Store::directory(game, root.clone())?
            .load_checkpoint()?
            .unwrap()
            .to_bytes()?,
        first.to_bytes()?
    );
    assert!(
        Store::directory(GameId::new(), root.clone())?
            .load_checkpoint()?
            .is_none()
    );
    assert!(Store::directory(game, "relative".into()).is_err());
    let before = fs::read(&path)?;
    assert!(
        slot.save_checkpoint(&checkpoint(GameId::new(), false)?)
            .is_err()
    );
    assert_eq!(fs::read(&path)?, before);
    let settings = Settings {
        master_volume: 0.5,
        shadows: false,
    };
    slot.save_settings(&settings)?;
    let settings_path = path.with_file_name("settings.json");
    let settings_bytes = fs::read(&settings_path)?;
    assert_eq!(fs::read(&path)?, before);
    assert_eq!(
        Store::directory(game, root.clone())?.load_settings()?,
        Some(settings)
    );
    slot.save_checkpoint(&checkpoint(game, false)?)?;
    assert_eq!(fs::read(&settings_path)?, settings_bytes);
    let other = GameId::new();
    assert!(
        Store::directory(other, root.clone())?
            .load_settings()?
            .is_none()
    );
    assert!(!root.join(other.to_string()).exists());
    Ok(())
}

#[test]
fn malformed_oversized_and_foreign_files_are_preserved_on_load_failure() -> Result<()> {
    let temp = Temporary::new()?;
    let game = GameId::new();
    let mut slot = Store::directory(game, temp.0.clone())?;
    let first = checkpoint(game, true)?;
    slot.save_checkpoint(&first)?;
    let path = temp.0.join(game.to_string()).join("checkpoint.json");
    for bytes in [
        b"not json".to_vec(),
        vec![b' '; MAX_CHECKPOINT_BYTES + 1],
        checkpoint(GameId::new(), false)?.to_bytes()?,
    ] {
        fs::write(&path, &bytes)?;
        assert!(slot.load_checkpoint().is_err());
        assert_eq!(fs::read(&path)?, bytes);
    }
    // A deliberate save replaces the old slot only after validating the full candidate.
    slot.save_checkpoint(&first)?;
    assert_eq!(
        slot.load_checkpoint()?.unwrap().to_bytes()?,
        first.to_bytes()?
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn trusted_root_links_are_resolved_but_game_and_file_links_are_rejected() -> Result<()> {
    use std::os::unix::fs::symlink;
    let temp = Temporary::new()?;
    let real_root = temp.0.join("real");
    fs::create_dir(&real_root)?;
    let root_link = temp.0.join("root-link");
    symlink(&real_root, &root_link)?;
    let game = GameId::new();
    let mut slot = Store::directory(game, root_link)?;
    let first = checkpoint(game, true)?;
    slot.save_checkpoint(&first)?;
    assert!(slot.load_checkpoint()?.is_some());
    let game_path = real_root.join(game.to_string());
    let file = game_path.join("checkpoint.json");
    let outside = temp.0.join("outside.json");
    fs::write(&outside, b"keep me")?;
    fs::remove_file(&file)?;
    symlink(&outside, &file)?;
    assert!(slot.load_checkpoint().is_err());
    assert!(slot.save_checkpoint(&first).is_err());
    assert_eq!(fs::read(&outside)?, b"keep me");
    fs::remove_file(&file)?;
    fs::remove_dir(&game_path)?;
    symlink(&temp.0, &game_path)?;
    assert!(slot.load_checkpoint().is_err());
    assert!(slot.save_checkpoint(&first).is_err());
    assert!(!temp.0.join("checkpoint.json").exists());
    assert!(slot.load_settings().is_err());
    assert!(slot.save_settings(&Settings::default()).is_err());
    assert!(!temp.0.join("settings.json").exists());
    Ok(())
}

#[test]
fn invalid_settings_are_never_overwritten_and_cannot_modify_the_checkpoint() -> Result<()> {
    let temp = Temporary::new()?;
    let game = GameId::new();
    let mut store = Store::directory(game, temp.0.clone())?;
    let checkpoint = checkpoint(game, true)?;
    store.save_checkpoint(&checkpoint)?;
    let directory = temp.0.join(game.to_string());
    let checkpoint_bytes = fs::read(directory.join("checkpoint.json"))?;
    let settings_path = directory.join("settings.json");
    let defaults = Settings::default();
    let mut future: serde_json::Value = serde_json::from_slice(&defaults.to_bytes(game)?)?;
    future["version"] = 2.into();
    for bytes in [
        b"invalid settings".to_vec(),
        vec![b' '; MAX_SETTINGS_BYTES + 1],
        defaults.to_bytes(GameId::new())?,
        serde_json::to_vec(&future)?,
        br#"{"version":1}"#.to_vec(),
    ] {
        fs::write(&settings_path, &bytes)?;
        assert!(store.load_settings().is_err());
        assert!(store.save_settings(&defaults).is_err());
        assert_eq!(fs::read(&settings_path)?, bytes);
        assert_eq!(
            fs::read(directory.join("checkpoint.json"))?,
            checkpoint_bytes
        );
    }
    // Repair is explicit; neither reading nor Apply silently replaces a rejected file.
    fs::remove_file(&settings_path)?;
    store.save_settings(&defaults)?;
    let bytes = fs::read(&settings_path)?;
    assert!(
        store
            .save_settings(&Settings {
                master_volume: -1.,
                ..defaults
            })
            .is_err()
    );
    assert_eq!(fs::read(&settings_path)?, bytes);
    assert_eq!(
        fs::read(directory.join("checkpoint.json"))?,
        checkpoint_bytes
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn settings_reject_linked_or_non_file_destinations_without_touching_other_data() -> Result<()> {
    use std::os::unix::fs::symlink;
    let temp = Temporary::new()?;
    let game = GameId::new();
    let real_root = temp.0.join("data");
    fs::create_dir(&real_root)?;
    let root_link = temp.0.join("root-link");
    symlink(&real_root, &root_link)?;
    let mut store = Store::directory(game, root_link)?;
    let settings = Settings {
        master_volume: 0.25,
        shadows: false,
    };
    store.save_settings(&settings)?;
    assert_eq!(store.load_settings()?, Some(settings));
    store.save_checkpoint(&checkpoint(game, true)?)?;
    let directory = real_root.join(game.to_string());
    let settings_path = directory.join("settings.json");
    let checkpoint_bytes = fs::read(directory.join("checkpoint.json"))?;
    let outside = temp.0.join("outside.json");
    fs::write(&outside, b"preserve external settings")?;
    fs::remove_file(&settings_path)?;
    for target in [outside.clone(), temp.0.join("missing.json")] {
        symlink(&target, &settings_path)?;
        assert!(store.load_settings().is_err());
        assert!(store.save_settings(&settings).is_err());
        assert!(
            fs::symlink_metadata(&settings_path)?
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            fs::read(directory.join("checkpoint.json"))?,
            checkpoint_bytes
        );
        fs::remove_file(&settings_path)?;
    }
    assert_eq!(fs::read(&outside)?, b"preserve external settings");
    assert!(!temp.0.join("missing.json").exists());
    fs::create_dir(&settings_path)?;
    assert!(store.load_settings().is_err());
    assert!(store.save_settings(&settings).is_err());
    assert!(settings_path.is_dir());
    assert_eq!(
        fs::read(directory.join("checkpoint.json"))?,
        checkpoint_bytes
    );
    Ok(())
}

#[test]
fn linux_data_home_is_absolute_and_uses_data_rather_than_editor_state() -> Result<()> {
    use std::ffi::OsString;
    let temp = Temporary::new()?;
    let environment = |xdg: Option<OsString>, home: Option<OsString>| {
        linux_directory(|key| match key {
            "XDG_DATA_HOME" => xdg.clone(),
            "HOME" => home.clone(),
            _ => None,
        })
    };
    assert_eq!(
        environment(Some(temp.0.clone().into()), None)?,
        temp.0.join("comet/games")
    );
    for xdg in [None, Some(OsString::new()), Some("relative".into())] {
        assert_eq!(
            environment(xdg, Some(temp.0.clone().into()))?,
            temp.0.join(".local/share/comet/games")
        );
    }
    assert!(environment(None, None).is_err());
    assert!(environment(None, Some("relative".into())).is_err());
    Ok(())
}
