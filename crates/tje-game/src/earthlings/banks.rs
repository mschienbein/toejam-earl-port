use crate::{Context, Result, call, ids};
use tje_mem::{Address, EarthlingDefinition, ram, rom};

/// G/src/earthlings/spawning_and_banks.c:53 (`CheckEarthlingOverlap`).
/// Atomic: elevator predicate and twenty ROM-definition/RAM-record comparisons.
pub fn check_earthling_overlap(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let definition = Address::<EarthlingDefinition>::new(arguments[0]);
    let bank = arguments[1] as i16;
    let x = arguments[2] as i16;
    let y = arguments[3] as i16;
    let level = arguments[4] as i16;
    if call(
        context,
        ids::ELEVATOR_ALLOWS_WORLD_PLACEMENT,
        &[x as u32, y as u32, level as u32],
    )? == 0
    {
        return Ok(Some(0));
    }
    let revision = context.revision();
    for slot in 0..20 {
        let earthling = ram::g_earthling_banks(revision)
            .at(i32::from(bank))
            .at(slot);
        let kind = earthling.kind().read(context);
        if kind != -1 {
            let other = rom::earthling_definitions(revision).at(i32::from(kind));
            let mut clearance = i16::from(other.radius().read(context)) << 2;
            clearance = clearance.wrapping_add(i16::from(definition.radius().read(context)) << 2);
            if i32::from(clearance)
                > (i32::from(x) - i32::from(earthling.x().read(context))).wrapping_abs()
                && i32::from(clearance)
                    > (i32::from(y) - i32::from(earthling.y().read(context))).wrapping_abs()
            {
                return Ok(Some(0));
            }
        }
    }
    Ok(Some(1))
}

/// G/src/earthlings/mailbox_earthling.c:24 (`EarthlingConfigureMailboxLevelBank`).
/// Atomic: direct RNG and mailbox RAM; biting mode deliberately retains contacts.
pub fn earthling_configure_mailbox_level_bank(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let mailbox = ram::g_mailbox_level_states(revision).at(i32::from(arguments[0] as i16));
    let level = arguments[1] as i16;
    if level >= 2 {
        mailbox.level().write(context, level as i8);
        if level >= 4
            && level != 12
            && (call(context, ids::GET_RANDOM_NUMBER, &[])? as u16) % 100 < 50
        {
            mailbox.mode().write(context, 1);
        } else {
            mailbox.mode().write(context, 0);
            mailbox.contact().at(0).write(context, 0);
            mailbox.contact().at(1).write(context, 0);
        }
        Ok(Some(1))
    } else {
        mailbox.level().write(context, 0);
        Ok(Some(0))
    }
}
