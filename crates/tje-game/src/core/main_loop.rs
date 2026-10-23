//! Finite, source-backed continuations of the original nonreturning core loop.
//! Device time and the recorded idle-draw policy belong to the native scheduler;
//! one Idle invocation performs at most one original RNG draw, never 27/frame.

use crate::call_frame::{CallSite, FrameLayout, LocalFrame, OutgoingArguments};
use crate::exits::{self, MainLoopBodies, MainLoopStage};
use crate::{Context, Result, call_at, ids};
use tje_mem::{Revision, ram, rom};

pub static BODIES: MainLoopBodies = MainLoopBodies {
    entry: ids::FURTHER_INITIALISATION,
    after_intro,
    after_menu,
    begin,
    idle,
};

fn site(revision: Revision, offset: u32) -> CallSite {
    CallSite::relative(ids::FURTHER_INITIALISATION, revision, offset)
}
fn set_word(context: &mut dyn Context, index: usize, value: u16) {
    let previous = context.data_register(index);
    context.set_data_register(index, (previous & 0xffff0000) | u32::from(value));
}

/// G/src/core/startup_and_main_loop.c:607-639 (`FurtherInitialisation`), matching
/// MOVEM D2/D3/A2-A4. Stop at the actual Sega/intro JSR, retaining the starfield
/// outgoing pair and saved-register frame rather than recursively running a
/// 1,200-frame modal child inside one function proof or inventing an RTS.
pub fn further_initialisation(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let _frame = LocalFrame::enter(context, FrameLayout::movem(0, 0x0c, 0x1c))?;
    context.set_address_register(2, ids::MOVE_DATA_TO_VRAM.metadata().address(revision));
    context.set_address_register(3, ram::g_mode2_reg_softcopy(revision).address());
    context.set_address_register(
        4,
        ids::COPY_PALETTE_TO_CRAM_SLOT.metadata().address(revision),
    );
    ram::g_mode2_reg_softcopy(revision).write(context, 0x60);
    call_at(
        context,
        ids::DISABLE_BLANKING_GRAPHICS,
        &[],
        site(revision, 0x1e),
    )?;
    let dispatcher = context.dispatcher();
    let dma = OutgoingArguments::push(context, &[ram::g_frame_counter(revision).address(), 0, 1])?;
    dma.call(
        context,
        dispatcher,
        ids::DMA_COPY_WORDS_TO_VRAM,
        site(revision, 0x30),
    )?;
    dma.call(
        context,
        dispatcher,
        ids::MUSIC_RESTORE_INITIAL_STATE,
        site(revision, 0x36),
    )?;
    dma.call(
        context,
        dispatcher,
        ids::INITIALISE_THE_Z80,
        site(revision, 0x3a),
    )?;
    dma.call(
        context,
        dispatcher,
        ids::LOAD_SOUND_DRIVER_TO_Z80,
        site(revision, 0x40),
    )?;
    dma.call(
        context,
        dispatcher,
        ids::MUSIC_UPDATE_SEQUENCER,
        site(revision, 0x46),
    )?;
    dma.call(
        context,
        dispatcher,
        ids::ENABLE_ALL_INTERRUPTS,
        site(revision, 0x4c),
    )?;
    let wait = OutgoingArguments::push(context, &[1])?;
    wait.call(
        context,
        dispatcher,
        ids::WAIT_FOR_NEXT_V_BLANK,
        site(revision, 0x54),
    )?;
    let mode = ram::g_mode2_reg_softcopy(revision).read(context) & 0x3c;
    ram::g_mode2_reg_softcopy(revision).write(context, mode);
    wait.release(context)?;
    dma.release(context)?;
    context.write_vdp_control(0x8104 | u16::from(mode));

    let seed = OutgoingArguments::push(context, &[12345])?;
    seed.call(
        context,
        dispatcher,
        ids::STACK_BASED_RANDOM,
        site(revision, 0x76),
    )?;
    seed.call(
        context,
        dispatcher,
        ids::PRE_GENERATE_RANDOMS,
        site(revision, 0x7c),
    )?;
    seed.call(
        context,
        dispatcher,
        ids::CLEAR_COLOUR_RAM,
        site(revision, 0x82),
    )?;
    seed.call(
        context,
        dispatcher,
        ids::CLEAR_VERTICAL_SCROLL_RAM,
        site(revision, 0x88),
    )?;
    seed.call(
        context,
        dispatcher,
        ids::CLEAR_VIDEO_RAM,
        site(revision, 0x8e),
    )?;
    seed.release(context)?;
    context.write_vdp_command(0x70000002);
    let bytes = (i32::from(rom::font_data_size(revision).read(context)) as u32) << 4;
    let font =
        OutgoingArguments::push(context, &[rom::image_text_base(revision).address(), bytes])?;
    font.call(
        context,
        dispatcher,
        ids::MOVE_DATA_TO_VRAM,
        site(revision, 0xae),
    )?;
    font.release(context)?;
    context.write_vdp_command(0x40000001);
    let bytes = (i32::from(rom::starry_background_size(revision).read(context)) as u32) << 4;
    let starfield = OutgoingArguments::push(
        context,
        &[rom::img_starfield_large(revision).address(), bytes],
    )?;
    starfield.call(
        context,
        dispatcher,
        ids::MOVE_DATA_TO_VRAM,
        site(revision, 0xce),
    )?;
    starfield.call(
        context,
        dispatcher,
        ids::TEXT_COPY_FONT_BANKS_TO_RAM,
        site(revision, 0xd4),
    )?;
    starfield.call(
        context,
        dispatcher,
        ids::SET_RAM_INDEXES,
        site(revision, 0xda),
    )?;
    // Deliberately do not release these eight bytes: the original keeps them
    // across both modal calls, then ADDQ.L #8 at the AfterMenu continuation.
    exits::call_transfer(
        ids::FURTHER_INITIALISATION,
        ids::SEGA_AND_INTRO_VIDEO_ROUTINE,
        ids::FURTHER_INITIALISATION
            .metadata()
            .address(revision)
            .wrapping_add(0xe0),
    )
}

/// G/src/core/startup_and_main_loop.c:640 (`FurtherInitialisation` after intro).
/// The preceding child's RTS has popped only its real return word. Eight bytes
/// of caller-owned starfield arguments and the twenty-byte MOVEM frame remain.
pub fn after_intro(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    exits::call_transfer(
        ids::FURTHER_INITIALISATION,
        ids::NEW_GAME_MAIN_MENU,
        ids::FURTHER_INITIALISATION
            .metadata()
            .address(context.revision())
            .wrapping_add(0xe6),
    )
}

/// G/src/core/startup_and_main_loop.c:641-676 (`FurtherInitialisation` after menu).
/// Preserve the compiler's retained HUD/palette/setup/seed outgoing areas through
/// the final LEA 56(SP),SP. D2's observed seed residue initializes the otherwise
/// uninitialized word-sized phase; it is not replaced with an invented zero.
pub fn after_menu(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    OutgoingArguments::existing(context, 2)?.release(context)?;
    let dispatcher = context.dispatcher();
    ram::g_requested_music_selection(revision).write(context, 0);
    context.write_vdp_command(0x40000000);
    let bytes = (i32::from(rom::level_tiles_size(revision).read(context)) as u32) << 4;
    let level = OutgoingArguments::push(context, &[rom::level_tiles(revision).address(), bytes])?;
    level.call(
        context,
        dispatcher,
        ids::MOVE_DATA_TO_VRAM,
        site(revision, 0x10c),
    )?;
    level.release(context)?;
    context.write_vdp_command(0x46600001);
    let bytes = (i32::from(rom::inventory_label_open_size(revision).read(context)) as u32) << 4;
    let label = OutgoingArguments::push(
        context,
        &[rom::inventory_label_open(revision).address(), bytes],
    )?;
    label.call(
        context,
        dispatcher,
        ids::MOVE_DATA_TO_VRAM,
        site(revision, 0x12c),
    )?;
    label.release(context)?;
    context.write_vdp_command(0x79a00000);
    let bytes = (i32::from(rom::inventory_label_identify_size(revision).read(context)) as u32) << 4;
    let label = OutgoingArguments::push(
        context,
        &[rom::inventory_label_identify(revision).address(), bytes],
    )?;
    label.call(
        context,
        dispatcher,
        ids::MOVE_DATA_TO_VRAM,
        site(revision, 0x14c),
    )?;
    label.release(context)?;
    context.write_vdp_command(0x74000003);
    let hud = OutgoingArguments::push(
        context,
        &[rom::hud_additional_tiles(revision).address(), 304],
    )?;
    hud.call(
        context,
        dispatcher,
        ids::MOVE_DATA_TO_VRAM,
        site(revision, 0x164),
    )?;
    hud.release(context)?;
    context.write_vdp_command(0x7f200003);
    let small_hud = OutgoingArguments::push(
        context,
        &[rom::hud_additional_tiles_small(revision).address(), 64],
    )?;
    small_hud.call(
        context,
        dispatcher,
        ids::MOVE_DATA_TO_VRAM,
        site(revision, 0x17c),
    )?;
    small_hud.call(
        context,
        dispatcher,
        ids::DRAW_STARFIELD_PLANE,
        site(revision, 0x180),
    )?;
    small_hud.call(
        context,
        dispatcher,
        ids::LOAD_LEVEL1_TILES,
        site(revision, 0x186),
    )?;
    let palette1 =
        OutgoingArguments::push(context, &[rom::palette_game_line1(revision).address(), 0])?;
    palette1.call(
        context,
        dispatcher,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        site(revision, 0x190),
    )?;
    let palette2 =
        OutgoingArguments::push(context, &[rom::palette_game_line2(revision).address(), 1])?;
    palette2.call(
        context,
        dispatcher,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        site(revision, 0x19c),
    )?;
    let palette3 =
        OutgoingArguments::push(context, &[rom::palette_game_line3(revision).address(), 2])?;
    palette3.call(
        context,
        dispatcher,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        site(revision, 0x1a8),
    )?;
    let palette4 =
        OutgoingArguments::push(context, &[rom::palette_game_line4(revision).address(), 3])?;
    palette4.call(
        context,
        dispatcher,
        ids::COPY_PALETTE_TO_CRAM_SLOT,
        site(revision, 0x1b4),
    )?;
    palette4.call(
        context,
        dispatcher,
        ids::SETUP_PLAYER,
        site(revision, 0x1ba),
    )?;
    palette4.call(
        context,
        dispatcher,
        ids::SETUP_THE_MAIN_GAME,
        site(revision, 0x1c0),
    )?;
    let position = OutgoingArguments::push(context, &[2])?;
    position.call(
        context,
        dispatcher,
        ids::SET_POSITION_ON_NEW_LEVEL,
        site(revision, 0x1ca),
    )?;
    let attributes = OutgoingArguments::push(context, &[1])?;
    attributes.call(
        context,
        dispatcher,
        ids::SET_CAMERA_TILE_PRIORITY_ENABLED,
        site(revision, 0x1d4),
    )?;
    let split = OutgoingArguments::push(context, &[1])?;
    split.call(
        context,
        dispatcher,
        ids::UPDATE_SCREEN_SPLIT_MODE,
        site(revision, 0x1dc),
    )?;
    let random = split.call(
        context,
        dispatcher,
        ids::GET_RANDOM_NUMBER,
        site(revision, 0x1e2),
    )?;
    // Emitted order is GetRandomNumber first, then the counter read and ASL.L.
    let frame_seed = u32::from(ram::g_frame_counter(revision).read(context)) << 16;
    context.set_data_register(2, frame_seed);
    let seed = OutgoingArguments::push(context, &[random | frame_seed])?;
    seed.call(
        context,
        dispatcher,
        ids::STACK_BASED_RANDOM,
        site(revision, 0x1fa),
    )?;
    seed.call(
        context,
        dispatcher,
        ids::GENERATE_LEVEL_SEEDS,
        site(revision, 0x200),
    )?;
    seed.call(context, dispatcher, ids::DRAW_HUD, site(revision, 0x206))?;
    seed.call(
        context,
        dispatcher,
        ids::SETUP_INITIAL_ELEVATOR,
        site(revision, 0x20c),
    )?;
    seed.call(
        context,
        dispatcher,
        ids::SETUP_INITIAL_INVENTORY,
        site(revision, 0x212),
    )?;
    seed.call(
        context,
        dispatcher,
        ids::SETUP_INITIAL_EARTHLINGS,
        site(revision, 0x218),
    )?;
    seed.call(
        context,
        dispatcher,
        ids::GET_PLAYER_INPUT,
        site(revision, 0x21c),
    )?;
    seed.release(context)?;
    split.release(context)?;
    attributes.release(context)?;
    position.release(context)?;
    palette4.release(context)?;
    palette3.release(context)?;
    palette2.release(context)?;
    palette1.release(context)?;
    small_hud.release(context)?;
    let x = ram::g_camera1_pending_x(revision).read(context);
    ram::g_camera1_committed_x(revision).write(context, x);
    let y = ram::g_camera1_pending_y(revision).read(context);
    ram::g_camera1_committed_y(revision).write(context, y);
    let x = ram::g_camera1_pending_x(revision).read(context);
    ram::g_camera2_pending_x(revision).write(context, x);
    let y = ram::g_camera1_pending_y(revision).read(context);
    ram::g_camera2_pending_y(revision).write(context, y);
    let mode = ram::g_mode2_reg_softcopy(revision).read(context) | 0x40;
    ram::g_mode2_reg_softcopy(revision).write(context, mode);
    context.set_data_register(0, 0x8104 | u32::from(mode));
    context.write_vdp_control(0x8104 | u16::from(mode));
    call_at(
        context,
        ids::ENABLE_BLANKING_GRAPHICS,
        &[],
        site(revision, 0x260),
    )?;
    exits::main_loop(ids::FURTHER_INITIALISATION, MainLoopStage::Begin)
}

/// G/src/core/startup_and_main_loop.c:677-703 (`FurtherInitialisation` loop body).
/// One logical work iteration, not one video frame. The original twenty-byte
/// saved frame persists; outgoing 4/8-byte areas are retained in source order.
pub fn begin(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let time = ram::g_frame_counter(revision).read(context);
    set_word(context, 3, time);
    let phase = (context.data_register(2) as u16).wrapping_add(1);
    set_word(context, 2, phase);
    let dispatcher = context.dispatcher();
    if phase & 1 != 0 {
        let argument = OutgoingArguments::push(context, &[0])?;
        argument.call(
            context,
            dispatcher,
            ids::UPDATE_SCREEN_SPLIT_MODE,
            site(revision, 0x276),
        )?;
        argument.call(
            context,
            dispatcher,
            ids::UPDATE_CAMERA_FOLLOWING,
            site(revision, 0x27c),
        )?;
        argument.call(
            context,
            dispatcher,
            ids::PROCESS_LAND,
            site(revision, 0x282),
        )?;
        argument.call(
            context,
            dispatcher,
            ids::UPDATE_AND_QUEUE_ENTITY_SPRITES,
            site(revision, 0x288),
        )?;
        argument.release(context)?;
    } else {
        call_at(
            context,
            ids::BUILD_PENDING_CAMERA_TILES,
            &[],
            site(revision, 0x292),
        )?;
        if phase & 2 != 0 {
            call_at(context, ids::PROCESS_EARTHLINGS, &[], site(revision, 0x29e))?;
            call_at(
                context,
                ids::ELEVATOR_UPDATE_ENTITY_VISIBILITY,
                &[],
                site(revision, 0x2a4),
            )?;
            call_at(context, ids::PROCESS_ELEVATORS, &[], site(revision, 0x2aa))?;
            call_at(
                context,
                ids::MAP_UPDATE_ITEMS_AND_PRESENTS,
                &[],
                site(revision, 0x2b0),
            )?;
            call_at(
                context,
                ids::PROCESS_TOMATO_RAIN,
                &[],
                site(revision, 0x2b6),
            )?;
        } else {
            let first = OutgoingArguments::push(context, &[0])?;
            first.call(
                context,
                dispatcher,
                ids::UPDATE_LAND_AROUND_PLAYER,
                site(revision, 0x2c0),
            )?;
            let second = OutgoingArguments::push(context, &[1])?;
            second.call(
                context,
                dispatcher,
                ids::UPDATE_LAND_AROUND_PLAYER,
                site(revision, 0x2ca),
            )?;
            second.call(
                context,
                dispatcher,
                ids::PROCESS_INPUT,
                site(revision, 0x2ce),
            )?;
            second.call(
                context,
                dispatcher,
                ids::PROCESS_PLAYERS,
                site(revision, 0x2d4),
            )?;
            second.call(
                context,
                dispatcher,
                ids::UPDATE_ALL_ENTITY_SHADOWS,
                site(revision, 0x2da),
            )?;
            second.release(context)?;
            first.release(context)?;
        }
        call_at(context, ids::PROCESS_MOVEMENT, &[], site(revision, 0x2e2))?;
        call_at(
            context,
            ids::UPDATE_ALL_ENTITY_VIEWPORT_SPRITES,
            &[],
            site(revision, 0x2e8),
        )?;
        call_at(
            context,
            ids::PROCESS_ENTITY_ANIMATION_MOTION,
            &[],
            site(revision, 0x2ee),
        )?;
        call_at(
            context,
            ids::UPDATE_SPEECH_AND_DIALOGUE,
            &[],
            site(revision, 0x2f4),
        )?;
    }
    // Original BRA.S at +0x2f4 reaches the recurrent CMP.W at +0x2fc.
    exits::main_loop(ids::FURTHER_INITIALISATION, MainLoopStage::Idle)
}

/// G/src/core/startup_and_main_loop.c:704-705 (`FurtherInitialisation` idle loop).
/// One CMP/branch or one discarded RNG draw. The platform drives repeated Idle
/// continuations using recorded draw counts and real interrupt progress; this
/// body neither fabricates elapsed clocks nor substitutes a VBlank-only wait.
pub fn idle(context: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let revision = context.revision();
    let previous = context.data_register(3) as u16;
    let current = ram::g_frame_counter(revision).read(context);
    let difference = previous.wrapping_sub(current);
    let negative = difference & 0x8000 != 0;
    let overflow = (previous ^ current) & (previous ^ difference) & 0x8000 != 0;
    let carry = current > previous;
    let flags = (u16::from(negative) << 3)
        | (u16::from(difference == 0) << 2)
        | (u16::from(overflow) << 1)
        | u16::from(carry);
    let status = context.status_register();
    context.set_status_register((status & !0x000f) | flags);
    if previous != current {
        return exits::main_loop(ids::FURTHER_INITIALISATION, MainLoopStage::Begin);
    }
    call_at(context, ids::GET_RANDOM_NUMBER, &[], site(revision, 0x2fc))?;
    exits::main_loop(ids::FURTHER_INITIALISATION, MainLoopStage::Idle)
}
