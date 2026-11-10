fn standalone(ctx) {
    !ctx.has_param("standalone") || ctx.param("standalone")
}

fn on_start(ctx) {
    this.collected = ctx.progress("collected", false);
    this.finished = this.collected;
    this.requested = false;
    this.reported = this.collected;
    ctx.pause_animation(ctx.param("animator"));
    if !standalone(ctx) {
        ctx.set_progress("ready", this.collected);
        if this.collected { ctx.set_scale(ctx.param("visual"), 0, 0, 0); }
        return;
    }
    if this.collected {
        ctx.set_scale(ctx.param("visual"), 0, 0, 0);
        ctx.set_hud("收集目标：1 / 1\n动作完成！重新按 E 前往第二关。");
        ctx.log("收集示例继续：目标已收集，重新按 E 前往第二关。");
    } else {
        ctx.set_hud("收集目标：0 / 1\n走近金色方块收集。");
        ctx.log("收集示例开始：走近金色方块，动作结束后重新按 E 前往第二关。");
    }
}

fn on_trigger(ctx, other, entered) {
    if !entered || this.collected || other.id() != ctx.param("player").id() {
        return;
    }
    ctx.set_scale(ctx.param("visual"), 0, 0, 0);
    ctx.set_progress("collected", true);
    ctx.play_animation(ctx.param("animator"));
    ctx.play_sound(ctx.entity());
    if !standalone(ctx) {
        ctx.set_progress("ready", false);
    } else {
        ctx.set_hud("收集目标：1 / 1\n动作播放中，请稍候。");
    }
    this.collected = true;
    ctx.log("已收集目标：1 / 1。");
}

fn on_fixed_update(ctx, dt) {
    if !this.collected || this.requested {
        return;
    }
    if !standalone(ctx) || !this.finished { return; }
    if !this.reported {
        this.reported = true;
        ctx.set_hud("收集目标：1 / 1\n动作完成！重新按 E 前往第二关。");
        ctx.log("收集动作播放完成。");
        // 完成提示这一帧的按下不能跳关；等待下一次按下。
        return;
    }
    if ctx.pressed("interact") {
        if ctx.request_scene(ctx.param("next_scene")) {
            this.requested = true;
            ctx.log("已请求前往第二关。");
        }
    }
}

fn on_animation_event(ctx, animator, name) {
    if this.collected && !this.finished && animator.id() == ctx.param("animator").id() && name == "动作完成" {
        this.finished = true;
        if !standalone(ctx) { ctx.set_progress("ready", true); }
    }
}

fn on_stop(ctx) {
    ctx.log("收集示例停止。");
}
