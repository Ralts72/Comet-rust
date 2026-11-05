fn on_start(ctx) {
    ctx.play_sound(ctx.entity());
    ctx.log("音频示例开始：E 重播提示音，空格停止提示音。");
}

fn on_fixed_update(ctx, dt) {
    if ctx.pressed("interact") {
        ctx.play_sound(ctx.entity());
        ctx.log("已重播提示音。");
    }
    if ctx.pressed("jump") {
        ctx.stop_sound(ctx.entity());
        ctx.log("已停止提示音。");
    }
}

fn on_stop(ctx) {
    ctx.log("音频示例停止。");
}
