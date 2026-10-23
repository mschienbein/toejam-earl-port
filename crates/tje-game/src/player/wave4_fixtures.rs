use tje_game::ids;
use tje_host::Host;
use tje_mem::ram;

pub(super) fn run(host: &mut Host) {
    let revision = host.revision();
    let e = ram::g_earthling_banks(revision).at(0).at(0);
    e.kind().write(&mut host.context(), 7);
    e.object().write(&mut host.context(), 25);
    e.level().write(&mut host.context(), 1);
    e.x().write(&mut host.context(), 512);
    e.y().write(&mut host.context(), 512);
    for number in 0..2 {
        let p = ram::g_player_info(revision).at(number);
        p.entity_index().write(&mut host.context(), number as i8);
        p.level().write(&mut host.context(), 1);
        p.y().write(&mut host.context(), 512);
        p.height().write(&mut host.context(), 0);
        p.draw_flags().write(&mut host.context(), 1);
        p.behavior_flags().write(&mut host.context(), 1);
        ram::g_player_elevator_flags(revision)
            .at(number)
            .write(&mut host.context(), 0);
        ram::g_player_control_remap_entities(revision)
            .at(number)
            .write(&mut host.context(), 0);
        ram::g_player_interaction_choice_active(revision)
            .at(number)
            .write(&mut host.context(), 0);
        for x in [400, 446, 447, 512, 577, 578, 624] {
            p.x().write(&mut host.context(), x);
            super::check(
                host,
                ids::EARTHLING_START_HORIZONTAL_ATTACK,
                &[e.address(), p.address()],
            );
        }
        p.x().write(&mut host.context(), 512);
        e.kind().write(&mut host.context(), 18);
        e.state().write(&mut host.context(), 0);
        e.timer().write(&mut host.context(), 30);
        let mole = ram::g_player_info(revision).at(25);
        mole.x().write(&mut host.context(), 512);
        mole.y().write(&mut host.context(), 512);
        mole.attachment_x_or_latch().write(&mut host.context(), 0);
        mole.attachment_y_or_latch().write(&mut host.context(), 0);
        p.sprite_state().write(&mut host.context(), 0);
        p.animation_group_mask().write(&mut host.context(), 1);
        ram::g_player_inventory(revision)
            .at(number)
            .at(0)
            .write(&mut host.context(), 1);
        super::check(host, ids::MOLE_ABILITY, &[number as u32, e.address()]);
        e.state().write(&mut host.context(), 3);
        e.timer().write(&mut host.context(), 14);
        p.sprite_state().write(&mut host.context(), 70);
        super::check(host, ids::MOLE_ABILITY, &[number as u32, e.address()]);
    }
}
