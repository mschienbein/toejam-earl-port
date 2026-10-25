use super::{game_paused, run_final_level_pause_menu};
use crate::{
    ids,
    ui::menus::test_support::{Access, Fixture},
};
use tje_mem::{Revision, ram};

#[test]
fn final_pause_keeps_every_mapped_poll_and_yields_only_at_backedges() {
    use Access::{Call, Yield};
    let read = ids::READ_MAPPED_PLAYER_INPUT;
    let panel = ids::DRAW_PAUSE_CHOICE_PANEL;
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = Fixture::new(revision, ids::RUN_FINAL_LEVEL_PAUSE_MENU, &[0]);
        context.expect(panel, 0x1a, &[0], 0);
        context.expect(ids::PLAY_SOUND, 0x28, &[6, 99], 0);
        context.expect(ids::WAIT_FOR_NEXT_V_BLANK, 0x32, &[1], 0);
        context.expect(ids::PLAY_SOUND, 0x40, &[20, 99], 0);
        context.expect(ids::AUDIO_REQUEST_CHANNEL_TEMPLATE_EFFECT, 0x48, &[0], 0);
        context.expect(ids::AUDIO_REQUEST_FM_CHANNEL_RELEASE, 0x4e, &[], 0);
        context.expect(ids::MUSIC_REQUEST_TRACK_POINTER_RESET, 0x54, &[], 0);
        context.expect(read, 0x60, &[0], 0x80);
        context.expect(read, 0x60, &[0], 0x80);
        context.expect(read, 0x60, &[0], 0);
        context.expect(read, 0x74, &[0], 2);
        context.expect(panel, 0x82, &[1], 0);
        context.expect(read, 0x94, &[0], 1);
        context.expect(panel, 0xa0, &[0], 0);
        context.expect(read, 0x74, &[0], 0);
        context.expect(read, 0xae, &[0], 0);
        context.expect(read, 0x74, &[0], 0);
        context.expect(read, 0xae, &[0], 0x40);
        context.expect(panel, 0xbe, &[u32::MAX], 0);
        context.expect(read, 0xda, &[0], 0x80);
        context.expect(read, 0xda, &[0], 0x40);
        context.expect(read, 0xda, &[0], 0);
        run_final_level_pause_menu(&mut context).unwrap();
        context.complete();
        let events = context.accesses.borrow();
        let first = events
            .iter()
            .position(|event| matches!(event, Call(id, _) if *id == read))
            .unwrap();
        let actual: Vec<_> = events[first..]
            .iter()
            .filter(|event| matches!(event, Call(_, _) | Yield))
            .cloned()
            .collect();
        assert_eq!(
            actual,
            [
                Call(read, 0x80),
                Yield,
                Call(read, 0x80),
                Yield,
                Call(read, 0),
                Call(read, 2),
                Call(panel, 0),
                Yield,
                Call(read, 1),
                Call(panel, 0),
                Yield,
                Call(read, 0),
                Call(read, 0),
                Yield,
                Call(read, 0),
                Call(read, 0x40),
                Call(panel, 0),
                Call(read, 0x80),
                Yield,
                Call(read, 0x40),
                Yield,
                Call(read, 0),
            ]
        );
        let counter = ram::g_frame_counter(revision).address();
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, Access::Read(address, _, _) if *address == counter)),
            "mapped input must not be replaced by a frame-counter proxy"
        );
    }
}

#[test]
fn game_pause_preserves_compound_render_and_two_controller_poll_sequences() {
    use Access::{Call, Read, Yield};
    let allocate = ids::ALLOCATE_ENTITY_WITH_EVICTION;
    let sprites = ids::ALLOCATE_ENTITY_VIEWPORT_SPRITES;
    let release = ids::ENTITY_RELEASE_ONE_BY_PRIORITY;
    let update = ids::UPDATE_AND_QUEUE_ENTITY_SPRITES;
    let read = ids::READ_MAPPED_PLAYER_INPUT;
    let free = ids::FREE_ENTITY;
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut context = Fixture::new(revision, ids::GAME_PAUSED, &[0]);
        let object = ram::g_player_info(revision).at(3);
        let flags = object.draw_flags().address();
        let ready = object.sprite_ready().address();
        context
            .yield_writes
            .extend([vec![(flags, 2)], vec![(ready, 1)]]);
        context.expect(allocate, 0x54, &[0], 3);
        context.expect(sprites, 0x12e, &[object.address(), 0], 0);
        context.expect(release, 0x120, &[], 0);
        context.expect(sprites, 0x12e, &[object.address(), 0], 0);
        context.expect(update, 0x144, &[], 0);
        context.expect(update, 0x144, &[], 0);
        context.expect(ids::PLAY_SOUND, 0x158, &[6, 99], 0);
        context.expect(ids::WAIT_FOR_NEXT_V_BLANK, 0x162, &[1], 0);
        context.expect(ids::PLAY_SOUND, 0x170, &[20, 99], 0);
        context.expect(ids::AUDIO_REQUEST_CHANNEL_TEMPLATE_EFFECT, 0x178, &[0], 0);
        context.expect(ids::AUDIO_REQUEST_FM_CHANNEL_RELEASE, 0x17e, &[], 0);
        context.expect(ids::MUSIC_REQUEST_TRACK_POINTER_RESET, 0x184, &[], 0);
        context.expect(read, 0x18e, &[1], 0x80);
        context.expect(read, 0x196, &[0], 0);
        context.expect(read, 0x18e, &[1], 0);
        context.expect(read, 0x196, &[0], 0);
        context.expect(read, 0x1a4, &[0], 0);
        context.expect(read, 0x1ae, &[1], 0);
        context.expect(read, 0x1a4, &[0], 0x80);
        context.expect(read, 0x1ae, &[1], 0);
        context.expect(free, 0x2aa, &[3], 0);
        context.expect(read, 0x2b2, &[1], 0x80);
        context.expect(read, 0x2ba, &[0], 0);
        context.expect(read, 0x2b2, &[1], 0);
        context.expect(read, 0x2ba, &[0], 0);
        game_paused(&mut context).unwrap();
        context.complete();
        let events = context.accesses.borrow();
        let actual: Vec<_> = events
            .iter()
            .filter(|event| match event {
                Call(id, _) => [allocate, sprites, release, update, read, free].contains(id),
                Read(address, 1, _) => *address == flags || *address == ready,
                Yield => true,
                _ => false,
            })
            .cloned()
            .collect();
        assert_eq!(
            actual,
            [
                Call(allocate, 3),
                Call(sprites, 0),
                Read(flags, 1, 0),
                Yield,
                Call(release, 0),
                Call(sprites, 0),
                Read(flags, 1, 2),
                Call(update, 0),
                Read(ready, 1, 0),
                Yield,
                Call(update, 0),
                Read(ready, 1, 1),
                Call(read, 0x80),
                Call(read, 0),
                Yield,
                Call(read, 0),
                Call(read, 0),
                Call(read, 0),
                Call(read, 0),
                Yield,
                Call(read, 0x80),
                Call(read, 0),
                Call(free, 0),
                Call(read, 0x80),
                Call(read, 0),
                Yield,
                Call(read, 0),
                Call(read, 0),
            ]
        );
        let counter = ram::g_frame_counter(revision).address();
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, Read(address, _, _) if *address == counter)),
            "compound polls must not add a frame-counter proxy"
        );
    }
}
