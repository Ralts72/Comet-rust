fn on_start(ctx) {
    this.requested = false;
    this.elapsed = 0.0;
    ctx.log("进入关卡：" + ctx.param("label") + "。按 E 前往另一关。");
}

fn on_fixed_update(ctx, dt) {
    if this.requested {
        return;
    }
    this.elapsed += dt;
    let auto_after = if ctx.has_param("auto_after") { ctx.param("auto_after") } else { 0.0 };
    if ctx.pressed("interact") || (auto_after > 0.0 && this.elapsed >= auto_after) {
        if ctx.request_scene(ctx.param("next_scene")) {
            this.requested = true;
            ctx.log("已请求前往另一关。");
        }
    }
}

fn on_stop(ctx) {
    ctx.log("离开关卡：" + ctx.param("label") + "。");
}
