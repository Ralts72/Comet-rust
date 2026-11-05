use super::*;
use comet_application::CheckpointAction;
use std::sync::Arc;

impl Editor {
    pub(super) fn preview_checkpoint_action(
        &mut self,
        action: CheckpointAction,
        assets: &mut AssetStore,
    ) -> Result<()> {
        ensure!(
            !self.is_busy() && !self.documents.is_blocking(),
            "请先完成当前编辑器操作"
        );
        let play = self.play.as_ref().context("请先运行场景")?;
        let scene = play
            .scene_asset()
            .context("Prefab 预览不支持检查点，请运行场景")?;
        let game_id = self
            .project
            .game_id()
            .context("项目缺少游戏身份，请重新打开项目")?;
        match action {
            CheckpointAction::Save => {
                let checkpoint = comet_core::Checkpoint::new(
                    game_id,
                    scene,
                    (*play.progress_snapshot()).clone(),
                )?;
                self.preview_checkpoint = Some(checkpoint);
                self.status = "预览检查点已保存；停止运行后清空".into();
            }
            CheckpointAction::Continue => {
                let checkpoint = self
                    .preview_checkpoint
                    .as_ref()
                    .context("尚无预览检查点，请先按 F5 保存")?;
                checkpoint.validate_for(game_id)?;
                let target = checkpoint.scene;
                ensure!(
                    assets.index().products.get(&target).is_some_and(
                        |entry| entry.kind == comet_asset_data::product::AssetKind::Scene
                    ),
                    "检查点关卡已不在当前项目中"
                );
                let snapshot = assets.expand_scene(target)?;
                let candidate = super::play::prepare_play(
                    snapshot,
                    assets,
                    Some(target),
                    Arc::new(checkpoint.progress.clone()),
                )?;
                if let Some(error) = candidate
                    .script_diagnostics()
                    .iter()
                    .find(|entry| entry.failed)
                {
                    anyhow::bail!(
                        "检查点场景初始化失败（对象 {}，脚本 {}）：{}",
                        error.entity,
                        error.asset,
                        error.message
                    );
                }
                self.play.as_mut().unwrap().accept_candidate(candidate)?;
                self.collect_script_diagnostics();
                self.selection.clear();
                self.picking.clear();
                self.drag_origin = None;
                self.status = "已继续预览检查点，从保存关卡入口开始；作者文档保持不变".into();
            }
        }
        Ok(())
    }
}
