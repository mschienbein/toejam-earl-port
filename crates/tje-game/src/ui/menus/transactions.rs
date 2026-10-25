use crate::{Context, Result, call, call_void, ids};
use tje_mem::{Address, ram};

fn selected_slot(context: &dyn Context, player: i32) -> i8 {
    let revision = context.revision();
    let row = i32::from(
        ram::g_player_inventory_selection_row(revision)
            .at(player)
            .read(context),
    );
    let scroll = i32::from(
        ram::g_player_inventory_scroll_row(revision)
            .at(player)
            .read(context),
    );
    let column = i32::from(
        ram::g_player_inventory_selection_column(revision)
            .at(player)
            .read(context),
    );
    row.wrapping_add(scroll)
        .wrapping_mul(2)
        .wrapping_add(column) as i8
}

/// G/src/ui/inventory.c:176 (`SetMailOrderPresents`).
/// Seed indexing uses a signed word displacement; mailbox row strides use longs.
pub fn set_mail_order_presents(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let player = i32::from(arguments[0] as i16);
    let level_offset = i32::from((arguments[1] as i16).wrapping_mul(2));
    let seed = ram::g_level_seeds(revision)
        .cast::<u16>()
        .byte_offset(level_offset)
        .read(context);
    let saved = call(context, ids::STACK_BASED_RANDOM, &[u32::from(seed)])?;
    let mut slot = 0;
    while slot < 3 {
        let present = call(context, ids::SELECT_RANDOM_PRESENT, &[])? as i16;
        if present == 17
            || present == 11
            || present == 21
            || (slot > 0
                && i16::from(
                    ram::g_player_mailbox_items(revision)
                        .at(player)
                        .at(0)
                        .read(context),
                ) == present)
            || (slot > 1
                && i16::from(
                    ram::g_player_mailbox_items(revision)
                        .at(player)
                        .at(1)
                        .read(context),
                ) == present)
        {
            continue;
        }
        ram::g_player_mailbox_items(revision)
            .at(player)
            .at(slot)
            .write(context, present as i8);
        let random = u32::from(call(context, ids::GET_RANDOM_NUMBER, &[])? as u16);
        ram::g_player_mailbox_prices(revision)
            .at(player)
            .at(slot)
            .write(context, (random % 3 + 3) as i8);
        slot += 1;
    }
    call(context, ids::STACK_BASED_RANDOM, &[saved])?;
    Ok(None)
}

/// G/src/ui/inventory.c:356 (`RandomizeInventory`).
pub fn randomize_inventory(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let inventory = ram::g_player_inventory(context.revision());
    for player in 0..2 {
        for slot in 0..16 {
            let entry = inventory.at(player).at(slot);
            if entry.read(context) != -1 && entry.read(context) != 27 {
                let present = call(context, ids::SELECT_RANDOM_PRESENT, &[])? as i8;
                entry.write(context, present);
            }
        }
    }
    Ok(None)
}

/// G/src/ui/inventory.c:567 (`InventoryDropSelectedPresent`).
pub fn inventory_drop_selected_present(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let player = i32::from(arguments[0] as i16);
    if ram::g_player_elevator_flags(revision)
        .at(player)
        .read(context)
        != 0
    {
        call_void(context, ids::PLAY_SOUND, &[26, 50])?;
        return Ok(None);
    }
    let mut slot = selected_slot(context, player);
    let inventory = ram::g_player_inventory(revision).at(player);
    let present = inventory.at(i32::from(slot)).read(context);
    while slot < 15 && inventory.at(i32::from(slot) + 1).read(context) != -1 {
        let next = inventory.at(i32::from(slot) + 1).read(context);
        inventory.at(i32::from(slot)).write(context, next);
        slot = slot.wrapping_add(1);
    }
    inventory.at(i32::from(slot)).write(context, -1);
    if present != -1 {
        call_void(
            context,
            ids::DROP_PRESENT,
            &[player as u32, i32::from(present) as u32],
        )?;
    }
    Ok(None)
}

/// G/src/ui/inventory.c:611 (`InventoryIdentifySelectedPresent`).
pub fn inventory_identify_selected_present(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let player = i32::from(arguments[0] as i16);
    let slot = selected_slot(context, player);
    let entry = ram::g_player_inventory(revision)
        .at(player)
        .at(i32::from(slot));
    let mut present = entry.read(context);
    let bucks = ram::g_player_bucks(revision).at(player);
    if bucks.read(context) < 2 || present == -1 {
        call_void(context, ids::PLAY_SOUND, &[26, 50])?;
        return Ok(Some(0));
    }
    let mut changed = false;
    while present == 26 {
        present = call(context, ids::SELECT_RANDOM_PRESENT, &[])? as i8;
        changed = true;
    }
    entry.write(context, present);
    let identified = ram::g_present_info(revision)
        .at(i32::from(present))
        .identified();
    if identified.read(context) == 0 {
        identified.write(context, 1);
        changed = true;
    }
    if changed {
        let next = bucks.read(context).wrapping_sub(2);
        bucks.write(context, next);
    } else {
        call_void(context, ids::PLAY_SOUND, &[26, 50])?;
    }
    Ok(Some(u32::from(changed)))
}

fn release_byte(context: &mut dyn Context, handle: Address<i8>) -> Result<()> {
    if handle.read(context) != 0 {
        let argument = i32::from(handle.read(context)) as u32;
        call_void(context, ids::FREE_ENTITY, &[argument])?;
        handle.write(context, 0);
    }
    Ok(())
}

fn release_run(context: &mut dyn Context, handles: Address<i8>, count: i32) -> Result<()> {
    for index in 0..count {
        release_byte(context, handles.offset(index))?;
    }
    Ok(())
}

/// G/src/ui/window_panels.c:16 (`ClearTransientEntities`).
/// Keep release/clear order, including conditionally cleared interaction state.
pub fn clear_transient_entities(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    release_run(
        context,
        ram::g_shared_projectile_entities(revision).at(0),
        12,
    )?;
    release_run(
        context,
        ram::g_control_remap_projectile_entities(revision).at(0),
        4,
    )?;
    release_run(context, ram::g_mole_effect_entities(revision).at(0), 6)?;
    release_run(context, ram::g_speech_bubble_entities(revision).at(0), 4)?;
    let life = ram::g_life_request_display_entity(revision);
    if life.read(context) != 0 {
        let argument = i32::from(life.read(context)) as u32;
        call_void(context, ids::FREE_ENTITY, &[argument])?;
        life.write(context, 0);
        ram::g_life_request_pending(revision).write(context, 0);
    }
    for player in 0..2 {
        release_byte(
            context,
            ram::g_player_active_terrain_effect_entities(revision).at(player),
        )?;
        release_byte(
            context,
            ram::g_player_one_shot_terrain_effect_entities(revision).at(player),
        )?;
        release_byte(
            context,
            ram::g_player_transient_effect_entities(revision).at(player),
        )?;
        release_byte(context, ram::g_player_decoy_entities(revision).at(player))?;
        release_byte(
            context,
            ram::g_player_promotion_display_entities(revision).at(player),
        )?;
        release_byte(context, ram::g_player_doorway_entities(revision).at(player))?;
        release_byte(
            context,
            ram::g_player_rain_cloud_states(revision).at(player),
        )?;
        release_byte(context, ram::g_timed_effect_entities(revision).at(player))?;
        release_byte(
            context,
            ram::g_player_control_remap_entities(revision).at(player),
        )?;
        release_byte(
            context,
            ram::g_player_wake_effect_entity_handles(revision).at(player),
        )?;
        release_byte(
            context,
            ram::g_player_boom_box_entities(revision).at(player),
        )?;
        let choice = ram::g_player_interaction_choice_entities(revision).at(player);
        if choice.read(context) != 0 {
            let argument = i32::from(choice.read(context)) as u32;
            call_void(context, ids::FREE_ENTITY, &[argument])?;
            choice.write(context, 0);
            ram::g_player_interaction_choice_active(revision)
                .at(player)
                .write(context, 0);
            ram::g_player_interaction_responses(revision)
                .at(player)
                .write(context, 0);
        }
        ram::g_player_tomato_rain_ticks(revision)
            .at(player)
            .write(context, 0);
    }
    call_void(context, ids::PRESENT_RELEASE_DROPPED_ENTITIES, &[])?;
    call_void(context, ids::TELEPHONE_CLEAR_ALL, &[])?;
    Ok(None)
}
