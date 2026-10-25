//! Presentation-only extras shared by the native and browser GPU frontends.
//! Nothing here can access an engine, cartridge, controller, or emulated clock.

use serde::{Deserialize, Serialize};

pub const EFFECTS_WGSL: &str = include_str!("effects.wgsl");

/// Stable GPU/JavaScript IDs. Missing settings always select the faithful path.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(feature = "cli", derive(clap::ValueEnum))]
#[serde(rename_all = "kebab-case")]
#[repr(u32)]
pub enum DisplayShader {
    #[default]
    Off = 0,
    Smooth = 1,
    Crt = 2,
    CrtCurved = 3,
}

impl DisplayShader {
    pub const ALL: [Self; 4] = [Self::Off, Self::Smooth, Self::Crt, Self::CrtCurved];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Off => "Off (original display)",
            Self::Smooth => "Smooth (soft bilinear)",
            Self::Crt => "CRT (flat)",
            Self::CrtCurved => "CRT (curved)",
        }
    }

    pub const fn from_id(id: u32) -> Option<Self> {
        match id {
            0 => Some(Self::Off),
            1 => Some(Self::Smooth),
            2 => Some(Self::Crt),
            3 => Some(Self::CrtCurved),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_are_stable_and_default_off() {
        assert_eq!(DisplayShader::default(), DisplayShader::Off);
        for (id, preset) in DisplayShader::ALL.into_iter().enumerate() {
            assert_eq!(preset as usize, id);
            assert_eq!(DisplayShader::from_id(id as u32), Some(preset));
            let serialized = serde_json::to_string(&preset).unwrap();
            assert_eq!(
                serde_json::from_str::<DisplayShader>(&serialized).unwrap(),
                preset
            );
        }
        assert_eq!(DisplayShader::from_id(4), None);
        assert_eq!(DisplayShader::from_id(u32::MAX), None);
    }
}
