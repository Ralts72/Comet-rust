//! Effective output preferences; the private file envelope supplies version and game identity.
use anyhow::{Context, Result, ensure};
use comet_core::GameId;
use serde::{Deserialize, Serialize};

pub(super) const MAX_SETTINGS_BYTES: usize = 4 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub(super) struct Settings {
    pub master_volume: f32,
    pub shadows: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            master_volume: 1.,
            shadows: true,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SettingsFile {
    version: u32,
    game_id: GameId,
    master_volume: f32,
    shadows: bool,
}

impl Settings {
    pub(super) fn validate(&self) -> Result<()> {
        ensure!(
            self.master_volume.is_finite() && (0. ..=1.).contains(&self.master_volume),
            "主音量必须是 0 到 1 的有限数值"
        );
        Ok(())
    }

    pub(super) fn from_bytes(bytes: &[u8], game_id: GameId) -> Result<Self> {
        ensure!(bytes.len() <= MAX_SETTINGS_BYTES, "游戏设置超过 4 KiB");
        let file: SettingsFile = serde_json::from_slice(bytes).context("解析游戏设置失败")?;
        ensure!(file.version == 1, "不支持的游戏设置版本");
        ensure!(file.game_id == game_id, "设置属于其他游戏");
        let settings = Self {
            master_volume: file.master_volume,
            shadows: file.shadows,
        };
        settings.validate()?;
        Ok(settings)
    }

    pub(super) fn to_bytes(self, game_id: GameId) -> Result<Vec<u8>> {
        self.validate()?;
        let bytes = serde_json::to_vec(&SettingsFile {
            version: 1,
            game_id,
            master_volume: self.master_volume,
            shadows: self.shadows,
        })
        .context("编码游戏设置失败")?;
        ensure!(bytes.len() <= MAX_SETTINGS_BYTES, "游戏设置超过 4 KiB");
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests;
