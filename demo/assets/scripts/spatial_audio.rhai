fn on_start(ctx) {
    this.time = 0.0;
    this.playing = true;
    ctx.log("空间音频示例开始：E 开关移动声源，环绕相机比较方向和距离。");
}

fn on_fixed_update(ctx, dt) {
    this.time += dt;
    let angle = 3.141592653589793 + this.time * 0.6;
    ctx.set_position(ctx.entity(), 3.0 * cos(angle), 0.7, 3.0 * sin(angle));
    if ctx.pressed("interact") {
        this.playing = !this.playing;
        if this.playing {
            ctx.play_sound(ctx.entity());
            ctx.log("已开启空间声源。");
        } else {
            ctx.stop_sound(ctx.entity());
            ctx.log("已关闭空间声源。");
        }
    }
}

fn on_stop(ctx) {
    ctx.log("空间音频示例停止。");
}
