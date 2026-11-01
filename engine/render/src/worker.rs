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
mod tests {
    use super::*;
    use crate::{Camera, UiFrame, ViewToken};
    use comet_core::WorldId;
    use std::{
        sync::{
            Arc, Condvar, Mutex,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };
    fn packet(number: u64) -> FramePacket {
        FramePacket {
            number,
            token: ViewToken {
                world: WorldId::new(),
                view: 1,
                target_generation: 1,
            },
            size: [64, 64],
            viewport: [0, 0, 64, 64],
            camera: Camera::default(),
            draws: vec![],
            ui: UiFrame::default(),
            pick: None,
            capture: None,
            shadows: true,
        }
    }
    struct Mock {
        gate: Arc<(Mutex<bool>, Condvar)>,
        started: std::sync::mpsc::Sender<()>,
        drops: Arc<AtomicUsize>,
        fail: bool,
    }
    impl FrameRenderer for Mock {
        fn name(&self) -> &'static str {
            "mock"
        }
        fn render(&mut self, frame: &FramePacket) -> Result<(bool, f64)> {
            assert_eq!(thread::current().name(), Some("comet-render"));
            if frame.number == 0 {
                let _ = self.started.send(());
                let (lock, cv) = &*self.gate;
                let mut open = lock.lock().unwrap();
                while !*open {
                    open = cv.wait(open).unwrap();
                }
            }
            if self.fail && frame.number == 1 {
                anyhow::bail!("simulated GPU failure");
            }
            Ok((true, 0.))
        }
    }
    impl Drop for Mock {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::SeqCst);
        }
    }
    #[test]
    fn one_pending_frame_applies_backpressure_and_closes_on_the_worker() -> Result<()> {
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let drops = Arc::new(AtomicUsize::new(0));
        let (started, active) = mpsc::channel();
        let g = gate.clone();
        let d = drops.clone();
        let mut worker = RenderWorker::spawn_with(move || {
            Ok(Mock {
                gate: g,
                started,
                drops: d,
                fail: false,
            })
        })?;
        worker.submit(packet(0))?;
        active.recv_timeout(Duration::from_secs(2))?;
        worker.submit(packet(1))?;
        let sender = worker.sender.as_ref().unwrap().clone();
        let (attempted, attempt) = mpsc::channel();
        let (completed, completion) = mpsc::channel();
        let submitter = thread::spawn(move || {
            attempted.send(()).unwrap();
            sender.send(packet(2)).unwrap();
            completed.send(()).unwrap();
        });
        attempt.recv_timeout(Duration::from_secs(2))?;
        assert!(completion.try_recv().is_err());
        *gate.0.lock().unwrap() = true;
        gate.1.notify_all();
        completion.recv_timeout(Duration::from_secs(2))?;
        submitter.join().unwrap();
        assert_eq!(worker.wait(2)?.number, 2);
        worker.close()?;
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        Ok(())
    }
    #[test]
    fn worker_failure_is_reported_and_queued_texture_deltas_can_retire() -> Result<()> {
        let gate = Arc::new((Mutex::new(true), Condvar::new()));
        let drops = Arc::new(AtomicUsize::new(0));
        let (started, _) = mpsc::channel();
        let d = drops.clone();
        let mut worker = RenderWorker::spawn_with(move || {
            Ok(Mock {
                gate,
                started,
                drops: d,
                fail: true,
            })
        })?;
        worker.submit(packet(0))?;
        worker.submit(packet(1))?;
        assert!(
            worker
                .wait(1)
                .unwrap_err()
                .to_string()
                .contains("simulated GPU failure")
        );
        worker.close()?;
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        Ok(())
    }
    #[test]
    fn closing_with_full_feedback_and_pending_frames_does_not_deadlock() -> Result<()> {
        let gate = Arc::new((Mutex::new(true), Condvar::new()));
        let drops = Arc::new(AtomicUsize::new(0));
        let (started, _) = mpsc::channel();
        let mut worker = RenderWorker::spawn_with(move || {
            Ok(Mock {
                gate,
                started,
                drops,
                fail: false,
            })
        })?;
        for n in 0..4 {
            worker.submit(packet(n))?;
        }
        worker.close()?;
        Ok(())
    }
}
