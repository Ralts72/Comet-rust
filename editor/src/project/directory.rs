//! One bounded directory read runs at a time; closing a picker never joins filesystem work.
use crate::jobs::{Event, Job};
use anyhow::{Context, Result, ensure};
use comet_core::{ProjectPaths, resolve_destination};
use std::{ffi::OsString, fs, path::PathBuf, sync::Arc};

const MAX_ENTRIES: usize = 4096;

#[derive(Clone)]
pub(super) enum Scope {
    Directories,
    Authors { project: PathBuf },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct RequestId(u64);
pub(super) struct Entry {
    pub name: OsString,
    pub path: PathBuf,
    pub directory: bool,
}
pub(super) struct Listing {
    pub directory: PathBuf,
    pub assets: Option<PathBuf>,
    pub entries: Vec<Entry>,
    pub roots: Vec<PathBuf>,
    pub truncated: bool,
}
struct Request {
    id: RequestId,
    scope: Scope,
    directory: Option<PathBuf>,
}
struct Running {
    id: RequestId,
    job: Job<Listing>,
}
#[derive(Default)]
pub(crate) struct DirectoryReader {
    next_id: u64,
    active: Option<RequestId>,
    running: Option<Running>,
    queued: Option<Request>,
    result: Option<Result<Arc<Listing>, String>>,
}
impl DirectoryReader {
    pub(super) fn request(
        &mut self,
        scope: Scope,
        directory: Option<PathBuf>,
    ) -> Result<RequestId> {
        self.next_id = self.next_id.checked_add(1).context("目录请求编号已耗尽")?;
        let id = RequestId(self.next_id);
        self.active = Some(id);
        self.result = None;
        self.queued = Some(Request {
            id,
            scope,
            directory,
        });
        self.start_queued();
        Ok(id)
    }
    pub(super) fn cancel(&mut self, id: RequestId) {
        if self.active == Some(id) {
            self.active = None;
            self.queued = None;
            self.result = None;
        }
    }
    pub(super) fn listing(&self, id: RequestId) -> Option<&Result<Arc<Listing>, String>> {
        (self.active == Some(id))
            .then_some(self.result.as_ref())
            .flatten()
    }
    pub fn poll(&mut self) {
        if let Some(running) = &self.running
            && let Some(Event::Finished(result)) = running.job.poll()
        {
            let id = running.id;
            // Finished workers have no remaining filesystem operation when Job::drop joins them.
            self.running = None;
            if self.active == Some(id) {
                self.result = Some(result.map(Arc::new));
            }
        }
        self.start_queued();
    }
    fn start_queued(&mut self) {
        if self.running.is_some() {
            return;
        }
        let Some(request) = self.queued.take() else {
            return;
        };
        let id = request.id;
        match Job::spawn("comet-directory-browser", move |_| {
            scan(request.scope, request.directory, MAX_ENTRIES)
        }) {
            Ok(job) => self.running = Some(Running { id, job }),
            Err(error) => self.result = Some(Err(format!("无法读取目录：{error:#}"))),
        }
    }
}

fn scan(scope: Scope, directory: Option<PathBuf>, limit: usize) -> Result<Listing> {
    let paths = match &scope {
        Scope::Directories => None,
        Scope::Authors { project } => {
            Some(ProjectPaths::open(project).context("无法打开项目资源目录")?)
        }
    };
    let directory = if let Some(paths) = &paths {
        let requested = directory.unwrap_or_else(|| paths.assets().to_owned());
        let requested = if requested.is_absolute() {
            requested
        } else {
            paths.assets().join(requested)
        };
        let resolved = resolve_destination(&requested)?;
        if resolved != paths.assets() {
            paths.resolve_asset_path(&requested)?;
        }
        resolved
    } else {
        directory
            .context("请选择目录")?
            .canonicalize()
            .context("无法定位目录")?
    };
    ensure!(directory.is_dir(), "所选路径不是目录");
    let mut entries = Vec::new();
    let mut truncated = false;
    for (number, entry) in fs::read_dir(&directory)
        .with_context(|| format!("无法读取目录 {}", directory.display()))?
        .enumerate()
    {
        if number >= limit {
            truncated = true;
            break;
        }
        let entry = entry.context("无法读取目录条目")?;
        let path = if let Some(paths) = &paths {
            match paths.resolve_asset_path(entry.path()) {
                Ok(path) => path,
                Err(_) => continue,
            }
        } else {
            match entry.path().canonicalize() {
                Ok(path) => path,
                Err(_) => continue,
            }
        };
        let metadata = match fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        let is_directory = metadata.is_dir();
        if !is_directory
            && (paths.is_none()
                || !metadata.is_file()
                || !path
                    .extension()
                    .is_some_and(|extension| extension == "scene" || extension == "prefab"))
        {
            continue;
        }
        entries.push(Entry {
            name: entry.file_name(),
            path,
            directory: is_directory,
        });
    }
    entries.sort_by(|a, b| {
        b.directory
            .cmp(&a.directory)
            .then(a.name.cmp(&b.name))
            .then(a.path.cmp(&b.path))
    });
    let roots = if paths.is_none() { roots() } else { Vec::new() };
    Ok(Listing {
        directory,
        assets: paths.map(|paths| paths.assets().to_owned()),
        entries,
        roots,
        truncated,
    })
}

fn roots() -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        (b'A'..=b'Z')
            .map(|letter| PathBuf::from(format!("{}:\\", char::from(letter))))
            .filter(|path| path.is_dir())
            .collect()
    }
    #[cfg(not(windows))]
    {
        vec![PathBuf::from("/")]
    }
}

#[cfg(test)]
mod tests;
