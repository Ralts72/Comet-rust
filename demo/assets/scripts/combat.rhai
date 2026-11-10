fn actor(ctx, key, label, player) {
    #{ entity: ctx.param(key), visual: ctx.param(key + "_visual"),
       model: ctx.param(key + "_model"), animator: ctx.param(key + "_animator"),
       label: label, player: player, health: if player {120} else {90},
       maximum: if player {120} else {90}, damage: if player {35} else {12},
       range: if player {1.9} else {1.7}, interval: if player {0.8} else {1.15},
       cooldown: 0.0, attacking: false, hit_done: false, pulse: 0.0, defeat_time: 0.0, removed: false }
}
fn begin_attack(ctx, a) {
    a.attacking = true; a.hit_done = false; a.cooldown = a.interval;
    ctx.set_animation_parameter(a.animator, "attacking", true);
    ctx.log("战斗攻击：" + a.label);
    a
}
fn defeated(actors) {
    let count = 0;
    for a in actors { if !a.player && a.health <= 0 { count += 1; } }
    count
}
fn show_hud(ctx, actors) {
    let count = defeated(actors);
    let text = "战斗样例\n练习者：" + actors[0].health + " / 120\n守卫 1：" + actors[1].health
        + " / 90　守卫 2：" + actors[2].health + " / 90\n击败：" + count + " / 2\n";
    if actors[0].health <= 0 { text += "失败！按 R 重新开始。"; }
    else if count == 2 { text += "胜利！按 R 重新开始。"; }
    else { text += "WASD 移动，J / 鼠标左键攻击；朝向目标，隔墙不命中。"; }
    ctx.set_hud(text);
}
fn strike(ctx, actors, index) {
    let a = actors[index];
    if a.health <= 0 || !a.attacking || a.hit_done { return actors; }
    a.hit_done = true; actors[index] = a;
    let p = ctx.position(a.entity);
    let origin = ctx.transform_point(a.visual, 0, 0, 0);
    let front = ctx.transform_point(a.visual, 0, 0, -1);
    let fx = front[0] - origin[0]; let fz = front[2] - origin[2];
    let length = sqrt(fx * fx + fz * fz);
    if length < 0.000001 { return actors; }
    fx /= length; fz /= length;
    for j in 0..actors.len() {
        let b = actors[j];
        if index == j || a.player == b.player || b.health <= 0 { continue; }
        let q = ctx.position(b.entity);
        let dx = q[0] - p[0]; let dz = q[2] - p[2];
        let distance = sqrt(dx * dx + dz * dz);
        if distance > a.range || abs(q[1] - p[1]) > 1.2 { continue; }
        if distance > 0.001 && (dx * fx + dz * fz) / distance <= 0.35 { continue; }
        if distance > 0.001 && ctx.raycast(p, [dx, q[1] - p[1], dz], 1, a.entity) != () { continue; }
        b.health -= a.damage; if b.health < 0 { b.health = 0; }
        b.pulse = 0.2;
        ctx.play_sound(b.entity);
        ctx.log("战斗命中：" + a.label + " → " + b.label + "，剩余 " + b.health);
        if b.health == 0 {
            b.attacking = false;
            ctx.set_animation_parameter(b.animator, "attacking", false);
            ctx.pause_animation(b.animator);
            ctx.set_character_motion_enabled(b.entity, false);
            if !b.player { ctx.stop_navigation(b.entity); }
        }
        actors[j] = b;
    }
    show_hud(ctx, actors);
    actors
}
fn on_start(ctx) {
    this.actors = [actor(ctx, "player", "练习者", true),
                   actor(ctx, "guard_1", "守卫 1", false), actor(ctx, "guard_2", "守卫 2", false)];
    show_hud(ctx, this.actors);
}
fn on_fixed_update(ctx, dt) {
    let actors = this.actors;
    let active = actors[0].health > 0 && defeated(actors) < 2;
    let target = ctx.position(actors[0].entity);
    for i in 0..actors.len() {
        let a = actors[i];
        if a.removed { continue; }
        if !a.player && a.health <= 0 {
            a.defeat_time += dt;
            let size = 0.9 * max(0.001, 1.0 - a.defeat_time / 0.4);
            ctx.set_scale(a.model, size, size, size);
            if a.defeat_time >= 1.2 {
                ctx.destroy_entity(a.entity);
                a.removed = true;
                ctx.log("战斗移除：" + a.label);
            }
            actors[i] = a;
            continue;
        }
        a.cooldown = max(0.0, a.cooldown - dt); a.pulse = max(0.0, a.pulse - dt);
        let size = 0.9 * (1.0 + a.pulse * 0.35);
        ctx.set_scale(a.model, size, size, size);
        if a.player {
            if active && !a.attacking && a.cooldown == 0.0 && ctx.pressed("primary") { a = begin_attack(ctx, a); }
        } else if active && a.health > 0 && !a.attacking {
            let p = ctx.position(a.entity); let dx = target[0] - p[0]; let dz = target[2] - p[2];
            let distance = sqrt(dx * dx + dz * dz);
            if distance >= a.range * 0.8 { ctx.navigate_to(a.entity, actors[0].entity); }
            else {
                ctx.stop_navigation(a.entity);
                if distance > 0.001 { ctx.face_character(a.entity, dx, dz); }
                if a.cooldown == 0.0 { a = begin_attack(ctx, a); }
            }
        }
        let moving = active && a.health > 0 && !a.attacking;
        ctx.set_character_motion_enabled(a.entity, moving);
        if !a.player && !moving { ctx.stop_navigation(a.entity); }
        actors[i] = a;
    }
    this.actors = actors; show_hud(ctx, actors);
}
fn on_animation_event(ctx, animator, name) {
    for i in 0..this.actors.len() {
        if animator.id() != this.actors[i].animator.id() { continue; }
        if name == "combat.strike" { this.actors = strike(ctx, this.actors, i); }
        if name == "combat.recover" && this.actors[i].health > 0 {
            this.actors[i].attacking = false;
            ctx.set_animation_parameter(animator, "attacking", false);
        }
    }
}
