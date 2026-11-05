use anyhow::Result;
use comet_core::math::Vec3;
use comet_runtime::{FIXED_DELTA, JointKind, JointStatus, PlaySession};
use comet_scene::ContentSource;

#[test]
fn authored_joint_demo_simulates_and_restarts_without_changing_its_source() -> Result<()> {
    let source = ContentSource::from_json(include_bytes!("../../../demo/assets/joints.scene"))?;
    assert!(source.instances.is_empty());
    let author = source.base;
    let baseline = author.clone();
    let mut play = PlaySession::start(author.clone())?;
    for _ in 0..180 {
        play.advance(FIXED_DELTA)?;
    }
    let snapshot = play.shared_snapshot();
    for owner in &snapshot.entities {
        let (kind, anchor, connected) = if let Some(point) = &owner.point_joint {
            (JointKind::Point, point.anchor, point.connected_anchor)
        } else if let Some(hinge) = &owner.hinge_joint {
            (
                JointKind::Hinge,
                hinge.frame.anchor,
                hinge.connected_frame.anchor,
            )
        } else if owner.slider_joint.is_some() {
            assert_eq!(
                play.joint_status(owner.id, JointKind::Slider),
                Some(JointStatus::Active)
            );
            assert!((owner.transform.translation[0] - 2.65).abs() < 0.015);
            assert!((owner.transform.translation[1] - 1.2).abs() < 0.015);
            continue;
        } else {
            continue;
        };
        assert_eq!(play.joint_status(owner.id, kind), Some(JointStatus::Active));
        let actual = owner
            .transform
            .matrix()
            .transform_point3(Vec3::from_array(anchor));
        assert!(
            actual.distance(Vec3::from_array(connected)) < 0.02,
            "{kind:?}: {actual:?}"
        );
    }
    assert_eq!(author, baseline);
    play.set_paused(true);
    play.restart()?;
    assert_eq!(play.snapshot(), baseline);
    assert!(play.is_paused());
    Ok(())
}
