use crate::{
    HostError, Result,
    abi::HookContext,
    catalogue::{self, FunctionCatalogue, FunctionSymbol},
};
use std::{collections::BTreeMap, sync::Arc};

pub type HookFn = fn(&mut HookContext<'_>) -> Result<Option<u32>>;

#[derive(Clone)]
pub struct Hook {
    pub symbol: Arc<FunctionSymbol>,
    pub returns_value: bool,
    pub function: HookFn,
}

#[derive(Default, Clone)]
pub struct HookRegistry {
    hooks: BTreeMap<u32, Hook>,
}

impl HookRegistry {
    pub fn register(
        &mut self,
        catalogue: &FunctionCatalogue,
        name: &str,
        returns_value: bool,
        function: HookFn,
    ) -> Result<()> {
        let symbol = catalogue
            .named(name)
            .ok_or_else(|| HostError::Metadata(format!("function not catalogued: {name}")))?
            .clone();
        if self.hooks.contains_key(&symbol.address) {
            return Err(HostError::Metadata(format!(
                "duplicate hook at {:06X}",
                symbol.address
            )));
        }
        self.hooks.insert(
            symbol.address,
            Hook {
                symbol: Arc::new(symbol),
                returns_value,
                function,
            },
        );
        Ok(())
    }

    pub fn at(&self, address: u32) -> Option<&Hook> {
        self.hooks.get(&address)
    }
    pub fn len(&self) -> usize {
        self.hooks.len()
    }
    pub fn is_empty(&self) -> bool {
        self.hooks.is_empty()
    }

    pub fn demo(catalogue: &FunctionCatalogue) -> Result<Self> {
        let mut registry = Self::default();
        registry.register(catalogue, "GetRandomNumber", true, get_random_number)?;
        registry.register(catalogue, "StackBasedRandom", true, stack_based_random)?;
        registry.register(
            catalogue,
            "DisableBlankingGraphics",
            false,
            disable_blanking_graphics,
        )?;
        Ok(registry)
    }

    /// Separate demonstration: GetRandomNumber is deliberately NOT translated
    /// in this registry. A Rust PreGenerateRandoms calls its original ROM body.
    pub fn nested_demo(catalogue: &FunctionCatalogue) -> Result<Self> {
        let mut registry = Self::default();
        registry.register(catalogue, "PreGenerateRandoms", false, pre_generate_randoms)?;
        Ok(registry)
    }
}

/// Park-Miller with Schrage's split, including the 68000's 32-bit wrapping
/// arithmetic for noncanonical seeds. No emulated compiler-runtime calls.
pub fn park_miller(state: i32) -> i32 {
    let high = state / 127_773;
    let low = state % 127_773;
    let next = 16_807_i32
        .wrapping_mul(low)
        .wrapping_sub(2_836_i32.wrapping_mul(high));
    if next > 0 {
        next
    } else {
        next.wrapping_add(i32::MAX)
    }
}

pub fn normalize_seed(mut seed: i32) -> i32 {
    if seed < 0 {
        seed = seed.wrapping_neg();
    }
    if seed == i32::MAX {
        seed = seed.wrapping_sub(i32::MAX);
    }
    if seed == 0 {
        seed = 1;
    }
    seed
}

pub fn get_random_number(context: &mut HookContext<'_>) -> Result<Option<u32>> {
    let next = park_miller(context.read_u32(catalogue::RNG_STATE) as i32);
    context.write_u32(catalogue::RNG_STATE, next as u32);
    Ok(Some(u32::from(next as u16)))
}

pub fn stack_based_random(context: &mut HookContext<'_>) -> Result<Option<u32>> {
    let previous = context.read_u32(catalogue::RNG_STATE);
    let seed = normalize_seed(context.argument_u32(0) as i32);
    context.write_u32(catalogue::RNG_STATE, seed as u32);
    Ok(Some(previous))
}

pub fn disable_blanking_graphics(context: &mut HookContext<'_>) -> Result<Option<u32>> {
    context.write_u8(0xFF8012, 0);
    context.write_u8(0xFF8016, 0);
    Ok(None)
}

pub fn pre_generate_randoms(context: &mut HookContext<'_>) -> Result<Option<u32>> {
    let saved = context.read_u32(catalogue::RNG_STATE);
    for index in 0..59 {
        let value = context.call_original(catalogue::RANDOM, &[])?;
        context.write_u16(0xFF916A + 2 * index, value as u16);
    }
    context.write_u16(0xFF91E0, 0);
    context.write_u32(catalogue::RNG_STATE, saved);
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_random_semantics() {
        let mut state = 1;
        for expected in [
            16_807,
            282_475_249,
            1_622_650_073,
            984_943_658,
            1_144_108_930,
        ] {
            state = park_miller(state);
            assert_eq!(state, expected);
        }
        assert_eq!(normalize_seed(i32::MIN), i32::MIN); // original overflow bug
        assert_eq!(normalize_seed(i32::MAX), 1);
        assert_eq!(normalize_seed(-12345), 12345);
    }
    #[test]
    fn registry_rejects_nonfunctions_and_duplicates() {
        let catalogue = FunctionCatalogue::rev02_demo();
        let mut registry = HookRegistry::demo(&catalogue).unwrap();
        assert!(
            registry
                .register(&catalogue, "gRNGState", true, get_random_number)
                .is_err()
        );
        assert!(
            registry
                .register(&catalogue, "GetRandomNumber", true, get_random_number)
                .is_err()
        );
    }
}
