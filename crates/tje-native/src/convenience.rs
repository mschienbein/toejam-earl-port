//! Local-only opt-in convenience features. The default path captures nothing.
use anyhow::{Context, Result, ensure};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};
use tje_engine_runtime::{MachineSnapshot, RewindBuffer};
use tje_native::{
    backend::GameBackend,
    config::{ConvenienceSettings, Settings},
    input::Input,
};
use tje_rom::Revision;
use winit::keyboard::KeyCode;

const REWIND_BYTES: usize = 64 * 1024 * 1024;
const MAX_STATE_BYTES: u64 = 16 * 1024 * 1024;
#[derive(Clone, Copy)]
pub enum Command {
    Save,
    Load,
    NextSlot,
    Rewind,
    ToggleSpeed,
}

pub struct Convenience {
    ring: RewindBuffer,
    speed_latched: bool,
}
impl Default for Convenience {
    fn default() -> Self {
        Self {
            ring: RewindBuffer::new(REWIND_BYTES),
            speed_latched: false,
        }
    }
}
impl Convenience {
    pub fn clear(&mut self) {
        self.ring.clear();
        self.speed_latched = false;
    }
    pub fn configure(&mut self, settings: &ConvenienceSettings) {
        if !settings.rewind {
            self.ring.clear();
        }
        if !settings.fast_forward {
            self.speed_latched = false;
        }
    }
    pub fn toggle_speed(&mut self) {
        self.speed_latched = !self.speed_latched;
    }
    pub fn fast(&self, settings: &Settings, input: &Input) -> bool {
        settings.convenience.fast_forward
            && (self.speed_latched || held(settings, input, KeyCode::Tab))
    }
    pub fn rewinding(&self, settings: &Settings, input: &Input) -> bool {
        settings.convenience.rewind && held(settings, input, KeyCode::Backspace)
    }
    pub fn capture(
        &mut self,
        backend: &dyn GameBackend,
        settings: &ConvenienceSettings,
    ) -> Result<()> {
        if settings.rewind && backend.status().frame.is_multiple_of(3) {
            self.ring.push(backend.snapshot()?);
        }
        Ok(())
    }
    pub fn rewind(&mut self, backend: &mut dyn GameBackend) -> Result<bool> {
        let Some(image) = self.ring.pop_before(backend.status().frame) else {
            return Ok(false);
        };
        backend.restore(&image)?;
        Ok(true)
    }
}

// Existing custom mappings win; enabling an extra never steals a game/hotkey.
fn available(settings: &Settings, key: KeyCode) -> bool {
    !settings
        .hotkeys
        .entries()
        .iter()
        .any(|(_, code)| *code == key)
        && !settings
            .players
            .iter()
            .flat_map(|player| &player.bindings)
            .any(|binding| binding.key == Some(key))
}
fn held(settings: &Settings, input: &Input, key: KeyCode) -> bool {
    available(settings, key) && input.held(key)
}
pub fn shortcut(settings: &Settings, key: KeyCode) -> Option<Command> {
    if !available(settings, key) {
        return None;
    }
    match key {
        KeyCode::F2 if settings.convenience.save_states => Some(Command::Save),
        KeyCode::F3 if settings.convenience.save_states => Some(Command::Load),
        KeyCode::F4 if settings.convenience.save_states => Some(Command::NextSlot),
        _ => None,
    }
}

fn slot_path(root: &Path, revision: Revision, slot: u8) -> Result<PathBuf> {
    ensure!(slot < 5, "save-state slot must be 1..5");
    Ok(root
        .join("save-states")
        .join(revision.sha1())
        .join(format!("slot-{}.tje-state", slot + 1)))
}
pub fn save(root: &Path, revision: Revision, slot: u8, image: &MachineSnapshot) -> Result<()> {
    let path = slot_path(root, revision, slot)?;
    let directory = path.parent().unwrap();
    fs::create_dir_all(directory)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
    }
    let temp = path.with_extension(format!("{}.tmp", std::process::id()));
    let result = (|| -> Result<()> {
        let mut options = fs::OpenOptions::new();
        options.create(true).truncate(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temp)?;
        file.write_all(image.as_bytes())?;
        file.sync_all()?;
        fs::rename(&temp, &path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result.with_context(|| format!("save local state {}", path.display()))
}
pub fn load(root: &Path, revision: Revision, slot: u8) -> Result<MachineSnapshot> {
    let path = slot_path(root, revision, slot)?;
    let file =
        fs::File::open(&path).with_context(|| format!("open local state {}", path.display()))?;
    ensure!(
        file.metadata()?.len() <= MAX_STATE_BYTES,
        "save state is too large"
    );
    use std::io::Read;
    let mut bytes = Vec::new();
    file.take(MAX_STATE_BYTES + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_STATE_BYTES,
        "save state is too large"
    );
    Ok(MachineSnapshot::from_bytes(bytes)?)
}

pub fn controls(
    ui: &mut egui::Ui,
    settings: &mut ConvenienceSettings,
    enabled: bool,
    recording: bool,
    command: &mut Option<Command>,
) -> bool {
    let before = settings.clone();
    ui.collapsing("Convenience extras — default off", |ui| {
        ui.checkbox(&mut settings.save_states, "Local save states (F2 save / F3 load / F4 next slot)");
        ui.checkbox(&mut settings.rewind, "Rewind — hold Backspace (64 MiB ring, every 3 frames)");
        ui.checkbox(&mut settings.fast_forward, "Fast-forward — hold Tab (4x, muted)");
        ui.small("Existing custom key mappings take priority. Saves contain copyrighted RAM; keep them private. Rewind/load are unavailable during recording or input replay.");
        ui.add_enabled_ui(enabled, |ui| {
            if settings.save_states {
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("save-state-slot").selected_text(format!("Slot {}", settings.slot + 1)).show_ui(ui, |ui| {
                        for slot in 0..5 { ui.selectable_value(&mut settings.slot, slot, format!("Slot {}", slot + 1)); }
                    });
                    if ui.button("Save state").clicked() { *command = Some(Command::Save); }
                    if ui.add_enabled(!recording, egui::Button::new("Load state")).clicked() { *command = Some(Command::Load); }
                });
            }
            if settings.rewind && ui.add_enabled(!recording, egui::Button::new("Rewind one checkpoint")).clicked() { *command = Some(Command::Rewind); }
            if settings.fast_forward && ui.button("Toggle 4x speed after closing settings").clicked() { *command = Some(Command::ToggleSpeed); }
        });
    });
    *settings != before
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_are_off_and_custom_keys_are_not_stolen() {
        let mut settings = Settings::default();
        assert!(shortcut(&settings, KeyCode::F2).is_none());
        settings.convenience.save_states = true;
        assert!(matches!(
            shortcut(&settings, KeyCode::F2),
            Some(Command::Save)
        ));
        settings.players[0].bindings[0].key = Some(KeyCode::F2);
        assert!(shortcut(&settings, KeyCode::F2).is_none());
    }
    #[test]
    fn slots_are_bounded_and_revision_separated() {
        assert!(slot_path(Path::new("private"), Revision::Rev00, 5).is_err());
        assert_ne!(
            slot_path(Path::new("private"), Revision::Rev00, 0).unwrap(),
            slot_path(Path::new("private"), Revision::Rev02, 0).unwrap()
        );
    }
}
