use super::*;
use crate::{Camera, PickRequest, UiFrame, ViewToken};
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
        draws: Arc::from([]),
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
        let mut frame = packet(2);
        frame.pick = Some(PickRequest::Rect {
            min: [0.; 2],
            max: [1.; 2],
        });
        sender.send(frame).unwrap();
        completed.send(()).unwrap();
    });
    attempt.recv_timeout(Duration::from_secs(2))?;
    assert!(completion.try_recv().is_err());
    *gate.0.lock().unwrap() = true;
    gate.1.notify_all();
    completion.recv_timeout(Duration::from_secs(2))?;
    submitter.join().unwrap();
    let feedback = worker.wait(2)?;
    assert_eq!(feedback.number, 2);
    let pick = feedback.pick.unwrap();
    assert_eq!(pick.token, feedback.token);
    assert!(pick.entities.is_empty());
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
