use super::frames::{call_from_void, with_frame};
use crate::call_frame::{CallSite, FrameLayout, OutgoingArguments};
use crate::{Context, Result, ids};
use tje_mem::{Address, ObjectState, ram};

/// G/src/player/active_present_falling_and_respawn.c:956 (`PlayerFinalizeLastLifeExit`).
/// Matching c_active_present_falling_and_respawn.s:2208-2298: MOVEM D2/A2-A3,
/// no LINK. Actual outgoing areas remain allocated across bank/HUD calls.
/// ShowGameOverAndReset's typed reset propagates through `?` with this live
/// frame intact; the reset path must never execute the surviving-player tail.
/// All return-PC offsets below agree in the two matched revision ELFs.
pub(super) fn player_finalize_last_life_exit(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    with_frame(context, FrameLayout::movem(0, 4, 0x0c), |context, _| {
        let revision = context.revision();
        let caller = ids::PLAYER_FINALIZE_LAST_LIFE_EXIT;
        let dispatcher = context.dispatcher();
        let mut player = Address::<ObjectState>::new(arguments[0]);
        let number = i32::from(arguments[1] as i16);
        let mode = ram::g_mode2_reg_softcopy(revision);
        context.set_address_register(2, player.address());
        context.set_data_register(
            2,
            (context.data_register(2) & 0xffff_0000) | u32::from(number as u16),
        );
        context.set_address_register(3, mode.address());
        call_from_void(context, caller, ids::DISABLE_BLANKING_GRAPHICS, &[], 0x18)?;
        mode.write(context, mode.read(context) & 0x3c);
        let mode_bits = u32::from(mode.read(context));
        context.set_data_register(1, mode_bits);
        context.set_data_register(0, mode_bits | 0x8104);
        context.write_vdp_control((mode_bits | 0x8104) as u16);
        let partner = number ^ 1;
        context.set_data_register(0, partner as u32);
        context.set_data_register(1, 1);
        // Matching compiler line2223 leaves this base live through the lookup.
        context.set_address_register(0, ram::g_player_lives(revision).address());
        if ram::g_player_lives(revision).at(partner).read(context) < 0 {
            call_from_void(
                context,
                caller,
                ids::TERMINATE_SPRITE_CHAIN_AT_HEAD,
                &[],
                0x48,
            )?;
            call_from_void(context, caller, ids::SHOW_GAME_OVER_AND_RESET, &[], 0x4e)?;
        }
        let lives = ram::g_player_lives(revision).at(number);
        lives.write(context, lives.read(context).wrapping_sub(1));
        let level = player.level().read(context) as u32;
        let earthling_arguments = OutgoingArguments::push(context, &[level])?;
        earthling_arguments.call_void(
            context,
            dispatcher,
            ids::EARTHLING_RELEASE_LEVEL_BANK,
            CallSite::relative(caller, revision, 0x6c),
        )?;
        let level = player.level().read(context) as u32;
        let item_arguments = OutgoingArguments::push(context, &[level])?;
        item_arguments.call_void(
            context,
            dispatcher,
            ids::MAP_RELEASE_LEVEL_ITEM_BANK,
            CallSite::relative(caller, revision, 0x7c),
        )?;
        item_arguments.release(context)?;
        earthling_arguments.release(context)?;
        super::frames::clear_byte(context, ram::g_split_screen_enabled(revision).address())?;
        call_from_void(context, caller, ids::SELECT_SPRITE_CHAIN_START, &[0], 0x8c)?;
        player.level().write(context, 99);
        if player.draw_flags().read(context) & 2 != 0 {
            call_from_void(
                context,
                caller,
                ids::RELEASE_ENTITY_VIEWPORT_SPRITES,
                &[player.address(), 0],
                0xa6,
            )?;
        }
        if player.draw_flags().read(context) & 4 != 0 {
            call_from_void(
                context,
                caller,
                ids::RELEASE_ENTITY_VIEWPORT_SPRITES,
                &[player.address(), 1],
                0xbc,
            )?;
        }
        super::frames::clear_byte(context, player.draw_flags().address())?;
        ram::g_player_rank(revision).at(number).write(context, 9);
        let hud_arguments = OutgoingArguments::push(context, &[number as u32])?;
        hud_arguments.call_void(
            context,
            dispatcher,
            ids::HUD_HIDE_PLAYER_SPRITES,
            CallSite::relative(caller, revision, 0xde),
        )?;
        // EXT.L/EOR.L/ASL.L for the partner, not the short-index word scale.
        player = ram::g_player_info(revision).at(number ^ 1);
        context.set_address_register(2, player.address());
        let level = player.level().read(context) as u32;
        let map_arguments = OutgoingArguments::push(context, &[0, level])?;
        map_arguments.call_void(
            context,
            dispatcher,
            ids::MAP_LOAD_PLAYER_LEVEL,
            CallSite::relative(caller, revision, 0x104),
        )?;
        let split_arguments = OutgoingArguments::push(context, &[1])?;
        split_arguments.call_void(
            context,
            dispatcher,
            ids::UPDATE_SCREEN_SPLIT_MODE,
            CallSite::relative(caller, revision, 0x10e),
        )?;
        call_from_void(context, caller, ids::DRAW_HUD, &[], 0x114)?;
        mode.write(context, mode.read(context) | 0x40);
        split_arguments.release(context)?;
        map_arguments.release(context)?;
        hud_arguments.release(context)?;
        context.write_vdp_control(0x8104 | u16::from(mode.read(context)));
        call_from_void(context, caller, ids::ENABLE_BLANKING_GRAPHICS, &[], 0x134)?;
        Ok(None)
    })
}
