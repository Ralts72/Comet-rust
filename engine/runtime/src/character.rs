//! Capsule motor: gameplay intent to a collision-constrained kinematic pose.
use crate::FIXED_DELTA;
use comet_core::{Action, ActionInput, EntityId};
use comet_scene::Character;
use rapier3d::{
    control::{CharacterAutostep, CharacterLength, KinematicCharacterController},
    prelude::*,
};

#[derive(Clone, Debug)]
pub struct CharacterState {
    pub entity: EntityId,
    pub position: [f32; 3],
    pub grounded: bool,
    pub vertical_velocity: f32,
}
pub(super) struct Motor {
    pub config: Character,
    pub state: CharacterState,
}
impl Motor {
    pub fn next_pose(
        &mut self,
        world: &PhysicsWorld,
        handle: RigidBodyHandle,
        collider_handle: ColliderHandle,
        input: &ActionInput,
    ) -> Pose {
        let dt = FIXED_DELTA as f32;
        let c = &self.config;
        let controlled = c.player_controlled && input.enabled;
        let jump = controlled
            && input
                .events
                .iter()
                .any(|e| e.action == Action::Jump && e.pressed);
        if self.state.grounded && self.state.vertical_velocity < 0. {
            self.state.vertical_velocity = 0.;
        }
        if jump && self.state.grounded {
            self.state.vertical_velocity = c.jump_speed;
        }
        self.state.vertical_velocity -= c.gravity * dt;
        let axes = if controlled { input.movement } else { [0.; 2] };
        let mut direction = Vector::new(axes[0], 0., -axes[1]);
        if direction.length_squared() > 1. {
            direction = direction.normalize();
        }
        let speed = c.move_speed
            * if controlled && input.held.contains(Action::Run) {
                c.run_multiplier
            } else {
                1.
            };
        let desired = direction * speed * dt + Vector::Y * self.state.vertical_velocity * dt;
        let controller = KinematicCharacterController {
            offset: CharacterLength::Absolute(0.01),
            autostep: (c.step_height > 0.).then_some(CharacterAutostep {
                max_height: CharacterLength::Absolute(c.step_height),
                // Require a landing width matching the capsule diameter.
                min_width: CharacterLength::Relative(1.0),
                include_dynamic_bodies: false,
            }),
            max_slope_climb_angle: c.max_slope_degrees.to_radians(),
            min_slope_slide_angle: c.max_slope_degrees.to_radians(),
            snap_to_ground: (c.snap_distance > 0.)
                .then_some(CharacterLength::Absolute(c.snap_distance)),
            // Keep repeated floor casts above the contact tolerance.
            normal_nudge_factor: 0.001,
            ..Default::default()
        };
        let collider = &world.colliders[collider_handle];
        let current = *world.bodies[handle].position();
        let queries = world.query_pipeline_with_filter(
            QueryFilter::default()
                .exclude_sensors()
                .exclude_rigid_body(handle)
                .groups(collider.collision_groups()),
        );
        let movement =
            controller.move_shape(dt, &queries, collider.shape(), &current, desired, |_| {});
        self.state.grounded = movement.grounded;
        if (movement.grounded && self.state.vertical_velocity < 0.)
            || (self.state.vertical_velocity > 0. && movement.translation.y + 1e-4 < desired.y)
        {
            self.state.vertical_velocity = 0.;
        }
        let mut next = current;
        next.translation += movement.translation;
        next
    }
}
