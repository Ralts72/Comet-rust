//! CPU jobs own their worker and result; progress and candidate acceptance stay separate.
use anyhow::Result;
use std::{
    sync::mpsc::{self, Receiver, Sender},
    thread::{self, JoinHandle},
};

pub(crate) struct Job<T> {
    progress: Receiver<String>,
    worker: Option<JoinHandle<std::result::Result<T, String>>>,
}
impl<T: Send + 'static> Job<T> {
    pub fn spawn(
        name: &str,
        work: impl FnOnce(&Sender<String>) -> Result<T> + Send + 'static,
    ) -> Result<Self> {
        let (sender, progress) = mpsc::channel();
        let worker = thread::Builder::new()
            .name(name.into())
            .spawn(move || work(&sender).map_err(|error| format!("{error:#}")))?;
        Ok(Self {
            progress,
            worker: Some(worker),
        })
    }
    pub fn poll(
        &mut self,
        mut stage: impl FnMut(String),
    ) -> Option<std::result::Result<T, String>> {
        let finished = self.worker.as_ref()?.is_finished();
        let result = if finished {
            Some(
                self.worker
                    .take()
                    .unwrap()
                    .join()
                    .unwrap_or_else(|_| Err("后台任务异常退出，未返回结果".into())),
            )
        } else {
            None
        };
        // Join before draining a completed job, so its final progress precedes the result.
        for message in self.progress.try_iter() {
            stage(message);
        }
        result
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
    use anyhow::ensure;
    use std::time::{Duration, Instant};

    fn wait<T: Send + 'static>(
        job: &mut Job<T>,
        stages: &mut Vec<String>,
    ) -> Result<std::result::Result<T, String>> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(result) = job.poll(|stage| stages.push(stage)) {
                return Ok(result);
            }
            ensure!(Instant::now() < deadline, "job did not finish");
            thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn progress_remains_nonblocking_and_finishes_before_one_owned_result() -> Result<()> {
        let (ready, started) = mpsc::channel();
        let (release, resume) = mpsc::channel();
        let mut job = Job::spawn("job-progress-test", move |sender| {
            sender.send("preparing".into())?;
            ready.send(())?;
            resume.recv()?;
            sender.send("prepared".into())?;
            Ok(Box::new(42))
        })?;
        let ready = started.recv_timeout(Duration::from_secs(5));
        let mut stages = vec![];
        let pending = job.poll(|stage| stages.push(stage));
        let initial = stages.clone();
        // Always release the worker before assertions: Job::drop intentionally joins it.
        let released = release.send(());
        ready?;
        released?;
        assert!(pending.is_none());
        assert_eq!(initial, ["preparing"]);
        assert_eq!(
            *wait(&mut job, &mut stages)?.map_err(anyhow::Error::msg)?,
            42
        );
        assert_eq!(stages, ["preparing", "prepared"]);
        assert!(
            job.poll(|_| panic!("retired job reported progress"))
                .is_none()
        );
        Ok(())
    }

    #[test]
    fn errors_and_panics_finish_instead_of_leaving_a_job_running() -> Result<()> {
        for panic in [false, true] {
            let mut job = Job::<()>::spawn("job-failure-test", move |sender| {
                sender.send("building".into())?;
                if panic {
                    panic!("worker failed");
                }
                anyhow::bail!("input failed")
            })?;
            let mut stages = vec![];
            let error = wait(&mut job, &mut stages)?.unwrap_err();
            assert!(error.contains(if panic {
                "后台任务异常退出"
            } else {
                "input failed"
            }));
            assert_eq!(stages, ["building"]);
            assert!(
                job.poll(|_| panic!("failure was delivered twice"))
                    .is_none()
            );
        }
        Ok(())
    }

    #[test]
    fn dropping_an_unconsumed_result_reclaims_it_on_the_owner_thread() -> Result<()> {
        struct Candidate(Sender<thread::ThreadId>);
        impl Drop for Candidate {
            fn drop(&mut self) {
                let _ = self.0.send(thread::current().id());
            }
        }
        let (dropped, receiver) = mpsc::channel();
        let job = Job::spawn("job-owner-test", move |_| Ok(Candidate(dropped)))?;
        let deadline = Instant::now() + Duration::from_secs(5);
        while !job.worker.as_ref().unwrap().is_finished() {
            ensure!(Instant::now() < deadline, "worker did not finish");
            thread::sleep(Duration::from_millis(2));
        }
        assert!(matches!(
            receiver.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        drop(job);
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(5))?,
            thread::current().id()
        );
        Ok(())
    }
}
