use crate::{Context, FunctionId, Result, call, ids};
use tje_mem::{Address, EarthlingState, ObjectState, ram, rom};

fn can_interact(context: &dyn Context, player: Address<ObjectState>, extra: Option<i8>) -> bool {
    let state = player.sprite_state().read(context);
    state == 0
        || state == 4
        || player.animation_group_mask().read(context) & 0x2040_0000 != 0
        || state == 76
        || extra == Some(state)
        || (state == 57 && player.velocity_y().read(context) == 0)
}

fn literal_tail(context: &dyn Context, id: FunctionId, bytes: u32) -> u32 {
    let metadata = id.metadata();
    metadata.address(context.revision()) + metadata.size(context.revision()) - bytes
}

fn restore_player_sprite(context: &mut dyn Context, player: Address<ObjectState>) -> Result<()> {
    let sprite = call(context, ids::GET_ACTIVE_PRESENT_SPRITE, &[player.address()])?;
    call(context, ids::SET_SPRITE, &[player.address(), sprite])?;
    Ok(())
}

/// G/src/earthlings/nice_earthling_touch.c:177 (`AI_WizardTouch`).
/// TimeAware: speech can wait for interrupt-owned uploads. Inline speech text is
/// read from the ROM tail (compiler `_L151`), never embedded in the native port.
pub fn ai_wizard_touch(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let number = i32::from(arguments[0] as i16);
    let earthling = Address::<EarthlingState>::new(arguments[1]);
    let player = ram::g_player_info(revision).at(number);
    let object = ram::g_player_info(revision).at(i32::from(earthling.object().read(context)));
    let interaction = ram::g_player_wizard_interaction_states(revision).at(number);
    let mut next = -1_i8;
    if !(can_interact(context, player, Some(71))
        && call(
            context,
            ids::ENTITY_IS_WITHIN_RANGE,
            &[object.address(), player.address(), 35, 15],
        )? != 0)
    {
        interaction.write(context, 0);
        return Ok(None);
    }
    if interaction.read(context) == 2 {
        return Ok(None);
    }
    earthling.state_delay().write(context, 1);
    if ram::g_player_interaction_choice_active(revision)
        .at(number)
        .read(context)
        != 0
        || (ram::g_player_info(revision).at(0).level().read(context)
            == ram::g_player_info(revision).at(1).level().read(context)
            && ram::g_player_wizard_interaction_states(revision)
                .at(number ^ 1)
                .read(context)
                != 0
            && ram::g_player_wizard_interaction_states(revision)
                .at(number ^ 1)
                .read(context)
                != 2)
    {
        return Ok(None);
    }
    if interaction.read(context) == 0 {
        if (earthling.state().read(context) == 4 || earthling.state().read(context) == 5)
            && object.frame_duration().read(context) == 0
        {
            earthling
                .state()
                .write(context, earthling.state().read(context).wrapping_add(2));
        }
        if earthling.state().read(context) >= 6 {
            next = call(
                context,
                ids::EARTHLING_SELECT_TURN_STATE,
                &[earthling.address()],
            )? as i8
        } else if earthling.state().read(context) <= 1 && object.frame().read(context) > 4 {
            let text = literal_tail(context, ids::AI_WIZARD_TOUCH, 12);
            let speaker = earthling.object().read(context) as u32;
            call(context, ids::GENERATE_SPEECH, &[speaker, text, 0])?;
            call(
                context,
                ids::PLAYER_CREATE_INTERACTION_CHOICE_DISPLAY,
                &[number as u32],
            )?;
            interaction.write(context, 1);
        }
    } else if object.frame_duration().read(context) == 0 {
        if interaction.read(context) == 1 {
            if ram::g_player_interaction_responses(revision)
                .at(number)
                .read(context)
                == 2
            {
                let bucks = ram::g_player_bucks(revision).at(number);
                if bucks.read(context) > 0 {
                    next = 2;
                    interaction.write(context, 3);
                    bucks.write(context, bucks.read(context).wrapping_sub(1));
                    call(context, ids::SET_SPRITE, &[player.address(), 71])?;
                    call(
                        context,
                        ids::PLAY_SOUND_EFFECT,
                        &[rom::sfx_hallelujah(revision).address(), 92, 1],
                    )?;
                } else {
                    call(context, ids::PLAY_SOUND, &[26, 50])?;
                }
            } else {
                interaction.write(context, 2);
                next = 5;
                object.velocity_x().write(context, 320);
            }
            ram::g_player_interaction_responses(revision)
                .at(number)
                .write(context, 0);
        } else if interaction.read(context) == 3 {
            let maximum = call(context, ids::PLAYER_GET_MAXIMUM_HEALTH, &[number as u32])?;
            let health = ram::g_player_health(revision).at(number);
            if i32::from(health.read(context)) < maximum as i32 {
                health.write(context, health.read(context).wrapping_add(1));
            } else {
                interaction.write(context, 2);
                next = 3;
                restore_player_sprite(context, player)?;
            }
        } else if earthling.state().read(context) == 3 {
            next = 5;
            object.velocity_x().write(context, 320);
        }
    }
    if next != -1 {
        call(
            context,
            ids::SET_EARTHLING_STATE,
            &[earthling.address(), next as u32],
        )?;
        object.frame().write(context, -1);
    }
    Ok(None)
}

/// G/src/earthlings/nice_earthling_touch.c:299 (`AI_OperaSingerTouch`).
/// TimeAware: speech waits; preserve the original successful-payment index toggle
/// before clearing the response, including its other-player side effect.
pub fn ai_opera_singer_touch(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let mut number = i32::from(arguments[0] as i16);
    let earthling = Address::<EarthlingState>::new(arguments[1]);
    let mut player = ram::g_player_info(revision).at(number);
    let object = ram::g_player_info(revision).at(i32::from(earthling.object().read(context)));
    let interaction = ram::g_player_opera_singer_interaction_states(revision).at(number);
    let mut next = -1_i8;
    if !(can_interact(context, player, Some(69))
        && call(
            context,
            ids::ENTITY_IS_WITHIN_RANGE,
            &[object.address(), player.address(), 35, 15],
        )? != 0)
    {
        interaction.write(context, 0);
        return Ok(None);
    }
    if interaction.read(context) == 2 {
        return Ok(None);
    }
    earthling.state_delay().write(context, 1);
    if ram::g_player_interaction_choice_active(revision)
        .at(number)
        .read(context)
        != 0
        || (ram::g_player_info(revision).at(0).level().read(context)
            == ram::g_player_info(revision).at(1).level().read(context)
            && ram::g_player_opera_singer_interaction_states(revision)
                .at(number ^ 1)
                .read(context)
                != 0
            && ram::g_player_opera_singer_interaction_states(revision)
                .at(number ^ 1)
                .read(context)
                != 2)
    {
        return Ok(None);
    }
    if interaction.read(context) == 0 {
        if earthling.state().read(context) >= 4 {
            next = 0
        } else if earthling.state().read(context) <= 1 {
            let text = literal_tail(context, ids::AI_OPERA_SINGER_TOUCH, 14);
            let speaker = earthling.object().read(context) as u32;
            call(context, ids::GENERATE_SPEECH, &[speaker, text, 0])?;
            call(
                context,
                ids::PLAYER_CREATE_INTERACTION_CHOICE_DISPLAY,
                &[number as u32],
            )?;
            interaction.write(context, 1);
        }
    } else if interaction.read(context) == 1 {
        if ram::g_player_interaction_responses(revision)
            .at(number)
            .read(context)
            == 2
        {
            let bucks = ram::g_player_bucks(revision).at(number);
            if bucks.read(context) >= 3 {
                next = 3;
                interaction.write(context, 3);
                bucks.write(context, bucks.read(context).wrapping_sub(3));
                call(context, ids::SET_SPRITE, &[player.address(), 69])?;
                number ^= 1;
                player = ram::g_player_info(revision).at(number);
                if ram::g_player_lives(revision).at(number).read(context) >= 0
                    && ram::g_split_screen_enabled(revision).read(context) == 0
                    && (player.sprite_state().read(context) == 0
                        || player.sprite_state().read(context) == 4
                        || player.animation_group_mask().read(context) & 0x2040_0000 != 0
                        || player.sprite_state().read(context) == 76
                        || player.animation_group_mask().read(context) & 0x0830_400c != 0)
                {
                    call(context, ids::SET_SPRITE, &[player.address(), 69])?;
                }
                call(
                    context,
                    ids::PLAY_SOUND_EFFECT,
                    &[rom::sfx_opera_sing(revision).address(), 90, 1],
                )?;
            } else {
                call(context, ids::PLAY_SOUND, &[26, 50])?;
            }
        } else {
            interaction.write(context, 2);
            next = 0;
        }
        ram::g_player_interaction_responses(revision)
            .at(number)
            .write(context, 0);
    } else if interaction.read(context) == 3 {
        if object.frame_duration().read(context) == 0 {
            interaction.write(context, 2);
            next = 0;
            restore_player_sprite(context, player)?;
            number ^= 1;
            player = ram::g_player_info(revision).at(number);
            if ram::g_player_lives(revision).at(number).read(context) >= 0
                && ram::g_split_screen_enabled(revision).read(context) == 0
                && player.sprite_state().read(context) == 69
            {
                restore_player_sprite(context, player)?;
            }
        } else {
            let mask = u32::from(player.draw_flags().read(context) & 6);
            call(context, ids::DO_VIKING_ABILITY, &[mask])?;
        }
    }
    if next != -1 {
        call(
            context,
            ids::SET_EARTHLING_STATE,
            &[earthling.address(), next as u32],
        )?;
        object.frame().write(context, -1);
    }
    Ok(None)
}

/// G/src/earthlings/nice_earthling_touch.c:108 (`AI_MailboxTouch`).
/// TimeAware: speech and mail-order opening require IRQ/VBlank progress. The
/// compiler `_L95` literal is the final twelve bytes of this ROM function.
pub fn ai_mailbox_touch(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let number = i32::from(arguments[0] as i16);
    let earthling = Address::<EarthlingState>::new(arguments[1]);
    if ram::g_player_interaction_choice_active(revision)
        .at(number)
        .read(context)
        != 0
    {
        return Ok(None);
    }
    let bank = if ram::g_mailbox_level_states(revision)
        .at(0)
        .level()
        .read(context)
        == earthling.level().read(context)
    {
        0
    } else {
        1
    };
    let mailbox = ram::g_mailbox_level_states(revision).at(bank);
    let player = ram::g_player_info(revision).at(number);
    let object = ram::g_player_info(revision).at(i32::from(earthling.object().read(context)));
    if mailbox.mode().read(context) != 1 {
        if can_interact(context, player, None)
            && call(
                context,
                ids::ENTITY_IS_WITHIN_RANGE,
                &[object.address(), player.address(), 30, 15],
            )? != 0
        {
            let contact = mailbox.contact().at(number);
            if contact.read(context) == 0 {
                let text = literal_tail(context, ids::AI_MAILBOX_TOUCH, 12);
                let speaker = earthling.object().read(context) as u32;
                call(context, ids::GENERATE_SPEECH, &[speaker, text, 0])?;
                call(
                    context,
                    ids::PLAYER_CREATE_INTERACTION_CHOICE_DISPLAY,
                    &[number as u32],
                )?;
                contact.write(context, 1);
            } else if contact.read(context) == 1 {
                if ram::g_player_interaction_responses(revision)
                    .at(number)
                    .read(context)
                    == 2
                {
                    call(
                        context,
                        ids::INVENTORY_OPEN_MAIL_ORDER_MENU,
                        &[number as u32],
                    )?;
                }
                contact.write(context, 2);
                ram::g_player_interaction_responses(revision)
                    .at(number)
                    .write(context, 0);
            }
        } else {
            mailbox.contact().at(number).write(context, 0)
        }
        return Ok(None);
    }
    if i16::from(earthling.state().read(context)) & 252 == 4 {
        if call(
            context,
            ids::ENTITY_IS_WITHIN_RANGE,
            &[object.address(), player.address(), 20, 10],
        )? != 0
        {
            call(
                context,
                ids::PLAYER_YOUCH,
                &[number as u32, earthling.address()],
            )?;
        }
        let timer = mailbox.timer().read(context).wrapping_sub(1);
        mailbox.timer().write(context, timer);
        if timer == 0 {
            call(context, ids::SET_EARTHLING_STATE, &[earthling.address(), 0])?;
            object.frame().write(context, -1);
            mailbox.timer().write(context, 175);
        } else if ram::g_pcm_effect_cooldown_ticks(revision).read(context) == 0 {
            call(
                context,
                ids::PLAY_SOUND_EFFECT,
                &[rom::sfx_mailbox_roar(revision).address(), 50, 0],
            )?;
            earthling.saved_x_or_status().write(context, 1);
        }
    } else if earthling.state().read(context) == 0 {
        if mailbox.timer().read(context) != 0 {
            mailbox
                .timer()
                .write(context, mailbox.timer().read(context).wrapping_sub(1))
        } else if player.sprite_state().read(context) != 5
            && player.sprite_state().read(context) != 6
            && call(
                context,
                ids::ENTITY_IS_WITHIN_RANGE,
                &[object.address(), player.address(), 100, 40],
            )? != 0
        {
            call(context, ids::SET_EARTHLING_STATE, &[earthling.address(), 3])?;
        } else if call(context, ids::GET_RANDOM_NUMBER, &[])? & 255 == 1 {
            call(context, ids::SET_EARTHLING_STATE, &[earthling.address(), 1])?;
        }
    } else if earthling.state().read(context) == 1 {
        if object.frame_duration().read(context) == 0 {
            call(context, ids::SET_EARTHLING_STATE, &[earthling.address(), 0])?;
            object.frame().write(context, -1);
        }
    } else if earthling.state().read(context) == 3 && object.frame_duration().read(context) == 0 {
        call(context, ids::SET_EARTHLING_STATE, &[earthling.address(), 4])?;
        earthling.timer().write(context, 0);
        mailbox.timer().write(context, 33);
        earthling.saved_x_or_status().write(context, 0);
    }
    Ok(None)
}

/// G/src/earthlings/nice_earthling_touch.c:249 (`AI_WisemanTouch`).
/// TimeAware: speech and identification inventory wait for real IRQs. The
/// `_L189` literal and its alignment occupy the final fourteen function bytes.
pub fn ai_wiseman_touch(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let number = i32::from(arguments[0] as i16);
    let earthling = Address::<EarthlingState>::new(arguments[1]);
    let player = ram::g_player_info(revision).at(number);
    let object = ram::g_player_info(revision).at(i32::from(earthling.object().read(context)));
    let interaction = ram::g_player_wiseman_interaction_states(revision).at(number);
    let mut next = -1_i8;
    if !(can_interact(context, player, None)
        && call(
            context,
            ids::ENTITY_IS_WITHIN_RANGE,
            &[object.address(), player.address(), 35, 15],
        )? != 0)
    {
        interaction.write(context, 0);
        return Ok(None);
    }
    if interaction.read(context) == 2 {
        return Ok(None);
    }
    earthling.state_delay().write(context, 1);
    if ram::g_player_interaction_choice_active(revision)
        .at(number)
        .read(context)
        != 0
        || (ram::g_player_info(revision).at(0).level().read(context)
            == ram::g_player_info(revision).at(1).level().read(context)
            && ram::g_player_wiseman_interaction_states(revision)
                .at(number ^ 1)
                .read(context)
                != 0
            && ram::g_player_wiseman_interaction_states(revision)
                .at(number ^ 1)
                .read(context)
                != 2)
    {
        return Ok(None);
    }
    if interaction.read(context) == 0 {
        if earthling.state().read(context) >= 2 {
            next = call(
                context,
                ids::EARTHLING_SELECT_TURN_STATE,
                &[earthling.address()],
            )? as i8
        } else if earthling.state().read(context) <= 1 {
            let text = literal_tail(context, ids::AI_WISEMAN_TOUCH, 14);
            let speaker = earthling.object().read(context) as u32;
            call(context, ids::GENERATE_SPEECH, &[speaker, text, 0])?;
            call(
                context,
                ids::PLAYER_CREATE_INTERACTION_CHOICE_DISPLAY,
                &[number as u32],
            )?;
            interaction.write(context, 1);
        }
    } else if interaction.read(context) == 1 {
        if ram::g_player_interaction_responses(revision)
            .at(number)
            .read(context)
            == 2
        {
            call(
                context,
                ids::PLAY_SOUND_EFFECT,
                &[rom::sfx_wiseman_think(revision).address(), 64, 1],
            )?;
            call(
                context,
                ids::INVENTORY_OPEN_IDENTIFICATION_MENU,
                &[number as u32],
            )?;
        }
        interaction.write(context, 3);
        ram::g_player_interaction_responses(revision)
            .at(number)
            .write(context, 0);
    } else if interaction.read(context) == 3
        && ram::g_player_inventory_open(revision)
            .at(number)
            .read(context)
            == 0
    {
        interaction.write(context, 2);
        next = 2;
        object.velocity_x().write(context, -320);
    }
    if next != -1 {
        call(
            context,
            ids::SET_EARTHLING_STATE,
            &[earthling.address(), next as u32],
        )?;
        object.frame().write(context, -1);
    }
    Ok(None)
}
