//! Opt-in original-bug repairs. Faithful bodies and their registry stay untouched.
use serde::{Deserialize, Serialize};

pub const VERSION: u32 = 1;
pub const PROFILE_ID: &str = "fixed/1";
pub const ALL: u32 = (1 << 20) - 1;
pub const PLACEMENT: u32 = (1 << 2) | (1 << 3) | (1 << 4);

#[derive(Debug, Clone, Copy)]
pub struct Fix {
    pub id: &'static str,
    pub title: &'static str,
    pub rev00_only: bool,
}
pub const MANIFEST: &[Fix] = &[
    Fix {
        id: "B017",
        title: "Bound rank recomputation",
        rev00_only: false,
    },
    Fix {
        id: "B018",
        title: "Finish ineligible health bars",
        rev00_only: false,
    },
    Fix {
        id: "B004",
        title: "Bound placement searches (linked B004–B006)",
        rev00_only: false,
    },
    Fix {
        id: "B005",
        title: "Retain player position on failed placement",
        rev00_only: false,
    },
    Fix {
        id: "B006",
        title: "Propagate item/telephone placement failure",
        rev00_only: false,
    },
    Fix {
        id: "B008",
        title: "Decline an exhausted Earthling bank",
        rev00_only: false,
    },
    Fix {
        id: "B011",
        title: "Free only acquired sprite tiles",
        rev00_only: false,
    },
    Fix {
        id: "B014",
        title: "Use the promoted player's speaker number",
        rev00_only: false,
    },
    Fix {
        id: "B007",
        title: "Pass the small-item recovery flag",
        rev00_only: false,
    },
    Fix {
        id: "B015",
        title: "Supply reunion/collection speakers",
        rev00_only: false,
    },
    Fix {
        id: "B001",
        title: "Clamp both rocket-skate axes",
        rev00_only: false,
    },
    Fix {
        id: "B002",
        title: "Cap and conserve shared health",
        rev00_only: false,
    },
    Fix {
        id: "R001",
        title: "REV 02 falling-skate and active-pose guards",
        rev00_only: true,
    },
    Fix {
        id: "R002",
        title: "REV 02 whirlwind terrain protection",
        rev00_only: true,
    },
    Fix {
        id: "R003",
        title: "REV 02 choice-prompt sleep gate",
        rev00_only: true,
    },
    Fix {
        id: "R004",
        title: "REV 02 map idle reset",
        rev00_only: true,
    },
    Fix {
        id: "R005",
        title: "REV 02 inventory idle reset",
        rev00_only: true,
    },
    Fix {
        id: "R006",
        title: "REV 02 title counter reset",
        rev00_only: true,
    },
    Fix {
        id: "R007",
        title: "REV 02 Sega counter reset",
        rev00_only: true,
    },
    Fix {
        id: "R008",
        title: "REV 02 lean terrain-motion gate",
        rev00_only: true,
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub enabled: bool,
    pub fixes: u32,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            fixes: ALL,
        }
    }
}
impl Settings {
    pub fn contains(self, id: &str) -> bool {
        MANIFEST
            .iter()
            .position(|fix| fix.id == id)
            .is_some_and(|index| self.fixes & (1 << index) != 0)
    }
    /// B004–B006 are one checked-result cutover: no unsafe half-migration.
    pub fn set(&mut self, id: &str, enabled: bool) -> Result<(), &'static str> {
        let index = MANIFEST
            .iter()
            .position(|fix| fix.id == id)
            .ok_or("unknown fixed-profile fix id")?;
        let bit = 1 << index;
        let mask = if bit & PLACEMENT != 0 { PLACEMENT } else { bit };
        if enabled {
            self.fixes |= mask;
        } else {
            self.fixes &= !mask;
        }
        Ok(())
    }
    pub fn valid(self) -> bool {
        self.fixes & !ALL == 0 && matches!(self.fixes & PLACEMENT, 0 | PLACEMENT)
    }
}

#[cfg(feature = "enhanced-mode")]
mod overrides;
#[cfg(feature = "enhanced-mode")]
pub use overrides::new_mode;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn profile_is_default_off_with_a_versioned_complete_manifest() {
        assert!(!Settings::default().enabled);
        assert_eq!(MANIFEST.len(), 20);
        assert_eq!(PROFILE_ID, format!("fixed/{VERSION}"));
        for fix in MANIFEST {
            assert!(Settings::default().contains(fix.id));
        }
        let mut settings = Settings::default();
        settings.set("B006", false).unwrap();
        assert!(settings.valid());
        for id in ["B004", "B005", "B006"] {
            assert!(!settings.contains(id));
        }
        settings.set("B004", true).unwrap();
        assert_eq!(settings.fixes, ALL);
        assert!(settings.set("P001", false).is_err());
        assert!(
            !Settings {
                fixes: ALL | (1 << 25),
                ..settings
            }
            .valid()
        );
    }
}
