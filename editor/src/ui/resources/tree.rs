use comet_asset_data::product::{AssetKind, ProductIndex};
use comet_asset_pipeline::{SourceAsset, SourceCatalog};
use comet_core::AssetId;
use std::{collections::BTreeMap, ffi::OsString, path::PathBuf};

#[derive(Clone)]
pub(super) struct Entry {
    pub id: AssetId,
    pub kind: AssetKind,
    pub label: String,
    pub source: SourceAsset,
    pub derived: bool,
    pub children: Vec<Entry>,
    search: String,
}

#[derive(Clone, Default)]
pub(super) struct Directory {
    pub path: PathBuf,
    pub directories: BTreeMap<OsString, Directory>,
    pub entries: Vec<Entry>,
}
impl Directory {
    pub fn build(sources: &SourceCatalog, index: &ProductIndex) -> Self {
        let mut derived = BTreeMap::<AssetId, Vec<Entry>>::new();
        let sources_by_id: BTreeMap<_, _> = sources.iter().map(|s| (s.id, s)).collect();
        for (&id, product) in &index.products {
            if let Some(origin) = &product.editor
                && origin.source != id
                && let Some(source) = sources_by_id.get(&origin.source)
            {
                derived.entry(origin.source).or_default().push(Entry {
                    id,
                    kind: product.kind,
                    label: origin.label.clone(),
                    source: (*source).clone(),
                    derived: true,
                    children: vec![],
                    search: origin.label.to_lowercase(),
                });
            }
        }
        let mut root = Self::default();
        for source in sources.iter() {
            let mut children = derived.remove(&source.id).unwrap_or_default();
            children.sort_by(|a, b| a.label.cmp(&b.label).then(a.id.cmp(&b.id)));
            let entry = Entry {
                id: source.id,
                kind: source.kind,
                label: source
                    .source
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into(),
                source: source.clone(),
                derived: false,
                children,
                search: source.source.to_string_lossy().to_lowercase(),
            };
            let mut directory = &mut root;
            for part in source
                .source
                .parent()
                .into_iter()
                .flat_map(|path| path.components())
            {
                let path = directory.path.join(part);
                directory = directory
                    .directories
                    .entry(part.as_os_str().into())
                    .or_insert_with(|| Self {
                        path,
                        ..Default::default()
                    });
            }
            directory.entries.push(entry);
        }
        root.sort();
        root
    }

    fn sort(&mut self) {
        // Keep frequently placed resources visible in a short project pane.
        self.entries.sort_by_key(|entry| {
            (
                match entry.kind {
                    AssetKind::Prefab => 0,
                    AssetKind::Data => 1,
                    _ => 2,
                },
                entry.source.source.clone(),
            )
        });
        for directory in self.directories.values_mut() {
            directory.sort();
        }
    }

    pub fn filter(&self, query: &str, kind: Option<AssetKind>) -> Self {
        let query = query.trim().to_lowercase();
        self.filtered(&query, kind)
    }

    fn filtered(&self, query: &str, kind: Option<AssetKind>) -> Self {
        let entries = self
            .entries
            .iter()
            .filter_map(|entry| {
                let source_matches = entry.search.contains(query);
                let mut found = entry.clone();
                found.children.retain(|child| {
                    (source_matches || child.search.contains(query))
                        && kind.is_none_or(|kind| kind == child.kind)
                });
                (source_matches && kind.is_none_or(|kind| kind == entry.kind)
                    || !found.children.is_empty())
                .then_some(found)
            })
            .collect();
        let directories = self
            .directories
            .iter()
            .filter_map(|(name, directory)| {
                let filtered = directory.filtered(query, kind);
                (!filtered.is_empty()).then(|| (name.clone(), filtered))
            })
            .collect();
        Self {
            path: self.path.clone(),
            directories,
            entries,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.directories.is_empty()
    }
}

pub(super) fn kind_label(kind: AssetKind) -> &'static str {
    match kind {
        AssetKind::Scene => "场景",
        AssetKind::Prefab => "Prefab",
        AssetKind::Mesh => "网格",
        AssetKind::Material => "材质",
        AssetKind::Texture => "纹理",
        AssetKind::Script => "脚本",
        AssetKind::Data => "数据",
        AssetKind::Navigation => "导航",
        AssetKind::Audio => "音频",
        AssetKind::Skeleton => "骨架",
        AssetKind::Skin => "蒙皮",
        AssetKind::AnimationClip => "动画片段",
        AssetKind::AnimationController => "动画控制器",
    }
}
