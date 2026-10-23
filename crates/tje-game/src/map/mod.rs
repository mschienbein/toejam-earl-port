use crate::{Context, Result, call, ids};
use tje_mem::ram;

/// G/src/map/random_regions_and_cleanup.c:11, the Schrage-split Park-Miller step.
/// Noncanonical seeds retain the original wrapping 32-bit intermediate arithmetic.
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
/// G/src/map/random_regions_and_cleanup.c:28, including minimum-signed negation.
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
/// G/src/map/random_regions_and_cleanup.c:11 (`GetRandomNumber`).
pub fn get_random_number(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let state = ram::g_rng_state(context.revision());
    let next = park_miller(state.read(context));
    state.write(context, next);
    Ok(Some(u32::from(next as u16)))
}
/// G/src/map/random_regions_and_cleanup.c:28 (`StackBasedRandom`).
pub fn stack_based_random(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let state = ram::g_rng_state(context.revision());
    let previous = state.read(context);
    state.write(context, normalize_seed(arguments[0] as i32));
    Ok(Some(previous as u32))
}
/// G/src/map/random_regions_and_cleanup.c:45 (`PreGenerateRandoms`). Rust callees
/// are dispatched by identity; a hybrid-only missing callee enters the real ROM.
pub fn pre_generate_randoms(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let saved = ram::g_rng_state(revision).read(context);
    for index in 0..59 {
        let value = call(context, ids::GET_RANDOM_NUMBER, &[])?;
        ram::g_cached_random_values(revision)
            .at(index)
            .write(context, value as u16);
    }
    ram::g_random_cache_index(revision).write(context, 0);
    ram::g_rng_state(revision).write(context, saved);
    Ok(None)
}

/// Available Rust entries owned by this subsystem.
pub static TRANSLATIONS: &[crate::Translation] = &[
    crate::Translation {
        id: crate::ids::GET_RANDOM_NUMBER,
        function: get_random_number,
    },
    crate::Translation {
        id: crate::ids::STACK_BASED_RANDOM,
        function: stack_based_random,
    },
    crate::Translation {
        id: crate::ids::PRE_GENERATE_RANDOMS,
        function: pre_generate_randoms,
    },
];

/// Reviewed original entries that return without clock/interrupt progress.
pub static ATOMIC_CAPABLE: &[crate::FunctionId] = &[
    crate::ids::GET_RANDOM_NUMBER,
    crate::ids::STACK_BASED_RANDOM,
    crate::ids::PRE_GENERATE_RANDOMS,
];
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
        assert_eq!(normalize_seed(i32::MIN), i32::MIN);
        assert_eq!(normalize_seed(i32::MAX), 1);
        assert_eq!(normalize_seed(-12345), 12345);
    }
}
