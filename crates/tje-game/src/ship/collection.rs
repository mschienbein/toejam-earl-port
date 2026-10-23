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
    let revision = context.revision();
    let player_number = arguments[0] as i16;
    let mode = ram::g_mode2_reg_softcopy(revision);
    crate::call(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
    let blanked = mode.read(context) & 0x3c;
    mode.write(context, blanked);
    context.write_vdp_control(0x8104 | u16::from(mode.read(context)));
    crate::call(
        context,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        &[rom::palette_title_screen_bank1(revision).address(), 1],
    )?;
    for page in 0..4 {
        for row in 0..8 {
            context
                .write_vdp_command(0x5000_0003_u32.wrapping_add(((page * 8 + row) as u32) << 23));
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
    }
    for row in 0..28 {
        for column in 0..40 {
            if row == 0 || row == 27 || column == 0 || column == 39 {
                let mut tile = (crate::call(context, ids::GET_RANDOM_NUMBER, &[])? as u16 & 3) + 41;
                if crate::call(context, ids::GET_RANDOM_NUMBER, &[])? & 1 != 0 {
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
    ram::g_frame_counter(revision).write(context, 0);
    ram::g_ship_background_animation_timer(revision).write(context, 0);
    ram::g_ship_screen_state(revision)
        .background_frame()
        .write(context, 1);
    crate::call(context, ids::ANIMATE_TRANSITION_BACKGROUND, &[20])?;
    let enabled = mode.read(context) | 0x40;
    mode.write(context, enabled);
    context.write_vdp_control(0x8104 | u16::from(mode.read(context)));
    crate::call(context, ids::PLAY_SOUND, &[6, 99])?;
    crate::call(context, ids::AUDIO_REQUEST_CHANNEL_TEMPLATE_EFFECT, &[0])?;
    crate::call(context, ids::WAIT_FOR_NEXT_V_BLANK, &[1])?;
    ram::g_current_music_selection(revision).write(context, 6);
    ram::g_requested_music_selection(revision).write(context, 6);
    crate::call(context, ids::AUDIO_REQUEST_FM_CHANNEL_RELEASE, &[])?;
    crate::call(context, ids::MUSIC_REQUEST_TRACK_POINTER_RESET, &[])?;
    let clap = rom::sfx_jam_clap(revision).address();
    crate::call(context, ids::PLAY_SOUND_EFFECT, &[clap, 90, 0])?;
    let mut sound_stage = 1_i16;
    while ram::g_frame_counter(revision).read(context) < 480 {
        crate::call(context, ids::ANIMATE_TRANSITION_BACKGROUND, &[20])?;
        if (sound_stage == 1 && ram::g_frame_counter(revision).read(context) > 20)
            || (sound_stage == 2 && ram::g_pcm_effect_cooldown_ticks(revision).read(context) == 0)
            || (sound_stage == 3 && ram::g_frame_counter(revision).read(context) > 60)
        {
            crate::call(context, ids::PLAY_SOUND_EFFECT, &[clap, 90, 0])?;
            sound_stage = sound_stage.wrapping_add(1);
        } else if sound_stage == 4 && ram::g_pcm_effect_cooldown_ticks(revision).read(context) == 0
        {
            crate::call(
                context,
                ids::PLAY_SOUND_EFFECT,
                &[rom::sfx_hallelujah(revision).address(), 92, 1],
            )?;
            sound_stage = sound_stage.wrapping_add(1);
        } else if sound_stage == 5 && ram::g_pcm_effect_cooldown_ticks(revision).read(context) == 0
        {
            crate::call(
                context,
                ids::PLAY_SOUND_EFFECT,
                &[rom::sfx_jammin(revision).address(), 90, 0],
            )?;
            sound_stage = sound_stage.wrapping_add(1);
        } else if sound_stage == 6 && ram::g_pcm_effect_cooldown_ticks(revision).read(context) == 0
        {
            ram::g_requested_music_selection(revision).write(context, 0);
        }
    }
    let blanked = mode.read(context) & 0x3c;
    mode.write(context, blanked);
    context.write_vdp_control(0x8104 | u16::from(mode.read(context)));
    crate::call(context, ids::RESTORE_GAMEPLAY_GRAPHICS, &[])?;
    let other = i32::from(player_number ^ 1);
    if ram::g_player_inventory_open(revision)
        .at(other)
        .read(context)
        != 0
    {
        crate::call(
            context,
            ids::INVENTORY_DEACTIVATE_DISPLAY_ENTITIES,
            &[other as u32],
        )?;
        ram::g_player_inventory_open(revision)
            .at(other)
            .write(context, 0);
    }
    crate::call(context, ids::SELECT_SPRITE_CHAIN_START, &[0])?;
    ram::g_split_screen_enabled(revision).write(context, 0);
    crate::call(context, ids::HUD_HIDE_PLAYER_SPRITES, &[2])?;
    ram::g_player_powerups(revision).at(0).write(context, -1);
    ram::g_player2_powerup(revision).write(context, -1);
    context.write_vdp_control(0x9100);
    context.write_vdp_control(0x9200);
    crate::call(context, ids::MAP_CLEAR_ACTIVE_LEVEL_OBJECTS, &[])?;
    crate::call(context, ids::PLAY_ENDING_SEQUENCE, &[])?;
    crate::call(context, ids::MAP_ENTER_FINAL_LEVEL, &[])?;
    Ok(None)
}

/// G/src/ship/ship_screen_and_collection.c:584 (`ShipPieceAbility`).
/// Preserve the omitted speaker's saved incoming D2 word and the exhausted
/// ten-piece search result; do not turn either original defect into a guard.
pub fn ship_piece_ability(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let omitted_speaker = context.data_register(2);
    let player_number = arguments[0] as i16;
    let ship_piece = Address::<EarthlingState>::new(arguments[1]);
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
    crate::call(context, ids::FREE_ENTITY, &[number as u32])?;
    ship_piece.kind().write(context, -1);
    ship_piece.object().write(context, 0);
    crate::call(
        context,
        ids::SHOW_SHIP_PIECES,
        &[
            i32::from(piece_number) as u32,
            i32::from(player_number) as u32,
        ],
    )?;
    if player_number == 0 || ram::g_split_screen_enabled(revision).read(context) == 0 {
        crate::call(context, ids::START_DIALOGUE, &[2, omitted_speaker])?;
    } else {
        let text = rom_symbol(revision, "GameText_020D36")
            .expect("generated ship collection text")
            .address;
        crate::call(context, ids::GENERATE_SPEECH, &[1, text, 1])?;
    }
    Ok(None)
}

/// G/src/ship/ship_screen_and_collection.c:652 (`ShowRandomUncollectedShipPiece`).
/// The all-zero assignment case intentionally retains its endless original loop.
pub fn show_random_uncollected_ship_piece(
    context: &mut dyn Context,
    _arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let mut piece = (crate::call(context, ids::GET_RANDOM_NUMBER, &[])? & 255) as i16;
    loop {
        piece = ((i32::from(piece) + 1) % 10) as i16;
        if ram::g_ship_piece_level_assignments(revision)
            .at(i32::from(piece))
            .read(context)
            != 0
        {
            break;
        }
    }
    crate::call(
        context,
        ids::SHOW_SHIP_PIECES,
        &[i32::from(piece) as u32, 2],
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
