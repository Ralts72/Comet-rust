fn on_start(ctx) {
    ctx.log("落体方块示例已启动：按 E 弹起方块。");
}

fn on_fixed_update(ctx, dt) {
    if ctx.pressed("interact") {
        let target = ctx.param("target");
        let impulse = ctx.param("impulse");
        ctx.impulse(target, 0.0, impulse, 0.0);
        ctx.log("已向落体方块施加向上冲量。");
    }
}

fn on_stop(ctx) {
    ctx.log("落体方块示例已停止。");
}
