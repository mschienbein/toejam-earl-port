//! Public symbol metadata only; no ROM contents. REV 02 identities come from
//! decomp `config/rev02/{catalogue,names}.json` (2026-10-23).

use crate::{HostError, Result};
use serde::Deserialize;
use std::collections::BTreeMap;

pub const RNG_STATE: u32 = 0xFF91E2;
pub const FRAME_COUNTER: u32 = 0xFF800E;
pub const STACK_BOTTOM: u32 = 0xFF0000;
// The first allocated global, gMode2RegSoftcopy, is at FF8000. The cartridge
// reset SP is validated against this boundary at load time.
pub const STACK_TOP: u32 = 0xFF8000;
pub const RANDOM: u32 = 0x004E7E;
pub const SEED_RANDOM: u32 = 0x004EE8;
pub const DISABLE_GRAPHICS: u32 = 0x00054C;
pub const PREGENERATE_RANDOMS: u32 = 0x004F16;
pub const MAIN_LOOP: u32 = 0x0014E6;
pub const MAIN_WAIT: u32 = 0x00157A;
pub const IDLE_RANDOM_RETURN: u32 = 0x001582;
pub const NEW_GAME_MENU: u32 = 0x02342A;
pub const INTRO: u32 = 0x027B44;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionSymbol {
    pub address: u32,
    pub size: u32,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct FunctionCatalogue {
    functions: BTreeMap<u32, FunctionSymbol>,
}

impl FunctionCatalogue {
    pub fn rev02_demo() -> Self {
        let functions = [
            (DISABLE_GRAPHICS, 14, "DisableBlankingGraphics"),
            (RANDOM, 106, "GetRandomNumber"),
            (SEED_RANDOM, 46, "StackBasedRandom"),
            (PREGENERATE_RANDOMS, 58, "PreGenerateRandoms"),
        ]
        .into_iter()
        .map(|(address, size, name)| {
            (
                address,
                FunctionSymbol {
                    address,
                    size,
                    name: name.into(),
                },
            )
        })
        .collect();
        Self { functions }
    }

    /// Load public metadata from a user's decomp checkout, without importing
    /// its disassembly, ROM hashes, source text, or data tables.
    pub fn from_decomp(catalogue_json: &str, names_json: &str) -> Result<Self> {
        #[derive(Deserialize)]
        struct Catalogue {
            version: String,
            functions: Vec<Entry>,
        }
        #[derive(Deserialize)]
        struct Entry {
            address: u32,
            size: u32,
            kind: String,
        }
        #[derive(Deserialize)]
        struct Names {
            version: String,
            symbols: Vec<Name>,
        }
        #[derive(Deserialize)]
        struct Name {
            address: String,
            name: String,
            kind: String,
        }
        let catalogue: Catalogue = serde_json::from_str(catalogue_json)?;
        let names: Names = serde_json::from_str(names_json)?;
        if catalogue.version != "rev02" || names.version != "rev02" {
            return Err(HostError::Metadata(
                "hybrid hooks require REV 02 metadata".into(),
            ));
        }
        let mut by_address = BTreeMap::new();
        for symbol in names.symbols.into_iter().filter(|s| s.kind == "function") {
            let address = u32::from_str_radix(symbol.address.trim_start_matches("0x"), 16)
                .map_err(|e| HostError::Metadata(e.to_string()))?;
            by_address.insert(address, symbol.name);
        }
        let functions = catalogue
            .functions
            .into_iter()
            .filter(|e| e.kind == "function")
            .map(|e| {
                (
                    e.address,
                    FunctionSymbol {
                        address: e.address,
                        size: e.size,
                        name: by_address
                            .remove(&e.address)
                            .unwrap_or_else(|| format!("func_{:06x}", e.address)),
                    },
                )
            })
            .collect();
        Ok(Self { functions })
    }

    pub fn at(&self, address: u32) -> Option<&FunctionSymbol> {
        self.functions.get(&address)
    }

    pub fn named(&self, name: &str) -> Option<&FunctionSymbol> {
        self.functions.values().find(|symbol| symbol.name == name)
    }
}
