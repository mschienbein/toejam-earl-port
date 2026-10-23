use crate::{Context, Result, call, ids};
use tje_mem::ram;

/// G/src/jamout/jamout.c:509 (`JamOutReleaseCharacterCopies`, REV 02 0x03B6D2).
/// Keep the redundant original handle reload before FreeEntity; the arrays are
/// not cleared, and each ToeJam release precedes Earl's test for the same slot.
pub fn release_character_copies(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    for slot in 0..9 {
        let toejam = ram::g_jam_toejam_entities(revision).at(slot);
        if toejam.read(context) != 0 {
            let _object_number = toejam.read(context);
            let handle = i32::from(toejam.read(context)) as u32;
            call(context, ids::FREE_ENTITY, &[handle])?;
        }
        let earl = ram::g_jam_earl_entities(revision).at(slot);
        if earl.read(context) != 0 {
            let _object_number = earl.read(context);
            let handle = i32::from(earl.read(context)) as u32;
            call(context, ids::FREE_ENTITY, &[handle])?;
        }
    }
    Ok(None)
}
