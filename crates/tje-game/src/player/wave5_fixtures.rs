use tje_game::{Context, ids};
use tje_host::Host;
use tje_mem::{Address, EarthlingDefinition, ram};

pub(super) fn run(host: &mut Host) {
    let revision = host.revision();
    let e = ram::g_earthling_banks(revision).at(0).at(0);
    e.kind().write(&mut host.context(), 7);
    e.object().write(&mut host.context(), 25);
    e.level().write(&mut host.context(), 1);
    e.x().write(&mut host.context(), 512);
    e.y().write(&mut host.context(), 512);
    let object = ram::g_player_info(revision).at(25);
    object.x().write(&mut host.context(), 512);
    object.y().write(&mut host.context(), 512);
    object.level().write(&mut host.context(), 1);
    object.draw_flags().write(&mut host.context(), 1);
    for number in 0..2 {
        let p = ram::g_player_info(revision).at(number);
        ram::g_player_interaction_choice_active(revision)
            .at(number)
            .write(&mut host.context(), 0);
        p.sprite_state().write(&mut host.context(), 0);
        p.animation_group_mask().write(&mut host.context(), 1);
        p.y().write(&mut host.context(), 512);
        for x in [500, 512, 532, 600] {
            p.x().write(&mut host.context(), x);
            for height in [0, 11, 40, 74, 75] {
                p.height().write(&mut host.context(), height);
                e.state().write(&mut host.context(), 0);
                super::check(host, ids::CUPID_ABILITY, &[number as u32, e.address()]);
            }
        }
    }
    for maps in [(1, 2), (2, 1), (3, 4)] {
        ram::g_player1_map_level(revision).write(&mut host.context(), maps.0);
        ram::g_player2_map_level(revision).write(&mut host.context(), maps.1);
        for residual in [0, 1, -1, i16::MIN] {
            host.context().set_data_register(1, residual as u32);
            object.height().write(&mut host.context(), 1);
            object.velocity_height().write(&mut host.context(), -1);
            object.frame_duration().write(&mut host.context(), 1);
            object.frame().write(&mut host.context(), 0);
            e.state().write(&mut host.context(), 3);
            e.health_visibility().at(0).write(&mut host.context(), 1);
            super::check(
                host,
                ids::EARTHLING_UPDATE_BOUNCING_DAMAGE_STATE,
                &[e.address()],
            );
        }
    }
    let definition =
        Address::<EarthlingDefinition>::new(ram::g_player_info(revision).at(24).address());
    // Labelled synthetic contact bounds in otherwise unused live object storage.
    definition
        .contact_distance_x()
        .write(&mut host.context(), 255);
    definition
        .contact_distance_y()
        .write(&mut host.context(), 255);
    let decoy = ram::g_player_info(revision).at(26);
    decoy.x().write(&mut host.context(), 600);
    decoy.y().write(&mut host.context(), 512);
    decoy.level().write(&mut host.context(), 1);
    ram::g_player_decoy_entities(revision)
        .at(0)
        .write(&mut host.context(), 26);
    ram::g_player_decoy_entities(revision)
        .at(1)
        .write(&mut host.context(), 0);
    e.state().write(&mut host.context(), 0);
    super::check(
        host,
        ids::EARTHLING_CHECK_DECOY_ATTACK_RANGE,
        &[e.address(), definition.address()],
    );
}
