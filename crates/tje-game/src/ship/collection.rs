use crate::call_frame::{CallSite, FrameLayout, LocalFrame, OutgoingArguments};
use crate::{Context, Result, ids};
use tje_mem::{Address, EarthlingState, ObjectState, Revision, ram, rom, rom_symbol};

fn player_record(revision: Revision, index: i16) -> Address<ObjectState> {
    // ShipPieceAbility emits ASL.W #7 / ADDA.W, not a full-width record product.
    ram::g_player_info(revision)
        .cast::<ObjectState>()
        .byte_offset(i32::from(index.wrapping_shl(7)))
}

/// G/src/ship/ship_screen_and_collection.c:614 (`ShipUpdatePendingPieceIndicator`).
/// The allocation result narrows to the signed handle byte, including zero: the
/// original does not add an allocation-failure guard before modifying the object.
pub fn ship_update_pending_piece_indicator(
    context: &mut dyn Context,
    _arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let active = ram::g_ship_piece_indicator_entity(revision);
    if active.read(context) != 0 {
        let object = ram::g_player_info(revision).at(i32::from(active.read(context)));
        if i32::from(object.y().read(context))
            - i32::from(ram::g_camera1_pending_y(revision).read(context))
            != 224
        {
            let number = i32::from(active.read(context));
            crate::call(context, ids::FREE_ENTITY, &[number as u32])?;
            active.write(context, 0);
        }
    } else if ram::g_ship_screen_state(revision)
        .pending_piece_notice_level()
        .read(context)
        != 0
    {
        if ram::g_split_screen_enabled(revision).read(context) == 0 {
            let number = crate::call(context, ids::ALLOCATE_ENTITY_WITH_EVICTION, &[0])? as i8;
            active.write(context, number);
            let object = ram::g_player_info(revision).at(i32::from(active.read(context)));
            let x = (i32::from(ram::g_camera1_pending_x(revision).read(context)) + 160) as i16;
            object.x().write(context, x);
            let y = (i32::from(ram::g_camera1_pending_y(revision).read(context)) + 224) as i16;
            object.y().write(context, y);
            object.height().write(context, 112);
            let level = ram::g_player1_map_level(revision).read(context);
            object.level().write(context, level);
            object.width().write(context, 96);
            object.height_pixels().write(context, 32);
            object.sprite_count().write(context, 3);
            object.tile_count().write(context, 48);
            let tile = object.tile().read(context) | 0x8000;
            object.tile().write(context, tile);
            let animation = rom::ship_piece_collection_animation(revision);
            let sequence = animation.sequence().read(context);
            object.sequence().write(context, sequence);
            let frames = animation.frames().read(context);
            object.frames().write(context, frames);
            object.frame_duration().write(context, 1);
            object.frame().write(context, -1);
        }
        ram::g_ship_screen_state(revision)
            .pending_piece_notice_level()
            .write(context, 0);
    }
    Ok(None)
}

/// G/src/ship/ship_screen_and_collection.c:173 (`PlayShipCompletionSequence`).
/// Keep the frame/cooldown-driven sound stages and the final ordinary-returning
/// ending/final-map calls; no completion flag or timing is fabricated.
pub fn play_ship_completion_sequence(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    // LINK -4 / MOVEM D2-D6,A2-A4; both words remain observable by
    // later frame-sensitive scene callees rather than living on the host stack.
    let frame = LocalFrame::enter(context, FrameLayout::link(4, 0x7c, 0x1c))?;
    let result = (|| {
        let revision = context.revision();
        let player_number = arguments[0] as i16;
        let mode = ram::g_mode2_reg_softcopy(revision);
        let dispatcher = context.dispatcher();
        let site =
            |offset| CallSite::relative(ids::PLAY_SHIP_COMPLETION_SEQUENCE, revision, offset);
        let page = frame.local::<i16>(-4);
        let sound_stage = frame.local::<i16>(-2);
        sound_stage.write(context, 1);
        let d3 = (context.data_register(3) & 0xffff_0000) | u32::from(player_number as u16);
        context.set_data_register(3, d3);
        context.set_address_register(4, mode.address());
        context.set_address_register(3, ram::g_frame_counter(revision).address());
        context.set_address_register(2, ram::g_pcm_effect_cooldown_ticks(revision).address());
        let wait = OutgoingArguments::push(context, &[1])?;
        wait.call(context, dispatcher, ids::WAIT_FOR_NEXT_V_BLANK, site(0x2e))?;
        let blanked = mode.read(context) & 0x3c;
        mode.write(context, blanked);
        let command = 0x8104 | u16::from(mode.read(context));
        wait.release(context)?;
        context.write_vdp_control(command);
        dispatcher.call_at(
            context,
            ids::COPY_PALETTE_TO_CRAM_SLOT,
            &[rom::palette_title_screen_bank1(revision).address(), 1],
            site(0x56),
        )?;
        page.write(context, 0);
        loop {
            for row in 0..8 {
                context.write_vdp_command(
                    0x5000_0003_u32
                        .wrapping_add(((i32::from(page.read(context)) * 8 + row) as u32) << 23),
                );
                for _ in 0..6 {
                    for column in 0..8 {
                        let tile = u16::from(
                            rom::transition_background_tile_offsets(revision)
                                .at(row)
                                .at(column)
                                .read(context),
                        ) + 1;
                        context.write_vdp_data(tile.wrapping_add(0xa000));
                    }
                }
            }
            let next = page.read(context).wrapping_add(1);
            page.write(context, next);
            if page.read(context) >= 4 {
                break;
            }
        }
        for row in 0..28 {
            for column in 0..40 {
                if row == 0 || row == 27 || column == 0 || column == 39 {
                    let mut tile =
                        (dispatcher.call_at(context, ids::GET_RANDOM_NUMBER, &[], site(0xec))?
                            as u16
                            & 3)
                            + 41;
                    if dispatcher.call_at(context, ids::GET_RANDOM_NUMBER, &[], site(0xfc))? & 1
                        != 0
                    {
                        tile |= 0x1800;
                    }
                    context.write_vdp_command(
                        0x5000_0003_u32
                            .wrapping_add((row as u32) << 23)
                            .wrapping_add((column as u32) << 17),
                    );
                    context.write_vdp_data(tile.wrapping_add(0x8580));
                }
            }
        }
        context.discard_u16(ram::g_frame_counter(revision).address())?;
        ram::g_frame_counter(revision).write(context, 0);
        context.discard_u8(ram::g_ship_background_animation_timer(revision).address())?;
        ram::g_ship_background_animation_timer(revision).write(context, 0);
        ram::g_ship_screen_state(revision)
            .background_frame()
            .write(context, 1);
        let background = OutgoingArguments::push(context, &[20])?;
        background.call(
            context,
            dispatcher,
            ids::ANIMATE_TRANSITION_BACKGROUND,
            site(0x164),
        )?;
        let enabled = mode.read(context) | 0x40;
        mode.write(context, enabled);
        let command = 0x8104 | u16::from(mode.read(context));
        background.release(context)?;
        context.write_vdp_control(command);
        // The source keeps PlaySound's two slots, then the effect slot, through
        // WaitForNextVBlank before performing the shared sixteen-byte cleanup.
        let noise = OutgoingArguments::push(context, &[6, 99])?;
        noise.call(context, dispatcher, ids::PLAY_SOUND, site(0x18a))?;
        let effect = OutgoingArguments::push(context, &[0])?;
        effect.call(
            context,
            dispatcher,
            ids::AUDIO_REQUEST_CHANNEL_TEMPLATE_EFFECT,
            site(0x192),
        )?;
        let wait = OutgoingArguments::push(context, &[1])?;
        wait.call(context, dispatcher, ids::WAIT_FOR_NEXT_V_BLANK, site(0x19c))?;
        wait.release(context)?;
        effect.release(context)?;
        noise.release(context)?;
        ram::g_current_music_selection(revision).write(context, 6);
        ram::g_requested_music_selection(revision).write(context, 6);
        dispatcher.call_at(
            context,
            ids::AUDIO_REQUEST_FM_CHANNEL_RELEASE,
            &[],
            site(0x1b6),
        )?;
        dispatcher.call_at(
            context,
            ids::MUSIC_REQUEST_TRACK_POINTER_RESET,
            &[],
            site(0x1bc),
        )?;
        let clap = rom::sfx_jam_clap(revision).address();
        dispatcher.call_at(context, ids::PLAY_SOUND_EFFECT, &[clap, 90, 0], site(0x1ce))?;
        while ram::g_frame_counter(revision).read(context) < 480 {
            dispatcher.call_at(
                context,
                ids::ANIMATE_TRANSITION_BACKGROUND,
                &[20],
                site(0x1de),
            )?;
            if (sound_stage.read(context) == 1 && ram::g_frame_counter(revision).read(context) > 20)
                || (sound_stage.read(context) == 2
                    && ram::g_pcm_effect_cooldown_ticks(revision).read(context) == 0)
                || (sound_stage.read(context) == 3
                    && ram::g_frame_counter(revision).read(context) > 60)
            {
                let sound = OutgoingArguments::push(context, &[clap, 90, 0])?;
                sound.call(context, dispatcher, ids::PLAY_SOUND_EFFECT, site(0x21a))?;
                let next = sound_stage.read(context).wrapping_add(1);
                sound_stage.write(context, next);
                sound.release(context)?;
            } else if sound_stage.read(context) == 4
                && ram::g_pcm_effect_cooldown_ticks(revision).read(context) == 0
            {
                let sound = OutgoingArguments::push(
                    context,
                    &[rom::sfx_hallelujah(revision).address(), 92, 1],
                )?;
                sound.call(context, dispatcher, ids::PLAY_SOUND_EFFECT, site(0x21a))?;
                let next = sound_stage.read(context).wrapping_add(1);
                sound_stage.write(context, next);
                sound.release(context)?;
            } else if sound_stage.read(context) == 5
                && ram::g_pcm_effect_cooldown_ticks(revision).read(context) == 0
            {
                let sound = OutgoingArguments::push(
                    context,
                    &[rom::sfx_jammin(revision).address(), 90, 0],
                )?;
                sound.call(context, dispatcher, ids::PLAY_SOUND_EFFECT, site(0x21a))?;
                let next = sound_stage.read(context).wrapping_add(1);
                sound_stage.write(context, next);
                sound.release(context)?;
            } else if sound_stage.read(context) == 6
                && ram::g_pcm_effect_cooldown_ticks(revision).read(context) == 0
            {
                context.discard_u8(ram::g_requested_music_selection(revision).address())?;
                ram::g_requested_music_selection(revision).write(context, 0);
            }
            context.poll_yield()?;
        }
        let blanked = mode.read(context) & 0x3c;
        mode.write(context, blanked);
        context.write_vdp_control(0x8104 | u16::from(mode.read(context)));
        dispatcher.call_at(context, ids::RESTORE_GAMEPLAY_GRAPHICS, &[], site(0x28a))?;
        let other = i32::from(player_number ^ 1);
        if ram::g_player_inventory_open(revision)
            .at(other)
            .read(context)
            != 0
        {
            dispatcher.call_at(
                context,
                ids::INVENTORY_DEACTIVATE_DISPLAY_ENTITIES,
                &[other as u32],
                site(0x2ae),
            )?;
            context.discard_u8(ram::g_player_inventory_open(revision).at(other).address())?;
            ram::g_player_inventory_open(revision)
                .at(other)
                .write(context, 0);
        }
        dispatcher.call_at(context, ids::SELECT_SPRITE_CHAIN_START, &[0], site(0x2ca))?;
        context.discard_u8(ram::g_split_screen_enabled(revision).address())?;
        ram::g_split_screen_enabled(revision).write(context, 0);
        dispatcher.call_at(context, ids::HUD_HIDE_PLAYER_SPRITES, &[2], site(0x2dc))?;
        ram::g_player_powerups(revision).at(0).write(context, -1);
        ram::g_player2_powerup(revision).write(context, -1);
        context.write_vdp_control(0x9100);
        context.write_vdp_control(0x9200);
        dispatcher.call_at(
            context,
            ids::MAP_CLEAR_ACTIVE_LEVEL_OBJECTS,
            &[],
            site(0x304),
        )?;
        dispatcher.call_at(context, ids::PLAY_ENDING_SEQUENCE, &[], site(0x30a))?;
        dispatcher.call_at(context, ids::MAP_ENTER_FINAL_LEVEL, &[], site(0x310))?;
        Ok(None)
    })();
    super::screen::leave_frame(context, frame, result)
}

/// G/src/ship/ship_screen_and_collection.c:584 (`ShipPieceAbility`).
/// Preserve the omitted speaker's saved incoming D2 word and the exhausted
/// ten-piece search result; do not turn either original defect into a guard.
pub fn ship_piece_ability(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let dispatcher = context.dispatcher();
    let site = |offset| CallSite::relative(ids::SHIP_PIECE_ABILITY, revision, offset);
    // G's MOVEM D2,D3,A2 saves the omitted StartDialogue speaker at the
    // first byte of this actual caller frame, not in an invented argument.
    let frame = LocalFrame::enter(context, FrameLayout::movem(0, 0x0c, 0x04))?;
    let result = (|| {
        let player_number = arguments[0] as i16;
        let ship_piece = Address::<EarthlingState>::new(arguments[1]);
        let d2 = (context.data_register(2) & 0xffff_0000) | u32::from(player_number as u16);
        context.set_data_register(2, d2);
        context.set_address_register(2, ship_piece.address());
        if ship_piece.object().read(context) == 0 {
            return Ok(None);
        }
        let player = player_record(revision, player_number);
        let _object_number = ship_piece.object().read(context);
        let mut piece_number = 0_i16;
        while piece_number < 10 {
            if ram::g_ship_piece_level_assignments(revision)
                .at(i32::from(piece_number))
                .read(context)
                == player.level().read(context)
            {
                break;
            }
            piece_number = piece_number.wrapping_add(1);
        }
        let number = i32::from(ship_piece.object().read(context));
        dispatcher.call_at(context, ids::FREE_ENTITY, &[number as u32], site(0x54))?;
        ship_piece.kind().write(context, -1);
        context.discard_u8(ship_piece.object().address())?;
        ship_piece.object().write(context, 0);
        // The original EXT.L immediately before the call makes all of D3 the
        // sign-extended selected piece, including the exhausted-search ten.
        context.set_data_register(3, i32::from(piece_number) as u32);
        dispatcher.call_at(
            context,
            ids::SHOW_SHIP_PIECES,
            &[
                i32::from(piece_number) as u32,
                i32::from(player_number) as u32,
            ],
            site(0x6e),
        )?;
        if player_number == 0 || ram::g_split_screen_enabled(revision).read(context) == 0 {
            dispatcher.call_at(context, ids::START_DIALOGUE, &[2], site(0x86))?;
        } else {
            let text = rom_symbol(revision, "GameText_020D36")
                .expect("generated ship collection text")
                .address;
            dispatcher.call_at(context, ids::GENERATE_SPEECH, &[1, text, 1], site(0x9e))?;
        }
        Ok(None)
    })();
    super::screen::leave_frame(context, frame, result)
}

/// G/src/ship/ship_screen_and_collection.c:652 (`ShowRandomUncollectedShipPiece`).
/// The all-zero assignment case intentionally retains its endless original loop.
pub fn show_random_uncollected_ship_piece(
    context: &mut dyn Context,
    _arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let site =
        |offset| CallSite::relative(ids::SHOW_RANDOM_UNCOLLECTED_SHIP_PIECE, revision, offset);
    // This caller has no prologue; its entry SP is already the original
    // outgoing base before the two ShowShipPieces slots and real return PC.
    let dispatcher = context.dispatcher();
    let mut piece =
        (dispatcher.call_at(context, ids::GET_RANDOM_NUMBER, &[], site(6))? & 255) as i16;
    loop {
        piece = ((i32::from(piece) + 1) % 10) as i16;
        if ram::g_ship_piece_level_assignments(revision)
            .at(i32::from(piece))
            .read(context)
            != 0
        {
            break;
        }
        context.poll_yield()?;
    }
    dispatcher.call_at(
        context,
        ids::SHOW_SHIP_PIECES,
        &[i32::from(piece) as u32, 2],
        site(0x42),
    )?;
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::player_record;
    use tje_mem::{Revision, ram};

    #[test]
    fn every_collection_player_word_keeps_compiled_record_offset_wrap() {
        for revision in [Revision::Rev02, Revision::Rev00] {
            let base = ram::g_player_info(revision).address();
            for bits in 0..=65535_u32 {
                let displacement = ((bits * 128) & 0xffff) as u16 as i16;
                assert_eq!(
                    player_record(revision, bits as u16 as i16).address(),
                    base.wrapping_add(i32::from(displacement) as u32)
                );
            }
            assert_eq!(player_record(revision, 512).address(), base);
            assert_eq!(player_record(revision, -1).address(), base - 128);
        }
    }
}
