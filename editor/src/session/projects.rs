use super::{Editor, SnapshotCache};
use crate::{documents::DocumentAction, project::ProjectCandidate};
use anyhow::{Result, ensure};
use comet_asset::AssetStore;
use comet_core::WorldId;
use comet_editor_core::Document;
use std::path::PathBuf;

pub(super) struct PendingProject {
    action: DocumentAction,
    world: WorldId,
    document_revision: u64,
    origin: PathBuf,
    target: PathBuf,
}

impl Editor {
    pub(crate) fn enable_recents(&mut self) {
        match crate::project::RecentProjects::user() {
            Ok(recent) => {
                self.recent_projects = recent;
                self.record_current_project();
            }
            Err(error) => self.recent_preference_error(error),
        }
    }

    pub(crate) fn record_current_project(&mut self) {
        if let Err(error) = self.recent_projects.record(&self.paths, &self.source) {
            self.recent_preference_error(error);
        }
    }

    pub(crate) fn remove_recent_project(&mut self, root: &std::path::Path) {
        if let Err(error) = self.recent_projects.remove(root) {
            self.recent_preference_error(error);
        }
    }

    fn recent_preference_error(&mut self, error: anyhow::Error) {
        let mut message = format!("最近项目记录不可用：{error:#}");
        let mut end = message.len().min(4096);
        while !message.is_char_boundary(end) {
            end -= 1;
        }
        message.truncate(end);
        self.recent_warning = Some(message);
    }

    pub(crate) fn project_root(&self) -> &std::path::Path {
        self.paths.root()
    }
    pub(crate) fn begin_project_transition(&mut self, action: DocumentAction) -> Result<()> {
        ensure!(!self.is_busy(), "请等待后台任务完成，再切换项目");
        let (root, scene, create) = match &action {
            DocumentAction::OpenProject { root, scene } => (root, scene.clone(), false),
            DocumentAction::NewProject(root) => {
                (root, comet_asset_pipeline::DEFAULT_SCENE.into(), true)
            }
            _ => anyhow::bail!("不是项目切换操作"),
        };
        let root = if create {
            self.paths.export_destination(root, &self.products)?
        } else {
            comet_core::resolve_destination(root)?
        };
        let products = self.product_locations.for_root(&root)?;
        self.check_project_cache(&products)?;
        let pending = PendingProject {
            action,
            world: self.document.world_id(),
            document_revision: self.document.revision(),
            origin: self.paths.root().to_owned(),
            target: root.clone(),
        };
        if create {
            self.project_load
                .create(root, products, Some(self.paths.assets().to_owned()))?;
        } else {
            self.project_load
                .start(root, products, scene, Some(self.paths.assets().to_owned()))?;
        }
        self.project_request = Some(pending);
        self.picking.clear();
        Ok(())
    }

    pub(crate) fn poll_project_transition(&mut self, assets: &mut AssetStore) {
        let Some(result) = self.project_load.poll() else {
            if self.project_load.is_running() {
                self.status = self.project_load.stage.clone();
            }
            return;
        };
        let Some(request) = self.project_request.take() else {
            return;
        };
        // Cancel or a later close request supersedes this load, even if its source was just published.
        if !self.documents.matches_action(&request.action) {
            self.status = "项目切换已取消，当前项目保持不变".into();
            return;
        }
        if self.project_load.created() {
            self.request_document_action(DocumentAction::OpenProject {
                root: request.target,
                scene: comet_asset_pipeline::DEFAULT_SCENE.into(),
            });
        }
        let result = result.map_err(anyhow::Error::msg).and_then(|candidate| {
            ensure!(
                self.document.world_id() == request.world
                    && self.document.revision() == request.document_revision
                    && self.paths.root() == request.origin,
                "原项目文档在加载期间变化，请重新打开项目"
            );
            self.accept_project_candidate(candidate, assets)
        });
        match result {
            Ok(()) => self.documents = Default::default(),
            Err(error) => {
                let error = format!("{error:#}");
                self.project_load.stage = "项目加载失败".into();
                self.project_load.failure = Some(error.clone());
                self.status = "项目加载失败，当前项目保持不变".into();
                self.documents.fail(error);
            }
        }
    }

    pub(super) fn accept_project_candidate(
        &mut self,
        mut candidate: ProjectCandidate,
        assets: &mut AssetStore,
    ) -> Result<()> {
        candidate.verify_current()?;
        self.check_project_cache(&candidate.products)?;
        let preferred_scene = candidate
            .prefab_asset()
            .is_none()
            .then_some(candidate.source_asset);
        let document = match candidate.prefab_asset() {
            Some(root) => Document::open_prefab(root, candidate.content, &mut candidate.assets)?,
            None => Document::open_source(candidate.content)?,
        };
        let current = comet_core::ProjectPaths::open(candidate.paths.root())?;
        ensure!(
            current.assets() == candidate.paths.assets(),
            "项目资源边界在接纳前发生变化，请重新打开"
        );
        ensure!(
            self.product_locations.for_project(&current)? == candidate.products,
            "项目产品缓存位置在加载期间发生变化，请重新打开"
        );
        let status = format!(
            "已打开项目：{} · {}",
            candidate.paths.root().display(),
            candidate
                .source
                .strip_prefix(candidate.paths.assets())?
                .display()
        );
        let changed = self.paths.root() != candidate.paths.root();
        if changed {
            self.export.reset_project(preferred_scene)?;
        } else if preferred_scene.is_some() {
            self.export.select_scene(preferred_scene);
        }
        self.export.retain_scenes(&candidate.sources);
        self.stop();
        if changed {
            self.console.reset();
        }
        self.picking.clear();
        self.paths = candidate.paths;
        self.products = candidate.products;
        self.source = candidate.source;
        self.project = candidate.panel;
        self.sources = candidate.sources;
        self.source_revision = candidate.assets.revision().to_owned();
        self.resource_selection = None;
        self.resource_build = Default::default();
        self.navigation = Default::default();
        self.snapshot_cache = Some(SnapshotCache {
            world: document.world_id(),
            document_revision: document.revision(),
            products_revision: self.source_revision.clone(),
            snapshot: candidate.snapshot.clone(),
        });
        self.document = document;
        self.last_snapshot = candidate.snapshot;
        self.camera = Default::default();
        self.error = None;
        self.status = status;
        *assets = candidate.assets;
        // Preferences cannot roll back an already accepted project or report it as a load failure.
        self.record_current_project();
        Ok(())
    }

    fn check_project_cache(&self, products: &std::path::Path) -> Result<()> {
        let products = comet_core::resolve_destination(products)?;
        let mut reserved = vec![self.paths.assets().to_owned()];
        match self.paths.root().join("assets").canonicalize() {
            Ok(current) => reserved.push(current),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        for assets in reserved {
            ensure!(
                !products.starts_with(&assets) && !assets.starts_with(&products),
                "新项目产品缓存不能与当前项目作者资源重叠"
            );
        }
        Ok(())
    }
}
