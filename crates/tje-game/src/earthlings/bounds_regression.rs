//! Opt-in reproduction of ReviewA RA-003 using original ROM calls on live state.
//! Register as an external Cargo test with tje-game/tje-host/tje-mem dependencies.
use tje_game::ids;
use tje_host::{Host, Mode, hooks::HookRegistry, input::InputScript};
use tje_mem::{Address, Ptr, RenderFrame, Revision, ram};

#[test]
#[ignore = "requires both private ROM revisions"]
fn signed_boundary_regressions_match_both_original_revisions() {
    let script =
        InputScript::parse(include_str!("../../../tje-host/examples/start-game.json")).unwrap();
    for revision in [Revision::Rev02, Revision::Rev00] {
        let key = if revision == Revision::Rev02 {
            "TJE_REV02_ROM"
        } else {
            "TJE_REV00_ROM"
        };
        let bytes = std::fs::read(std::env::var_os(key).expect(key)).unwrap();
        let registry =
            HookRegistry::filtered(revision, &[], &[ids::EARTHLING_MOVE_WITHIN_MAP_BOUNDS])
                .unwrap();
        let mut host = Host::new(&bytes, Mode::Reference, registry).unwrap();
        for frame in 0..1300 {
            host.run_frame(script.at(frame)).unwrap();
        }
        for (label, x, y, width, height, vx, vy, axes) in [
            ("review-equality", 24, 500, 15, 0, -256, 0, 1),
            ("review-negative-x-difference", 24, 500, 16, 0, -4096, 0, 1),
            ("review-negative-y-difference", 500, 24, 0, 16, 0, -4096, 2),
            ("RH03-negative-left", 40, 500, 24, 0, -5120, 0, 1),
            ("RH03-negative-top", 500, 40, 0, 24, 0, -5120, 2),
            ("right-boundary", 2650, 500, 16, 0, 4096, 0, 1),
            ("bottom-boundary", 500, 1760, 0, 16, 0, 4096, 2),
        ] {
            let objects = ram::g_player_info(revision);
            let object = objects.at(6);
            let earthling = ram::g_earthling_banks(revision).at(0).at(0);
            let step = Address::<RenderFrame>::new(
                ram::g_sprite_tile_upload_buffer(revision).address() + 8000,
            );
            {
                let mut context = host.context();
                for slot in 0..40 {
                    objects.at(slot).draw_flags().write(&mut context, 0);
                }
                earthling.kind().write(&mut context, 1);
                earthling.state().write(&mut context, 0);
                earthling.object().write(&mut context, 6);
                earthling.health_visibility().at(1).write(&mut context, 2);
                ram::g_player_inventory_open(revision)
                    .at(0)
                    .write(&mut context, 0);
                ram::g_player1_map_level(revision).write(&mut context, 1);
                object
                    .earthling()
                    .write(&mut context, Ptr::new(earthling.address()));
                object.level().write(&mut context, 1);
                object.entity_index().write(&mut context, 6);
                object.draw_flags().write(&mut context, 1);
                object.behavior_flags().write(&mut context, 0);
                object.x().write(&mut context, x);
                object.y().write(&mut context, y);
                object.width().write(&mut context, width);
                object.height_pixels().write(&mut context, height);
                object.fraction_x().write(&mut context, 0);
                object.fraction_y().write(&mut context, 0);
                object.animation_motion_axes().write(&mut context, axes);
                object.velocity_x().write(&mut context, vx);
                object.velocity_y().write(&mut context, vy);
                step.motion_x().write(&mut context, 1);
                step.motion_y().write(&mut context, 1);
            }
            host.check_fixture(
                ids::EARTHLING_MOVE_WITHIN_MAP_BOUNDS
                    .metadata()
                    .address(revision),
                &[step.address(), object.address()],
            )
            .unwrap_or_else(|error| panic!("{revision:?} {label}: {error}"));
        }
        assert_eq!(host.checks.mismatches, 0);
        println!("SIGNED_BOUNDS {revision:?}: 7 checked, 0 mismatches");
    }
}
