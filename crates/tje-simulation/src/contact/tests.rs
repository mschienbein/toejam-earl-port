use super::*;
use crate::player_tests::{fixture, level, point};
use crate::players::{PlayerAnimationStep, PlayerInput, VoluntaryScale};
use tje_four_player::{Archetype, input::InputFrame};

fn frame(tick: u64) -> ContactFrame {
    ContactFrame {
        tick,
        hero_ids: SeatId::ALL.map(|seat| ActorId::new(u16::from(seat.get()), 1)),
        elevator: SeatMask::NONE,
        suppressed: SeatMask::NONE,
    }
}
fn request(seat: SeatId, kind: Kind, effect: ContactEffect) -> ContactRequest {
    ContactRequest {
        source: PopulationId {
            level: level(2),
            local: kind.index() as u16,
        },
        actor: ActorId::new(32 + kind.index() as u16, 1).unwrap(),
        seat,
        kind,
        position: point(400, 400),
        interest: SeatMask::ALL,
        animation_finished: false,
        effect,
    }
}
fn apply(
    coop: &mut CoopState,
    services: &mut PlayerServices,
    world: &mut crate::player_tests::World,
    tick: u64,
    request: ContactRequest,
) -> ContactOutcome {
    let catalog = PresentCatalog::new([1; 27]).unwrap();
    let identity = [0; 32];
    let ships = [ShipBinding {
        source: PopulationId {
            level: level(2),
            local: 0,
        },
        part: 7,
    }];
    let mut sink = PlayerContacts::new(
        coop,
        services,
        world,
        frame(tick),
        ContactBindings {
            identity: &identity,
            presents: &catalog,
            ship_parts: &ships,
        },
    )
    .unwrap();
    let result = sink.apply(request).unwrap();
    sink.finish().apply(services, world).unwrap();
    result
}

#[test]
fn every_seat_and_archetype_takes_real_contact_damage_and_live_eligibility_changes() {
    for archetype in [Archetype::ToeJam, Archetype::Earl] {
        for seat in SeatId::ALL {
            for (kind, effect) in [
                (Kind::Devil, ContactEffect::Hurt { damage: 5 }),
                (Kind::Lawnmower, ContactEffect::Squish { damage: 5 }),
                (Kind::Boogieman, ContactEffect::Boogie { damage: 5 }),
            ] {
                let (mut coop, mut services, mut world) = fixture(archetype, seat);
                coop.players[seat.index()].motion.position = point(400, 400);
                services
                    .ui(&mut coop, seat, crate::players::UiIntent::OpenInventory)
                    .unwrap();
                let catalog = PresentCatalog::new([1; 27]).unwrap();
                let identity = [0; 32];
                let mut sink = PlayerContacts::new(
                    &mut coop,
                    &mut services,
                    &world,
                    frame(1),
                    ContactBindings {
                        identity: &identity,
                        presents: &catalog,
                        ship_parts: &[],
                    },
                )
                .unwrap();
                let first = request(seat, kind, effect);
                assert_eq!(sink.apply(first).unwrap(), ContactOutcome::Applied);
                assert!(!sink.hero(seat).unwrap().receives_effects);
                assert_eq!(sink.apply(first).unwrap(), ContactOutcome::Rejected);
                sink.finish().apply(&mut services, &mut world).unwrap();
                if matches!(effect, ContactEffect::Squish { .. }) {
                    assert_eq!(
                        coop.players[seat.index()].health,
                        archetype.base_health() - 5
                    );
                } else {
                    assert_eq!(coop.players[seat.index()].counters.pending_hurt_damage, 5);
                }
                assert!(!services.audio.is_empty());
                services.validate(&coop).unwrap();
            }
        }
    }
}
#[test]
fn all_contact_effects_mutate_real_resources_statuses_and_generations() {
    for seat in SeatId::ALL {
        let (mut coop, mut services, mut world) = fixture(Archetype::Earl, seat);
        coop.players[seat.index()].motion.position = point(400, 400);
        coop.players[seat.index()].inventory[0] = Some(
            services
                .resources
                .presents
                .wrapper_for(PresentKind::ExtraLife),
        );
        coop.players[seat.index()].inventory[1] = Some(
            services
                .resources
                .presents
                .wrapper_for(PresentKind::ExtraBuck),
        );
        assert_eq!(
            apply(
                &mut coop,
                &mut services,
                &mut world,
                1,
                request(seat, Kind::Mole, ContactEffect::MoleCapture)
            ),
            ContactOutcome::Applied
        );
        assert_eq!(
            services.runtime[seat.index()].mode,
            PlayerMode::MoleCaptured
        );
        assert_eq!(
            apply(
                &mut coop,
                &mut services,
                &mut world,
                2,
                request(seat, Kind::Mole, ContactEffect::MoleTheft { random: 1 })
            ),
            ContactOutcome::Applied
        );
        assert_eq!(
            coop.players[seat.index()]
                .inventory
                .iter()
                .flatten()
                .count(),
            1
        );
        let visual = services.runtime[seat.index()]
            .contacts
            .mole
            .unwrap()
            .visual
            .unwrap();
        assert_eq!(
            services.resources.effect(visual).unwrap().kind,
            EffectKind::StolenPresent
        );
        assert_eq!(
            apply(
                &mut coop,
                &mut services,
                &mut world,
                2,
                request(seat, Kind::Mole, ContactEffect::MoleTheft { random: 0 })
            ),
            ContactOutcome::Rejected
        );
        assert_eq!(
            apply(
                &mut coop,
                &mut services,
                &mut world,
                3,
                request(seat, Kind::Mole, ContactEffect::MoleRelease)
            ),
            ContactOutcome::Complete
        );
        assert!(services.runtime[seat.index()].contacts.mole.is_none());
        assert_eq!(
            apply(
                &mut coop,
                &mut services,
                &mut world,
                4,
                request(seat, Kind::Santa, ContactEffect::SantaDrops { count: 3 })
            ),
            ContactOutcome::Complete
        );
        assert_eq!(services.resources.drops().count(), 3);
        assert!(
            services
                .resources
                .drops()
                .all(|(_, drop)| matches!(drop.owner, EffectOwner::Actor(_)))
        );
        assert_eq!(
            apply(
                &mut coop,
                &mut services,
                &mut world,
                5,
                request(seat, Kind::ShipPiece, ContactEffect::ShipPiece)
            ),
            ContactOutcome::Complete
        );
        assert_eq!(services.resources.ship_parts, 1 << 7);
        assert_eq!(
            apply(
                &mut coop,
                &mut services,
                &mut world,
                6,
                request(seat, Kind::ShipPiece, ContactEffect::ShipPiece)
            ),
            ContactOutcome::Rejected
        );
        assert_eq!(
            apply(
                &mut coop,
                &mut services,
                &mut world,
                7,
                request(seat, Kind::Shark, ContactEffect::SharkAlert)
            ),
            ContactOutcome::Applied
        );
        assert_eq!(
            services.runtime[seat.index()].contacts.shark_alert_ticks,
            128
        );
        let mut shot = request(
            seat,
            Kind::Cupid,
            ContactEffect::Projectile {
                remap: true,
                position: point(400, 400),
                velocity: [256, 0, 0],
                acceleration: [0, 0, -32],
            },
        );
        assert_eq!(
            apply(&mut coop, &mut services, &mut world, 8, shot),
            ContactOutcome::Applied
        );
        shot.kind = Kind::Chickens;
        shot.effect = ContactEffect::Projectile {
            remap: false,
            position: point(400, 400),
            velocity: [256, 0, 768],
            acceleration: [0, 0, -32],
        };
        assert_eq!(
            apply(&mut coop, &mut services, &mut world, 8, shot),
            ContactOutcome::Applied
        );
        assert_eq!(world.births.len(), 3);
        let effect = Effect::stationary(
            seat,
            PresentKind::Decoy,
            EffectKind::Decoy,
            level(2),
            point(400, 400),
            100,
        );
        let decoy = services.resources.spawn_effect(effect).unwrap();
        for tick in 9..12 {
            assert_eq!(
                apply(
                    &mut coop,
                    &mut services,
                    &mut world,
                    tick,
                    request(
                        seat,
                        Kind::Devil,
                        ContactEffect::DecoyHit {
                            decoy: Effect::actor_id(decoy)
                        }
                    )
                ),
                ContactOutcome::Applied
            );
        }
        assert!(services.resources.effect(decoy).is_none());
        assert!(services.resources.known_effect(decoy));
        assert!(world.retirements.contains(&decoy));
        assert_eq!(
            apply(
                &mut coop,
                &mut services,
                &mut world,
                13,
                request(seat, Kind::Whirlwind, ContactEffect::Whirlwind)
            ),
            ContactOutcome::Complete
        );
        assert_eq!(services.runtime[seat.index()].mode, PlayerMode::Whirlwind);
        services.validate(&coop).unwrap();
    }
}
#[test]
fn paid_services_have_identified_choices_exact_costs_and_finite_completion() {
    for seat in SeatId::ALL {
        for (kind, service, cost) in [
            (Kind::Mailbox, Service::MailOrder, 0),
            (Kind::Wizard, Service::Heal, 1),
            (Kind::Wiseman, Service::Identify, 2),
            (Kind::OperaSinger, Service::Opera, 3),
        ] {
            let (mut coop, mut services, mut world) = fixture(Archetype::ToeJam, seat);
            coop.players[seat.index()].motion.position = point(400, 400);
            coop.players[seat.index()].bucks = 10;
            coop.players[seat.index()].health = 1;
            let mut event = request(
                seat,
                kind,
                ContactEffect::Service {
                    service,
                    stage: ServiceStage::Offer,
                    cost,
                },
            );
            assert_eq!(
                apply(&mut coop, &mut services, &mut world, 1, event),
                ContactOutcome::Applied
            );
            assert_eq!(coop.players[seat.index()].bucks, 10);
            services.runtime[seat.index()].contacts.response = Some((event.source, true));
            event.effect = ContactEffect::Service {
                service,
                stage: ServiceStage::Begin,
                cost,
            };
            assert_eq!(
                apply(&mut coop, &mut services, &mut world, 2, event),
                ContactOutcome::Applied
            );
            assert_eq!(
                coop.players[seat.index()].bucks,
                10 - if service == Service::Identify {
                    0
                } else {
                    cost
                }
            );
            let before = (coop.clone(), services.clone());
            assert_eq!(
                apply(&mut coop, &mut services, &mut world, 2, event),
                ContactOutcome::Rejected
            );
            assert_eq!((coop.clone(), services.clone()), before);
            event.effect = ContactEffect::Service {
                service,
                stage: ServiceStage::Tick,
                cost: 0,
            };
            if service == Service::Heal {
                for tick in 3..26 {
                    event.animation_finished = true;
                    apply(&mut coop, &mut services, &mut world, tick, event);
                }
                assert_eq!(
                    coop.players[seat.index()].health,
                    Archetype::ToeJam.base_health()
                );
            } else if matches!(service, Service::MailOrder | Service::Identify) {
                assert_eq!(
                    coop.players[seat.index()].ui.overlay,
                    if service == Service::MailOrder {
                        Overlay::MailOrder
                    } else {
                        Overlay::Identify
                    }
                );
                if service == Service::MailOrder {
                    assert_eq!(services.resources.offers().len(), 3);
                    assert!(
                        services
                            .resources
                            .offers()
                            .iter()
                            .all(|offer| offer.stock == MailStock::Unlimited
                                && (3..=5).contains(&offer.price))
                    );
                }
                coop.players[seat.index()].ui.overlay = Overlay::Closed;
                assert_eq!(
                    apply(&mut coop, &mut services, &mut world, 3, event),
                    ContactOutcome::Complete
                );
            } else {
                assert_eq!(
                    apply(&mut coop, &mut services, &mut world, 3, event),
                    ContactOutcome::Applied
                );
                event.animation_finished = true;
                assert_eq!(
                    apply(&mut coop, &mut services, &mut world, 4, event),
                    ContactOutcome::Complete
                );
            }
            event.effect = ContactEffect::Service {
                service,
                stage: ServiceStage::End,
                cost: 0,
            };
            assert_eq!(
                apply(&mut coop, &mut services, &mut world, 30, event),
                ContactOutcome::Complete
            );
            assert!(services.runtime[seat.index()].contacts.offer.is_none());
            services.validate(&coop).unwrap();
        }
    }
}
#[test]
fn rejected_capacity_and_malformed_contacts_leave_resources_and_rng_unchanged() {
    let (mut coop, mut services, mut world) = fixture(Archetype::ToeJam, SeatId::ALL[0]);
    for _ in 0..128 {
        let drop = DroppedPresent {
            owner: EffectOwner::World,
            level: level(2),
            position: point(400, 400),
            present: services.resources.presents.wrapper_for(PresentKind::Food),
        };
        services
            .resources
            .spawn_drop_batch(&[drop], &mut [None])
            .unwrap();
    }
    let before = (coop.clone(), services.clone());
    assert_eq!(
        apply(
            &mut coop,
            &mut services,
            &mut world,
            1,
            request(
                SeatId::ALL[0],
                Kind::Santa,
                ContactEffect::SantaDrops { count: 3 }
            )
        ),
        ContactOutcome::Rejected
    );
    assert_eq!((coop.clone(), services.clone()), before);
}
#[test]
fn movement_status_and_authored_callbacks_have_distinct_saved_admissions() {
    let (mut coop, mut services, world) = fixture(Archetype::ToeJam, SeatId::ALL[0]);
    let seat = SeatId::ALL[0];
    services.runtime[0].effective_input = PlayerInput {
        directions: 8,
        ..Default::default()
    };
    coop.players[0].motion.velocity = [320, 0, 0];
    services.runtime[0].authored_motion.axes = 3;
    let before = coop.players[0].motion.position;
    services
        .advance_movement(&mut coop, 0, [VoluntaryScale::Normal; 4], &world)
        .unwrap();
    assert_eq!(coop.players[0].motion.position, before);
    let step = PlayerAnimationStep {
        descriptor: 0,
        sequence_index: 0,
        ticks_left: 2,
        flip_horizontal: false,
        held: false,
        motion: Some([2, 0]),
    };
    services
        .apply_animation(&mut coop, seat, 0, step, VoluntaryScale::Normal, &world)
        .unwrap();
    assert_eq!(coop.players[0].motion.position.x, before.x + 640);
    assert!(
        !services
            .apply_animation(&mut coop, seat, 0, step, VoluntaryScale::Normal, &world)
            .unwrap()
    );
    services
        .apply_animation(
            &mut coop,
            seat,
            2,
            PlayerAnimationStep {
                motion: None,
                ..step
            },
            VoluntaryScale::Normal,
            &world,
        )
        .unwrap();
    assert_eq!(coop.players[0].motion.position.x, before.x + 640);
    services.runtime[0].authored_motion.axes = 0;
    services
        .advance_movement(&mut coop, 2, [VoluntaryScale::Snared; 4], &world)
        .unwrap();
    assert_eq!(coop.players[0].motion.position.x, before.x + 880);
    services
        .enter_transition(&mut coop, SeatMask::new(1).unwrap())
        .unwrap();
    coop.players[0].motion.velocity[0] = 1000;
    services
        .advance_movement(&mut coop, 4, [VoluntaryScale::Normal; 4], &world)
        .unwrap();
    assert_eq!(coop.players[0].motion.position.x, before.x + 880);
}
#[test]
fn audio_queue_roundtrips_and_unknown_music_callbacks_fail_closed() {
    let mut queue = crate::players::AudioQueue::default();
    let first = queue
        .enqueue(SeatId::ALL[0], pcm(PcmSound::ToeJamYouch, 50, 0))
        .unwrap();
    let next = queue
        .enqueue(SeatId::ALL[3], AudioCue::Template { effect: 4 })
        .unwrap();
    assert!(queue.acknowledge(next).is_err());
    queue.acknowledge(first).unwrap();
    assert_eq!(queue.front().unwrap().sequence, next);
    let bytes = bincode::encode_to_vec(&queue, bincode::config::standard()).unwrap();
    let (restored, _): (crate::players::AudioQueue, _) =
        bincode::decode_from_slice(&bytes, bincode::config::standard()).unwrap();
    assert_eq!(queue, restored);
    restored.validate().unwrap();
    for revision in tje_mem::Revision::ALL {
        assert!(crate::players::CheckedAudioCatalog::from_rom(&[], revision).is_err());
        let file = if revision == tje_mem::Revision::Rev00 {
            "rev00.bin"
        } else {
            "rev02.bin"
        };
        let Ok(mut rom) = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../roms")
                .join(file),
        ) else {
            continue;
        };
        let checked = crate::players::CheckedAudioCatalog::from_rom(&rom, revision).unwrap();
        assert!(checked.commands > 0);
        assert!(
            checked
                .callbacks()
                .iter()
                .all(|&callback| callback == tje_game::ids::MUSIC_PROCESS_TEMPLATE_RESTART_HOOK)
        );
        let templates = tje_mem::rom_data_units(revision)
            .iter()
            .find(|unit| unit.name == "SongChannelTemplates")
            .unwrap();
        let callback = tje_game::ids::PROCESS_PLAYERS.metadata().address(revision);
        rom[templates.address as usize + 8..templates.address as usize + 12]
            .copy_from_slice(&callback.to_be_bytes());
        assert!(crate::players::CheckedAudioCatalog::from_rom(&rom, revision).is_err());
    }
}

#[test]
fn four_pad_ui_dispatch_is_edge_owned_and_cannot_repeat_transactions() {
    let (mut coop, mut services, mut world) = fixture(Archetype::ToeJam, SeatId::ALL[0]);
    let open = InputFrame::new(0, [0x120; 4], [0; 4]).unwrap();
    let result = dispatch_inputs(&mut coop, &mut services, 0, open, &mut world).unwrap();
    assert!(result.advanced);
    assert!(
        coop.players
            .iter()
            .all(|player| player.ui.overlay == Overlay::Inventory)
    );
    let before = (coop.clone(), services.clone());
    assert!(
        !dispatch_inputs(&mut coop, &mut services, 0, open, &mut world)
            .unwrap()
            .advanced
    );
    assert_eq!((coop.clone(), services.clone()), before);
    let release = InputFrame::new(1, [0x100; 4], [0; 4]).unwrap();
    dispatch_inputs(&mut coop, &mut services, 4, release, &mut world).unwrap();
    for seat in SeatId::ALL {
        coop.players[seat.index()].inventory[0] = Some(
            services
                .resources
                .presents
                .wrapper_for(PresentKind::ExtraBuck),
        );
    }
    let use_present = InputFrame::new(2, [0x110; 4], [0; 4]).unwrap();
    dispatch_inputs(&mut coop, &mut services, 8, use_present, &mut world).unwrap();
    for player in &coop.players {
        assert_eq!(player.bucks, 1);
        assert!(player.inventory[0].is_none());
        assert_eq!(player.ui.overlay, Overlay::Closed);
    }
    services.validate(&coop).unwrap();
}

#[test]
fn roses_wait_for_growth_and_latch_each_hero_until_exit() {
    let (mut coop, mut services, mut world) = fixture(Archetype::ToeJam, SeatId::ALL[0]);
    for player in &mut coop.players {
        player.motion.position = point(400, 400);
    }
    let effect = Effect::stationary(
        SeatId::ALL[0],
        PresentKind::RoseBushes,
        EffectKind::RoseBush,
        level(2),
        point(400, 400),
        1000,
    );
    let id = services.resources.spawn_effect(effect).unwrap();
    services.tick_effects(&mut coop, 0, &mut world).unwrap();
    let health = coop.players.each_ref().map(|player| player.health);
    assert_eq!(services.resources.effect(id).unwrap().contact_mask, 0);
    services.set_effect_animation(id, true).unwrap();
    services.tick_effects(&mut coop, 4, &mut world).unwrap();
    for player in &coop.players {
        assert_eq!(player.counters.pending_hurt_damage, 4);
    }
    for frame in 1..=48 {
        services
            .tick_players(
                &mut coop,
                4 + frame * 4,
                [PlayerInput::default(); 4],
                &mut world,
            )
            .unwrap();
    }
    services.tick_effects(&mut coop, 200, &mut world).unwrap();
    for seat in SeatId::ALL {
        assert_eq!(coop.players[seat.index()].health, health[seat.index()] - 4);
        coop.players[seat.index()].motion.position = point(500, 500);
    }
    services.tick_effects(&mut coop, 204, &mut world).unwrap();
    assert_eq!(services.resources.effect(id).unwrap().contact_mask, 0);
    for player in &mut coop.players {
        player.motion.position = point(400, 400);
    }
    services.tick_effects(&mut coop, 208, &mut world).unwrap();
    for frame in 1..=48 {
        services
            .tick_players(
                &mut coop,
                208 + frame * 4,
                [PlayerInput::default(); 4],
                &mut world,
            )
            .unwrap();
    }
    for seat in SeatId::ALL {
        assert_eq!(coop.players[seat.index()].health, health[seat.index()] - 8);
    }
}

#[test]
fn mystery_identification_uses_the_catalogue_without_consuming_failed_choices() {
    let (mut coop, mut services, _) = fixture(Archetype::ToeJam, SeatId::ALL[0]);
    let seat = SeatId::ALL[0];
    let actor = ActorId::new(ActorId::POPULATION_START, 1).unwrap();
    services
        .resources
        .add_service(TradeService {
            actor,
            source: Some(PopulationId {
                level: level(2),
                local: 0,
            }),
            kind: ServiceKind::Identify,
            level: level(2),
            position: coop.players[0].motion.position,
        })
        .unwrap();
    coop.players[0].bucks = 10;
    let mystery = services
        .resources
        .presents
        .wrapper_for(PresentKind::Mystery);
    coop.players[0].inventory[0] = Some(mystery);
    let requests = [crate::resources::ResourceRequest {
        seat,
        action: crate::resources::ResourceAction::Identify {
            slot: 0,
            service: actor,
        },
    }];
    let mut output = [crate::resources::ResourceOutcome {
        request: 0,
        result: Err(crate::resources::ResourceFailure::Unavailable),
    }];
    let rng_before = services.resources.effect_rng;
    services
        .resources
        .transact(&mut coop, 0, &requests, &mut output, |rng| {
            rng.bounded(8);
            Ok(PresentKind::Mystery)
        })
        .unwrap();
    assert_eq!(
        output[0].result,
        Err(crate::resources::ResourceFailure::Unavailable)
    );
    assert_eq!(services.resources.effect_rng, rng_before);
    assert_eq!(coop.players[0].inventory[0], Some(mystery));
    assert_eq!(coop.players[0].bucks, 10);
    services
        .resources
        .transact(&mut coop, 4, &requests, &mut output, |_| {
            Ok(PresentKind::Jackpot)
        })
        .unwrap();
    assert_eq!(
        output[0].result,
        Ok(crate::resources::ResourceReceipt::Identified(
            PresentKind::Jackpot
        ))
    );
    assert_eq!(coop.players[0].bucks, 8);
    let actual = coop.players[0].inventory[0].unwrap();
    assert_eq!(
        services.resources.presents.kind(actual),
        PresentKind::Jackpot
    );
    assert!(services.resources.presents.identified(actual));
}
