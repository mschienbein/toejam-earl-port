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
    pub fn index(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
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
    LeftThumb,
    RightThumb,
    StickUp,
    StickDown,
    StickLeft,
    StickRight,
}

impl PadButton {
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
            Self::LeftThumb => Button::LeftThumb,
            Self::RightThumb => Button::RightThumb,
            Self::StickUp => return gamepad.value(Axis::LeftStickY) > deadzone,
            Self::StickDown => return gamepad.value(Axis::LeftStickY) < -deadzone,
            Self::StickLeft => return gamepad.value(Axis::LeftStickX) < -deadzone,
            Self::StickRight => return gamepad.value(Axis::LeftStickX) > deadzone,
        };
        gamepad.is_pressed(button)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub button: GameButton,
    pub key: Option<KeyCode>,
    pub gamepad: Option<PadButton>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerBindings {
    pub enabled: bool,
    /// Connected devices are assigned stable slots in connection order.
    pub gamepad_slot: usize,
    pub deadzone: f32,
    pub bindings: Vec<Binding>,
}

impl PlayerBindings {
    fn defaults(keys: [KeyCode; 8], gamepad_slot: usize) -> Self {
        use GameButton::*;
        let buttons = [Up, Down, Left, Right, A, B, C, Start];
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
        let mut bindings: Vec<_> = buttons
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
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Controllers {
    pub connected: [bool; 2],
    pub buttons: [[bool; 8]; 2],
}

pub struct Input {
    keys: HashSet<KeyCode>,
    gamepads: Gilrs,
    slots: Vec<Option<GamepadId>>,
}

impl Input {
    pub fn new() -> Result<Self> {
        let gamepads =
            Gilrs::new().map_err(|error| anyhow::anyhow!("initialize gilrs: {error}"))?;
        let slots = gamepads
            .gamepads()
            .map(|(id, pad)| {
                println!("GAMEPAD connected id={id} name={}", pad.name());
                Some(id)
            })
            .collect();
        Ok(Self {
            keys: HashSet::with_capacity(24),
            gamepads,
            slots,
        })
    }

    /// Both OS events and optional frame-script injection enter through here.
    /// Returns true only on a fresh key-down, so hotkeys do not autorepeat.
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

    pub fn poll_gamepads(&mut self) {
        while let Some(event) = self.gamepads.next_event() {
            match event.event {
                gilrs::EventType::Connected => {
                    if !self.slots.contains(&Some(event.id)) {
                        if let Some(slot) = self.slots.iter_mut().find(|slot| slot.is_none()) {
                            *slot = Some(event.id);
                        } else {
                            self.slots.push(Some(event.id));
                        }
                    }
                    println!(
                        "GAMEPAD connected id={} name={}",
                        event.id,
                        self.gamepads.gamepad(event.id).name()
                    );
                }
                gilrs::EventType::Disconnected => {
                    for slot in &mut self.slots {
                        if *slot == Some(event.id) {
                            *slot = None;
                        }
                    }
                    println!("GAMEPAD disconnected id={}", event.id);
                }
                _ => {}
            }
        }
    }

    pub fn controllers(&self, bindings: &[PlayerBindings; 2]) -> Controllers {
        let mut result = Controllers::default();
        for (index, player) in bindings.iter().enumerate() {
            result.connected[index] = player.enabled;
            if !player.enabled {
                continue;
            }
            let gamepad = self
                .slots
                .get(player.gamepad_slot)
                .and_then(|id| *id)
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

/// Private test sessions inject *keyboard* events through the normal bindings,
/// not backend controller state. Intervals count scheduler ticks from power-on;
/// ticks continue while paused, so a script can test pause/unpause hotkeys.
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
        // Release only after the last overlapping hold of a given key expires.
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
}
