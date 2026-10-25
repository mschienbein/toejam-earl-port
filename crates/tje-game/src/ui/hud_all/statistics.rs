use super::super::draws::literal;
use crate::{Context, Result, call_void, ids};
use tje_mem::{Ptr, ram, rom};

/// G/src/ui/inventory.c:735 (`InventoryDrawBucks`), c_inventory.s REV02:2087–2136,
/// REV00:2082–2131. Functional: stable signed-byte currency and ordered tile writes.
pub fn inventory_draw_bucks(context: &mut dyn Context, arguments: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let player = if arguments[0] as i16 == 0 { 0 } else { 1 };
    context.write_vdp_command(
        (if player == 0 {
            0x56000003_u32
        } else {
            0x5c800003
        })
        .wrapping_add(6 << 17),
    );
    let bucks = ram::g_player_bucks(revision).at(player);
    if bucks.read(context) < 100 {
        context.write_vdp_data(0x85b4);
    }
    let value = i32::from(bucks.read(context)) as u32;
    call_void(context, ids::TEXT_WRITE_DECIMAL_TILES, &[value])?;
    if bucks.read(context) < 10 {
        context.write_vdp_data(0x85b4);
    }
    Ok(None)
}

/// G/src/ui/inventory.c:757 (`InventoryDrawPlayerStats`), c_inventory.s
/// REV02:2140–2211, REV00:2135–2206. Functional: currency, points and rank writes.
/// Points compare as unsigned words; decimal conversion narrows its own slot.
pub fn inventory_draw_player_stats(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let number = arguments[0] as i16;
    let mut column = 32_u32;
    call_void(
        context,
        ids::INVENTORY_DRAW_BUCKS,
        &[i32::from(number) as u32],
    )?;
    let player = if number == 0 { 0 } else { 1 };
    let points = ram::g_player_points(revision).at(player);
    if points.read(context) > 99 {
        column -= 1;
    }
    context.write_vdp_command(
        (if player == 0 {
            0x56000003_u32
        } else {
            0x5c800003
        })
        .wrapping_add(column << 17),
    );
    let value = u32::from(points.read(context));
    call_void(context, ids::TEXT_WRITE_DECIMAL_TILES, &[value])?;
    if points.read(context) > 999 {
        // Two separately addressed six-byte literals in the matching unit.
        let text =
            literal(context, "GameText_00A1C0")?.wrapping_add(if player == 0 { 0 } else { 6 });
        call_void(context, ids::TEXT_WRITE_STRING_TILES, &[text])?;
    }
    context.write_vdp_command(if player == 0 { 0x55a80003 } else { 0x5c280003 });
    let rank = i16::from(ram::g_player_rank(revision).at(player).read(context));
    let text = rom::rank_chars_table(revision)
        .at_word_index(rank)
        .read(context)
        .address();
    call_void(context, ids::TEXT_WRITE_STRING_TILES, &[text])?;
    Ok(None)
}

/// G/src/ui/inventory.c:933 (`InventoryDrawMailOrderOffers`), c_inventory.s
/// REV02:2647–2762, REV00:2642–2757. Functional: finite descriptor/text writes.
/// Mailbox rows use full-long player scaling; signed-char descriptor indexes use
/// the emitted signed word displacements. All strings remain user-ROM views.
pub fn inventory_draw_mail_order_offers(
    context: &mut dyn Context,
    arguments: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let player = i32::from(arguments[0] as i16);
    let (base_row, mut object) = if player == 0 {
        (5, ram::g_player1_inventory_item_entities(revision).at(0))
    } else {
        (18, ram::g_player2_inventory_item_entities(revision).at(0))
    };
    for row in 0_i16..3 {
        let present = ram::g_player_mailbox_items(revision)
            .at_long_index(player)
            .at_word_index(row)
            .read(context);
        let appearance = i16::from(
            ram::g_present_info(revision)
                .at_word_index(i16::from(present))
                .appearance()
                .read(context),
        );
        object.frames().write(
            context,
            Ptr::new(
                rom::present_sprite_table(revision)
                    .at_word_index(appearance)
                    .address(),
            ),
        );
        object.sprite_ready().write(context, 0);
        object
            .y()
            .write(context, ((i32::from(row) * 2 + base_row + 1) * 8) as i16);
        object.x().write(context, 21);
        let flags = object.behavior_flags().read(context) & !0x40;
        object.behavior_flags().write(context, flags);
        object.frame().write(context, 0);
        if ram::g_inventory_labels_suppressed(revision).read(context) == 0 {
            context.write_vdp_command(
                0x500a0003_u32.wrapping_add(((i32::from(row) * 2 + base_row) as u32) << 23),
            );
            let text = if ram::g_present_info(revision)
                .at_word_index(i16::from(present))
                .identified()
                .read(context)
                != 0
            {
                if present == 12
                    && ram::g_player_lives(revision)
                        .at_long_index(player ^ 1)
                        .read(context)
                        < 0
                {
                    literal(context, "InventoryNameUnFallStr")?
                } else {
                    rom::inventory_name_string_table(revision)
                        .at_word_index(i16::from(present))
                        .read(context)
                        .address()
                }
            } else {
                literal(context, "InventoryNameUnidentifiedStr")?
            };
            call_void(context, ids::TEXT_WRITE_STRING_TILES, &[text])?;
            let leader = literal(context, "InventoryNameDotLeaderStr")?;
            call_void(context, ids::TEXT_WRITE_STRING_TILES, &[leader])?;
            let price = i32::from(
                ram::g_player_mailbox_prices(revision)
                    .at_long_index(player)
                    .at_word_index(row)
                    .read(context),
            ) as u32;
            call_void(context, ids::TEXT_WRITE_DECIMAL_TILES, &[price])?;
        }
        object = object.offset_long_index(1);
    }
    Ok(None)
}
