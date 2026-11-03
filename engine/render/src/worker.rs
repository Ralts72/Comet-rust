use crate::{FrameFeedback, FramePacket, Surface, backend, pick};
use anyhow::{Context, Result};
use std::{
    sync::mpsc::{self, Receiver, SyncSender},
    thread::{self, JoinHandle},
};
pub(crate) trait FrameRenderer {
    fn name(&self) -> &'static str;
    fn render(&mut self, frame: &FramePacket) -> Result<(bool, f64)>;
}
pub struct RenderWorker {
    sender: Option<SyncSender<FramePacket>>,
    feedback: Option<Receiver<Result<FrameFeedback, String>>>,
    join: Option<JoinHandle<()>>,
}
impl RenderWorker {
    pub fn spawn(surface: Option<Surface>) -> Result<Self> {
        Self::spawn_with(move || backend::Renderer::new(surface))
    }
    fn spawn_with<F, R>(factory: F) -> Result<Self>
    where
        F: FnOnce() -> Result<R> + Send + 'static,
        R: FrameRenderer + 'static,
    {
        let (sender, receiver) = mpsc::sync_channel::<FramePacket>(1);
        let (feedback, results) = mpsc::sync_channel(2);
        let (ready, started) = mpsc::sync_channel(1);
        let join = thread::Builder::new()
            .name("comet-render".into())
            .spawn(move || {
                let mut renderer = match factory() {
                    Ok(r) => {
                        let _ = ready.send(Ok(()));
                        r
                    }
                    Err(e) => {
                        let _ = ready.send(Err(format!("{e:#}")));
                        return;
                    }
                };
                while let Ok(frame) = receiver.recv() {
                    let result = renderer
                        .render(&frame)
                        .map(|(presented, gpu_ms)| FrameFeedback {
                            number: frame.number,
                            token: frame.token,
                            presented,
                            backend: renderer.name().into(),
                            worker_thread: thread::current().name().unwrap_or("render").into(),
                            draw_count: frame.draws.len(),
                            gpu_ms,
                            pick: pick(&frame),
                        })
                        .map_err(|e| format!("frame {}: {e:#}", frame.number));
                    let failed = result.is_err();
                    if feedback.send(result).is_err() || failed {
                        break;
                    }
                }
            })?;
        match started.recv().context("render worker did not start")? {
            Ok(()) => Ok(Self {
                sender: Some(sender),
                feedback: Some(results),
                join: Some(join),
            }),
            Err(e) => {
                let _ = join.join();
                anyhow::bail!(e)
            }
        }
    }
    pub fn submit(&self, frame: FramePacket) -> Result<()> {
        frame.validate()?;
        self.sender
            .as_ref()
            .context("render worker closed")?
            .send(frame)
            .map_err(|_| anyhow::anyhow!("render worker disconnected"))
    }
    pub fn drain(&self) -> Result<Vec<FrameFeedback>> {
        self.feedback
            .as_ref()
            .context("render worker closed")?
            .try_iter()
            .map(|r| r.map_err(anyhow::Error::msg))
            .collect()
    }
    pub fn wait(&self, number: u64) -> Result<FrameFeedback> {
        loop {
            let result = self
                .feedback
                .as_ref()
                .context("render worker closed")?
                .recv()
                .context("render worker stopped")?
                .map_err(anyhow::Error::msg)?;
            if result.number >= number {
                return Ok(result);
            }
        }
    }
    pub fn close(&mut self) -> Result<()> {
        self.sender.take();
        self.feedback.take();
        if let Some(join) = self.join.take() {
            join.join()
                .map_err(|_| anyhow::anyhow!("render worker panicked"))?;
        }
        Ok(())
    }
}
impl Drop for RenderWorker {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

#[cfg(test)]
#[path = "../tests/unit/worker.rs"]
mod tests;
