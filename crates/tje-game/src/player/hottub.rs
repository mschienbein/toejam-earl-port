use super::frames::{
    advance_random_index, call_from, clear_byte, clear_word, read_long, with_frame,
};
use crate::call_frame::FrameLayout;
use crate::{Context, Result, ids};
use tje_mem::{Address, EarthlingState, ram, rom};

fn tub_speech(
    context: &mut dyn Context,
    player: u32,
    literal_offset: u32,
    attached: u32,
) -> Result<()> {
    let revision = context.revision();
    // G/build/rev02/asm/c_hottub_ability.s:307-330: six separately emitted
    // source literal labels after the catalogued code extent, also in REV 00.
    // Only addresses are represented here; all text is read from the user's ROM.
    let function = ids::HOTTUB_ABILITY.metadata();
    let text = function
        .address(revision)
        .wrapping_add(function.size(revision))
        .wrapping_add(literal_offset);
    let return_offset = match literal_offset {
        0 => 0x1fc,
        22 => 0x292,
        6 | 14 | 28 | 36 => 0x2f8,
        _ => unreachable!("source literal offset"),
    };
    call_from(
        context,
        ids::HOTTUB_ABILITY,
        ids::GENERATE_SPEECH,
        &[player, text, attached],
        return_offset,
    )?;
    Ok(())
}

/// G/src/player/hottub_ability.c:19 (`HottubAbility`).
pub(super) fn hottub_ability(context: &mut dyn Context) -> Result<Option<u32>> {
    // G/build/{rev00,rev02}/asm/c_hottub_ability.s:4-304. The
    // jump-windup group and giggle choices use byte BTST memory operands.
    with_frame(
        context,
        FrameLayout::movem(0, 0x3c, 0x1c),
        |context, stack| {
            let number = i32::from(stack.argument_word(0).read(context));
            let earthling = Address::<EarthlingState>::new(stack.argument_slot(1).read(context));
            let revision = context.revision();
            let player = super::indexing::player_by_word_index(revision, number);
            let mut sprite = -1_i16;
            if ram::g_player_powerups(revision).at(number).read(context) != 1 && {
                let difference = i32::from(player.y().read(context))
                    - (i32::from(earthling.y().read(context)) - 4);
                (call_from(
                    context,
                    ids::HOTTUB_ABILITY,
                    ids::ABS,
                    &[difference as u32],
                    0x5c,
                )? as i32)
                    < 5
            } {
                let counter = ram::g_player_jump_counters(revision).at(number);
                if player.sprite_state().read(context) == 11
                    && i32::from(earthling.x().read(context)) - i32::from(player.x().read(context))
                        > 34
                {
                    sprite = 31;
                    clear_byte(context, counter.address())?;
                } else if player.sprite_state().read(context) == 10
                    && i32::from(player.x().read(context)) - i32::from(earthling.x().read(context))
                        > 34
                {
                    sprite = 30;
                    clear_byte(context, counter.address())?;
                } else if Address::<u8>::new(player.address() + 85).read(context) & 0x80 != 0 {
                    let previous = counter.read(context);
                    let next = previous.wrapping_add(1);
                    counter.write(context, next);
                    if counter.read(context) > 4 {
                        sprite = i16::from(player.sprite_state().read(context)) + 4;
                        let tub = ram::g_player_info(revision)
                            .at(i32::from(earthling.object().read(context)));
                        tub.behavior_flags()
                            .write(context, tub.behavior_flags().read(context) & !1);
                        player.height().write(context, 1);
                    }
                } else if read_long(context, player.animation_group_mask().address()) & 0x100 != 0 {
                    if player.frame_duration().read(context) == 0 {
                        call_from(
                            context,
                            ids::HOTTUB_ABILITY,
                            ids::STOP_PLAYER,
                            &[player.address()],
                            0x110,
                        )?;
                        let tub = ram::g_player_info(revision)
                            .at(i32::from(earthling.object().read(context)));
                        tub.behavior_flags()
                            .write(context, tub.behavior_flags().read(context) | 1);
                        player.x().write(context, earthling.x().read(context));
                        player.y().write(context, earthling.y().read(context));
                        player.height().write(context, 35);
                        sprite = i16::from(player.sprite_state().read(context)) + 4;
                        player.acceleration_height().write(context, -64);
                    }
                } else if read_long(context, player.animation_group_mask().address()) & 0x200 != 0 {
                    if player.height().read(context) < 22 {
                        call_from(
                            context,
                            ids::HOTTUB_ABILITY,
                            ids::STOP_PLAYER,
                            &[player.address()],
                            0x166,
                        )?;
                        player.height().write(context, 22);
                        player
                            .draw_flags()
                            .write(context, player.draw_flags().read(context) & !0x40);
                        sprite = i16::from(player.sprite_state().read(context)) + 4;
                        call_from(
                            context,
                            ids::HOTTUB_ABILITY,
                            ids::PLAY_SOUND_EFFECT,
                            &[rom::sfx_splash(revision).address(), 50, 0],
                            0x18e,
                        )?;
                    }
                } else if read_long(context, player.animation_group_mask().address()) & 0x400 != 0 {
                    if player.frame_duration().read(context) == 0 {
                        sprite = 119;
                        clear_word(context, player.height().address())?;
                        clear_byte(context, counter.address())?;
                    }
                } else if player.sprite_state().read(context) == 119 {
                    if number == 0 {
                        if ram::g_player_processed_input(revision).at(0).read(context) & 4 != 0 {
                            sprite = 120;
                            player
                                .behavior_flags()
                                .write(context, player.behavior_flags().read(context) | 0x20);
                        } else if call_from(
                            context,
                            ids::HOTTUB_ABILITY,
                            ids::GET_RANDOM_NUMBER,
                            &[],
                            0x1e0,
                        )? & 31
                            == 0
                        {
                            player
                                .x()
                                .write(context, player.x().read(context).wrapping_sub(20));
                            tub_speech(context, 0, 0, 0)?;
                            player
                                .x()
                                .write(context, player.x().read(context).wrapping_add(20));
                        } else if call_from(
                            context,
                            ids::HOTTUB_ABILITY,
                            ids::GET_RANDOM_NUMBER,
                            &[],
                            0x20a,
                        )? & 31
                            == 0
                        {
                            let index = advance_random_index(context);
                            let low = Address::<u8>::new(
                                ram::g_cached_random_values(revision)
                                    .at(i32::from(index))
                                    .address()
                                    + 1,
                            )
                            .read(context);
                            let offset = if low & 1 != 0 { 6 } else { 14 };
                            tub_speech(context, 0, offset, 1)?;
                        }
                    } else if number == 1 {
                        if ram::g_player_processed_input(revision).at(1).read(context) & 8 != 0 {
                            sprite = 120;
                            player
                                .behavior_flags()
                                .write(context, player.behavior_flags().read(context) | 0x20);
                        } else if call_from(
                            context,
                            ids::HOTTUB_ABILITY,
                            ids::GET_RANDOM_NUMBER,
                            &[],
                            0x274,
                        )? & 31
                            == 0
                        {
                            player
                                .x()
                                .write(context, player.x().read(context).wrapping_add(20));
                            tub_speech(context, 1, 22, 0)?;
                            player
                                .x()
                                .write(context, player.x().read(context).wrapping_sub(20));
                        } else if ram::g_player_info(revision)
                            .at(0)
                            .sprite_state()
                            .read(context)
                            != 119
                            && call_from(
                                context,
                                ids::HOTTUB_ABILITY,
                                ids::GET_RANDOM_NUMBER,
                                &[],
                                0x2a8,
                            )? & 31
                                == 0
                        {
                            let index = advance_random_index(context);
                            let low = Address::<u8>::new(
                                ram::g_cached_random_values(revision)
                                    .at(i32::from(index))
                                    .address()
                                    + 1,
                            )
                            .read(context);
                            let offset = if low & 1 != 0 { 28 } else { 36 };
                            tub_speech(context, 1, offset, 1)?;
                        }
                    }
                    let previous = counter.read(context);
                    let next = previous.wrapping_add(1);
                    counter.write(context, next);
                    if counter.read(context) > 5 {
                        let health = ram::g_player_health(revision).at(number);
                        let previous = health.read(context);
                        let next = previous.wrapping_add(1);
                        health.write(context, next);
                    }
                } else if player.sprite_state().read(context) == 120
                    && player.frame_duration().read(context) == 0
                {
                    if number == 0 {
                        sprite = 23;
                        player
                            .x()
                            .write(context, player.x().read(context).wrapping_sub(55));
                        player.height().write(context, 42);
                    } else {
                        sprite = 22;
                        player
                            .x()
                            .write(context, player.x().read(context).wrapping_add(67));
                        player.height().write(context, 39);
                    }
                    player
                        .draw_flags()
                        .write(context, player.draw_flags().read(context) | 0x40);
                    player
                        .behavior_flags()
                        .write(context, player.behavior_flags().read(context) & !0x20);
                    player.acceleration_height().write(context, -32);
                    ram::g_player_landing_counter_seed(revision)
                        .at(number)
                        .write(context, 2);
                }
            }
            if sprite != -1 {
                call_from(
                    context,
                    ids::HOTTUB_ABILITY,
                    ids::SET_SPRITE,
                    &[player.address(), sprite as u32],
                    0x384,
                )?;
            }
            Ok(None)
        },
    )
}
