use super::*;

#[test]
fn effective_defaults_roundtrip_under_a_required_private_game_envelope() -> Result<()> {
    let game = GameId::new();
    assert_eq!(Settings::default().master_volume, 1.);
    assert!(Settings::default().shadows);
    for master_volume in [0., 0.25, 1.] {
        for shadows in [false, true] {
            let settings = Settings {
                master_volume,
                shadows,
            };
            let bytes = settings.to_bytes(game)?;
            assert_eq!(Settings::from_bytes(&bytes, game)?, settings);
            let value: serde_json::Value = serde_json::from_slice(&bytes)?;
            assert_eq!(value["version"], 1);
            assert_eq!(value["game_id"], game.to_string());
            assert_eq!(value.as_object().unwrap().len(), 4);
            assert!(Settings::from_bytes(&bytes, GameId::new()).is_err());
        }
    }
    Ok(())
}

#[test]
fn settings_reject_unknown_missing_null_and_mistyped_fields_without_defaulting_them() -> Result<()>
{
    let game = GameId::new();
    let value: serde_json::Value = serde_json::from_slice(&Settings::default().to_bytes(game)?)?;
    for key in ["version", "game_id", "master_volume", "shadows"] {
        let mut missing = value.clone();
        missing.as_object_mut().unwrap().remove(key);
        assert!(
            Settings::from_bytes(&serde_json::to_vec(&missing)?, game).is_err(),
            "{key}"
        );
        let mut null = value.clone();
        null[key] = serde_json::Value::Null;
        assert!(
            Settings::from_bytes(&serde_json::to_vec(&null)?, game).is_err(),
            "{key}"
        );
    }
    for (key, invalid) in [
        ("version", serde_json::json!(2)),
        ("version", serde_json::json!("1")),
        (
            "game_id",
            serde_json::json!("00000000-0000-0000-0000-000000000000"),
        ),
        ("game_id", serde_json::json!(42)),
        ("master_volume", serde_json::json!("0.5")),
        ("master_volume", serde_json::json!(false)),
        ("master_volume", serde_json::json!(-0.1)),
        ("master_volume", serde_json::json!(1.1)),
        ("shadows", serde_json::json!(1)),
        ("future", serde_json::json!(true)),
    ] {
        let mut candidate = value.clone();
        candidate[key] = invalid;
        assert!(
            Settings::from_bytes(&serde_json::to_vec(&candidate)?, game).is_err(),
            "{candidate}"
        );
    }
    Ok(())
}

#[test]
fn settings_budget_is_checked_before_parsing_and_nonfinite_values_never_encode() -> Result<()> {
    let game = GameId::new();
    let error = Settings::from_bytes(&vec![b' '; MAX_SETTINGS_BYTES + 1], game).unwrap_err();
    assert!(error.to_string().contains("4 KiB"));
    for master_volume in [
        f32::NAN,
        f32::INFINITY,
        f32::NEG_INFINITY,
        -f32::EPSILON,
        1. + f32::EPSILON,
    ] {
        let settings = Settings {
            master_volume,
            ..Default::default()
        };
        assert!(settings.validate().is_err());
        assert!(settings.to_bytes(game).is_err());
    }
    let huge =
        format!(r#"{{"version":1,"game_id":"{game}","master_volume":1e100,"shadows":true}}"#);
    assert!(Settings::from_bytes(huge.as_bytes(), game).is_err());
    Ok(())
}
