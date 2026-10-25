//! Bounded gameplay callbacks for EnhancedMode's explicit, separate registry.
//! These profiles change simulation visibility; they are not faithful proofs or
//! complete widescreen composition. No frontend selects them automatically.
use tje_engine_runtime::enhanced::EnhancedProfile;
use tje_game::{Context, Result, Translation, call_void, ids};
use tje_mem::{Address, Bus, EarthlingState, ObjectState, Revision, ram};

pub const WIDESCREEN_PROFILE: EnhancedProfile =
    EnhancedProfile::new("extended-view-400/1", WIDESCREEN_TRANSLATIONS, &[]);
pub const EXPERIMENTAL_512_PROFILE: EnhancedProfile = EnhancedProfile::new(
    "experimental-extended-view-512/1",
    EXPERIMENTAL_TRANSLATIONS,
    &[],
);

static WIDESCREEN_TRANSLATIONS: &[Translation] = &[
    Translation {
        id: ids::IS_ENTITY_VISIBLE_IN_VIEWPORT,
        function: is_visible::<40>,
    },
    Translation {
        id: ids::LOAD_IN_EARTHLINGS,
        function: load_earthlings::<40>,
    },
    Translation {
        id: ids::ELEVATOR_UPDATE_ENTITY_VISIBILITY,
        function: update_elevators::<40>,
    },
];
static EXPERIMENTAL_TRANSLATIONS: &[Translation] = &[
    Translation {
        id: ids::IS_ENTITY_VISIBLE_IN_VIEWPORT,
        function: is_visible::<96>,
    },
    Translation {
        id: ids::LOAD_IN_EARTHLINGS,
        function: load_earthlings::<96>,
    },
    Translation {
        id: ids::ELEVATOR_UPDATE_ENTITY_VISIBILITY,
        function: update_elevators::<96>,
    },
];

fn interest<const MARGIN: i32>(x: i16, y: i16) -> bool {
    i32::from(x) > -160 - MARGIN
        && i32::from(x) < 480 + MARGIN
        && i32::from(y) > -112
        && i32::from(y) < 336
}

/// G/src/sprites/entity_visibility.c:11-42, with only horizontal bounds changed.
fn visible<const MARGIN: i32, B: Bus + ?Sized>(
    bus: &B,
    revision: Revision,
    entity: Address<ObjectState>,
    viewport: i16,
) -> bool {
    let (x, y, height) = if viewport == 0 {
        if ram::g_player_inventory_open(revision).at(0).read(bus) != 0
            || entity.level().read(bus) != ram::g_player1_map_level(revision).read(bus)
        {
            return false;
        }
        (
            entity
                .x()
                .read(bus)
                .wrapping_sub(ram::g_camera1_pending_x(revision).read(bus)),
            entity
                .y()
                .read(bus)
                .wrapping_sub(ram::g_camera1_pending_y(revision).read(bus)),
            if ram::g_split_screen_enabled(revision).read(bus) == 0 {
                224
            } else {
                112
            },
        )
    } else {
        if ram::g_split_screen_enabled(revision).read(bus) == 0
            || ram::g_player2_inventory_open(revision).read(bus) != 0
            || entity.level().read(bus) != ram::g_player2_map_level(revision).read(bus)
        {
            return false;
        }
        (
            entity
                .x()
                .read(bus)
                .wrapping_sub(ram::g_camera2_pending_x(revision).read(bus)),
            entity
                .y()
                .read(bus)
                .wrapping_sub(ram::g_camera2_pending_y(revision).read(bus)),
            112,
        )
    };
    i32::from(entity.width().read(bus)) + i32::from(x) >= -MARGIN
        && i32::from(x) - i32::from(entity.width().read(bus)) <= 320 + MARGIN
        && i32::from(y) + 10 > 0
        && i32::from(y)
            - i32::from(entity.height_pixels().read(bus))
            - i32::from(entity.height().read(bus))
            <= height
}

fn is_visible<const MARGIN: i32>(context: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    Ok(Some(u32::from(visible::<MARGIN, _>(
        context,
        context.revision(),
        Address::new(args[0]),
        args[1] as i16,
    ))))
}

/// G/src/earthlings/spawning_and_banks.c:426-473. Random placement, parity,
/// vertical interest, entity budgets and original allocation/free stay intact.
fn load_earthlings<const MARGIN: i32>(
    context: &mut dyn Context,
    args: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    let records = Address::<EarthlingState>::new(args[0]);
    let mut index = ram::g_earthling_update_parity(revision).read(context);
    while index < 20 {
        let earthling = records.offset(i32::from(index));
        if earthling.object().read(context) != 0 {
            let object =
                ram::g_player_info(revision).at(i32::from(earthling.object().read(context)));
            earthling.x().write(context, object.x().read(context));
            earthling.y().write(context, object.y().read(context));
        }
        let tx = earthling
            .x()
            .read(context)
            .wrapping_sub(ram::g_camera1_pending_x(revision).read(context));
        let ty = earthling
            .y()
            .read(context)
            .wrapping_sub(ram::g_camera1_pending_y(revision).read(context));
        let ex = earthling
            .x()
            .read(context)
            .wrapping_sub(ram::g_camera2_pending_x(revision).read(context));
        let ey = earthling
            .y()
            .read(context)
            .wrapping_sub(ram::g_camera2_pending_y(revision).read(context));
        let visibility = earthling.health_visibility().at(1);
        if earthling.kind().read(context) != -1 {
            let primary = earthling.level().read(context)
                == ram::g_player1_map_level(revision).read(context)
                && interest::<MARGIN>(tx, ty);
            let secondary = earthling.level().read(context)
                == ram::g_player2_map_level(revision).read(context)
                && ram::g_split_screen_enabled(revision).read(context) != 0
                && interest::<MARGIN>(ex, ey);
            let flags = visibility.read(context) & !6;
            visibility.write(
                context,
                flags | if primary { 2 } else { 0 } | if secondary { 4 } else { 0 },
            );
        } else {
            visibility.write(context, 0);
        }
        if visibility.read(context) & 6 != 0 {
            if earthling.object().read(context) == 0 {
                call_void(
                    context,
                    ids::EARTHLING_ALLOCATE_ENTITIES,
                    &[earthling.address()],
                )?;
            }
        } else {
            if earthling.object().read(context) != 0 {
                let object = earthling.object().read(context) as u32;
                call_void(context, ids::FREE_ENTITY, &[object])?;
                earthling.object().write(context, 0);
            }
            if earthling.second_object().read(context) != 0 {
                let object = earthling.second_object().read(context) as u32;
                call_void(context, ids::FREE_ENTITY, &[object])?;
                earthling.second_object().write(context, 0);
            }
        }
        index = index.wrapping_add(2);
    }
    Ok(None)
}

/// G/src/elevator/elevator_scene.c:203-235, with only horizontal interest changed.
fn update_elevators<const MARGIN: i32>(
    context: &mut dyn Context,
    _: &[u32],
) -> Result<Option<u32>> {
    let revision = context.revision();
    for bank in 0..2 {
        for kind in 0..2 {
            let elevator = ram::g_elevator_records(revision).at(bank).at(kind);
            let tx = elevator
                .x()
                .read(context)
                .wrapping_sub(ram::g_camera1_pending_x(revision).read(context));
            let ty = elevator
                .y()
                .read(context)
                .wrapping_sub(ram::g_camera1_pending_y(revision).read(context));
            let ex = elevator
                .x()
                .read(context)
                .wrapping_sub(ram::g_camera2_pending_x(revision).read(context));
            let ey = elevator
                .y()
                .read(context)
                .wrapping_sub(ram::g_camera2_pending_y(revision).read(context));
            let primary = elevator.level().read(context)
                == ram::g_player1_map_level(revision).read(context)
                && interest::<MARGIN>(tx, ty);
            let secondary = elevator.level().read(context)
                == ram::g_player2_map_level(revision).read(context)
                && ram::g_split_screen_enabled(revision).read(context) != 0
                && interest::<MARGIN>(ex, ey);
            if elevator.state().read(context) != 11
                && elevator.state().read(context) != 13
                && elevator.level().read(context) > 0
                && (primary || secondary)
            {
                if elevator.entity().read(context) == 0 {
                    call_void(
                        context,
                        ids::ELEVATOR_CREATE_ENTITY_PAIR,
                        &[elevator.address()],
                    )?;
                }
            } else {
                if elevator.entity().read(context) != 0 {
                    let number = i32::from(elevator.entity().read(context));
                    call_void(context, ids::FREE_ENTITY, &[number as u32])?;
                    let number = i32::from(elevator.entity().read(context)) + 2;
                    call_void(context, ids::FREE_ENTITY, &[number as u32])?;
                    elevator.entity().write(context, 0);
                }
                if elevator.state().read(context) == 11 {
                    elevator.level().write(context, -1);
                }
            }
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tje_mem::AddressSpace;

    #[test]
    fn profiles_have_separate_width_identities_and_only_the_three_overrides() {
        assert_ne!(WIDESCREEN_PROFILE.id, EXPERIMENTAL_512_PROFILE.id);
        for revision in Revision::ALL {
            for profile in [WIDESCREEN_PROFILE, EXPERIMENTAL_512_PROFILE] {
                let mode =
                    tje_engine_runtime::enhanced::EnhancedMode::new(profile, revision).unwrap();
                assert_eq!(mode.profile_id(), profile.id);
                assert_eq!(profile.translations.len(), 3);
                assert!(profile.framed_translations.is_empty());
            }
        }
        assert_eq!(
            tje_engine_runtime::enhanced::EnhancedMode::default().profile_id(),
            "faithful"
        );
    }

    #[test]
    fn activation_expands_only_x_and_preserves_strict_edges() {
        for x in [-200, 520] {
            assert!(!interest::<40>(x, 0));
        }
        for x in [-199, -160, 479, 519] {
            assert!(interest::<40>(x, 0));
        }
        for y in [-112, 336] {
            assert!(!interest::<40>(0, y));
        }
        for y in [-111, 335] {
            assert!(interest::<40>(0, y));
        }
        assert!(interest::<96>(-255, 0));
        assert!(!interest::<96>(-256, 0));
        assert!(interest::<96>(575, 0));
        assert!(!interest::<96>(576, 0));
    }

    #[test]
    fn sprite_culling_expands_symmetrically_but_keeps_levels_inventory_and_height() {
        for revision in Revision::ALL {
            let mut memory = AddressSpace::new(vec![0; 8], revision);
            ram::g_player1_map_level(revision).write(&mut memory, 2);
            ram::g_player2_map_level(revision).write(&mut memory, 3);
            let entity = ram::g_player_info(revision).at(6);
            entity.level().write(&mut memory, 2);
            entity.y().write(&mut memory, 10);
            for x in [-40, 360] {
                entity.x().write(&mut memory, x);
                assert!(visible::<40, _>(&memory, revision, entity, 0));
                assert!(!visible::<0, _>(&memory, revision, entity, 0));
            }
            for x in [-41, 361] {
                entity.x().write(&mut memory, x);
                assert!(!visible::<40, _>(&memory, revision, entity, 0));
            }
            entity.x().write(&mut memory, 0);
            entity.y().write(&mut memory, 113);
            assert!(visible::<40, _>(&memory, revision, entity, 0));
            ram::g_split_screen_enabled(revision).write(&mut memory, 1);
            assert!(!visible::<40, _>(&memory, revision, entity, 0));
            entity.y().write(&mut memory, 112);
            assert!(visible::<40, _>(&memory, revision, entity, 0));
            assert!(!visible::<40, _>(&memory, revision, entity, 1));
            entity.level().write(&mut memory, 3);
            assert!(visible::<40, _>(&memory, revision, entity, 1));
            ram::g_player2_inventory_open(revision).write(&mut memory, 1);
            assert!(!visible::<40, _>(&memory, revision, entity, 1));
            entity.level().write(&mut memory, 2);
            ram::g_player_inventory_open(revision)
                .at(0)
                .write(&mut memory, 1);
            assert!(!visible::<40, _>(&memory, revision, entity, 0));
        }
    }
}
