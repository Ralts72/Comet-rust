fn on_animation_event(ctx, animator, name) {
    if animator.id() != ctx.param("animator").id() || name != "footstep" { return; }
    let state = ctx.animation_state(animator);
    if (state.state == "ground" || state.state == "crouch") && ctx.animation_parameter(animator, "grounded") && ctx.animation_parameter(animator, "speed") > 0.2 {
        ctx.play_sound(ctx.entity());
    }
}
