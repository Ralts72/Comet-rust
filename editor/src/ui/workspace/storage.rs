use super::*;
use anyhow::{Context, Result, ensure};
use std::{collections::BTreeSet, fs, path::PathBuf};

const MAX_BYTES: usize = 64 * 1024;

#[derive(Default)]
pub(super) struct Storage {
    path: Option<PathBuf>,
    warning: Option<String>,
    writable: bool,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    version: u32,
    tree: Tree<Pane>,
}

impl Storage {
    pub(super) fn enable(&mut self, tree: &mut Tree<Pane>) {
        let result = (|| -> Result<()> {
            let path =
                crate::preferences::directory(std::env::consts::OS, |key| std::env::var_os(key))?
                    .join("workspace.json");
            self.load(path, tree)?;
            Ok(())
        })();
        if let Err(error) = result {
            self.warning = Some(format!("{error:#}"));
        }
    }
    fn load(&mut self, path: PathBuf, tree: &mut Tree<Pane>) -> Result<()> {
        self.path = Some(path.clone());
        self.writable = false;
        if let Some(saved) = read(&path)? {
            *tree = saved;
        }
        self.writable = true;
        Ok(())
    }
    pub(super) fn allow_save(&mut self) {
        self.writable = self.path.is_some();
    }
    pub(super) fn save(&mut self, tree: &Tree<Pane>) {
        if !self.writable {
            return;
        }
        let result = (|| -> Result<()> {
            let path = self.path.as_ref().context("layout storage disabled")?;
            let bytes = encode(tree)?;
            crate::preferences::check_file(path)?;
            comet_core::atomic_write(path, &bytes).context("save editor workspace")
        })();
        self.warning = result.err().map(|error| format!("{error:#}"));
    }
    pub(super) fn warning(&self, ui: &mut egui::Ui) {
        if let Some(warning) = &self.warning {
            ui.separator();
            ui.colored_label(egui::Color32::LIGHT_YELLOW, "布局偏好未恢复或保存");
            ui.weak("当前布局仍可使用；恢复默认布局后可重新保存。");
            ui.collapsing("布局诊断", |ui| {
                ui.set_max_width(420.);
                ui.label(warning);
            });
        }
    }
}

fn read(path: &std::path::Path) -> Result<Option<Tree<Pane>>> {
    crate::preferences::check_file(path)?;
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("read workspace"),
    };
    let bytes = comet_core::read_bounded(file, MAX_BYTES).context("workspace exceeds 64 KiB")?;
    let mut saved: File = serde_json::from_slice(&bytes).context("decode workspace")?;
    ensure!(saved.version == 1, "unsupported workspace version");
    validate(&saved.tree)?;
    saved.tree.tiles.recompute_next_tile_id();
    Ok(Some(Tree::new(
        egui::Id::unique("editor-workspace"),
        saved.tree.root.context("workspace root missing")?,
        saved.tree.tiles,
    )))
}
fn encode(tree: &Tree<Pane>) -> Result<Vec<u8>> {
    validate(tree)?;
    let bytes = serde_json::to_vec_pretty(&File {
        version: 1,
        tree: tree.clone(),
    })?;
    ensure!(
        bytes.len() <= MAX_BYTES,
        "workspace encoding exceeds 64 KiB"
    );
    Ok(bytes)
}
fn validate(tree: &Tree<Pane>) -> Result<()> {
    fn visit(
        tree: &Tree<Pane>,
        id: TileId,
        depth: usize,
        seen: &mut BTreeSet<u64>,
        panes: &mut BTreeSet<Pane>,
    ) -> Result<()> {
        ensure!(
            depth <= 16 && seen.len() < 32 && id.0 < 1_000_000 && seen.insert(id.0),
            "invalid, repeated or excessive workspace node"
        );
        match tree.tiles.get(id).context("dangling workspace node")? {
            Tile::Pane(pane) => ensure!(panes.insert(*pane), "duplicate workspace pane"),
            Tile::Container(container) => {
                match container {
                    Container::Grid(_) => anyhow::bail!("unsupported workspace grid"),
                    Container::Linear(row) => {
                        ensure!(
                            row.shares.iter().all(|(_, share)| share.is_finite()
                                && *share > 0.
                                && *share <= 1_000_000.),
                            "invalid workspace shares"
                        );
                    }
                    Container::Tabs(tabs) => ensure!(
                        tabs.active.is_none_or(|id| tabs.children.contains(&id)),
                        "invalid active workspace tab"
                    ),
                }
                for &child in container.children() {
                    visit(tree, child, depth + 1, seen, panes)?;
                }
            }
        }
        Ok(())
    }
    let mut seen = BTreeSet::new();
    let mut panes = BTreeSet::new();
    visit(
        tree,
        tree.root.context("workspace root missing")?,
        0,
        &mut seen,
        &mut panes,
    )?;
    ensure!(
        seen.len() == tree.tiles.iter().count() && panes == Pane::ALL.into(),
        "workspace contains missing or unreachable panes"
    );
    Ok(())
}

#[cfg(test)]
mod tests;
