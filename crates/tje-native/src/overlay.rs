//! Native-only controls/settings UI; it consumes no backend-specific state.
use crate::{
    config::{DisplayShader, Filter, PixelShape, Scale, Settings, Storage},
    input::{GameButton, Input, PadPress},
};
use std::path::PathBuf;
use tje_engine_cranelift::CompileProgress;
use winit::{event::WindowEvent, keyboard::KeyCode, window::Window};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Capture {
    Key(usize, GameButton),
    Pad(usize, GameButton),
}

#[derive(Default)]
pub struct Actions {
    pub settings_changed: bool,
    pub restore_defaults: bool,
    pub import: Option<PathBuf>,
    pub resume: bool,
    pub refresh_devices: bool,
    pub dismiss: bool,
    pub forget_rom: bool,
    pub reporting: crate::reports::Actions,
}

pub struct Frame {
    pub jobs: Vec<egui::ClippedPrimitive>,
    pub textures: egui::TexturesDelta,
    pub pixels_per_point: f32,
    pub repaint_after: std::time::Duration,
}

pub struct Info<'a> {
    pub has_game: bool,
    pub paused: bool,
    pub recording: bool,
    pub loading: Option<(tje_rom::Revision, CompileProgress)>,
    pub replay: Option<(u64, u64)>,
    pub audio: &'a str,
    pub storage: &'a Storage,
    pub input: &'a Input,
    pub devices: &'a [String],
}

pub struct Overlay {
    context: egui::Context,
    state: Option<egui_winit::State>,
    pub open: bool,
    pending: Option<Capture>,
    pub message: Option<String>,
    path: String,
    tab: usize,
    player: usize,
    forget_confirmation: bool,
    pub reports: crate::reports::Ui,
}

impl Overlay {
    pub fn new() -> Self {
        Self {
            context: egui::Context::default(),
            state: None,
            open: false,
            pending: None,
            message: None,
            path: String::new(),
            tab: 0,
            player: 0,
            forget_confirmation: false,
            reports: crate::reports::Ui::new(),
        }
    }
    pub fn initialize(&mut self, window: &Window) {
        self.state = Some(egui_winit::State::new(
            self.context.clone(),
            egui::ViewportId::ROOT,
            window,
            Some(window.scale_factor() as f32),
            window.theme(),
            None,
        ));
    }
    pub fn event(&mut self, window: &Window, event: &WindowEvent) -> bool {
        let repaint = self
            .state
            .as_mut()
            .is_some_and(|state| state.on_window_event(window, event).repaint);
        // egui marks RedrawRequested as repaint-worthy too. Re-requesting it
        // here would create an unpaced, self-sustaining presentation loop.
        repaint && !matches!(event, WindowEvent::RedrawRequested)
    }
    pub fn toggle(&mut self) {
        self.open = !self.open;
        self.pending = None;
    }
    pub fn capturing(&self) -> bool {
        self.pending.is_some()
    }
    pub fn cancel(&mut self) {
        if self.pending.take().is_none() {
            self.open = false;
        }
    }
    pub fn show_error(&mut self, error: impl Into<String>) {
        self.message = Some(error.into());
    }

    pub fn capture_key(&mut self, settings: &mut Settings, key: KeyCode) -> bool {
        let Some(Capture::Key(player, button)) = self.pending else {
            return false;
        };
        match settings.bind_key(player, button, Some(key)) {
            Ok(()) => {
                self.pending = None;
                self.message = Some(format!("P{} {button:?} keyboard: {key:?}", player + 1));
                true
            }
            Err(error) => {
                self.message = Some(error.to_string());
                false
            }
        }
    }
    pub fn capture_pad(&mut self, settings: &mut Settings, press: PadPress) -> bool {
        let Some(Capture::Pad(player, button)) = self.pending else {
            return false;
        };
        if settings.players[player].gamepad_slot != press.slot {
            self.message = Some(format!(
                "That is slot {}. Select it for P{} first, or press the assigned controller.",
                press.slot + 1,
                player + 1
            ));
            return false;
        }
        match settings.bind_pad(player, button, Some(press.button)) {
            Ok(()) => {
                self.pending = None;
                self.message = Some(format!(
                    "P{} {button:?} gamepad: {:?}",
                    player + 1,
                    press.button
                ));
                true
            }
            Err(error) => {
                self.message = Some(error.to_string());
                false
            }
        }
    }

    pub fn draw(
        &mut self,
        window: &Window,
        settings: &mut Settings,
        info: Info<'_>,
    ) -> (Frame, Actions) {
        let raw = self.state.as_mut().unwrap().take_egui_input(window);
        if info.has_game
            && info.loading.is_none()
            && !info.paused
            && !self.open
            && self.message.is_none()
            && !self.reports.visible()
            && !info.audio.starts_with("Audio unavailable")
        {
            return (
                Frame {
                    jobs: Vec::new(),
                    textures: egui::TexturesDelta::default(),
                    pixels_per_point: self.context.pixels_per_point(),
                    repaint_after: std::time::Duration::from_secs(1),
                },
                Actions::default(),
            );
        }
        let context = self.context.clone();
        let mut actions = Actions::default();
        let output = context.run_ui(raw, |root| {
            if let Some((revision, progress)) = info.loading {
                egui::CentralPanel::default().show(root, |ui| {
                    ui.heading(format!("Preparing ToeJam & Earl — {revision}"));
                    ui.label("The original instructions are compiled to native code on this computer. No 68000 interpreter is used.");
                    let (label, fraction) = match progress {
                        CompileProgress::Lifting => ("Reading verified game instructions", None),
                        CompileProgress::CheckingCache => ("Checking remembered native-code cache", None),
                        CompileProgress::Compiling { completed, total } => ("Compiling native game", Some(if total == 0 { 0.0 } else { completed as f32 / total as f32 })),
                        CompileProgress::WritingCache => ("Saving private compiled cache", Some(1.0)),
                        CompileProgress::Ready { .. } => ("Starting native game", Some(1.0)),
                    };
                    ui.label(label);
                    if let Some(fraction) = fraction { ui.add(egui::ProgressBar::new(fraction).show_percentage()); }
                    ui.label("The cartridge is supplied once and remembered. Later launches load the compiled cache automatically.");
                    ui.small(format!("Private cache: {}", info.storage.cache_directory.display()));
                    if ui.button(format!("Settings ({:?})", settings.hotkeys.settings)).clicked() { self.open = true; }
                });
            } else if !info.has_game {
                egui::CentralPanel::default().show(root, |ui| {
                    ui.heading("ToeJam & Earl");
                    ui.label("Supply your cartridge once; it is remembered for later launches. No game data is included.");
                    ui.add_space(12.0);
                    self.import_ui(ui, &mut actions, info.storage.rom_path().is_file());
                    if ui.button(format!("Settings and controls ({:?})", settings.hotkeys.settings)).clicked() { self.open = true; }
                    ui.small(format!("Cartridge and recordings: {}", info.storage.directory.display()));
                });
            } else if info.paused || info.audio.starts_with("Audio unavailable") {
                egui::Window::new("Game status").resizable(false).collapsible(false)
                    .anchor(egui::Align2::LEFT_TOP, [8.0, 8.0]).show(&context, |ui| {
                        if info.paused { ui.label("Paused"); if ui.button("Resume").clicked() { actions.resume = true; } }
                        if info.audio.starts_with("Audio unavailable") { ui.colored_label(egui::Color32::YELLOW, info.audio); }
                        if ui.button(format!("Settings ({:?})", settings.hotkeys.settings)).clicked() { self.open = true; }
                    });
            }
            if self.open {
                let mut open = self.open;
                egui::Window::new("Settings — game paused").open(&mut open).default_width(620.0)
                    .default_height(520.0).vscroll(true).show(&context, |ui| {
                        ui.horizontal(|ui| {
                            ui.selectable_value(&mut self.tab, 0, "Video / audio");
                            ui.selectable_value(&mut self.tab, 1, "Controls");
                            ui.selectable_value(&mut self.tab, 2, "Cartridge / help");
                        });
                        ui.separator();
                        match self.tab {
                            0 => self.general(ui, settings, &info, &mut actions),
                            1 => self.controls(ui, settings, info.input, &mut actions),
                            _ => {
                                if info.loading.is_none() {
                                    self.import_ui(ui, &mut actions, info.storage.rom_path().is_file());
                                } else { ui.label("Wait for native preparation before replacing or forgetting the cartridge."); }
                                ui.separator();
                                for (name, key) in settings.hotkeys.entries() { ui.label(format!("{name}: {key:?}")); }
                                ui.label("Recording starts from a hard reset. Opening settings/focus pause adds no recording frames; resetting/importing/exiting saves the current session.");
                                ui.label(format!("Recording: {}", if info.recording { "active" } else { "off" }));
                                if let Some((frame, total)) = info.replay { ui.label(format!("Input replay: {frame}/{total} frames. F5 restarts; F9 recording is disabled during replay.")); }
                                ui.small(format!("Data: {}", info.storage.directory.display()));
                                ui.small(format!("Settings: {}", info.storage.config_directory.display()));
                                ui.small(format!("Cache: {}", info.storage.cache_directory.display()));
                            }
                        }
                        if let Some(capture) = self.pending {
                            ui.separator();
                            ui.colored_label(egui::Color32::YELLOW, match capture {
                                Capture::Key(player, button) => format!("P{} {button:?}: press a keyboard key. Escape cancels.", player + 1),
                                Capture::Pad(player, button) => format!("P{} {button:?}: release controls, then press a pad button or move a stick. Escape cancels.", player + 1),
                            });
                        }
                        if let Some(message) = &self.message { ui.separator(); ui.colored_label(egui::Color32::YELLOW, message); }
                        ui.separator();
                        ui.label("Changes apply immediately and are saved. Conflicting controls are rejected, never silently stolen.");
                        self.reports.controls(ui);
                        if ui.button("Restore default settings and controls").clicked() { actions.restore_defaults = true; self.pending = None; }
                    });
                if self.open && !open { self.pending = None; }
                self.open = open;
            } else if info.has_game && let Some(message) = &self.message {
                egui::Window::new("Notice").collapsible(false).default_width(500.0).show(&context, |ui| {
                    ui.label(message);
                    if ui.button("Dismiss").clicked() { actions.dismiss = true; }
                });
            }
            self.reports.windows(&context, &mut actions.reporting);
        });
        let repaint_after = output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .map_or(std::time::Duration::from_secs(1), |viewport| {
                viewport.repaint_delay
            })
            .max(std::time::Duration::from_millis(16));
        self.state
            .as_mut()
            .unwrap()
            .handle_platform_output(window, output.platform_output);
        let frame = Frame {
            jobs: context.tessellate(output.shapes, output.pixels_per_point),
            textures: output.textures_delta,
            pixels_per_point: output.pixels_per_point,
            repaint_after,
        };
        (frame, actions)
    }

    fn import_ui(&mut self, ui: &mut egui::Ui, actions: &mut Actions, remembered: bool) {
        ui.label(
            "Choose or replace your cartridge: drop it on the window, or enter its full file path:",
        );
        ui.add(
            egui::TextEdit::singleline(&mut self.path)
                .hint_text("Cartridge .bin, .md, .gen, .smd or .zip")
                .desired_width(f32::INFINITY),
        );
        if ui.button("Import cartridge").clicked() {
            let path = self.path.trim().trim_matches('"');
            if path.is_empty() {
                self.message = Some("Choose your ToeJam & Earl REV 02 or REV 00 cartridge file, or drop it on this window.".into());
            } else {
                actions.import = Some(PathBuf::from(path));
            }
        }
        ui.label("Only the original ToeJam & Earl REV 02 / REV 00 retail dumps are accepted. ZIP must contain exactly one supported cartridge. Unknown games, modified images and bad dumps are rejected without replacing your saved cartridge.");
        if remembered {
            if self.forget_confirmation {
                ui.colored_label(egui::Color32::YELLOW, "Forget the saved cartridge and this app's compiled cache? Controls/settings are kept.");
                ui.horizontal(|ui| {
                    if ui.button("Forget now").clicked() {
                        actions.forget_rom = true;
                        self.forget_confirmation = false;
                    }
                    if ui.button("Cancel").clicked() {
                        self.forget_confirmation = false;
                    }
                });
            } else if ui
                .button("Forget saved cartridge and compiled cache...")
                .clicked()
            {
                self.forget_confirmation = true;
            }
        }
        if let Some(message) = &self.message {
            ui.colored_label(egui::Color32::YELLOW, message);
        }
    }

    fn general(
        &mut self,
        ui: &mut egui::Ui,
        settings: &mut Settings,
        info: &Info<'_>,
        actions: &mut Actions,
    ) {
        ui.heading("Video");
        ui.horizontal(|ui| {
            ui.label("Pixel shape");
            actions.settings_changed |= ui
                .selectable_value(
                    &mut settings.pixel_shape,
                    PixelShape::Native,
                    "Native (NTSC)",
                )
                .changed();
            actions.settings_changed |= ui
                .selectable_value(
                    &mut settings.pixel_shape,
                    PixelShape::Square,
                    "Square pixels",
                )
                .changed();
        });
        ui.horizontal(|ui| {
            ui.label("Scale mode");
            actions.settings_changed |= ui
                .selectable_value(&mut settings.scale, Scale::Integer, "Integer")
                .changed();
            actions.settings_changed |= ui
                .selectable_value(&mut settings.scale, Scale::Fit, "Fit (non-integer)")
                .changed();
        });
        ui.small("Integer keeps whole source-line multiples at the chosen pixel shape; windows too small for 1× use Fit.");
        ui.horizontal(|ui| {
            ui.label("Filter");
            actions.settings_changed |= ui
                .selectable_value(&mut settings.filter, Filter::Sharp, "Sharp bilinear")
                .changed();
            actions.settings_changed |= ui
                .selectable_value(&mut settings.filter, Filter::Nearest, "Nearest")
                .changed();
            actions.settings_changed |= ui
                .selectable_value(&mut settings.filter, Filter::Linear, "Linear")
                .changed();
        });
        egui::ComboBox::from_label("Display shader")
            .selected_text(settings.display_shader.label())
            .show_ui(ui, |ui| {
                for preset in DisplayShader::ALL {
                    actions.settings_changed |= ui
                        .selectable_value(&mut settings.display_shader, preset, preset.label())
                        .changed();
                }
            });
        ui.small("Opt-in GPU effects only. Smooth uses soft bilinear; CRT uses the selected filter. Pixel shape and scaling are unchanged.");
        actions.settings_changed |= ui.checkbox(&mut settings.vsync, "Vsync").changed();
        actions.settings_changed |= ui
            .checkbox(&mut settings.fullscreen, "Fullscreen")
            .changed();
        actions.settings_changed |= ui
            .add(
                egui::Slider::new(&mut settings.window_scale, 1..=8)
                    .text("Window scale (logical pixels)"),
            )
            .changed();
        ui.small("Window scale respects OS high-DPI scaling; fullscreen keeps this size for the next windowed session.");
        actions.settings_changed |= ui
            .checkbox(
                &mut settings.pause_on_focus_loss,
                "Pause while the window is unfocused",
            )
            .changed();
        ui.separator();
        ui.heading("Audio");
        actions.settings_changed |= ui
            .add(egui::Slider::new(&mut settings.volume, 0.0..=1.0).text("Volume"))
            .changed();
        egui::ComboBox::from_id_salt("audio-output")
            .selected_text(
                settings
                    .audio_device
                    .as_deref()
                    .unwrap_or("System default (follow changes)"),
            )
            .show_ui(ui, |ui| {
                actions.settings_changed |= ui
                    .selectable_value(
                        &mut settings.audio_device,
                        None,
                        "System default (follow changes)",
                    )
                    .changed();
                for name in info.devices {
                    actions.settings_changed |= ui
                        .selectable_value(&mut settings.audio_device, Some(name.clone()), name)
                        .changed();
                }
            });
        ui.label(info.audio);
        if ui.button("Refresh audio devices").clicked() {
            actions.refresh_devices = true;
        }
    }

    fn controls(
        &mut self,
        ui: &mut egui::Ui,
        settings: &mut Settings,
        input: &Input,
        actions: &mut Actions,
    ) {
        ui.horizontal(|ui| {
            if ui
                .selectable_value(&mut self.player, 0, "Player 1")
                .changed()
            {
                self.pending = None;
            }
            if ui
                .selectable_value(&mut self.player, 1, "Player 2")
                .changed()
            {
                self.pending = None;
            }
        });
        let player = self.player;
        actions.settings_changed |= ui
            .checkbox(
                &mut settings.players[player].enabled,
                "Player controller connected",
            )
            .changed();
        let before_slot = settings.players[player].gamepad_slot;
        let mut selected = if settings.players[player].gamepad_enabled {
            Some(before_slot)
        } else {
            None
        };
        egui::ComboBox::from_id_salt("gamepad-slot")
            .selected_text(match selected {
                Some(slot) => format!("Gamepad slot {}", slot + 1),
                None => "Keyboard only".into(),
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut selected, None, "Keyboard only");
                for slot in 0..input.slots().count().max(2) {
                    let description = input
                        .slots()
                        .find(|(index, _, _)| *index == slot)
                        .map(|(_, name, connected)| {
                            format!(
                                "Slot {}: {name}{}",
                                slot + 1,
                                if connected { "" } else { " (disconnected)" }
                            )
                        })
                        .unwrap_or_else(|| format!("Slot {}: waiting for controller", slot + 1));
                    ui.selectable_value(&mut selected, Some(slot), description);
                }
            });
        if selected
            != settings.players[player]
                .gamepad_enabled
                .then_some(before_slot)
        {
            if let Some(slot) = selected {
                settings.assign_slot(player, slot);
            } else {
                settings.players[player].gamepad_enabled = false;
            }
            self.pending = None;
            actions.settings_changed = true;
        }
        ui.small("Slots retain unplugged device identities. Selecting the other player's slot swaps assignments.");
        actions.settings_changed |= ui
            .add(
                egui::Slider::new(&mut settings.players[player].deadzone, 0.0..=0.99)
                    .clamping(egui::SliderClamping::Edits)
                    .text("Stick deadzone"),
            )
            .changed();
        egui::ScrollArea::horizontal().show(ui, |ui| {
            egui::Grid::new("bindings").striped(true).show(ui, |ui| {
                ui.label("Action");
                ui.label("Keyboard");
                ui.label("Gamepad");
                ui.end_row();
                for button in GameButton::ALL {
                    ui.label(format!("{button:?}"));
                    ui.horizontal(|ui| {
                        let key = settings.players[player]
                            .bindings
                            .iter()
                            .find_map(|binding| {
                                (binding.button == button).then_some(binding.key).flatten()
                            });
                        if ui
                            .button(key.map_or_else(|| "Unbound".into(), |key| format!("{key:?}")))
                            .clicked()
                        {
                            self.pending = Some(Capture::Key(player, button));
                            self.message = None;
                        }
                        if ui.small_button("Clear").clicked() {
                            settings.bind_key(player, button, None).unwrap();
                            actions.settings_changed = true;
                        }
                    });
                    ui.horizontal(|ui| {
                        let mut label = String::new();
                        for binding in settings.players[player]
                            .bindings
                            .iter()
                            .filter(|binding| binding.button == button)
                        {
                            if let Some(pad) = binding.gamepad {
                                if !label.is_empty() {
                                    label.push_str(" / ");
                                }
                                label.push_str(&format!("{pad:?}"));
                            }
                        }
                        if label.is_empty() {
                            label.push_str("Unbound");
                        }
                        if ui.button(label).clicked() {
                            if settings.players[player].gamepad_enabled {
                                self.pending = Some(Capture::Pad(player, button));
                                self.message = None;
                            } else {
                                self.message = Some(
                                    "Select a gamepad slot before remapping pad controls.".into(),
                                );
                            }
                        }
                        if ui.small_button("Clear").clicked() {
                            settings.bind_pad(player, button, None).unwrap();
                            actions.settings_changed = true;
                        }
                    });
                    ui.end_row();
                }
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capture_conflict_can_be_cancelled_and_does_not_steal_another_binding() {
        let mut overlay = Overlay::new();
        let mut settings = Settings::default();
        overlay.pending = Some(Capture::Key(0, GameButton::A));
        overlay.open = true;
        assert!(!overlay.capture_key(&mut settings, KeyCode::KeyW));
        assert!(overlay.message.as_ref().unwrap().contains("P2"));
        assert!(overlay.pending.is_some());
        overlay.cancel();
        assert!(overlay.open);
        assert!(overlay.pending.is_none());
        overlay.pending = Some(Capture::Key(0, GameButton::A));
        assert!(overlay.capture_key(&mut settings, KeyCode::KeyI));
        assert!(overlay.pending.is_none());
        settings.validate().unwrap();
    }
    #[test]
    fn pad_capture_requires_the_selected_player_device() {
        let mut overlay = Overlay::new();
        let mut settings = Settings::default();
        overlay.pending = Some(Capture::Pad(1, GameButton::A));
        assert!(!overlay.capture_pad(
            &mut settings,
            PadPress {
                slot: 0,
                button: crate::input::PadButton::North
            }
        ));
        assert!(overlay.capture_pad(
            &mut settings,
            PadPress {
                slot: 1,
                button: crate::input::PadButton::North
            }
        ));
        settings.validate().unwrap();
    }
}
