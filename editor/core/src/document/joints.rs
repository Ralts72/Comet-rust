use super::*;
use comet_scene::{HingeJoint, PointJoint, SliderJoint};

/// Structural joint edits apply to authored entities; prefab fields use ordinary overrides.
pub enum JointEdit {
    Point(Option<PointJoint>),
    Hinge(Option<HingeJoint>),
    Slider(Option<SliderJoint>),
}

impl Document {
    pub fn preview_joint(
        &mut self,
        id: EntityId,
        edit: JointEdit,
        resolver: &mut impl PrefabResolver,
    ) -> Result<()> {
        self.preview_component(id, resolver, |entity| match edit {
            JointEdit::Point(value) => entity.point_joint = value,
            JointEdit::Hinge(value) => entity.hinge_joint = value,
            JointEdit::Slider(value) => entity.slider_joint = value,
        })
    }
}
