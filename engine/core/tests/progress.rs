use anyhow::Result;
use comet_core::{
    AssetId, CHECKPOINT_VERSION, Checkpoint, EntityId, GameId, MAX_CHECKPOINT_BYTES,
    MAX_PROGRESS_BYTES, MAX_PROGRESS_ENTRIES, MAX_PROGRESS_KEY_BYTES, MAX_PROGRESS_TEXT_BYTES,
    ProgressEntry, ProgressValue, ProgressValues,
};

fn entry(key: &str, value: ProgressValue) -> ProgressEntry {
    ProgressEntry {
        scene: AssetId::new(),
        owner: EntityId::new(),
        key: key.into(),
        value,
    }
}

#[test]
fn checkpoint_roundtrip_has_canonical_scopes_and_preserves_leaf_types() -> Result<()> {
    let mut first = entry("中文🌠", ProgressValue::Bool(true));
    first.scene = "00000000-0000-4000-8000-000000000001".parse()?;
    let mut entries = vec![first.clone()];
    for (key, value) in [
        ("integer", ProgressValue::Integer(i64::MAX)),
        ("number", ProgressValue::Number(1.25)),
        ("text", ProgressValue::Text("换关\n完成🌠".into())),
    ] {
        let mut next = first.clone();
        next.key = key.into();
        next.value = value;
        entries.push(next);
    }
    let mut other_scene = first.clone();
    other_scene.scene = AssetId::new();
    entries.push(other_scene);
    let mut other_owner = first;
    other_owner.owner = EntityId::new();
    entries.push(other_owner);
    let game = GameId::new();
    let scene = AssetId::new();
    let checkpoint = Checkpoint::new(game, scene, ProgressValues { entries })?;
    let bytes = checkpoint.to_bytes()?;
    let mut reversed = checkpoint.clone();
    reversed.progress.entries.reverse();
    assert_eq!(bytes, reversed.to_bytes()?);
    let decoded = Checkpoint::from_bytes(&bytes)?;
    assert_eq!(decoded, checkpoint);
    decoded.validate_for(game)?;
    assert!(decoded.validate_for(GameId::new()).is_err());
    assert_eq!(decoded.version, CHECKPOINT_VERSION);
    assert_eq!(decoded.scene, scene);
    Ok(())
}

#[test]
fn duplicate_scopes_invalid_leaves_and_entry_limits_are_rejected() -> Result<()> {
    let first = entry("collected", ProgressValue::Bool(true));
    let mut values = ProgressValues {
        entries: vec![first.clone(), first.clone()],
    };
    assert!(values.validate().is_err());
    values.entries.truncate(1);
    for value in [
        ProgressValue::Number(f64::NAN),
        ProgressValue::Number(f64::INFINITY),
        ProgressValue::Text("a".repeat(MAX_PROGRESS_TEXT_BYTES + 1)),
        ProgressValue::Text("bad\0text".into()),
    ] {
        values.entries[0].value = value;
        assert!(values.validate().is_err());
    }
    values.entries[0].value = ProgressValue::Integer(1);
    for key in [
        "".into(),
        " \t".into(),
        "x\ny".into(),
        "a".repeat(MAX_PROGRESS_KEY_BYTES + 1),
    ] {
        values.entries[0].key = key;
        assert!(values.validate().is_err());
    }
    values.entries = (0..MAX_PROGRESS_ENTRIES)
        .map(|index| {
            let mut entry = first.clone();
            entry.key = format!("k{index}");
            entry
        })
        .collect();
    let mut extra = first;
    extra.key = "extra".into();
    values.entries.push(extra);
    assert!(values.validate().is_err());
    let oversized = serde_json::to_vec(&values)?;
    assert!(serde_json::from_slice::<ProgressValues>(&oversized).is_err());
    Ok(())
}

#[test]
fn progress_budget_counts_json_escaping_and_checkpoint_checks_size_before_parsing() -> Result<()> {
    let game = GameId::new();
    let scene = AssetId::new();
    // Four short raw strings expand sixfold when JSON encodes control characters.
    let entries = (0..4)
        .map(|index| {
            entry(
                &format!("escaped{index}"),
                ProgressValue::Text("\u{0001}".repeat(800)),
            )
        })
        .collect();
    let values = ProgressValues { entries };
    assert!(serde_json::to_vec(&values)?.len() > MAX_PROGRESS_BYTES);
    assert!(Checkpoint::new(game, scene, values).is_err());
    let error = Checkpoint::from_bytes(&vec![b'?'; MAX_CHECKPOINT_BYTES + 1]).unwrap_err();
    assert!(error.to_string().contains("byte budget"));
    let valid = Checkpoint::new(game, scene, ProgressValues::default())?;
    let mut json = serde_json::to_value(valid)?;
    for (field, value) in [
        ("version", serde_json::json!(2)),
        (
            "game_id",
            serde_json::json!("00000000-0000-0000-0000-000000000000"),
        ),
        ("unknown", serde_json::json!(true)),
    ] {
        let mut invalid = json.clone();
        invalid[field] = value;
        assert!(Checkpoint::from_bytes(&serde_json::to_vec(&invalid)?).is_err());
    }
    json["progress"] = serde_json::json!([{
        "scene":scene,"owner":EntityId::new(),"key":"x","value":{"type":"entity","value":EntityId::new()}
    }]);
    assert!(Checkpoint::from_bytes(&serde_json::to_vec(&json)?).is_err());
    Ok(())
}
