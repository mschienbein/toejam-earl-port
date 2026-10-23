use crate::{HostError, Result};
use genesis_config::{GenesisButton, GenesisController, GenesisControllerType, GenesisInputs};
use jgenesis_common::{frontend::MappableInputs, input::Player};
use serde::Deserialize;

/// Holds are half-open video-frame intervals, counted from power-on at zero.
/// Overlapping holds combine their buttons; outside a hold every button is up.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputScript {
    #[serde(default)]
    pub p2_connected: bool,
    #[serde(default)]
    pub holds: Vec<InputHold>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputHold {
    pub start: u64,
    pub end: u64,
    #[serde(default)]
    pub p1: Vec<Button>,
    #[serde(default)]
    pub p2: Vec<Button>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Button {
    Up,
    Down,
    Left,
    Right,
    A,
    B,
    C,
    Start,
}

impl From<Button> for GenesisButton {
    fn from(button: Button) -> Self {
        match button {
            Button::Up => Self::Up,
            Button::Down => Self::Down,
            Button::Left => Self::Left,
            Button::Right => Self::Right,
            Button::A => Self::A,
            Button::B => Self::B,
            Button::C => Self::C,
            Button::Start => Self::Start,
        }
    }
}

impl InputScript {
    pub fn parse(json: &str) -> Result<Self> {
        let script: Self = serde_json::from_str(json)?;
        for hold in &script.holds {
            if hold.start >= hold.end {
                return Err(HostError::Input("hold must have start < end".into()));
            }
            if !script.p2_connected && !hold.p2.is_empty() {
                return Err(HostError::Input(
                    "P2 input requires p2_connected=true".into(),
                ));
            }
        }
        Ok(script)
    }

    pub fn at(&self, frame: u64) -> GenesisInputs {
        let mut inputs = GenesisInputs {
            p1: GenesisController::new(GenesisControllerType::ThreeButton),
            p2: if self.p2_connected {
                GenesisController::new(GenesisControllerType::ThreeButton)
            } else {
                GenesisController::None
            },
        };
        for hold in self
            .holds
            .iter()
            .filter(|hold| hold.start <= frame && frame < hold.end)
        {
            for (names, player) in [(&hold.p1, Player::One), (&hold.p2, Player::Two)] {
                for &button in names {
                    inputs.set_field(button.into(), player, true);
                }
            }
        }
        inputs
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scripts_are_strict_and_half_open() {
        let script =
            InputScript::parse(r#"{"holds":[{"start":2,"end":4,"p1":["start"]}]}"#).unwrap();
        assert_ne!(script.at(1), script.at(2));
        assert_eq!(script.at(1), script.at(4));
        assert!(InputScript::parse(r#"{"holds":[{"start":1,"end":2,"p1":["typo"]}]}"#).is_err());
        assert!(InputScript::parse(r#"{"holds":[{"start":2,"end":2}]}"#).is_err());
        assert!(InputScript::parse(r#"{"holds":[{"start":1,"end":2,"p2":["a"]}]}"#).is_err());
    }
}
