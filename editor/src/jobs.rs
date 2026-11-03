//! CPU jobs report progress and return owned candidates; UI state stays on the main thread.
use anyhow::Result;
use std::{
    sync::mpsc::{self, Receiver, Sender},
    thread::{self, JoinHandle},
};

pub(crate) enum Event<T> {
    Stage(String),
    Finished(std::result::Result<T, String>),
}
pub(crate) struct Job<T> {
    receiver: Receiver<Event<T>>,
    worker: Option<JoinHandle<()>>,
}
impl<T: Send + 'static> Job<T> {
    pub fn spawn(
        name: &str,
        work: impl FnOnce(&Sender<Event<T>>) -> Result<T> + Send + 'static,
    ) -> Result<Self> {
        let (sender, receiver) = mpsc::channel();
        let worker = thread::Builder::new().name(name.into()).spawn(move || {
            let result = work(&sender).map_err(|error| format!("{error:#}"));
            let _ = sender.send(Event::Finished(result));
        })?;
        Ok(Self {
            receiver,
            worker: Some(worker),
        })
    }
    pub fn poll(&self) -> Option<Event<T>> {
        match self.receiver.try_recv() {
            Ok(event) => Some(event),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Event::Finished(Err("后台任务异常退出，未返回结果".into())))
            }
        }
    }
}
impl<T> Drop for Job<T> {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn errors_and_panics_finish_instead_of_leaving_a_job_running() -> Result<()> {
        for panic in [false, true] {
            let job = Job::<()>::spawn("job-failure-test", move |_| {
                if panic {
                    panic!("worker failed");
                }
                anyhow::bail!("input failed")
            })?;
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                if let Some(Event::Finished(result)) = job.poll() {
                    assert!(result.is_err());
                    break;
                }
                assert!(Instant::now() < deadline, "job did not finish");
                thread::sleep(Duration::from_millis(2));
            }
        }
        Ok(())
    }
}
