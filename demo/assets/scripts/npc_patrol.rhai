fn on_start(ctx) {
    this.mode = "patrol";
    this.to_b = true;
    this.wait = 0.0;
    ctx.log("导航巡逻开始。");
}

fn on_fixed_update(ctx, dt) {
    let npc = ctx.entity();
    let player = ctx.param("player");
    let changed = false;
    let chasing = false;
    if type_of(player) == "Entity" {
        let position = ctx.position(npc);
        let target = ctx.position(player);
        let dx = target[0] - position[0];
        let dz = target[2] - position[2];
        let height = target[1] - position[1];
        let distance_squared = dx * dx + dz * dz;
        let detect = ctx.param("detect_distance");
        let forget = ctx.param("forget_distance");
        if distance_squared < detect * detect && height > -2.0 && height < 2.0 {
            if this.mode != "chase" {
                ctx.log("进入追逐。");
                changed = true;
            }
            this.mode = "chase";
            this.wait = 0.0;
        } else if this.mode == "chase" && distance_squared > forget * forget {
            this.mode = "return";
            this.to_b = false;
            changed = true;
            ctx.log("脱离追逐，返回巡逻。");
        }
        chasing = this.mode == "chase";
    } else if this.mode == "chase" {
        this.mode = "return";
        this.to_b = false;
        changed = true;
        ctx.log("目标已清除，返回巡逻。");
    }
    if this.wait > 0.0 {
        this.wait -= dt;
        ctx.stop_navigation(npc);
        return;
    }
    // The query describes the previous goal; a new chase/return goal has not steered yet.
    if !changed && !chasing && ctx.navigation_status(npc) == "arrived" {
        this.mode = "patrol";
        this.to_b = !this.to_b;
        this.wait = ctx.param("wait_seconds");
        ctx.stop_navigation(npc);
        ctx.log("巡逻到点，等待后折返。");
        return;
    }
    if chasing {
        ctx.navigate_to(npc, player);
    } else {
        let point = if this.to_b { ctx.param("patrol_b") } else { ctx.param("patrol_a") };
        ctx.navigate_to(npc, point);
    }
}

fn on_stop(ctx) {
    ctx.log("导航巡逻停止。");
}
