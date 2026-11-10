use crate::{Application, Camera};
use anyhow::{Context, Result, ensure};
use comet_audio::{AudioFrame, AudioReport, AudioState, ListenerPose, Output};
use comet_core::{
    WorldId,
    math::{DVec3, Mat3, Quat},
};
use std::time::{Duration, Instant};

#[derive(Default)]
pub(crate) struct AudioHost {
    output: Output,
    last_frame: Option<AudioFrame>,
    last_error: Option<String>,
    scope: Option<WorldId>,
}

impl AudioHost {
    pub fn submit(
        &mut self,
        app: &mut impl Application,
        world: WorldId,
        camera: Camera,
        active: bool,
    ) {
        let prepared = app.audio_frame().and_then(|frame| {
            let Some(mut frame) = frame else {
                return Ok(None);
            };
            ensure!(frame.world == world, "音频帧不属于当前视图世界");
            if frame.voices.iter().any(|voice| voice.spatial.is_some()) {
                frame.listener = Some(listener(camera)?);
            }
            frame.paused |= !active;
            Ok(Some(frame))
        });
        let scope = if matches!(&prepared, Ok(None)) {
            None
        } else {
            Some(world)
        };
        if self.scope != scope {
            self.last_error = None;
            self.scope = scope;
        }
        match prepared {
            Ok(frame) => {
                self.last_frame = frame;
                self.apply(app);
            }
            Err(error) => {
                self.last_frame = None;
                // A content failure must not discard an independently accepted host setting.
                let _ = self.output.set_master_gain(app.audio_master_gain());
                self.output.reject(world, error);
                self.notify(app, "音频帧准备失败".into());
            }
        }
    }

    // Visibility events may arrive without another redraw, so pause the device immediately.
    pub fn pause(&mut self, app: &mut impl Application) {
        if let Some(frame) = &mut self.last_frame {
            frame.paused = true;
            self.apply(app);
        }
    }

    fn apply(&mut self, app: &mut impl Application) {
        let gain_error = self.output.set_master_gain(app.audio_master_gain()).err();
        if let Err(error) = self.output.submit(self.last_frame.as_ref()) {
            self.notify(app, format!("{error:#}"));
        } else if let Some(error) = gain_error {
            // Reject a bad control value without poisoning this world's valid audio frame.
            self.notify(app, format!("主音量设置无效：{error:#}"));
        }
    }

    fn notify(&mut self, app: &mut impl Application, fallback: String) {
        let mut message = self.output.report().error.unwrap_or(fallback);
        message.truncate(message.floor_char_boundary(4096));
        if self.last_error.as_deref() != Some(&message) {
            app.audio_error(&message);
            self.last_error = Some(message);
        }
    }

    pub fn shutdown(&mut self) {
        self.output.shutdown();
        self.last_frame = None;
    }

    pub fn report(&self) -> AudioReport {
        self.output.report()
    }

    /// Short offscreen runs can finish before the device's first asynchronous callback.
    /// Observe the existing handles without submitting another frame or advancing the world.
    pub fn wait_for_progress(&self, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        loop {
            let report = self.report();
            if report.state != AudioState::Ready
                || report.active_voices == 0
                || report
                    .voices
                    .iter()
                    .any(|voice| voice.position_seconds > 0.)
                || Instant::now() >= deadline
            {
                return;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

fn listener(camera: Camera) -> Result<ListenerPose> {
    let eye = camera.eye.as_dvec3();
    let target = camera.target.as_dvec3();
    ensure!(
        eye.is_finite() && target.is_finite(),
        "监听相机位置必须有限"
    );
    let forward = (target - eye)
        .try_normalize()
        .context("监听相机需要有效朝向")?;
    let right = forward
        .cross(DVec3::Y)
        .try_normalize()
        .context("监听相机朝向不能平行于上轴")?;
    let up = right.cross(forward);
    let rotation = Quat::from_mat3(&Mat3::from_cols(
        right.as_vec3(),
        up.as_vec3(),
        -forward.as_vec3(),
    ))
    .normalize();
    Ok(ListenerPose {
        position: camera.eye.to_array(),
        rotation: rotation.to_array(),
    })
}

#[cfg(test)]
mod tests;
