//! Explicit versioned enhanced profiles. Native sprite allocation stays 320px;
//! only logical world interest expands. Host wings consume published descriptors.
use crate::presentation::{PriorityPatch, PublishedView};
use tje_engine_runtime::enhanced::{
    Context as EnhancedContext, Continuation, EnhancedMode, EnhancedProfile, OverrideStep,
    ResumableTranslation, Sidecar,
};
use tje_game::{Context, FunctionId, Result, Translation, call_void, ids};
use tje_mem::{Address, EarthlingState, ram};
#[cfg(test)]
use tje_mem::{Bus, ObjectState, Revision};

impl Sidecar for PublishedView {
    const KEY: &'static str = "extended-view/publication";
    const VERSION: u32 = 1;

    fn configuration_identity(&self) -> &[u8] {
        &self.width_identity
    }

    fn validate(&self) -> Result<()> {
        if profile(self.width).is_none()
            || self.width_identity != self.width.to_le_bytes()
            || self.published_index > 1
            || self
                .entities
                .iter()
                .flatten()
                .any(|entity| entity.count > 12)
        {
            return Err(tje_game::Error::Platform(
                "invalid extended-view sidecar".into(),
            ));
        }
        Ok(())
    }
}

pub static CORE_OVERRIDES: &[ResumableTranslation] = &[
    ResumableTranslation {
        id: ids::HANDLE_V_BLANK,
        function: publish,
    },
    ResumableTranslation {
        id: ids::BUILD_VIEWPORT_SPRITE_LINKS,
        function: queue_publication,
    },
    ResumableTranslation {
        id: ids::MAP_CLEAR_TILE_PRIORITY_NEAR_PLAYER,
        function: clear_priority,
    },
    ResumableTranslation {
        id: ids::HUD_POSITION_LEVEL_NUMBER_SPRITES,
        function: hud_level,
    },
    ResumableTranslation {
        id: ids::HUD_POSITION_ACTION_ICON_SPRITE,
        function: hud_action,
    },
    ResumableTranslation {
        id: ids::HUD_POSITION_HEALTH_BAR_SPRITES,
        function: hud_health,
    },
    ResumableTranslation {
        id: ids::HUD_POSITION_LIVES_SPRITE,
        function: hud_lives,
    },
];
pub static WORLD_OVERRIDES: &[ResumableTranslation] = &[ResumableTranslation {
    id: ids::PROCESS_INPUT,
    function: process_input,
}];

macro_rules! scene_overrides {
    ($( $name:ident => $id:ident ),+ $(,)?) => {
        $(fn $name(context: &mut dyn EnhancedContext, _: &mut Continuation) -> Result<OverrideStep> {
            mark_authored(context)?;
            Ok(OverrideStep::ContinueOriginal)
        })+
        pub static SCENE_OVERRIDES: &[ResumableTranslation] = &[
            $(ResumableTranslation { id: ids::$id, function: $name }),+
        ];
    };
}
scene_overrides! {
    map_scene => DRAW_GAME_MAP,
    pause_scene => GAME_PAUSED,
    menu_scene => NEW_GAME_MAIN_MENU,
    title_scene => TITLE_SCREEN_VIDEO,
    intro_scene => SEGA_AND_INTRO_VIDEO_ROUTINE,
    ending_scene => PLAY_ENDING_SEQUENCE,
    ship_completion_scene => PLAY_SHIP_COMPLETION_SEQUENCE,
    ship_scene => SHOW_SHIP_PIECES,
    elevator_scene => ELEVATOR_BEGIN_RIDE_SCENE,
    split_scene => ENTER_SPLIT_SCREEN,
    shared_scene => ENTER_SHARED_SCREEN,
    game_over_scene => SHOW_GAME_OVER_AND_RESET,
}

static OVERRIDE_SETS: &[&[ResumableTranslation]] =
    &[CORE_OVERRIDES, WORLD_OVERRIDES, SCENE_OVERRIDES];

macro_rules! profiles {
    ($( $width:literal => ($margin:literal, $name:literal) ),+ $(,)?) => {
        pub const fn profile(width: u16) -> Option<EnhancedProfile> {
            match width {
                $($width => Some(EnhancedProfile::new($name, &[
                    Translation { id: ids::LOAD_IN_EARTHLINGS, function: load_earthlings::<$margin> },
                    Translation { id: ids::ELEVATOR_UPDATE_ENTITY_VISIBILITY, function: update_elevators::<$margin> },
                ], &[]).with_resumable_sets(OVERRIDE_SETS)),)+
                _ => None,
            }
        }
    };
}
profiles! {
    320 => (0, "extended-view-native-presentation/2"),
    400 => (40, "extended-view-400/2"),
    408 => (44, "experimental-extended-view-408/2"),
    416 => (48, "experimental-extended-view-416/2"),
    424 => (52, "experimental-extended-view-424/2"),
    432 => (56, "experimental-extended-view-432/2"),
    440 => (60, "experimental-extended-view-440/2"),
    448 => (64, "experimental-extended-view-448/2"),
    456 => (68, "experimental-extended-view-456/2"),
    464 => (72, "experimental-extended-view-464/2"),
    472 => (76, "experimental-extended-view-472/2"),
    480 => (80, "experimental-extended-view-480/2"),
    488 => (84, "experimental-extended-view-488/2"),
    496 => (88, "experimental-extended-view-496/2"),
    504 => (92, "experimental-extended-view-504/2"),
    512 => (96, "experimental-extended-view-512/2"),
}
pub const WIDESCREEN_PROFILE: EnhancedProfile = match profile(400) {
    Some(profile) => profile,
    None => unreachable!(),
};
pub const EXPERIMENTAL_512_PROFILE: EnhancedProfile = match profile(512) {
    Some(profile) => profile,
    None => unreachable!(),
};

pub fn configure(mode: &mut EnhancedMode, width: u16) -> Result<()> {
    mode.sidecars_mut().register(PublishedView::new(width))
}

pub fn mark_authored(context: &mut dyn EnhancedContext) -> Result<()> {
    context
        .sidecars_mut()
        .get_mut::<PublishedView>()?
        .world_seen = false;
    Ok(())
}

pub fn mark_world(context: &mut dyn EnhancedContext) -> Result<()> {
    context
        .sidecars_mut()
        .get_mut::<PublishedView>()?
        .world_seen = true;
    Ok(())
}

fn process_input(context: &mut dyn EnhancedContext, _: &mut Continuation) -> Result<OverrideStep> {
    mark_world(context)?;
    Ok(OverrideStep::ContinueOriginal)
}

fn publish(
    context: &mut dyn EnhancedContext,
    continuation: &mut Continuation,
) -> Result<OverrideStep> {
    if continuation.stage == 0 {
        let revision = context.revision();
        continuation.locals[0] = u32::from(
            ram::g_sprite_graphics_upload_pending(revision).read(context) != 0
                && ram::g_terrain_priority_update_pending(revision).read(context) == 0
                && ram::g_v_blank_graphics_paused(revision).read(context) == 0,
        );
        continuation.stage = 1;
        return Ok(OverrideStep::CallOriginal {
            function: ids::HANDLE_V_BLANK,
            arguments: vec![],
        });
    }
    let prior = context.sidecars().get::<PublishedView>()?;
    let publication = PublishedView::publication(context, context.revision(), prior.world_seen);
    let state = context.sidecars_mut().get_mut::<PublishedView>()?;
    state.world = publication.world;
    if publication.world {
        state.split = publication.split;
        state.views = publication.views;
        state.hud = publication.hud;
        if continuation.locals[0] != 0 && state.pending_ready {
            state.published_index ^= 1;
            state.pending_ready = false;
            state.published = true;
        }
    }
    Ok(OverrideStep::Return(None))
}

fn queue_publication(
    context: &mut dyn EnhancedContext,
    continuation: &mut Continuation,
) -> Result<OverrideStep> {
    if continuation.stage == 0 {
        continuation.stage = 1;
        return Ok(OverrideStep::CallOriginal {
            function: ids::BUILD_VIEWPORT_SPRITE_LINKS,
            arguments: vec![],
        });
    }
    let entities = PublishedView::capture_entities(context, context.revision());
    let state = context.sidecars_mut().get_mut::<PublishedView>()?;
    state.entities[state.published_index ^ 1] = entities;
    state.pending_ready = true;
    Ok(OverrideStep::Return(None))
}

fn clear_priority(
    context: &mut dyn EnhancedContext,
    continuation: &mut Continuation,
) -> Result<OverrideStep> {
    if continuation.stage == 0 {
        let revision = context.revision();
        let player = ram::g_terrain_priority_update_player(revision).read(context);
        if !(0..2).contains(&player) {
            return Err(tje_game::Error::Platform(
                "invalid terrain-priority player".into(),
            ));
        }
        let entity = ram::g_player_info(revision).at(i32::from(player));
        let patch = PriorityPatch {
            active: true,
            center_x: entity.x().read(context) >> 3,
            center_y: entity.y().read(context) >> 3,
            viewport: u8::from(
                player == 1 && ram::g_split_screen_enabled(revision).read(context) != 0,
            ),
            level: entity.level().read(context),
        };
        context
            .sidecars_mut()
            .get_mut::<PublishedView>()?
            .priority_patches[player as usize] = patch;
        continuation.stage = 1;
        return Ok(OverrideStep::CallOriginal {
            function: ids::MAP_CLEAR_TILE_PRIORITY_NEAR_PLAYER,
            arguments: vec![],
        });
    }
    Ok(OverrideStep::Return(None))
}

fn hud_level(
    context: &mut dyn EnhancedContext,
    continuation: &mut Continuation,
) -> Result<OverrideStep> {
    hud(
        context,
        continuation,
        ids::HUD_POSITION_LEVEL_NUMBER_SPRITES,
        0,
    )
}
fn hud_action(
    context: &mut dyn EnhancedContext,
    continuation: &mut Continuation,
) -> Result<OverrideStep> {
    hud(
        context,
        continuation,
        ids::HUD_POSITION_ACTION_ICON_SPRITE,
        1,
    )
}
fn hud_health(
    context: &mut dyn EnhancedContext,
    continuation: &mut Continuation,
) -> Result<OverrideStep> {
    hud(
        context,
        continuation,
        ids::HUD_POSITION_HEALTH_BAR_SPRITES,
        2,
    )
}
fn hud_lives(
    context: &mut dyn EnhancedContext,
    continuation: &mut Continuation,
) -> Result<OverrideStep> {
    hud(context, continuation, ids::HUD_POSITION_LIVES_SPRITE, 3)
}
fn hud(
    context: &mut dyn EnhancedContext,
    continuation: &mut Continuation,
    function: FunctionId,
    kind: u8,
) -> Result<OverrideStep> {
    if continuation.stage == 0 {
        let arguments = context.entry_arguments()?;
        continuation.locals[0] = arguments[0];
        continuation.stage = 1;
        return Ok(OverrideStep::CallOriginal {
            function,
            arguments,
        });
    }
    let revision = context.revision();
    if ram::g_split_screen_enabled(revision).read(context) != 0
        && ram::g_player_inventory_open(revision).at(0).read(context) == 0
        && ram::g_player2_inventory_open(revision).read(context) == 0
    {
        let player = continuation.locals[0] as usize;
        let margin = (context.sidecars().get::<PublishedView>()?.width as i16 - 320) / 2;
        let (first, count, delta) = match (player, kind) {
            (0, 0) => (11, 2, margin),
            (1, 0) => (13, 2, margin),
            (0, 1) => (21, 1, margin),
            (1, 1) => (22, 1, margin),
            (0, 2) => (15, 2, -margin),
            (1, 2) => (17, 2, -margin),
            (0, 3) => (19, 1, -margin),
            (1, 3) => (20, 1, -margin),
            _ => {
                return Err(tje_game::Error::Platform(
                    "invalid extended-view HUD player".into(),
                ));
            }
        };
        for number in first..first + count {
            let x = ram::g_hardware_sprite_table(revision).at(number).x();
            x.write(context, x.read(context).wrapping_add(delta));
        }
    }
    Ok(OverrideStep::Return(None))
}

fn interest<const MARGIN: i32>(x: i16, y: i16) -> bool {
    i32::from(x) > -160 - MARGIN
        && i32::from(x) < 480 + MARGIN
        && i32::from(y) > -112
        && i32::from(y) < 336
}

/// G/src/sprites/entity_visibility.c:11-42, with only horizontal bounds changed.
#[cfg(test)]
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
    fn profiles_have_separate_width_identities_and_keep_native_sprite_allocation() {
        assert_ne!(WIDESCREEN_PROFILE.id, EXPERIMENTAL_512_PROFILE.id);
        for revision in Revision::ALL {
            for profile in [WIDESCREEN_PROFILE, EXPERIMENTAL_512_PROFILE] {
                let mode =
                    tje_engine_runtime::enhanced::EnhancedMode::new(profile, revision).unwrap();
                assert_eq!(mode.profile_id(), profile.id);
                assert_eq!(profile.translations.len(), 2);
                assert!(
                    !profile
                        .translations
                        .iter()
                        .any(|entry| entry.id == ids::IS_ENTITY_VISIBLE_IN_VIEWPORT)
                );
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

    #[test]
    fn publication_width_is_authenticated_snapshot_configuration() {
        let mut state = PublishedView::new(400);
        assert_eq!(state.configuration_identity(), &400u16.to_le_bytes());
        assert!(state.validate().is_ok());
        state.width = 512;
        assert!(state.validate().is_err());
    }
}
