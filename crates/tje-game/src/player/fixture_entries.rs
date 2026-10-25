//! Caller-written fixture frames; no argument snapshot is passed to a translation.
use tje_game::{Context, FunctionId, ids};
use tje_host::Host;

pub(super) fn check_argument_root(
    host: &mut Host,
    id: FunctionId,
    arguments: &[u32],
) -> Option<tje_host::Result<u32>> {
    // These are actual source JSR returns, with equal offsets in both revisions.
    let (caller, offset) = match id {
        ids::PLAYER_START_FALLING => (ids::PLAYER_UPDATE_HURT_RECOVERY_AND_IDLE, 0x188),
        ids::MAKE_PLAYER_FALL => (ids::CHECK_PLAYER_FALLING, 0x2fa),
        ids::PLAYER_UPDATE_TERRAIN_SINKING => (ids::PROCESS_PLAYERS, 0xb8),
        ids::PLAYER_UPDATE_HURT_RECOVERY_AND_IDLE => (ids::PROCESS_PLAYERS, 0x60),
        ids::PLAYER_RESOLVE_DESCENDING_TERRAIN => (ids::PROCESS_PLAYERS, 0x78),
        ids::PLAYER_ADVANCE_TO_NEXT_LEVEL => (ids::PLAYER_RELOCATE_TO_PARTNER_OR_NEXT_LEVEL, 0x68),
        ids::PLAYER_RELOCATE_TO_PARTNER_OR_NEXT_LEVEL => (ids::PROCESS_INVENTORY, 0x3ba),
        ids::KILL_THE_PLAYER => (ids::PLAYER_UPDATE_HEALTH_AND_LIVES, 0x7e),
        ids::PLAYER_FINALIZE_LAST_LIFE_EXIT => (ids::PLAYER_UPDATE_DEATH_ANIMATION, 0x8e),
        ids::PLAYER_UPDATE_DEATH_ANIMATION => (ids::PROCESS_PLAYERS, 0xc4),
        ids::PLAYER_CHOOSE_DIRECTIONAL_MOVEMENT => (ids::SPRITE_CHECKS, 0x212),
        ids::SPRITE_CHECKS => (ids::PROCESS_INPUT, 0xaa),
        _ => return None,
    };
    assert_eq!(
        usize::from(id.metadata().abi.argument_count),
        arguments.len()
    );
    let revision = host.revision();
    let mut context = host.context();
    let base = context.stack_pointer().expect("fixture caller stack");
    let mut sp = base;
    for &argument in arguments.iter().rev() {
        sp = sp.wrapping_sub(4);
        context.write_u32(sp, argument);
    }
    sp = sp.wrapping_sub(4);
    context.write_u32(sp, caller.metadata().address(revision) + offset);
    context
        .set_stack_pointer(sp)
        .expect("prepared fixture entry");
    let result = host
        .check_entry_fixture(id.metadata().address(revision))
        .map(|_| 0);
    host.context()
        .set_stack_pointer(base)
        .expect("restore fixture caller stack");
    Some(result)
}
