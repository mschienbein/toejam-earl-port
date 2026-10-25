use crate::input::{GameButton, PadButton, PlayerBindings};
use anyhow::{Context, Result, bail, ensure};
use clap::ValueEnum;
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};
pub use tje_display::DisplayShader;
use winit::keyboard::KeyCode;

const SETTINGS_VERSION: u32 = 3;
fn legacy_version() -> u32 {
    1
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, ValueEnum, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PixelShape {
    #[default]
    Native,
    Square,
}

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
    Sharp,
    Nearest,
    Linear,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Hotkeys {
    pub settings: KeyCode,
    pub pause: KeyCode,
    pub reset: KeyCode,
    pub record: KeyCode,
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
            settings: KeyCode::F1,
            pause: KeyCode::Escape,
            reset: KeyCode::F5,
            record: KeyCode::F9,
            fullscreen: KeyCode::F11,
            scale: KeyCode::F6,
            filter: KeyCode::F7,
            vsync: KeyCode::F8,
            volume_up: KeyCode::PageUp,
            volume_down: KeyCode::PageDown,
        }
    }
}

impl Hotkeys {
    pub fn entries(&self) -> [(&'static str, KeyCode); 10] {
        [
            ("Settings", self.settings),
            ("Pause", self.pause),
            ("Reset", self.reset),
            ("Record", self.record),
            ("Fullscreen", self.fullscreen),
            ("Scale", self.scale),
            ("Filter", self.filter),
            ("Vsync", self.vsync),
            ("Volume up", self.volume_up),
            ("Volume down", self.volume_down),
        ]
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    #[serde(default = "legacy_version")]
    pub version: u32,
    pub window_scale: u32,
    pub pixel_shape: PixelShape,
    pub scale: Scale,
    pub filter: Filter,
    pub display_shader: DisplayShader,
    pub fullscreen: bool,
    pub vsync: bool,
    pub volume: f32,
    #[serde(default)]
    pub pause_on_focus_loss: bool,
    pub audio_device: Option<String>,
    pub gamepad_devices: Vec<crate::input::SavedPad>,
    pub players: [PlayerBindings; 2],
    pub hotkeys: Hotkeys,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            window_scale: 3,
            pixel_shape: PixelShape::Native,
            scale: Scale::Integer,
            filter: Filter::Sharp,
            display_shader: DisplayShader::Off,
            fullscreen: false,
            vsync: true,
            volume: 0.8,
            pause_on_focus_loss: true,
            audio_device: None,
            gamepad_devices: Vec::new(),
            players: [PlayerBindings::one(), PlayerBindings::two()],
            hotkeys: Hotkeys::default(),
        }
    }
}

impl Settings {
    fn parse(contents: &str) -> Result<Self> {
        let table: toml::Table = toml::from_str(contents)?;
        let explicit_scale = table.contains_key("scale");
        let explicit_shape = table.contains_key("pixel_shape");
        let mut settings: Self = toml::Value::Table(table).try_into()?;
        if settings.version < 3 && explicit_scale && !explicit_shape {
            // Old Integer used square pixels, whereas old Fit corrected the
            // NTSC aspect. An absent mode adopts the new Native + Integer default.
            settings.pixel_shape = match settings.scale {
                Scale::Integer => PixelShape::Square,
                Scale::Fit => PixelShape::Native,
            };
        }
        settings.migrate()?;
        Ok(settings)
    }

    fn migrate(&mut self) -> Result<()> {
        ensure!(
            (1..=SETTINGS_VERSION).contains(&self.version),
            "unsupported settings version {}; this application supports 1..={SETTINGS_VERSION}",
            self.version
        );
        if self.version == 1 {
            // Preserve existing controls, including a user binding on the newly
            // introduced settings shortcut. The settings button is also always
            // available in the on-screen toolbar.
            let occupied = |key| {
                self.players
                    .iter()
                    .flat_map(|p| &p.bindings)
                    .any(|b| b.key == Some(key))
                    || self
                        .hotkeys
                        .entries()
                        .iter()
                        .skip(1)
                        .any(|(_, other)| *other == key)
            };
            if occupied(self.hotkeys.settings) {
                self.hotkeys.settings = [
                    KeyCode::F1,
                    KeyCode::F2,
                    KeyCode::F3,
                    KeyCode::F4,
                    KeyCode::F10,
                    KeyCode::F12,
                    KeyCode::F13,
                    KeyCode::F14,
                ]
                .into_iter()
                .find(|&key| !occupied(key))
                .context("no free settings shortcut in legacy configuration")?;
            }
            self.version = 2;
        }
        if self.version == 2 {
            self.version = SETTINGS_VERSION;
        }
        self.validate()
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.gamepad_devices.len() <= 32,
            "at most 32 gamepad slot identities can be saved"
        );
        ensure!(
            self.version == SETTINGS_VERSION,
            "unsupported settings version {}",
            self.version
        );
        ensure!(
            (1..=8).contains(&self.window_scale),
            "window_scale must be 1..8"
        );
        ensure!(
            self.volume.is_finite() && (0.0..=1.0).contains(&self.volume),
            "volume must be 0..1"
        );
        ensure!(
            self.audio_device
                .as_ref()
                .is_none_or(|name| !name.trim().is_empty()),
            "audio device name cannot be empty"
        );
        let hotkeys = self.hotkeys.entries();
        for (index, &(name, key)) in hotkeys.iter().enumerate() {
            ensure!(
                !hotkeys[..index].iter().any(|(_, other)| *other == key),
                "{key:?} is assigned to more than one hotkey ({name})"
            );
        }
        for (index, player) in self.players.iter().enumerate() {
            ensure!(
                player.deadzone.is_finite() && (0.0..1.0).contains(&player.deadzone),
                "gamepad deadzone must be 0 <= value < 1"
            );
            ensure!(player.gamepad_slot < 32, "gamepad slot must be below 32");
            for binding in &player.bindings {
                if let Some(key) = binding.key {
                    if let Some((name, _)) = hotkeys.iter().find(|(_, hotkey)| *hotkey == key) {
                        bail!("{key:?} is reserved for {name}; choose a different game key");
                    }
                    for (other_index, other) in self.players.iter().enumerate() {
                        ensure!(
                            !other.bindings.iter().any(|b| b.key == Some(key)
                                && (index != other_index || b.button != binding.button)),
                            "{key:?} is already assigned to another player/action"
                        );
                    }
                }
                if let Some(pad) = binding.gamepad {
                    ensure!(
                        !player
                            .bindings
                            .iter()
                            .any(|b| b.gamepad == Some(pad) && b.button != binding.button),
                        "P{} {pad:?} is already assigned to another action",
                        index + 1
                    );
                }
            }
        }
        if self.players.iter().all(|p| p.enabled && p.gamepad_enabled) {
            ensure!(
                self.players[0].gamepad_slot != self.players[1].gamepad_slot,
                "players cannot share a gamepad slot; select another device or Keyboard only"
            );
        }
        Ok(())
    }

    pub fn bind_key(
        &mut self,
        player: usize,
        button: GameButton,
        key: Option<KeyCode>,
    ) -> Result<()> {
        ensure!(player < 2, "invalid player");
        if let Some(key) = key {
            if let Some((name, _)) = self
                .hotkeys
                .entries()
                .into_iter()
                .find(|(_, code)| *code == key)
            {
                bail!("{key:?} is reserved for {name}");
            }
            for (index, bindings) in self.players.iter().enumerate() {
                if let Some(binding) = bindings
                    .bindings
                    .iter()
                    .find(|b| b.key == Some(key) && (index != player || b.button != button))
                {
                    bail!(
                        "{key:?} is already P{} {:?}; clear that binding first",
                        index + 1,
                        binding.button
                    );
                }
            }
        }
        self.players[player].set_key(button, key);
        Ok(())
    }

    pub fn bind_pad(
        &mut self,
        player: usize,
        button: GameButton,
        pad: Option<PadButton>,
    ) -> Result<()> {
        ensure!(player < 2, "invalid player");
        if let Some(pad) = pad
            && let Some(binding) = self.players[player]
                .bindings
                .iter()
                .find(|b| b.gamepad == Some(pad) && b.button != button)
        {
            bail!(
                "{pad:?} is already P{} {:?}; clear that binding first",
                player + 1,
                binding.button
            );
        }
        self.players[player].set_pad(button, pad);
        Ok(())
    }

    /// Selecting the other player's device swaps slots, never duplicates input.
    pub fn assign_slot(&mut self, player: usize, slot: usize) {
        let old = self.players[player].gamepad_slot;
        let other = 1 - player;
        if self.players[other].gamepad_slot == slot {
            self.players[other].gamepad_slot = old;
        }
        self.players[player].gamepad_slot = slot;
        self.players[player].gamepad_enabled = true;
    }
}

pub struct Storage {
    pub directory: PathBuf,
    pub config_directory: PathBuf,
    pub cache_directory: PathBuf,
}

impl Storage {
    pub fn new(override_path: Option<PathBuf>) -> Result<Self> {
        let (directory, config_directory, cache_directory) = if let Some(path) = override_path {
            (path.clone(), path.clone(), path.join("cache"))
        } else {
            let dirs = ProjectDirs::from("org", "toejam-earl-port", "ToeJam & Earl")
                .context("no user data directory; specify --data-dir")?;
            (
                dirs.data_local_dir().to_path_buf(),
                dirs.config_dir().to_path_buf(),
                dirs.cache_dir().to_path_buf(),
            )
        };
        for path in [&directory, &config_directory, &cache_directory] {
            fs::create_dir_all(path).with_context(|| format!("create {}", path.display()))?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&cache_directory, fs::Permissions::from_mode(0o700))
                .context("protect private native-code cache directory")?;
        }
        Ok(Self {
            directory,
            config_directory,
            cache_directory,
        })
    }

    pub fn rom_path(&self) -> PathBuf {
        self.directory.join("cartridge.bin")
    }

    /// An explicit selection replaces the remembered cartridge after successful
    /// compilation. Otherwise a saved copy starts automatically, without a picker.
    pub fn startup_rom(&self, explicit: Option<&Path>) -> Option<(PathBuf, bool)> {
        explicit
            .map(|path| (path.to_path_buf(), true))
            .or_else(|| self.rom_path().is_file().then(|| (self.rom_path(), false)))
    }

    pub fn forget_rom(&self) -> Result<()> {
        match fs::remove_file(self.rom_path()) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("forget saved cartridge"),
        }
        match fs::remove_dir_all(self.cache_directory.join("engine-native")) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("remove private compiled cartridge cache"),
        }
        Ok(())
    }

    pub fn store_rom(&self, bytes: &[u8]) -> Result<()> {
        let temp = self
            .directory
            .join(format!("cartridge-{}.importing", std::process::id()));
        let result = (|| -> Result<()> {
            let mut file = fs::File::create(&temp)?;
            file.write_all(bytes)?;
            file.sync_all()?;
            fs::rename(&temp, self.rom_path())?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result.context("store verified cartridge; previous cartridge was not replaced")
    }

    pub fn load(&self) -> Result<Settings> {
        let path = self.config_directory.join("config.toml");
        let legacy = self.directory.join("config.toml");
        let source = if !path.exists() && legacy != path && legacy.is_file() {
            &legacy
        } else {
            &path
        };
        let settings = match fs::read_to_string(source) {
            Ok(contents) => {
                Settings::parse(&contents).with_context(|| format!("read {}", source.display()))?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Settings::default());
            }
            Err(error) => return Err(error).with_context(|| format!("read {}", source.display())),
        };
        if source != &path {
            self.save(&settings)?;
            fs::remove_file(source)
                .context("remove migrated legacy config after saving the new file")?;
        }
        Ok(settings)
    }

    pub fn save(&self, settings: &Settings) -> Result<()> {
        settings.validate()?;
        let path = self.config_directory.join("config.toml");
        let temp = self
            .config_directory
            .join(format!("config-{}.tmp", std::process::id()));
        let result = (|| -> Result<()> {
            let mut file = fs::OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(&temp)?;
            file.write_all(toml::to_string_pretty(settings)?.as_bytes())?;
            file.sync_all()?;
            fs::rename(&temp, &path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result.with_context(|| format!("save {}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_roundtrip_and_reject_invalid_settings() {
        let defaults = Settings::default();
        let mut restored: Settings = toml::from_str(&toml::to_string(&defaults).unwrap()).unwrap();
        restored.migrate().unwrap();
        assert_eq!(defaults, restored);
        restored.volume = f32::NAN;
        assert!(restored.validate().is_err());
        assert!(toml::from_str::<Settings>("typo = true").is_err());
        restored = defaults.clone();
        restored.window_scale = 0;
        assert!(restored.validate().is_err());
        restored = defaults.clone();
        restored.hotkeys.pause = restored.hotkeys.reset;
        assert!(restored.validate().is_err());
        restored = defaults.clone();
        restored.players[0].deadzone = 1.0;
        assert!(restored.validate().is_err());
        restored = defaults;
        restored.version = 999;
        assert!(restored.migrate().is_err());
    }

    #[test]
    fn display_defaults_and_legacy_explicit_choices_migrate_without_shape_changes() {
        for version in ["", "version = 1\n", "version = 2\n"] {
            let unset = Settings::parse(version).unwrap();
            assert_eq!(unset.version, SETTINGS_VERSION);
            assert_eq!(unset.pixel_shape, PixelShape::Native);
            assert_eq!(unset.scale, Scale::Integer);
            assert_eq!(unset.filter, Filter::Sharp);
            assert_eq!(unset.display_shader, DisplayShader::Off);
            for (scale, shape) in [
                (Scale::Integer, PixelShape::Square),
                (Scale::Fit, PixelShape::Native),
            ] {
                for filter in [Filter::Nearest, Filter::Linear] {
                    let values = format!(
                        "{version}scale = \"{}\"\nfilter = \"{}\"\n",
                        if scale == Scale::Integer {
                            "integer"
                        } else {
                            "fit"
                        },
                        if filter == Filter::Nearest {
                            "nearest"
                        } else {
                            "linear"
                        },
                    );
                    let migrated = Settings::parse(&values).unwrap();
                    assert_eq!(migrated.version, SETTINGS_VERSION);
                    assert_eq!(migrated.pixel_shape, shape);
                    assert_eq!(migrated.scale, scale);
                    assert_eq!(migrated.filter, filter);
                    assert_eq!(
                        Settings::parse(&toml::to_string(&migrated).unwrap()).unwrap(),
                        migrated
                    );
                }
            }
        }
        for pixel_shape in [PixelShape::Native, PixelShape::Square] {
            for scale in [Scale::Integer, Scale::Fit] {
                let settings = Settings {
                    pixel_shape,
                    scale,
                    ..Settings::default()
                };
                let restored = Settings::parse(&toml::to_string(&settings).unwrap()).unwrap();
                assert_eq!(restored, settings);
            }
        }
    }

    #[test]
    fn optional_display_presets_roundtrip_without_changing_scaling_or_filter() {
        for display_shader in DisplayShader::ALL {
            let settings = Settings {
                display_shader,
                filter: Filter::Nearest,
                pixel_shape: PixelShape::Square,
                scale: Scale::Fit,
                ..Settings::default()
            };
            assert_eq!(
                Settings::parse(&toml::to_string(&settings).unwrap()).unwrap(),
                settings
            );
        }
        assert_eq!(
            Settings::parse("version = 3\n").unwrap().display_shader,
            DisplayShader::Off
        );
    }

    #[test]
    fn legacy_migration_preserves_controls_and_avoids_new_hotkey_conflict() {
        let old = "[hotkeys]\npause = \"Escape\"\n";
        let mut settings: Settings = toml::from_str(old).unwrap();
        assert_eq!(settings.version, 1);
        settings.players[0].bindings[0].key = Some(KeyCode::F1);
        settings.migrate().unwrap();
        assert_eq!(settings.version, SETTINGS_VERSION);
        assert_eq!(settings.hotkeys.record, KeyCode::F9);
        assert_eq!(settings.hotkeys.settings, KeyCode::F2);
        assert_eq!(settings.players[0].bindings[0].key, Some(KeyCode::F1));
        assert!(!settings.pause_on_focus_loss);
    }

    #[test]
    fn binding_conflicts_are_rejected_without_changing_the_old_binding() {
        let mut settings = Settings::default();
        let original = settings.clone();
        assert!(
            settings
                .bind_key(0, GameButton::A, Some(KeyCode::KeyW))
                .is_err()
        );
        assert!(
            settings
                .bind_key(0, GameButton::A, Some(KeyCode::F9))
                .is_err()
        );
        assert!(
            settings
                .bind_pad(0, GameButton::A, Some(PadButton::South))
                .is_err()
        );
        assert_eq!(settings, original);
        settings
            .bind_key(0, GameButton::A, Some(KeyCode::KeyI))
            .unwrap();
        settings
            .bind_pad(0, GameButton::A, Some(PadButton::North))
            .unwrap();
        settings.validate().unwrap();
        settings.bind_pad(0, GameButton::Up, None).unwrap();
        assert!(
            settings.players[0]
                .bindings
                .iter()
                .filter(|b| b.button == GameButton::Up)
                .all(|b| b.gamepad.is_none())
        );
    }

    #[test]
    fn assigning_a_taken_gamepad_slot_swaps_players() {
        let mut settings = Settings::default();
        settings.assign_slot(0, 1);
        assert_eq!(
            [
                settings.players[0].gamepad_slot,
                settings.players[1].gamepad_slot
            ],
            [1, 0]
        );
        settings.validate().unwrap();
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
        settings.pause_on_focus_loss = true;
        settings.audio_device = Some("Example output".into());
        settings
            .bind_key(0, GameButton::Up, Some(KeyCode::KeyI))
            .unwrap();
        storage.save(&settings).unwrap();
        storage.save(&settings).unwrap(); // Atomic replacement must work on Windows too.
        assert_eq!(storage.load().unwrap(), settings);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn migrates_old_data_directory_config_only_after_a_successful_new_save() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("tje-config-migration-{nonce}"));
        let data = root.join("data");
        let config = root.join("config");
        fs::create_dir_all(&data).unwrap();
        fs::create_dir_all(&config).unwrap();
        fs::write(data.join("config.toml"), "volume = 0.3\n").unwrap();
        let storage = Storage {
            directory: data.clone(),
            config_directory: config.clone(),
            cache_directory: root.join("cache"),
        };
        assert_eq!(storage.load().unwrap().volume, 0.3);
        assert!(config.join("config.toml").is_file());
        assert!(!data.join("config.toml").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn invalid_or_future_config_is_reported_without_overwriting_it() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("tje-invalid-config-{nonce}"));
        let storage = Storage::new(Some(root.clone())).unwrap();
        for source in ["version = 999\n", "volume = 20.0\n", "not valid TOML"] {
            fs::write(root.join("config.toml"), source).unwrap();
            assert!(storage.load().is_err());
            assert_eq!(
                fs::read_to_string(root.join("config.toml")).unwrap(),
                source
            );
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn second_launch_selects_saved_cartridge_without_a_prompt_and_forget_keeps_settings() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("tje-remembered-cartridge-{nonce}"));
        let first = Storage::new(Some(root.clone())).unwrap();
        assert!(first.startup_rom(None).is_none());
        first.save(&Settings::default()).unwrap();
        // Selection/storage test only: identification of real images remains in
        // tje-rom and the opt-in two-launch production-engine integration test.
        first
            .store_rom(b"authored storage fixture, not a cartridge")
            .unwrap();
        let second = Storage::new(Some(root.clone())).unwrap();
        assert_eq!(second.startup_rom(None), Some((second.rom_path(), false)));
        assert_eq!(
            second.startup_rom(Some(Path::new("replacement.bin"))),
            Some(("replacement.bin".into(), true))
        );
        fs::create_dir_all(second.cache_directory.join("engine-native")).unwrap();
        fs::write(
            second.cache_directory.join("engine-native/authored-test"),
            b"not code",
        )
        .unwrap();
        second.forget_rom().unwrap();
        assert!(second.startup_rom(None).is_none());
        assert!(!second.cache_directory.join("engine-native").exists());
        assert!(second.config_directory.join("config.toml").is_file());
        fs::remove_dir_all(root).unwrap();
    }
}
