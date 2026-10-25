use super::{general_menu_handler, new_game_main_menu};
use crate::{
    ids,
    ui::menus::{
        MENU_FRAME_CANDIDATES,
        test_support::{Access, Fixture},
    },
};
use tje_mem::{Bus, Revision, ram, rom};

#[test]
fn wrappers_keep_the_header_slot_and_preserve_the_full_selection() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for (id, header, items) in [
            (
                ids::MENU_WHO_HANDLER,
                0,
                rom::menu_who_items(revision).address(),
            ),
            (
                ids::MENU_CONTROLLER_OPTIONS_HANDLER,
                3,
                rom::menu_controller_options_items(revision).address(),
            ),
            (
                ids::MENU_WHAT_HANDLER,
                1,
                rom::menu_what_items(revision).address(),
            ),
            (
                ids::MENU_JAM_OUT_HANDLER,
                2,
                rom::menu_jam_out_items(revision).address(),
            ),
        ] {
            for selection in [0_u32, 1, u32::MAX] {
                let mut context = Fixture::new(revision, id, &[]);
                let saved_d2 = context.data[2];
                let first = if header == 0 { 8 } else { 10 };
                context.expect(ids::ADD_MENU_HEADER, first, &[header], 0);
                context.expect(
                    ids::GENERAL_MENU_HANDLER,
                    first + 26,
                    &[2, items, 0],
                    selection,
                );
                context.expect(ids::REMOVE_MENU_HEADER, first + 38, &[], 0x76543210);
                let entry = MENU_FRAME_CANDIDATES
                    .iter()
                    .find(|entry| entry.id == id)
                    .unwrap();
                assert_eq!((entry.function)(&mut context).unwrap(), Some(selection));
                context.complete();
                assert_eq!(context.data[2], saved_d2);
                assert_eq!(context.calls[1].1, 0xff7fe8);
            }
        }
    }
}

#[test]
fn general_menu_loads_only_used_argument_bytes_and_retains_compiler_call_areas() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let items = 0xff4000;
        let mut context = Fixture::new(
            revision,
            ids::GENERAL_MENU_HANDLER,
            &[0xaabbfffe, items, 0x11220001],
        );
        context.memory.write_u16(items + 6, 7);
        let saved_data = context.data;
        let saved_addresses = context.addresses;
        context.expect(ids::WAIT_FOR_NEXT_V_BLANK, 0x2a, &[1], 0);
        context.expect(ids::DRAW_STARFIELD_PLANE, 0x54, &[], 0);
        context.expect(ids::DRAW_MENU_WINDOW_FRAME, 0x5c, &[0], 0);
        context.expect(
            ids::COPY_PALETTE_TO_CRAM_SLOT,
            0x6a,
            &[rom::palette_game_line1(revision).address(), 0],
            0,
        );
        context.expect(
            ids::COPY_PALETTE_TO_CRAM_SLOT,
            0x7a,
            &[rom::palette_game_line4(revision).address(), 3],
            0,
        );
        context.expect(ids::FORCE_TEXT_BACKGROUND_COLOR_TO_BLACK, 0x84, &[1], 0);
        context.expect(ids::MENU_DRAW_TEXT_ITEMS, 0x90, &[items, 1], 0);
        context.expect(ids::UPDATE_AND_QUEUE_ENTITY_SPRITES, 0x98, &[], 0);
        context.expect(ids::WAIT_FOR_NEXT_V_BLANK, 0x9e, &[1], 0);
        context.expect(ids::UPLOAD_QUEUED_SPRITE_GRAPHICS, 0xa8, &[0, 0], 0);
        context.expect(ids::ENABLE_TEXT_FONT_ANIMATION, 0xd4, &[], 0);
        context.expect(ids::GET_CTRL2_INPUT, 0xee, &[], 0);
        context.expect(ids::GET_CTRL1_INPUT, 0xf6, &[], 0);
        context.expect(ids::CREATE_MENU_CURSOR_ENTITY, 0x104, &[], 0);
        context.expect(ids::UPDATE_AND_QUEUE_ENTITY_SPRITES, 0x118, &[], 0);
        context.expect(ids::WAIT_FOR_NEXT_V_BLANK, 0x11e, &[1], 0);
        context.expect(ids::UPLOAD_QUEUED_SPRITE_GRAPHICS, 0x128, &[0, 0], 0);
        context.expect(ids::TEXT_ANIMATE_FONT_BANKS, 0x132, &[8], 0);
        context.expect(ids::SELECT_VIEWPORT_SPRITE_CHAIN, 0x13a, &[0], 0);
        context.expect(ids::GET_RANDOM_NUMBER, 0x140, &[], 0);
        context.expect(ids::GET_CTRL2_INPUT, 0x146, &[], 0);
        context.expect(ids::GET_CTRL1_INPUT, 0x14e, &[], 0x80);
        context.expect(
            ids::DISABLE_TEXT_FONT_ANIMATION_IF_MENUS_CLOSED,
            0x242,
            &[],
            0,
        );
        context.expect(ids::RELEASE_MENU_CURSOR_ENTITY, 0x268, &[], 0);
        context.expect(ids::FORCE_TEXT_BACKGROUND_COLOR_TO_BLACK, 0x272, &[0], 0);
        assert_eq!(general_menu_handler(&mut context).unwrap(), Some(7));
        context.complete();
        assert_eq!(&context.data[2..7], &saved_data[2..7]);
        assert_eq!(&context.addresses[2..], &saved_addresses[2..]);
        assert_eq!(context.calls[9].1, 0xff7fa8, "44-byte argument area");
        let arguments: Vec<_> = context.accesses.borrow().iter().filter(|access| {
            matches!(access, Access::Read(address, _, _) if (0xff8004..0xff8010).contains(address))
        }).cloned().collect();
        assert_eq!(
            arguments,
            [
                Access::Read(0xff8006, 2, 0xfffe),
                Access::Read(0xff8008, 4, items),
                Access::Read(0xff800e, 2, 1)
            ]
        );
        let pending = ram::g_sprite_graphics_upload_pending(revision).address();
        assert!(context.accesses.borrow().windows(2).any(|events| events ==
            [Access::Discard(pending, 1), Access::Write(pending, 1, 0)]));
    }
}

#[test]
fn new_game_clears_have_discarded_reads_and_the_missing_slot_stays_lazy() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for choice in [0_u32, 1, 2] {
            let mut context = Fixture::new(revision, ids::NEW_GAME_MAIN_MENU, &[0xaabbcdef]);
            context.expect(ids::MENU_WHO_HANDLER, 0x1e, &[], choice);
            if choice == 1 {
                context.expect(ids::GET_CTRL2_INPUT, 0x84, &[], 0);
            }
            if choice == 2 {
                context.expect(ids::GET_CTRL1_INPUT, 0xb4, &[], 0);
            }
            context.expect(ids::MENU_WHAT_HANDLER, 0xcc, &[], 4);
            new_game_main_menu(&mut context).unwrap();
            context.complete();
            let events = context.accesses.borrow();
            assert!(events.contains(&Access::Read(0xff8006, 2, 0xcdef)));
            assert!(
                !events
                    .iter()
                    .any(|event| matches!(event, Access::Read(0xff8004, 4, _)))
            );
            let swapped = ram::g_controller_ports_swapped(revision).address();
            assert!(events.windows(2).any(
                |events| events == [Access::Discard(swapped, 1), Access::Write(swapped, 1, 0)]
            ));
        }
    }
}

#[test]
fn game_over_keeps_sound_slots_and_does_not_invent_a_reset_return() {
    use crate::{Error, ui::draws::literal};
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = Fixture::new(revision, ids::SHOW_GAME_OVER_AND_RESET, &[]);
        let text = literal(&context, "BogusGameOverStr").unwrap();
        context.expect(ids::DRAW_MENU_WINDOW_FRAME, 0x12, &[0], 0);
        context.expect(ids::TEXT_WRITE_STRING_TILES, 0x2a, &[text], 0);
        context.expect(ids::FORCE_TEXT_BACKGROUND_COLOR_TO_BLACK, 0x34, &[1], 0);
        context.expect(ids::ENABLE_TEXT_FONT_ANIMATION, 0x50, &[], 0);
        context.expect(ids::WAIT_FOR_NEXT_V_BLANK, 0x60, &[1], 0);
        context.expect(ids::TEXT_ANIMATE_FONT_BANKS, 0x6a, &[8], 0);
        context.expect(ids::READ_MAPPED_PLAYER_INPUT, 0x80, &[1], 0);
        context.expect(ids::READ_MAPPED_PLAYER_INPUT, 0x8c, &[0], 0x80);
        context.expect(ids::PLAY_SOUND, 0xa6, &[6, 99], 0);
        context.expect(ids::PLAY_SOUND, 0xb2, &[0, 98], 0);
        context.expect(ids::AUDIO_REQUEST_FM_CHANNEL_RELEASE, 0xb8, &[], 0);
        context.expect(ids::MUSIC_REQUEST_TRACK_POINTER_RESET, 0xbe, &[], 0);
        context.expect(ids::CLEAR_COLOUR_RAM, 0xdc, &[], 0);
        context.expect(ids::CLEAR_VERTICAL_SCROLL_RAM, 0xe2, &[], 0);
        context.expect(ids::CLEAR_VIDEO_RAM, 0xe8, &[], 0);
        assert!(matches!(super::show_game_over_and_reset(&mut context),
            Err(Error::Untranslated(id)) if id == ids::RESTART_FROM_RESET_ENTRY));
        context.calls_complete();
        assert_eq!(context.sp, 0xff7ff8, "nonreturning frame stays abandoned");
        let counter = ram::g_frame_counter(revision).address();
        assert!(context.accesses.borrow().windows(2).any(|events| events ==
            [Access::Discard(counter, 2), Access::Write(counter, 2, 0)]));
        assert_eq!(
            context.calls[11].1, 0xff7fe4,
            "retained silence/noise arguments"
        );
    }
}
