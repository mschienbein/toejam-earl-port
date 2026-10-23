//! Shared revision-selected ROM symbol views; game text stays in the user's ROM.
use crate::{Context, Error, Result};
use tje_mem::rom_symbol;

pub(crate) fn literal(context: &dyn Context, name: &str) -> Result<u32> {
    rom_symbol(context.revision(), name)
        .map(|symbol| symbol.address)
        .ok_or_else(|| Error::Platform(format!("missing UI ROM symbol {name}")))
}
