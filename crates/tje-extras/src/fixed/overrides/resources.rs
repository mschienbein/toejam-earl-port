use super::{Context as EnhancedContext, Continuation, OverrideStep, original};
use tje_game::{Context, Result, call, call_void, ids};
use tje_mem::{Address, ObjectState, ram, rom};

/// B008: check both bank scans before touching the selected record. Full banks
/// decline the spawn without reading or writing a neighbour's first record.
pub(super) fn earthling_present(c: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    let r = c.revision();
    let player = Address::<ObjectState>::new(args[0]);
    let bank = if ram::g_earthling_bank_users(r).at(0).read(c) == 0
        || ram::g_earthling_bank_levels(r).at(0).read(c) != player.level().read(c)
    {
        1
    } else {
        0
    };
    let records = ram::g_earthling_banks(r).at(bank);
    let available = (0..20)
        .find(|&i| records.at(i).kind().read(c) == -1)
        .or_else(|| {
            (0..20).find(|&i| {
                let record = records.at(i);
                !matches!(record.kind().read(c), 0 | 9) && record.object().read(c) == 0
            })
        });
    let Some(slot) = available else {
        return Ok(Some(0));
    };
    let mut earthling = records.at(slot);
    let mut terrain = args[1] as i16;
    let mut x = 0_i16;
    let mut y = 0_i16;
    let mut selection = 0_i16;
    for _ in 0..4 {
        x = player
            .x()
            .read(c)
            .wrapping_add(if call(c, ids::GET_RANDOM_NUMBER, &[])? & 1 != 0 {
                40
            } else {
                -40
            });
        y = player
            .y()
            .read(c)
            .wrapping_add(if call(c, ids::GET_RANDOM_NUMBER, &[])? & 1 != 0 {
                25
            } else {
                -25
            });
        let camera = u32::from(player.draw_flags().read(c) & 2 == 0);
        selection = call(
            c,
            ids::GET_MAP_TERRAIN_AT_WORLD_POSITION,
            &[camera, x as u32, y as u32],
        )? as i16;
        if (terrain == 0x80 && selection == 0x80) || (terrain != 0x80 && selection != 0x80) {
            break;
        }
    }
    terrain = selection;
    if player.level().read(c) == 0 {
        earthling.kind().write(c, 5);
    } else if terrain == 0x80 {
        earthling.kind().write(c, 14);
    } else if terrain == 0x90 || terrain == 0x50 {
        earthling.kind().write(c, 22);
    } else {
        loop {
            let kind = ((call(c, ids::GET_RANDOM_NUMBER, &[])? as u16) % 22 + 1) as i8;
            earthling.kind().write(c, kind);
            if [17, 20, 11, 10, 9].contains(&kind) {
                let mut found = false;
                let mut chosen = false;
                for i in 0..20 {
                    let other = records.at(i);
                    if earthling != other && other.kind().read(c) == earthling.kind().read(c) {
                        if other.object().read(c) == 0 {
                            earthling = other;
                            chosen = true;
                        }
                        found = true;
                        break;
                    }
                }
                if !found {
                    let level = player.level().read(c) as u32;
                    chosen = earthling.kind().read(c) != 9
                        || call(
                            c,
                            ids::EARTHLING_CONFIGURE_MAILBOX_LEVEL_BANK,
                            &[bank as u32, level],
                        )? != 0;
                }
                if chosen {
                    break;
                }
            } else if kind != 14
                && (kind != 8 || terrain == 0x20)
                && (kind != 13 || terrain == 0x10)
            {
                break;
            }
        }
    }
    earthling.level().write(c, player.level().read(c));
    earthling.x().write(c, x);
    earthling.y().write(c, y);
    earthling.saved_y().write(c, y);
    let mut height = 5;
    if matches!(earthling.kind().read(c), 18 | 19) {
        earthling.saved_x_or_status().write(c, 0);
        earthling.saved_y().write(c, 0);
    } else if earthling.kind().read(c) == 7 {
        earthling.saved_x_or_status().write(c, 1);
        height = 30;
    } else {
        earthling.saved_x_or_status().write(c, x);
    }
    let definition = rom::earthling_definitions(r).at(i32::from(earthling.kind().read(c)));
    earthling
        .health_visibility()
        .at(0)
        .write(c, definition.health().read(c));
    earthling.health_visibility().at(1).write(c, 0);
    let effect = [
        player.entity_index().read(c) as u32,
        earthling.x().read(c) as u32,
        (i32::from(earthling.y().read(c)) + 1) as u32,
        height,
        earthling.level().read(c) as u32,
    ];
    call(c, ids::PRESENT_START_TRANSIENT_EFFECT, &effect)?;
    let mut object = call(c, ids::ALLOCATE_ENTITY, &[])? as i16;
    if object == 0 {
        object = call(c, ids::ALLOCATE_ENTITY_WITH_EVICTION, &[1])? as i16;
    }
    if object != 0 {
        call_void(c, ids::FREE_ENTITY, &[object as u32])?;
    }
    Ok(Some(1))
}

/// B011: the optional tile run is owned only after a successful acquisition.
/// Shared-tile entities never free the incoming D4 residue on selector failure.
pub(super) fn viewport_sprites(c: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    let entity = Address::<ObjectState>::new(args[0]);
    let viewport = args[1] as i16;
    let tile_count = i16::from(entity.tile_count().read(c));
    let tile = if entity.draw_flags().read(c) & 6 == 0 && entity.behavior_flags().read(c) & 4 == 0 {
        let tile = call(c, ids::ALLOCATE_SPRITE_TILE_RUN, &[tile_count as u32])? as i16;
        if tile == 0 {
            return Ok(None);
        }
        entity
            .tile()
            .write(c, (entity.tile().read(c) & 0xf800) | tile as u16);
        Some(tile)
    } else {
        None
    };
    let (bit, slots) = if viewport == 0 {
        (2, entity.primary_sprites())
    } else {
        (4, entity.secondary_sprites())
    };
    let count = u32::from(entity.sprite_count().read(c));
    let status = call(
        c,
        ids::ALLOCATE_SPRITE_SLOTS,
        &[count, viewport as u32, 1, slots.address()],
    )?;
    if status == 0 {
        entity
            .draw_flags()
            .write(c, entity.draw_flags().read(c) | bit);
    } else if let Some(tile) = tile {
        call_void(
            c,
            ids::FREE_SPRITE_TILE_RUN,
            &[tile as u32, tile_count as u32],
        )?;
    }
    Ok(None)
}

/// B017: rank eight is the last real rank; do not read its zero sentinel as
/// another promotion threshold. The resource/HUD sequence stays original.
pub(super) fn rank_reset(
    c: &mut dyn EnhancedContext,
    k: &mut Continuation,
) -> Result<OverrideStep> {
    let r = c.revision();
    if k.stage == 0 {
        let p = c.entry_arguments()?[0] as i16 as i32;
        k.locals[0] = p as u32;
        let points = ram::g_player_points(r).at(p).read(c);
        let mut rank = 0_i8;
        while rank < 8 && points >= call(c, ids::GET_SCORE_FOR_PROMO, &[rank as u32])? as u16 {
            rank += 1;
        }
        ram::g_player_rank(r).at(p).write(c, rank);
        let bucks = ram::g_player_bucks(r).at(p);
        bucks.write(c, 0);
        let lives = ram::g_player_lives(r).at(p);
        if ram::g_player_selection(r).read(c) == 2 {
            lives.write(c, 0);
        } else {
            lives.write(c, 3);
            bucks.write(c, 3);
        }
        let health = call(c, ids::PLAYER_GET_MAXIMUM_HEALTH, &[p as u32])? as i8;
        ram::g_player_health(r).at(p).write(c, health);
        ram::g_player_health_before_swimming(r).at(p).write(c, -1);
        ram::g_player_powerups(r).at(p).write(c, -1);
        if super::selected(c, "B018")? && ineligible(c, p) {
            let arguments = empty_bar(c, p);
            return Ok(original(k, 1, ids::DMA_COPY_WORDS_TO_VRAM, arguments));
        }
        return Ok(original(
            k,
            1,
            ids::HUD_INITIALIZE_HEALTH_BAR,
            vec![p as u32, 1],
        ));
    }
    let p = k.locals[0];
    if k.stage == 1 {
        return Ok(original(k, 2, ids::HUD_LOAD_ACTION_ICON_SPRITE, vec![p]));
    }
    if k.stage == 2 {
        return Ok(original(k, 3, ids::HUD_LOAD_LIVES_SPRITE_TILES, vec![p]));
    }
    Ok(OverrideStep::Return(None))
}

/// B018: the immediate constructor must share the animator's eligibility. Its
/// defined ineligible result is the original empty bar, displayed health zero;
/// actual health is never temporarily hidden from interrupts or simulation.
pub(super) fn health_bar(
    c: &mut dyn EnhancedContext,
    k: &mut Continuation,
) -> Result<OverrideStep> {
    if k.stage != 0 {
        return Ok(OverrideStep::Return(None));
    }
    let args = c.entry_arguments()?;
    let p = args[0] as i16 as i32;
    if args[1] as i16 == 0 || !ineligible(c, p) {
        return Ok(OverrideStep::ContinueOriginal);
    }
    let arguments = empty_bar(c, p);
    Ok(original(k, 1, ids::DMA_COPY_WORDS_TO_VRAM, arguments))
}
fn ineligible(c: &mut dyn Context, p: i32) -> bool {
    let r = c.revision();
    ram::g_player_lives(r).at(p).read(c) < 0
        || matches!(ram::g_player_info(r).at(p).sprite_state().read(c), 5 | 6)
        || ram::g_player1_map_level(r).read(c) == 26
}
fn empty_bar(c: &mut dyn Context, p: i32) -> Vec<u32> {
    let r = c.revision();
    ram::g_hud_displayed_health(r).at(p).write(c, 0);
    let (tile, sprite, base) = if p == 0 {
        (0x7b4_u32, 15, 23_i16)
    } else {
        (0x7d4, 17, 31)
    };
    let maximum = if ram::g_player_lives(r).at(p).read(c) >= 0 {
        base + i16::from(ram::g_player_rank(r).at(p).read(c)) * 4
    } else {
        base
    };
    let sprites = ram::g_hardware_sprite_table(r);
    for (slot, extra) in [(sprite, 0_u16), (sprite + 1, 4)] {
        sprites.at(slot).size().write(c, 12);
        sprites
            .at(slot)
            .tile()
            .write(c, tile as u16 + 0x8000 + extra);
    }
    let offset = i32::from(ram::g_sprite_tile_buffer_offset(r).read(c));
    let buffer = ram::g_sprite_tile_upload_buffer(r)
        .cast::<u8>()
        .offset(offset);
    for index in 0..256 {
        buffer.offset(index).write(c, 0);
    }
    for unit in 0..maximum.clamp(0, 64) {
        let row = i32::from(unit / 8);
        let column = i32::from((unit % 8) / 2);
        let pixels = if unit & 1 == 0 { 0xd0 } else { 0x0d };
        for scan in 0..3 {
            let address = buffer.offset(row * 32 + column + scan * 4);
            address.write(c, address.read(c) | pixels);
        }
    }
    vec![buffer.address(), tile << 5, 128]
}
