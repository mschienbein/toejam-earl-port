use anyhow::{Context, Result, ensure};
use gilrs::{Axis, Button, Gamepad, GamepadId, Gilrs};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, path::Path};
use winit::keyboard::KeyCode;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum GameButton {
    Up,
    Down,
    Left,
    Right,
    A,
    B,
    C,
    Start,
}
impl GameButton {
    pub const ALL: [Self; 8] = [
        Self::Up,
        Self::Down,
        Self::Left,
        Self::Right,
        Self::A,
        Self::B,
        Self::C,
        Self::Start,
    ];
    pub fn index(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
pub enum PadButton {
    DPadUp,
    DPadDown,
    DPadLeft,
    DPadRight,
    South,
    East,
    West,
    North,
    Start,
    Select,
    LeftTrigger,
    RightTrigger,
    LeftTrigger2,
    RightTrigger2,
    LeftThumb,
    RightThumb,
    C,
    Z,
    Mode,
    StickUp,
    StickDown,
    StickLeft,
    StickRight,
    RightStickUp,
    RightStickDown,
    RightStickLeft,
    RightStickRight,
}

impl PadButton {
    pub fn from_button(button: Button) -> Option<Self> {
        Some(match button {
            Button::DPadUp => Self::DPadUp,
            Button::DPadDown => Self::DPadDown,
            Button::DPadLeft => Self::DPadLeft,
            Button::DPadRight => Self::DPadRight,
            Button::South => Self::South,
            Button::East => Self::East,
            Button::West => Self::West,
            Button::North => Self::North,
            Button::Start => Self::Start,
            Button::Select => Self::Select,
            Button::LeftTrigger => Self::LeftTrigger,
            Button::RightTrigger => Self::RightTrigger,
            Button::LeftTrigger2 => Self::LeftTrigger2,
            Button::RightTrigger2 => Self::RightTrigger2,
            Button::LeftThumb => Self::LeftThumb,
            Button::RightThumb => Self::RightThumb,
            Button::C => Self::C,
            Button::Z => Self::Z,
            Button::Mode => Self::Mode,
            Button::Unknown => return None,
        })
    }

    fn axis(axis: Axis) -> Option<(Self, Self)> {
        Some(match axis {
            Axis::LeftStickX => (Self::StickRight, Self::StickLeft),
            Axis::LeftStickY => (Self::StickUp, Self::StickDown),
            Axis::RightStickX => (Self::RightStickRight, Self::RightStickLeft),
            Axis::RightStickY => (Self::RightStickUp, Self::RightStickDown),
            _ => return None,
        })
    }

    fn pressed(self, gamepad: &Gamepad<'_>, deadzone: f32) -> bool {
        let button = match self {
            Self::DPadUp => Button::DPadUp,
            Self::DPadDown => Button::DPadDown,
            Self::DPadLeft => Button::DPadLeft,
            Self::DPadRight => Button::DPadRight,
            Self::South => Button::South,
            Self::East => Button::East,
            Self::West => Button::West,
            Self::North => Button::North,
            Self::Start => Button::Start,
            Self::Select => Button::Select,
            Self::LeftTrigger => Button::LeftTrigger,
            Self::RightTrigger => Button::RightTrigger,
            Self::LeftTrigger2 => Button::LeftTrigger2,
            Self::RightTrigger2 => Button::RightTrigger2,
            Self::LeftThumb => Button::LeftThumb,
            Self::RightThumb => Button::RightThumb,
            Self::C => Button::C,
            Self::Z => Button::Z,
            Self::Mode => Button::Mode,
            Self::StickUp => return gamepad.value(Axis::LeftStickY) > deadzone,
            Self::StickDown => return gamepad.value(Axis::LeftStickY) < -deadzone,
            Self::StickLeft => return gamepad.value(Axis::LeftStickX) < -deadzone,
            Self::StickRight => return gamepad.value(Axis::LeftStickX) > deadzone,
            Self::RightStickUp => return gamepad.value(Axis::RightStickY) > deadzone,
            Self::RightStickDown => return gamepad.value(Axis::RightStickY) < -deadzone,
            Self::RightStickLeft => return gamepad.value(Axis::RightStickX) < -deadzone,
            Self::RightStickRight => return gamepad.value(Axis::RightStickX) > deadzone,
        };
        gamepad.is_pressed(button)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub button: GameButton,
    pub key: Option<KeyCode>,
    pub gamepad: Option<PadButton>,
}
fn enabled() -> bool {
    true
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PlayerBindings {
    pub enabled: bool,
    #[serde(default = "enabled")]
    pub gamepad_enabled: bool,
    /// Slots retain their device identity while unplugged for stable reconnects.
    pub gamepad_slot: usize,
    pub deadzone: f32,
    pub bindings: Vec<Binding>,
}

impl PlayerBindings {
    fn defaults(keys: [KeyCode; 8], gamepad_slot: usize) -> Self {
        use GameButton::*;
        let pads = [
            PadButton::DPadUp,
            PadButton::DPadDown,
            PadButton::DPadLeft,
            PadButton::DPadRight,
            PadButton::West,
            PadButton::South,
            PadButton::East,
            PadButton::Start,
        ];
        let mut bindings: Vec<_> = GameButton::ALL
            .into_iter()
            .zip(keys)
            .zip(pads)
            .map(|((button, key), gamepad)| Binding {
                button,
                key: Some(key),
                gamepad: Some(gamepad),
            })
            .collect();
        for (button, gamepad) in [
            (Up, PadButton::StickUp),
            (Down, PadButton::StickDown),
            (Left, PadButton::StickLeft),
            (Right, PadButton::StickRight),
        ] {
            bindings.push(Binding {
                button,
                key: None,
                gamepad: Some(gamepad),
            });
        }
        Self {
            enabled: true,
            gamepad_enabled: true,
            gamepad_slot,
            deadzone: 0.4,
            bindings,
        }
    }
    pub fn one() -> Self {
        use KeyCode::*;
        Self::defaults(
            [
                ArrowUp, ArrowDown, ArrowLeft, ArrowRight, KeyZ, KeyX, KeyC, Enter,
            ],
            0,
        )
    }
    pub fn two() -> Self {
        use KeyCode::*;
        Self::defaults([KeyW, KeyS, KeyA, KeyD, KeyG, KeyH, KeyJ, Space], 1)
    }
    pub fn set_key(&mut self, button: GameButton, key: Option<KeyCode>) {
        for binding in self.bindings.iter_mut().filter(|b| b.button == button) {
            binding.key = None;
        }
        if let Some(binding) = self.bindings.iter_mut().find(|b| b.button == button) {
            binding.key = key;
        } else {
            self.bindings.push(Binding {
                button,
                key,
                gamepad: None,
            });
        }
    }
    pub fn set_pad(&mut self, button: GameButton, gamepad: Option<PadButton>) {
        for binding in self.bindings.iter_mut().filter(|b| b.button == button) {
            binding.gamepad = None;
        }
        if let Some(binding) = self.bindings.iter_mut().find(|b| b.button == button) {
            binding.gamepad = gamepad;
        } else {
            self.bindings.push(Binding {
                button,
                key: None,
                gamepad,
            });
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Controllers {
    pub connected: [bool; 2],
    pub buttons: [[bool; 8]; 2],
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SavedPad {
    pub uuid: [u8; 16],
    pub name: String,
}

struct Slot<T> {
    id: Option<T>,
    uuid: [u8; 16],
    name: String,
}
struct Slots<T> {
    entries: Vec<Slot<T>>,
}
impl<T: Copy + Eq> Slots<T> {
    fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }
    fn from_saved(saved: &[SavedPad]) -> Self {
        let mut slots = Self::new();
        slots.entries.extend(saved.iter().map(|pad| Slot {
            id: None,
            uuid: pad.uuid,
            name: pad.name.clone(),
        }));
        slots
    }
    fn connect(&mut self, id: T, uuid: [u8; 16], name: &str) {
        if self.entries.iter().any(|slot| slot.id == Some(id)) {
            return;
        }
        if let Some(slot) = self.entries.iter_mut().find(|slot| {
            slot.id.is_none() && slot.uuid == uuid && (uuid != [0; 16] || slot.name == name)
        }) {
            slot.id = Some(id);
            slot.name = name.into();
        } else if self.entries.len() < 32 {
            self.entries.push(Slot {
                id: Some(id),
                uuid,
                name: name.into(),
            });
        }
    }
    fn disconnect(&mut self, id: T) {
        for slot in &mut self.entries {
            if slot.id == Some(id) {
                slot.id = None;
            }
        }
    }
    fn index(&self, id: T) -> Option<usize> {
        self.entries.iter().position(|slot| slot.id == Some(id))
    }
    fn id(&self, index: usize) -> Option<T> {
        self.entries.get(index).and_then(|slot| slot.id)
    }
}

#[derive(Clone, Copy)]
pub struct PadPress {
    pub slot: usize,
    pub button: PadButton,
}
#[derive(Default)]
pub struct InputEvents {
    pub devices_changed: bool,
    pub pressed: Option<PadPress>,
}

pub struct Input {
    keys: HashSet<KeyCode>,
    gamepads: Gilrs,
    slots: Slots<GamepadId>,
    armed_axes: HashSet<(GamepadId, PadButton)>,
}

impl Input {
    pub fn new(saved: &[SavedPad]) -> Result<Self> {
        let gamepads =
            Gilrs::new().map_err(|error| anyhow::anyhow!("initialize gilrs: {error}"))?;
        let mut slots = Slots::from_saved(saved);
        for (id, pad) in gamepads.gamepads() {
            println!("GAMEPAD connected id={id} name={}", pad.name());
            slots.connect(id, pad.uuid(), pad.name());
        }
        Ok(Self {
            keys: HashSet::with_capacity(24),
            gamepads,
            slots,
            armed_axes: HashSet::with_capacity(16),
        })
    }
    pub fn key(&mut self, key: KeyCode, pressed: bool) -> bool {
        if pressed {
            self.keys.insert(key)
        } else {
            self.keys.remove(&key);
            false
        }
    }
    pub fn clear_keys(&mut self) {
        self.keys.clear();
    }
    pub fn saved_devices(&self) -> Vec<SavedPad> {
        self.slots
            .entries
            .iter()
            .map(|slot| SavedPad {
                uuid: slot.uuid,
                name: slot.name.clone(),
            })
            .collect()
    }
    pub fn slots(&self) -> impl Iterator<Item = (usize, &str, bool)> {
        self.slots
            .entries
            .iter()
            .enumerate()
            .map(|(index, slot)| (index, slot.name.as_str(), slot.id.is_some()))
    }

    pub fn poll_gamepads(&mut self) -> InputEvents {
        let mut result = InputEvents::default();
        while let Some(event) = self.gamepads.next_event() {
            let mut pressed = None;
            match event.event {
                gilrs::EventType::Connected => {
                    let pad = self.gamepads.gamepad(event.id);
                    self.slots.connect(event.id, pad.uuid(), pad.name());
                    result.devices_changed = true;
                    println!("GAMEPAD connected id={} name={}", event.id, pad.name());
                }
                gilrs::EventType::Disconnected => {
                    self.slots.disconnect(event.id);
                    self.armed_axes.retain(|(id, _)| *id != event.id);
                    result.devices_changed = true;
                    println!("GAMEPAD disconnected id={}", event.id);
                }
                gilrs::EventType::ButtonPressed(button, _) => {
                    pressed = PadButton::from_button(button);
                }
                gilrs::EventType::AxisChanged(axis, value, _) => {
                    if let Some((positive, negative)) = PadButton::axis(axis) {
                        if value.abs() < 0.35 {
                            self.armed_axes.remove(&(event.id, positive));
                            self.armed_axes.remove(&(event.id, negative));
                        } else if value.abs() > 0.65 {
                            let button = if value > 0.0 { positive } else { negative };
                            if self.armed_axes.insert((event.id, button)) {
                                pressed = Some(button);
                            }
                        }
                    }
                }
                _ => {}
            }
            if result.pressed.is_none()
                && let (Some(button), Some(slot)) = (pressed, self.slots.index(event.id))
            {
                result.pressed = Some(PadPress { slot, button });
            }
        }
        result
    }

    pub fn controllers(&self, bindings: &[PlayerBindings; 2]) -> Controllers {
        let mut result = Controllers::default();
        for (index, player) in bindings.iter().enumerate() {
            result.connected[index] = player.enabled;
            if !player.enabled {
                continue;
            }
            let gamepad = player
                .gamepad_enabled
                .then(|| self.slots.id(player.gamepad_slot))
                .flatten()
                .map(|id| self.gamepads.gamepad(id))
                .filter(Gamepad::is_connected);
            for binding in &player.bindings {
                let keyboard = binding.key.is_some_and(|key| self.keys.contains(&key));
                let pad = gamepad.as_ref().is_some_and(|pad| {
                    binding
                        .gamepad
                        .is_some_and(|button| button.pressed(pad, player.deadzone))
                });
                result.buttons[index][binding.button.index()] |= keyboard || pad;
            }
        }
        result
    }
}

/// Key injection uses the normal keyboard binding/hotkey handler. Pausing the
/// game does not pause these scheduler ticks, allowing pause/resume tests.
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputScript {
    pub holds: Vec<KeyHold>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyHold {
    pub start: u64,
    pub end: u64,
    pub keys: Vec<KeyCode>,
}
impl InputScript {
    pub fn load(path: &Path) -> Result<Self> {
        let script: Self = serde_json::from_str(&std::fs::read_to_string(path)?)
            .context("read keyboard input script")?;
        for hold in &script.holds {
            ensure!(hold.start < hold.end, "keyboard hold must have start < end");
        }
        Ok(script)
    }
    pub fn changes(&self, tick: u64, changes: &mut Vec<(KeyCode, bool)>) {
        changes.clear();
        for hold in &self.holds {
            if hold.end == tick {
                for &key in &hold.keys {
                    if !self.holds.iter().any(|other| {
                        other.start <= tick && tick < other.end && other.keys.contains(&key)
                    }) {
                        changes.push((key, false));
                    }
                }
            }
        }
        for hold in &self.holds {
            if hold.start == tick {
                changes.extend(hold.keys.iter().map(|&key| (key, true)));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn overlapping_script_holds_do_not_release_early() {
        let script: InputScript = serde_json::from_str(r#"{"holds":[{"start":1,"end":3,"keys":["Enter"]},{"start":2,"end":4,"keys":["Enter"]}]}"#).unwrap();
        let mut changes = Vec::new();
        script.changes(1, &mut changes);
        assert_eq!(changes, [(KeyCode::Enter, true)]);
        script.changes(3, &mut changes);
        assert!(changes.is_empty());
        script.changes(4, &mut changes);
        assert_eq!(changes, [(KeyCode::Enter, false)]);
    }
    #[test]
    fn reconnect_retains_slots_and_new_pads_do_not_steal_disconnected_players() {
        let mut slots = Slots::new();
        slots.connect(1, [1; 16], "One");
        slots.connect(2, [2; 16], "Two");
        slots.disconnect(1);
        slots.connect(3, [3; 16], "Three");
        assert_eq!(slots.id(0), None);
        assert_eq!(slots.id(1), Some(2));
        assert_eq!(slots.id(2), Some(3));
        slots.connect(4, [1; 16], "One reconnected");
        assert_eq!(slots.id(0), Some(4));
        assert_eq!(slots.id(1), Some(2));
    }
    #[test]
    fn identical_pad_models_still_get_distinct_slots() {
        let mut slots = Slots::new();
        slots.connect(1, [1; 16], "Pad");
        slots.connect(2, [1; 16], "Pad");
        slots.disconnect(1);
        slots.connect(3, [1; 16], "Pad");
        assert_eq!(slots.id(0), Some(3));
        assert_eq!(slots.id(1), Some(2));
    }
    #[test]
    fn saved_identities_restore_assignments_when_enumeration_order_changes() {
        let saved = vec![
            SavedPad {
                uuid: [1; 16],
                name: "One".into(),
            },
            SavedPad {
                uuid: [2; 16],
                name: "Two".into(),
            },
        ];
        let mut slots = Slots::from_saved(&saved);
        slots.connect(9, [2; 16], "Two");
        slots.connect(8, [1; 16], "One");
        assert_eq!(slots.id(0), Some(8));
        assert_eq!(slots.id(1), Some(9));
    }
}
