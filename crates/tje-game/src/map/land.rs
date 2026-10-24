use crate::call_frame::{CallSite, FrameLayout, LocalFrame, OutgoingArguments};
use crate::{Context, Result, call, call_at, ids};
use tje_mem::{Address, ObjectState, ram, rom};

/// G/src/map/land_processing.c:43 (`AllocateEntityWithEviction`).
pub fn allocate_entity_with_eviction(c: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    let r = c.revision();
    if args[0] as i16 == 0 {
        let object = call(c, ids::ALLOCATE_ENTITY, &[])? as i16;
        if object != 0 {
            return Ok(Some(i32::from(object) as u32));
        }
    }
    for i in 0..6 {
        let handle = ram::g_mole_effect_entities(r).at(i);
        let object = handle.read(c);
        if object != 0 {
            call(c, ids::FREE_ENTITY, &[i32::from(object) as u32])?;
            handle.write(c, 0);
            return Ok(Some(
                i32::from(call(c, ids::ALLOCATE_ENTITY, &[])? as i16) as u32
            ));
        }
    }
    for i in 0..4 {
        let handle = ram::g_speech_bubble_entities(r).at(i);
        let object = handle.read(c);
        if object != 0 {
            call(c, ids::FREE_ENTITY, &[i32::from(object) as u32])?;
            handle.write(c, 0);
            return Ok(Some(
                i32::from(call(c, ids::ALLOCATE_ENTITY, &[])? as i16) as u32
            ));
        }
    }
    if call(c, ids::PRESENT_RELEASE_FIRST_SHARED_TOMATO, &[])? != 0 {
        return Ok(Some(
            i32::from(call(c, ids::ALLOCATE_ENTITY, &[])? as i16) as u32
        ));
    }
    for bank in 0..2 {
        for entry in 0..32 {
            let handle = ram::g_object_table(r).at(bank).at(entry).object();
            let object = handle.read(c);
            if object != 0 {
                call(c, ids::FREE_ENTITY, &[i32::from(object) as u32])?;
                handle.write(c, 0);
                return Ok(Some(
                    i32::from(call(c, ids::ALLOCATE_ENTITY, &[])? as i16) as u32
                ));
            }
        }
    }
    for bank in 0..2 {
        if ram::g_earthling_bank_users(r).at(bank).read(c) == 0 {
            continue;
        }
        let records = ram::g_earthling_banks(r).at(bank);
        for entry in 0..20 {
            let handle = records.at(entry).second_object();
            let object = handle.read(c);
            if object != 0 {
                call(c, ids::FREE_ENTITY, &[i32::from(object) as u32])?;
                handle.write(c, 0);
                return Ok(Some(
                    i32::from(call(c, ids::ALLOCATE_ENTITY, &[])? as i16) as u32
                ));
            }
        }
        for entry in 0..20 {
            let record = records.at(entry);
            if record.kind().read(c) != 0 {
                let object = record.object().read(c);
                if object != 0 {
                    call(c, ids::FREE_ENTITY, &[i32::from(object) as u32])?;
                    record.object().write(c, 0);
                    return Ok(Some(
                        i32::from(call(c, ids::ALLOCATE_ENTITY, &[])? as i16) as u32
                    ));
                }
            }
        }
    }
    Ok(Some(0))
}

/// G/src/map/land_processing.c:91 (`EntityReleaseOneByPriority`).
pub fn entity_release_one_by_priority(c: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let object = call(c, ids::ALLOCATE_ENTITY_WITH_EVICTION, &[1])? as i16;
    if object != 0 {
        call(c, ids::FREE_ENTITY, &[i32::from(object) as u32])?;
    }
    Ok(None)
}

/// G/src/map/land_processing.c:310 (`EntityStartLandPathAnimation`).
pub fn entity_start_land_path_animation(c: &mut dyn Context) -> Result<Option<u32>> {
    super::frame_support::in_frame(c, FrameLayout::movem(0, 0x1c, 0x04), |c, frame| {
        let r = c.revision();
        let camera = i32::from(frame.argument_word(0).read(c));
        let x = frame.argument_word(1).read(c);
        let y = frame.argument_word(2).read(c);
        for animation in 0..8 {
            c.set_data_register(4, (c.data_register(4) & 0xffff0000) | animation as u32);
            let number = ram::g_land_path_animation_entities(r).at(animation).read(c);
            let object = ram::g_player_info(r).at_word_index(i16::from(number));
            c.set_address_register(2, object.address());
            if object.frame_duration().read(c) == 0 {
                object.frame().write(c, -1);
                object.frame_duration().write(c, 1);
                object.x().write(c, x);
                object.y().write(c, y);
                if ram::g_land_path_traversal_direction(r).read(c) & 5 != 0 {
                    let site = CallSite::relative(ids::ENTITY_START_LAND_PATH_ANIMATION, r, 0x5c);
                    let delta =
                        (call_at(c, ids::GET_RANDOM_NUMBER, &[], site)? as u16 % 97) as i16 + 16;
                    object.x().write(c, object.x().read(c).wrapping_add(delta));
                } else {
                    let site = CallSite::relative(ids::ENTITY_START_LAND_PATH_ANIMATION, r, 0x84);
                    let delta =
                        (call_at(c, ids::GET_RANDOM_NUMBER, &[], site)? as u16 % 33) as i16 + 16;
                    object.y().write(c, object.y().read(c).wrapping_add(delta));
                }
                if ram::g_split_screen_enabled(r).read(c) == 0
                    || ram::g_player_inventory_open(r).at(camera).read(c) == 0
                {
                    object
                        .behavior_flags()
                        .write(c, object.behavior_flags().read(c) & !0x40);
                }
                let site = CallSite::relative(ids::ENTITY_START_LAND_PATH_ANIMATION, r, 0xd2);
                call_at(c, ids::PLAY_SOUND, &[16, 50], site)?;
                break;
            }
        }
        let site = CallSite::relative(ids::ENTITY_START_LAND_PATH_ANIMATION, r, 0xe6);
        call_at(c, ids::UPDATE_AND_QUEUE_ENTITY_SPRITES, &[], site)?;
        Ok(None)
    })
}

/// G/src/map/land_processing.c:340 (`MapAnimateMetatilePathChange`).
pub fn map_animate_metatile_path_change(c: &mut dyn Context) -> Result<Option<u32>> {
    // G matching asm:1099-1103 and1232-1252/1310-1330: real LINK12
    // locals and a retained12-byte effect area underneath the24-byte queue area.
    super::frame_support::in_frame(c, FrameLayout::link(12, 0x7c, 0x04), |c, frame| {
        let r = c.revision();
        let camera = frame.argument_word(0).read(c);
        let row = frame.argument_word(1).read(c);
        let column = frame.argument_word(2).read(c);
        let tile_row = frame.local::<i16>(-2);
        let tile_column = frame.local::<i16>(-4);
        let first_row = frame.local::<i16>(-6);
        let first_column = frame.local::<i16>(-8);
        let cell = frame.local::<u16>(-10);
        let (map, row_cursor, column_cursor) = if camera == 0 {
            (
                ram::g_player1_level_map(r),
                ram::g_camera1_tile_row_cursor(r),
                ram::g_camera1_tile_column_cursor(r),
            )
        } else {
            (
                ram::g_player2_level_map(r),
                ram::g_camera2_tile_row_cursor(r),
                ram::g_camera2_tile_column_cursor(r),
            )
        };
        let value = map.at_word_index(row).at_word_index(column).read(c);
        cell.write(c, value);
        let value = row_cursor.read(c);
        first_row.write(c, value);
        let value = column_cursor.read(c);
        first_column.write(c, value);
        tile_row.write(c, ((i32::from(row) + 1) << 3) as i16);
        tile_column.write(c, ((i32::from(column) + 1) << 4) as i16);
        if i32::from(first_row.read(c)) > i32::from(tile_row.read(c)) + 7
            || i32::from(tile_row.read(c))
                > i32::from(ram::g_camera_column_upload_height(r).read(c))
                    + i32::from(first_row.read(c))
                    - 1
            || i32::from(first_column.read(c)) > i32::from(tile_column.read(c)) + 15
            || i32::from(tile_column.read(c)) > i32::from(first_column.read(c)) + 44
        {
            return Ok(None);
        }
        let dispatcher = c.dispatcher();
        if ram::g_land_path_traversal_direction(r).read(c) & 5 != 0 {
            for step in 0..8_i16 {
                let (offset, effect) = if ram::g_land_path_traversal_direction(r).read(c) & 1 != 0 {
                    (7 - step, 5 - step)
                } else {
                    (step, step + 2)
                };
                if i32::from(first_row.read(c)) > i32::from(tile_row.read(c)) + i32::from(offset)
                    || i32::from(tile_row.read(c)) + i32::from(offset)
                        > i32::from(ram::g_camera_column_upload_height(r).read(c))
                            + i32::from(first_row.read(c))
                            - 1
                {
                    continue;
                }
                let values = [
                    i32::from(camera) as u32,
                    ((i32::from(tile_column.read(c)) << 3) + 8) as u32,
                    (((i32::from(tile_row.read(c)) + i32::from(effect)) << 3) + 8) as u32,
                ];
                let effect_args = OutgoingArguments::push(c, &values)?;
                let site = CallSite::relative(ids::MAP_ANIMATE_METATILE_PATH_CHANGE, r, 0x16a);
                effect_args.call(c, dispatcher, ids::ENTITY_START_LAND_PATH_ANIMATION, site)?;
                let values = [
                    u32::from(cell.read(c)),
                    i32::from(tile_row.read(c)) as u32,
                    i32::from(tile_column.read(c)) as u32,
                    i32::from(offset) as u32,
                    i32::from(camera) as u32,
                    i32::from(first_column.read(c)) as u32,
                ];
                let queue_args = OutgoingArguments::push(c, &values)?;
                let site = CallSite::relative(ids::MAP_ANIMATE_METATILE_PATH_CHANGE, r, 0x19a);
                queue_args.call(c, dispatcher, ids::MAP_QUEUE_VISIBLE_METATILE_ROW, site)?;
                queue_args.release(c)?;
                effect_args.release(c)?;
            }
        } else {
            for step in 0..16_i16 {
                let (offset, effect) = if ram::g_land_path_traversal_direction(r).read(c) & 8 != 0 {
                    (15 - step, 13 - step)
                } else {
                    (step, step + 2)
                };
                if i32::from(first_column.read(c))
                    > i32::from(tile_column.read(c)) + i32::from(offset)
                    || i32::from(tile_column.read(c)) + i32::from(offset)
                        > i32::from(first_column.read(c)) + 44
                {
                    continue;
                }
                let values = [
                    i32::from(camera) as u32,
                    (((i32::from(tile_column.read(c)) + i32::from(effect)) << 3) + 12) as u32,
                    ((i32::from(tile_row.read(c)) << 3) + 8) as u32,
                ];
                let effect_args = OutgoingArguments::push(c, &values)?;
                let site = CallSite::relative(ids::MAP_ANIMATE_METATILE_PATH_CHANGE, r, 0x220);
                effect_args.call(c, dispatcher, ids::ENTITY_START_LAND_PATH_ANIMATION, site)?;
                let values = [
                    u32::from(cell.read(c)),
                    i32::from(tile_row.read(c)) as u32,
                    i32::from(tile_column.read(c)) as u32,
                    i32::from(offset) as u32,
                    i32::from(camera) as u32,
                    i32::from(first_row.read(c)) as u32,
                ];
                let queue_args = OutgoingArguments::push(c, &values)?;
                let site = CallSite::relative(ids::MAP_ANIMATE_METATILE_PATH_CHANGE, r, 0x250);
                queue_args.call(c, dispatcher, ids::MAP_QUEUE_VISIBLE_METATILE_COLUMN, site)?;
                queue_args.release(c)?;
                effect_args.release(c)?;
            }
        }
        Ok(None)
    })
}

/// G/src/map/land_processing.c:405 (`EntityReserveLandPathAnimations`).
pub fn entity_reserve_land_path_animations(c: &mut dyn Context) -> Result<Option<u32>> {
    // Both matching listings: MOVEM D2-D4/A2 without LINK.
    let frame = LocalFrame::enter(c, FrameLayout::movem(0, 0x1c, 0x04))?;
    let r = c.revision();
    let camera = i32::from(frame.argument_word(0).read(c));
    c.set_data_register(
        2,
        (c.data_register(2) & 0xffff_0000) | u32::from(camera as u16),
    );
    // The original performs these two unused parameter loads too.
    let _ = frame.argument_word(1).read(c);
    let _ = frame.argument_word(2).read(c);
    let row_pending = ram::g_camera_row_upload_pending(r).at(camera);
    let column_pending = ram::g_camera_column_upload_pending(r).at(camera);
    while row_pending.read(c) != 0 || column_pending.read(c) != 0 {
        if row_pending.read(c) != 0 {
            super::waits::wait_pending_byte(c, row_pending)?;
        } else {
            super::waits::wait_pending_byte(c, column_pending)?;
        }
    }
    let level = if camera == 0 {
        ram::g_player1_map_level(r).read(c)
    } else {
        ram::g_player2_map_level(r).read(c)
    };
    c.set_data_register(
        3,
        (c.data_register(3) & 0xffff_0000) | i32::from(level) as u32 & 0xffff,
    );
    for animation in 0..8 {
        let site = CallSite::relative(ids::ENTITY_RESERVE_LAND_PATH_ANIMATIONS, r, 0x4c);
        let number = call_at(c, ids::ALLOCATE_ENTITY_WITH_EVICTION, &[0], site)? as i16;
        ram::g_land_path_animation_entities(r)
            .at(animation)
            .write(c, number as i8);
        // Matching asm:1376-1379 uses ASL.W #7 then ADDA.W.
        let object = ram::g_player_info(r).at_word_index(number);
        c.set_address_register(2, object.address());
        object.level().write(c, level);
        object.width().write(c, 32);
        object.height_pixels().write(c, 32);
        object.sprite_count().write(c, 1);
        object.tile_count().write(c, 16);
        object.tile().write(c, object.tile().read(c) | 0x8000);
        let data = rom::transient_effect_animation(r);
        object.sequence().write(c, data.sequence().read(c));
        object.frames().write(c, data.frames().read(c));
        object.frame_phase().write(c, 2);
        loop {
            // G matching asm :1367/1408 keeps the animation counter in D4.W.
            // AllocateEntityViewportSprites can consume D4.B on its bug path.
            let saved_d4 = c.data_register(4);
            c.set_data_register(4, (saved_d4 & 0xffff0000) | animation as u32);
            let site = CallSite::relative(ids::ENTITY_RESERVE_LAND_PATH_ANIMATIONS, r, 0xb0);
            let result = call_at(
                c,
                ids::ALLOCATE_ENTITY_VIEWPORT_SPRITES,
                &[object.address(), camera as u32],
                site,
            );
            c.set_data_register(4, saved_d4);
            result?;
            if object.draw_flags().read(c) & 6 != 0 {
                break;
            }
            let site = CallSite::relative(ids::ENTITY_RESERVE_LAND_PATH_ANIMATIONS, r, 0xc2);
            call_at(c, ids::ENTITY_RELEASE_ONE_BY_PRIORITY, &[], site)?;
        }
        object
            .draw_flags()
            .write(c, object.draw_flags().read(c) | 0x80);
        object
            .behavior_flags()
            .write(c, object.behavior_flags().read(c) | 0x40);
    }
    frame.leave(c)?;
    Ok(None)
}

/// G/src/map/land_processing.c:473-480; matching asm:1483-1498 performs
/// three separate conditional byte reads, not one cached switch operand.
fn advance_marked_position(c: &mut dyn Context, row: &mut i16, column: Address<i16>) {
    let direction = ram::g_land_path_traversal_direction(c.revision());
    if direction.read(c) == 2 {
        column.write(c, column.read(c).wrapping_add(1));
    } else if direction.read(c) == 8 {
        column.write(c, column.read(c).wrapping_sub(1));
    } else if direction.read(c) == 1 {
        *row = row.wrapping_sub(1);
    } else {
        *row = row.wrapping_add(1);
    }
}

/// G/src/map/land_processing.c:494; matching asm:1539-1540 loads player2
/// first, then compares the player1 byte. The operand order is observable.
fn marked_banks_share_level(c: &dyn Context) -> bool {
    let r = c.revision();
    let second = ram::g_player2_map_level(r).read(c);
    second == ram::g_player1_map_level(r).read(c)
}

/// G/src/map/land_processing.c:445 (`MapAnimateMarkedLandPaths`).
/// Both matching listings: LINK A6,#-32, MOVEM D2-D6/A2-A4; the saved
/// durations occupy A6-32..A6-7, current_column A6-2 and object_number A6-4.
/// The first reservation consumes entry D6.W and the unwritten column word.
/// These residues are retained, not replaced by zero or the starting cell.
pub fn map_animate_marked_land_paths(c: &mut dyn Context) -> Result<Option<u32>> {
    let frame = LocalFrame::enter(c, FrameLayout::link(32, 0x7c, 0x1c))?;
    let r = c.revision();
    let mut camera = frame.argument_word(0).read(c);
    let row = frame.argument_word(1).read(c);
    let column = frame.argument_word(2).read(c);
    let saved = frame.local::<i8>(-32);
    let current_column = frame.local::<i16>(-2);
    let object_number = frame.local::<i16>(-4);
    let banks = frame.local::<i16>(-6);
    let mut current_row = c.data_register(6) as i16;
    c.set_data_register(
        2,
        (c.data_register(2) & 0xffff_0000) | u32::from(camera as u16),
    );
    c.set_data_register(
        3,
        (c.data_register(3) & 0xffff_0000) | u32::from(row as u16),
    );
    c.set_data_register(
        4,
        (c.data_register(4) & 0xffff_0000) | u32::from(column as u16),
    );
    c.set_address_register(2, ram::g_land_path_traversal_direction(r).address());
    c.set_address_register(3, ram::g_player_info(r).address());
    for animation in 0..26 {
        let duration = ram::g_player_info(r).at(animation).frame_duration();
        saved.offset(animation).write(c, duration.read(c));
        duration.write(c, 0);
    }
    c.set_data_register(5, (c.data_register(5) & 0xffff_0000) | 26);
    banks.write(c, 0);
    loop {
        c.set_data_register(
            2,
            (c.data_register(2) & 0xffff_0000) | u32::from(camera as u16),
        );
        c.set_data_register(
            6,
            (c.data_register(6) & 0xffff_0000) | u32::from(current_row as u16),
        );
        let arguments = [
            i32::from(camera) as u32,
            i32::from(current_row) as u32,
            i32::from(current_column.read(c)) as u32,
        ];
        // Linked return PCs: REV 02 0x1f550, REV 00 0x1f50e.
        let site = CallSite::relative(ids::MAP_ANIMATE_MARKED_LAND_PATHS, r, 0x58);
        call_at(
            c,
            ids::ENTITY_RESERVE_LAND_PATH_ANIMATIONS,
            &arguments,
            site,
        )?;
        let mut result = 0_i8;
        while result != -1 {
            current_row = row;
            current_column.write(c, column);
            ram::g_land_path_traversal_direction(r).write(c, -1);
            result = 0;
            while result == 0 {
                c.set_data_register(
                    6,
                    (c.data_register(6) & 0xffff_0000) | u32::from(current_row as u16),
                );
                let arguments = [
                    i32::from(camera) as u32,
                    i32::from(current_row) as u32,
                    i32::from(current_column.read(c)) as u32,
                ];
                // The original-bug shape path consumes current_row in D6.B.
                let site = CallSite::relative(ids::MAP_ANIMATE_MARKED_LAND_PATHS, r, 0x8a);
                result = call_at(c, ids::MAP_CONSUME_PATH_CELL_DIRECTIONS, &arguments, site)? as i8;
                if result != -1 {
                    // Reload the real column slot after the consuming callee;
                    // its original unchecked stores may alias this caller local.
                    let animation_arguments = [
                        i32::from(camera) as u32,
                        i32::from(current_row) as u32,
                        i32::from(current_column.read(c)) as u32,
                    ];
                    let site = CallSite::relative(ids::MAP_ANIMATE_MARKED_LAND_PATHS, r, 0xae);
                    call_at(
                        c,
                        ids::MAP_ANIMATE_METATILE_PATH_CHANGE,
                        &animation_arguments,
                        site,
                    )?;
                }
                advance_marked_position(c, &mut current_row, current_column);
            }
        }
        for animation in 0..8 {
            // MOVE.B to A6-3 then EXT.W sign-extends that byte into A6-4.
            let number = ram::g_land_path_animation_entities(r).at(animation).read(c);
            frame.local::<i8>(-3).write(c, number);
            let number = object_number.read(c) as i8;
            object_number.write(c, i16::from(number));
            let object = ram::g_player_info(r).at(i32::from(number));
            c.set_address_register(4, object.address());
            while object.frame_duration().read(c) != 0 {
                let site = CallSite::relative(ids::MAP_ANIMATE_MARKED_LAND_PATHS, r, 0x110);
                call_at(c, ids::UPDATE_AND_QUEUE_ENTITY_SPRITES, &[], site)?;
                super::waits::wait_pending_byte(
                    c,
                    ram::g_sprite_graphics_upload_pending(r).cast(),
                )?;
            }
            let site = CallSite::relative(ids::MAP_ANIMATE_MARKED_LAND_PATHS, r, 0x12c);
            let arguments = [i32::from(object_number.read(c)) as u32];
            call_at(c, ids::FREE_ENTITY, &arguments, site)?;
            ram::g_land_path_animation_entities(r)
                .at(animation)
                .write(c, 0);
        }
        if !marked_banks_share_level(c) {
            break;
        }
        if ram::g_split_screen_enabled(r).read(c) == 0 {
            let site = CallSite::relative(ids::MAP_ANIMATE_MARKED_LAND_PATHS, r, 0x164);
            call_at(
                c,
                ids::COPY_LEVEL_BETWEEN_PLAYERS,
                &[i32::from(camera) as u32],
                site,
            )?;
            break;
        }
        camera ^= 1;
        banks.write(c, banks.read(c).wrapping_add(1));
        if banks.read(c) >= 2 {
            break;
        }
    }
    for animation in 0..26 {
        ram::g_player_info(r)
            .at(animation)
            .frame_duration()
            .write(c, saved.offset(animation).read(c));
    }
    frame.leave(c)?;
    Ok(None)
}

// Matching ProcessLand return PCs; REV 02's whirlwind exclusions change the
// second player's offsets. Each row is ToeJam, split Earl, shared-view Earl.
fn process_site(c: &dyn Context, branch: usize, operation: usize) -> CallSite {
    let sites = if c.revision() == tje_mem::Revision::Rev02 {
        [
            [0x1f754, 0x1f76e, 0x1f782, 0x1f78e, 0x1f7c6, 0x1f7d0],
            [0x1f85c, 0x1f878, 0x1f88e, 0x1f89c, 0x1f978, 0x1f984],
            [0x1f904, 0x1f91e, 0x1f932, 0x1f940, 0x1f978, 0x1f984],
        ]
    } else {
        [
            [0x1f712, 0x1f72a, 0x1f73e, 0x1f74a, 0x1f772, 0x1f77c],
            [0x1f808, 0x1f824, 0x1f83a, 0x1f848, 0x1f900, 0x1f90c],
            [0x1f89c, 0x1f8b6, 0x1f8ca, 0x1f8d8, 0x1f900, 0x1f90c],
        ]
    };
    CallSite::new(sites[branch][operation])
}

/// G/src/map/land_processing.c:542-546/563-567/578-582. REV02's two
/// CMPI.B instructions reread sprite_state unless the first comparison matches.
fn path_change_can_fall(
    c: &dyn Context,
    terrain: Address<u8>,
    object: Address<ObjectState>,
) -> bool {
    terrain.read(c) == 0x90
        && object.height().read(c) == 0
        && (c.revision() == tje_mem::Revision::Rev00
            || (object.sprite_state().read(c) != 78 && object.sprite_state().read(c) != 79))
        && object.animation_group_mask().read(c) & 0x8020 == 0
}

/// G/src/map/land_processing.c:527 (`ProcessLand`), including REV 02 whirlwind exclusions.
pub fn process_land(c: &mut dyn Context) -> Result<Option<u32>> {
    // Both matching listings: MOVEM D2-D3/A2-A3 without LINK.
    let frame = LocalFrame::enter(c, FrameLayout::movem(0, 0x0c, 0x0c))?;
    let r = c.revision();
    let dispatcher = c.dispatcher();
    c.set_address_register(2, ram::g_player_info(r).address());
    for player in 0..2 {
        let object = ram::g_player_info(r).at(player);
        if ram::g_player_lives(r).at(player).read(c) < 0
            || object.height().read(c) > 4
            || object.level().read(c) <= 1
            || ram::g_player_elevator_flags(r).at(player).read(c) != 0
        {
            continue;
        }
        let row = i32::from((object.y().read(c) >> 6).wrapping_sub(1));
        let column = i32::from((object.x().read(c) >> 7).wrapping_sub(1));
        c.set_data_register(2, (c.data_register(2) & 0xffff_0000) | row as u32 & 0xffff);
        c.set_data_register(
            3,
            (c.data_register(3) & 0xffff_0000) | column as u32 & 0xffff,
        );
        if !(0..26).contains(&row) || !(0..19).contains(&column) {
            continue;
        }
        let camera = if player == 0 || ram::g_split_screen_enabled(r).read(c) == 0 {
            0
        } else {
            1
        };
        let (map, level) = if camera == 0 {
            (ram::g_player1_level_map(r), ram::g_player1_map_level(r))
        } else {
            (ram::g_player2_level_map(r), ram::g_player2_map_level(r))
        };
        let branch = if player == 0 {
            0
        } else if camera == 1 {
            1
        } else {
            2
        };
        let edge_site = process_site(c, branch, 0);
        if map.at(row).at(column).read(c) & 0x4000 == 0
            || call_at(
                c,
                ids::PLAYER_IS_NEAR_TERRAIN_EDGE,
                &[player as u32],
                edge_site,
            )? == 0
        {
            continue;
        }
        c.set_data_register(2, row as u32);
        let outgoing = OutgoingArguments::push(c, &[camera as u32, row as u32, column as u32])?;
        let site = process_site(c, branch, 1);
        outgoing.call(c, dispatcher, ids::MAP_ANIMATE_MARKED_LAND_PATHS, site)?;
        // ORIGINAL shared-view Earl path damages/rechecks bank zero as ToeJam.
        let damage_player = if player == 1 && camera == 0 {
            0
        } else {
            player
        };
        let map_level = i32::from(level.read(c)) as u32;
        let damage = OutgoingArguments::push(c, &[damage_player as u32, map_level])?;
        let site = process_site(c, branch, 2);
        damage.call(c, dispatcher, ids::EARTHLING_DAMAGE_ON_HAZARD_TERRAIN, site)?;
        c.set_address_register(3, object.address());
        let update = OutgoingArguments::push(c, &[player as u32])?;
        let site = process_site(c, branch, 3);
        update.call(c, dispatcher, ids::UPDATE_LAND_AROUND_PLAYER, site)?;
        OutgoingArguments::existing(c, 6)?.release(c)?;
        let terrain = if player == 0 {
            ram::g_player1_terrain_underfoot(r)
        } else {
            ram::g_player2_terrain_underfoot(r)
        };
        if path_change_can_fall(c, terrain, object) {
            let stop = OutgoingArguments::push(c, &[object.address()])?;
            let site = process_site(c, branch, 4);
            stop.call(c, dispatcher, ids::STOP_PLAYER, site)?;
            let falling = OutgoingArguments::push(c, &[object.address(), player as u32])?;
            let site = process_site(c, branch, 5);
            falling.call(c, dispatcher, ids::PLAYER_START_FALLING, site)?;
            OutgoingArguments::existing(c, 3)?.release(c)?;
        }
    }
    frame.leave(c)?;
    Ok(None)
}

#[cfg(test)]
#[path = "land_replay_tests.rs"]
mod replay_tests;
