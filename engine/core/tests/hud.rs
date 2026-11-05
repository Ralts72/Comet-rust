use anyhow::Result;
use comet_core::{
    EntityId, HudFrame, HudLabel, MAX_HUD_BYTES, MAX_HUD_LABEL_BYTES, MAX_HUD_LABELS, WorldId,
};
use std::sync::Arc;

fn label(text: impl Into<Arc<str>>) -> HudLabel {
    HudLabel {
        owner: EntityId::new(),
        text: text.into(),
    }
}
fn frame(labels: Vec<HudLabel>) -> HudFrame {
    HudFrame {
        world: WorldId::new(),
        labels: labels.into(),
    }
}

#[test]
fn text_limits_count_utf8_bytes_and_reject_nul_without_rewriting_text() -> Result<()> {
    let text = format!("{}🌠", "中".repeat((MAX_HUD_LABEL_BYTES - 4) / 3));
    assert_eq!(text.len(), MAX_HUD_LABEL_BYTES);
    let accepted = frame(vec![label(text.clone())]);
    accepted.validate()?;
    assert_eq!(accepted.labels[0].text.as_ref(), text);
    assert!(frame(vec![label(format!("{text}a"))]).validate().is_err());
    assert!(frame(vec![label("开始\0结束")]).validate().is_err());
    // Plain text permits line breaks, tabs and an empty label; clear removes its owner separately.
    frame(vec![label("任务\n\t0 / 1"), label("")]).validate()?;
    Ok(())
}

#[test]
fn frames_bound_total_text_and_owners_independently() -> Result<()> {
    let mut labels: Vec<_> = (0..MAX_HUD_BYTES / MAX_HUD_LABEL_BYTES)
        .map(|_| label("a".repeat(MAX_HUD_LABEL_BYTES)))
        .collect();
    frame(labels.clone()).validate()?;
    labels.push(label("a"));
    assert!(frame(labels).validate().is_err());

    let mut labels: Vec<_> = (0..MAX_HUD_LABELS).map(|_| label("")).collect();
    frame(labels.clone()).validate()?;
    labels.push(label(""));
    assert!(frame(labels).validate().is_err());
    let entry = label("goal");
    assert!(frame(vec![entry.clone(), entry]).validate().is_err());
    frame(vec![]).validate()?;
    Ok(())
}
