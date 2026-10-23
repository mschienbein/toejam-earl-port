use crate::input::PlayerBindings;
use anyhow::{Context, Result, ensure};
use clap::ValueEnum;
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};
use winit::keyboard::KeyCode;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, ValueEnum, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Scale {
    #[default]
    Integer,
    Fit,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, ValueEnum, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Filter {
    #[default]
    Nearest,
    Linear,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Hotkeys {
    pub pause: KeyCode,
    pub reset: KeyCode,
    pub fullscreen: KeyCode,
    pub scale: KeyCode,
    pub filter: KeyCode,
    pub vsync: KeyCode,
    pub volume_up: KeyCode,
    pub volume_down: KeyCode,
}

impl Default for Hotkeys {
    fn default() -> Self {
        Self {
            pause: KeyCode::Escape,
            reset: KeyCode::F5,
            fullscreen: KeyCode::F11,
            scale: KeyCode::F6,
            filter: KeyCode::F7,
            vsync: KeyCode::F8,
            volume_up: KeyCode::PageUp,
            volume_down: KeyCode::PageDown,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub window_scale: u32,
    pub scale: Scale,
    pub filter: Filter,
    pub fullscreen: bool,
    pub vsync: bool,
    pub volume: f32,
    pub players: [PlayerBindings; 2],
    pub hotkeys: Hotkeys,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            window_scale: 3,
            scale: Scale::Integer,
            filter: Filter::Nearest,
            fullscreen: false,
            vsync: true,
            volume: 0.8,
            players: [PlayerBindings::one(), PlayerBindings::two()],
            hotkeys: Hotkeys::default(),
        }
    }
}

impl Settings {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            (1..=8).contains(&self.window_scale),
            "window_scale must be 1..8"
        );
        ensure!(
            self.volume.is_finite() && (0.0..=1.0).contains(&self.volume),
            "volume must be 0..1"
        );
        for player in &self.players {
            ensure!(
                player.deadzone.is_finite() && (0.0..1.0).contains(&player.deadzone),
                "gamepad deadzone must be 0 <= value < 1"
            );
        }
        Ok(())
    }
}

pub struct Storage {
    pub directory: PathBuf,
}

impl Storage {
    pub fn new(override_path: Option<PathBuf>) -> Result<Self> {
        let directory = match override_path {
            Some(path) => path,
            None => ProjectDirs::from("org", "toejam-earl-port", "ToeJam & Earl")
                .context("no user data directory; specify --data-dir")?
                .data_local_dir()
                .to_path_buf(),
        };
        fs::create_dir_all(&directory)
            .with_context(|| format!("create {}", directory.display()))?;
        Ok(Self { directory })
    }

    pub fn rom_path(&self) -> PathBuf {
        self.directory.join("cartridge.bin")
    }

    pub fn load(&self) -> Result<Settings> {
        let path = self.directory.join("config.toml");
        let settings = match fs::read_to_string(&path) {
            Ok(contents) => {
                toml::from_str(&contents).with_context(|| format!("read {}", path.display()))?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Settings::default(),
            Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
        };
        settings.validate()?;
        Ok(settings)
    }

    pub fn save(&self, settings: &Settings) -> Result<()> {
        settings.validate()?;
        fs::write(
            self.directory.join("config.toml"),
            toml::to_string_pretty(settings)?,
        )
        .context("save config.toml")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_roundtrip_and_reject_invalid_settings() {
        let defaults = Settings::default();
        let mut restored: Settings = toml::from_str(&toml::to_string(&defaults).unwrap()).unwrap();
        restored.validate().unwrap();
        assert_eq!(restored.players[0].bindings[0].key, Some(KeyCode::ArrowUp));
        restored.volume = f32::NAN;
        assert!(restored.validate().is_err());
        assert!(toml::from_str::<Settings>("typo = true").is_err());
    }

    #[test]
    fn settings_and_bindings_persist_in_an_isolated_data_directory() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("tje-native-{}-{nonce}", std::process::id()));
        let storage = Storage::new(Some(path.clone())).unwrap();
        let mut settings = storage.load().unwrap();
        settings.window_scale = 4;
        settings.scale = Scale::Fit;
        settings.filter = Filter::Linear;
        settings.fullscreen = true;
        settings.vsync = false;
        settings.volume = 0.25;
        settings.players[0].bindings[0].key = Some(KeyCode::KeyI);
        storage.save(&settings).unwrap();
        let restored = storage.load().unwrap();
        assert_eq!(restored.window_scale, 4);
        assert_eq!(restored.scale, Scale::Fit);
        assert_eq!(restored.filter, Filter::Linear);
        assert!(restored.fullscreen);
        assert!(!restored.vsync);
        assert_eq!(restored.volume, 0.25);
        assert_eq!(restored.players[0].bindings[0].key, Some(KeyCode::KeyI));
        std::fs::remove_dir_all(path).unwrap();
    }
}
