//! Project candidates own CPU data only; the main thread accepts them as an editor session.
use super::ProjectPanel;
use crate::jobs::{Event, Job};
use anyhow::{Context, Result, ensure};
use comet_asset::AssetStore;
use comet_asset_data::product::Product;
use comet_asset_pipeline::{SourceCatalog, SourceMeta};
use comet_core::{AssetId, ProjectPaths, digest, resolve_destination};
use comet_scene::{ContentSource, SceneSnapshot};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

pub(crate) struct ProductLocations {
    initial_root: PathBuf,
    initial_assets: PathBuf,
    initial_products: PathBuf,
    projects: PathBuf,
}

impl ProductLocations {
    pub fn new(initial: &ProjectPaths, products: &Path) -> Result<Self> {
        let products = checked_products(initial, products)?;
        let projects = products
            .parent()
            .context("产品目录缺少父目录")?
            .join("projects");
        Ok(Self {
            initial_root: initial.root().to_owned(),
            initial_assets: initial.assets().to_owned(),
            initial_products: products,
            projects,
        })
    }

    pub fn for_project(&self, paths: &ProjectPaths) -> Result<PathBuf> {
        let products = self.for_root(paths.root())?;
        ensure_disjoint(&products, paths.assets())?;
        Ok(products)
    }

    pub fn for_root(&self, root: &Path) -> Result<PathBuf> {
        let root = resolve_destination(root)?;
        let products = if root == self.initial_root {
            self.initial_products.clone()
        } else {
            self.projects
                .join(digest(root.as_os_str().as_encoded_bytes()))
        };
        let products = resolve_destination(&products)?;
        ensure_disjoint(&products, &resolve_destination(&root.join("assets"))?)?;
        ensure_disjoint(&products, &self.initial_assets)?;
        Ok(products)
    }
}

fn ensure_disjoint(products: &Path, assets: &Path) -> Result<()> {
    ensure!(
        !products.starts_with(assets) && !assets.starts_with(products),
        "产品缓存目录不能与作者资源目录重叠：{}",
        products.display()
    );
    Ok(())
}

fn checked_products(paths: &ProjectPaths, products: &Path) -> Result<PathBuf> {
    let products = resolve_destination(products)?;
    ensure_disjoint(&products, paths.assets())?;
    Ok(products)
}

fn protect_assets(products: &Path, protected_assets: Option<&Path>) -> Result<()> {
    if let Some(assets) = protected_assets {
        ensure_disjoint(products, &resolve_destination(assets)?)?;
    }
    Ok(())
}

pub(crate) fn verify_prefab_identity(
    paths: &ProjectPaths,
    source: &Path,
    root: Option<AssetId>,
) -> Result<()> {
    if let Some(root) = root {
        let source = paths.resolve_asset_path(source)?;
        ensure!(
            source
                .extension()
                .is_some_and(|extension| extension == "prefab")
                && SourceMeta::load(&source)?.id == root,
            "Prefab 作者身份已变化，请重新打开文档"
        );
    }
    Ok(())
}

pub(crate) struct ProjectCandidate {
    pub paths: ProjectPaths,
    pub products: PathBuf,
    pub source: PathBuf,
    pub content: ContentSource,
    pub assets: AssetStore,
    pub sources: SourceCatalog,
    pub snapshot: Arc<SceneSnapshot>,
    pub source_asset: AssetId,
    pub panel: ProjectPanel,
    source_digest: String,
    protected_assets: Option<PathBuf>,
}

impl ProjectCandidate {
    pub fn load(root: PathBuf, products: PathBuf, source: PathBuf) -> Result<Self> {
        Self::prepare(root, products, source, None, |_| {})
    }

    pub fn prefab_asset(&self) -> Option<AssetId> {
        self.source
            .extension()
            .is_some_and(|extension| extension == "prefab")
            .then_some(self.source_asset)
    }

    /// A completed background candidate may wait before the UI accepts it.
    pub fn verify_current(&self) -> Result<()> {
        let current = ProjectPaths::open(self.paths.root())?;
        ensure!(
            current.assets() == self.paths.assets(),
            "项目资源目录在加载期间发生变化，请重新打开"
        );
        let products = checked_products(&current, &self.products)?;
        ensure!(
            products == self.products,
            "产品目录在加载期间发生变化，请重新打开"
        );
        protect_assets(&products, self.protected_assets.as_deref())?;
        let source = current.resolve_asset_path(&self.source)?;
        ensure!(
            source == self.source
                && digest(&std::fs::read(&source).context("复查作者文件失败")?)
                    == self.source_digest,
            "作者文件在加载期间发生变化，请重新打开项目"
        );
        ensure!(
            SourceMeta::load(&source)?.id == self.source_asset,
            "作者文件身份在加载期间发生变化，请重新打开项目"
        );
        ensure!(
            comet_asset_pipeline::ProjectSettings::load(&current)? == self.panel.settings,
            "项目设置在加载期间发生变化，请重新打开项目"
        );
        Ok(())
    }

    fn prepare(
        root: PathBuf,
        products: PathBuf,
        author_file: PathBuf,
        protected_assets: Option<PathBuf>,
        mut stage: impl FnMut(String),
    ) -> Result<Self> {
        stage("正在检查项目目录".into());
        let paths = ProjectPaths::open(root)?;
        let products = checked_products(&paths, &products)?;
        protect_assets(&products, protected_assets.as_deref())?;
        let source = paths.resolve_asset_path(&author_file)?;
        ensure!(
            source
                .extension()
                .is_some_and(|extension| extension == "scene" || extension == "prefab"),
            "作者文件必须使用 .scene 或 .prefab 后缀"
        );
        let panel = ProjectPanel::from_settings(
            comet_asset_pipeline::ProjectSettings::ensure_game_id(&paths)
                .context("初始化项目游戏身份失败")?,
        );
        let prefab = source
            .extension()
            .is_some_and(|extension| extension == "prefab");
        stage("正在构建项目资源".into());
        let report =
            comet_asset_pipeline::prepare(paths.root(), &products).context("项目资源构建失败")?;
        let current = ProjectPaths::open(paths.root())?;
        ensure!(
            current.assets() == paths.assets(),
            "项目资源目录在加载期间发生变化，请重新打开"
        );
        ensure!(
            checked_products(&paths, &products)? == products,
            "产品目录在加载期间发生变化，请重新打开"
        );
        protect_assets(&products, protected_assets.as_deref())?;
        ensure!(
            paths.resolve_asset_path(&author_file)? == source,
            "作者文件路径在加载期间发生变化，请重新打开项目"
        );
        stage("正在校验作者文件及依赖产品".into());
        let source_bytes =
            std::fs::read(&source).with_context(|| format!("读取作者文件 {}", source.display()))?;
        let source_digest = digest(&source_bytes);
        let content = ContentSource::from_json(&source_bytes)?;
        let source_asset = SourceMeta::load(&source)?.id;
        let mut assets = AssetStore::open(&products)?;
        let product = assets.load(source_asset)?;
        let built = match (prefab, &product.payload) {
            (false, Product::Scene(built)) | (true, Product::Prefab(built)) => built,
            _ => anyhow::bail!("作者文件对应产品类型不匹配，请重新打开项目"),
        };
        // Authoring readers canonicalize declarations; accepted products retain their original bytes.
        let mut built = built.clone();
        built.canonicalize();
        ensure!(
            built == content,
            "作者文件在资源构建后发生变化，请重新打开项目"
        );
        let snapshot = crate::resources::validate_sources(
            &[Arc::new(content.clone())],
            &mut assets,
            prefab.then_some(source_asset),
        )
        .context("项目文档校验失败")?;
        let sources = SourceCatalog::read(&paths, assets.index())?;
        stage(format!(
            "项目已准备：{} 个更新，{} 个复用",
            report.changed, report.reused
        ));
        let candidate = Self {
            paths,
            products,
            source,
            content,
            assets,
            sources,
            snapshot: Arc::new(snapshot),
            source_asset,
            panel,
            source_digest,
            protected_assets,
        };
        candidate.verify_current()?;
        Ok(candidate)
    }
}

#[derive(Default)]
pub(crate) struct ProjectLoad {
    job: Option<Job<ProjectCandidate>>,
    pub stage: String,
    pub failure: Option<String>,
    created: Arc<AtomicBool>,
}

impl ProjectLoad {
    pub fn is_running(&self) -> bool {
        self.job.is_some()
    }

    pub fn start(
        &mut self,
        root: PathBuf,
        products: PathBuf,
        source: PathBuf,
        protected_assets: Option<PathBuf>,
    ) -> Result<()> {
        self.launch(root, products, source, false, protected_assets)
    }

    pub fn create(
        &mut self,
        root: PathBuf,
        products: PathBuf,
        protected_assets: Option<PathBuf>,
    ) -> Result<()> {
        self.launch(
            root,
            products,
            comet_asset_pipeline::DEFAULT_SCENE.into(),
            true,
            protected_assets,
        )
    }

    pub fn created(&self) -> bool {
        self.created.load(Ordering::Acquire)
    }

    fn launch(
        &mut self,
        root: PathBuf,
        products: PathBuf,
        source: PathBuf,
        create: bool,
        protected_assets: Option<PathBuf>,
    ) -> Result<()> {
        ensure!(!self.is_running(), "项目加载已在进行");
        self.created = Arc::new(AtomicBool::new(false));
        let created = self.created.clone();
        let job = Job::spawn("comet-project-load", move |sender| {
            if create {
                let root = resolve_destination(&root)?;
                let products = resolve_destination(&products)?;
                ensure_disjoint(&products, &resolve_destination(&root.join("assets"))?)?;
                protect_assets(&products, protected_assets.as_deref())?;
                let _ = sender.send(Event::Stage("正在创建项目".into()));
                comet_asset_pipeline::create_project(&root)?;
                created.store(true, Ordering::Release);
            }
            ProjectCandidate::prepare(root, products, source, protected_assets, |stage| {
                let _ = sender.send(Event::Stage(stage));
            })
        })?;
        self.job = Some(job);
        self.stage = "正在打开项目".into();
        self.failure = None;
        Ok(())
    }

    pub fn poll(&mut self) -> Option<std::result::Result<ProjectCandidate, String>> {
        while let Some(job) = &self.job {
            match job.poll()? {
                Event::Stage(stage) => self.stage = stage,
                Event::Finished(result) => {
                    self.job.take();
                    if let Err(error) = &result {
                        self.failure = Some(error.clone());
                    }
                    return Some(result);
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests;
