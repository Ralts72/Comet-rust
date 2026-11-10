fn on_start(ctx) {
    this.unlocked = false;
    this.requested = false;
    this.count = -1;
    this.ready = -1;
    ctx.set_hud(ctx.param("label") + "\n收集目标：0 / 3\nWASD 移动，Space 跳跃，Ctrl 蹲伏；走近目标收集。");
}

fn on_fixed_update(ctx, dt) {
    let targets = [ctx.param("target_1"), ctx.param("target_2"), ctx.param("target_3")];
    let count = 0;
    let ready = 0;
    for target in targets {
        if ctx.progress_of(target, "collected", false) { count += 1; }
        if ctx.progress_of(target, "ready", false) { ready += 1; }
    }
    if count != this.count || ready != this.ready {
        this.count = count;
        this.ready = ready;
        let hint = if count < 3 { "走近剩余目标收集。" }
            else if ready < 3 { "等待收集动作完成。" }
            else { "出口已打开！走到出口，重新按 E 前往另一关。" };
        ctx.set_hud(ctx.param("label") + "\n收集目标：" + count + " / 3\n" + hint);
    }
    if ready < 3 || this.requested { return; }
    if !this.unlocked {
        this.unlocked = true;
        let gate = ctx.position(ctx.param("gate"));
        ctx.teleport(ctx.param("gate"), gate[0], gate[1] + 4.0, gate[2]);
        ctx.log("三个目标动作完成，出口已打开。");
        return;
    }
    let position = ctx.position(ctx.param("player"));
    let exit = ctx.param("exit");
    let dx = position[0] - exit[0];
    let dz = position[2] - exit[2];
    if dx * dx + dz * dz <= 2.25 && ctx.pressed("interact") {
        this.requested = ctx.request_scene(ctx.param("next_scene"));
    }
}
