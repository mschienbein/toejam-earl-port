use super::*;
use tje_simulation::{FrameId, Phase};
use tje_world::{
    Anchors, Cell, CellPosition, Density, Dimensions, LevelGeometry, LevelSize, WorldConfig,
    geometry::{Resources, Shape},
    loading::{ImportKind, ImportProgress, ImportSpec},
    population::{Population, PopulationBudget, StableBits},
};

struct ImportTerrain;
impl Resources for ImportTerrain {
    fn parameter(&self, _: LevelId, _: u8) -> tje_world::Result<(u8, u8)> {
        Err(tje_world::Error::Resource(
            "fixture imports final cells, not generation parameters",
        ))
    }
    fn shape(&self, _: Shape, _: u8, _: bool) -> tje_world::Result<u8> {
        Err(tje_world::Error::Resource(
            "fixture imports final cells, not generation shapes",
        ))
    }
    fn alternative(&self, _: u8, _: u32) -> tje_world::Result<Cell> {
        Err(tje_world::Error::Resource(
            "fixture has no alternative table",
        ))
    }
    fn transition(&self, _: u8, _: u8, _: u32) -> tje_world::Result<Option<Cell>> {
        Err(tje_world::Error::Resource(
            "fixture has no transition table",
        ))
    }
    fn base_tile(&self, _: u8, _: u8, _: u8) -> tje_world::Result<u16> {
        Ok(0)
    }
    fn overlay_tile(&self, _: u8, _: u8, _: u8) -> tje_world::Result<u16> {
        Ok(0)
    }
    fn property(&self, _: u16) -> tje_world::Result<u8> {
        Ok(0x10)
    }
}
fn authored_world() -> WorldState {
    let config = WorldConfig {
        root_seed: 1,
        rom_sha1: [1; 20],
        size: LevelSize::Original,
        density: Density::Original,
    };
    let dimensions = Dimensions::new(8, 8).unwrap();
    let anchors = Anchors {
        arrival: CellPosition { x: 1, y: 1 },
        exit: CellPosition { x: 6, y: 6 },
        ship_piece: None,
        regions: Vec::new(),
    };
    let geometry = LevelGeometry::new(
        dimensions,
        vec![Cell::from_word(3).unwrap(); 64],
        vec![0; 64],
        anchors.clone(),
    )
    .unwrap();
    let population = Population {
        budget: PopulationBudget {
            items: 0,
            earthlings: 0,
            reserved_items: 0,
            promotion_numerator: 1,
            promotion_denominator: 1,
        },
        items: Vec::new(),
        earthlings: Vec::new(),
        collected: StableBits::new(128).unwrap(),
        removed: StableBits::new(64).unwrap(),
    };
    let spec = ImportSpec {
        config: config.clone(),
        level: LevelId::new(2).unwrap(),
        kind: ImportKind::Designed {
            course_identity: [2; 32],
        },
        dimensions,
        anchors,
        population,
        source_digest: ImportSpec::content_digest(dimensions, &geometry).unwrap(),
    };
    let mut world = WorldState::new(config).unwrap();
    world.request_import(spec).unwrap();
    assert_eq!(
        world
            .advance_import(&geometry, &ImportTerrain, 1024)
            .unwrap(),
        ImportProgress::Ready
    );
    world
}
struct FrozenFixture {
    world: WorldState,
    scene: PublishedScene,
    catalogue: [u8; 32],
    live_even_ui: Overlay,
}
impl FrozenPublicationSource for FrozenFixture {
    fn scene(&self) -> &PublishedScene {
        &self.scene
    }
    fn world(&self) -> &WorldState {
        &self.world
    }
    fn geometry_epoch(&self, level: LevelId) -> Result<u64> {
        self.world
            .floor(level)
            .map(|floor| floor.geometry_epoch())
            .ok_or(Error::Invalid("fixture floor absent"))
    }
    fn resource_identity(&self) -> [u8; 32] {
        self.catalogue
    }
}
fn fixture(resources: &ResourceCatalog) -> FrozenFixture {
    let level = LevelId::new(2).unwrap();
    let stamp = PublicationStamp {
        hardware_frame: 1,
        phase: Phase::PresentationA,
        epoch: 1,
    };
    let frame = resources
        .selected_frame(
            resources
                .hero(tje_four_player::Archetype::ToeJam, 0)
                .unwrap(),
            0,
        )
        .unwrap()
        .0;
    let mut scene = PublishedScene {
        kind: PublishedKind::World,
        stamp: Some(stamp),
        ..Default::default()
    };
    scene.geometry.push(GeometryReference { level, epoch: 0 });
    scene.views = std::array::from_fn(|index| PublishedView::World {
        seat: SeatId::ALL[index],
        level,
        origin: [128, 64],
        attributes: 0,
        starfield: false,
    });
    scene.actors.push(PublishedActor {
        id: DrawId::Actor(ActorId::new(0, 1).unwrap()),
        level,
        position: [248, 164],
        elevation: 0,
        frame,
        style: PublishedStyle {
            draw_late: false,
            flip_horizontal: false,
            flip_vertical: false,
            high_priority: true,
            palette_bank: 0,
            character: None,
            seat: None,
            mask_pixels: false,
            submerged: false,
            shadow: None,
        },
    });
    FrozenFixture {
        world: authored_world(),
        scene,
        catalogue: resources.identity(),
        live_even_ui: Overlay::Closed,
    }
}
#[test]
fn authored_surface_is_genuine_centered_native_density() {
    let mut compositor = Compositor::new();
    let mut original = vec![Color::BLACK; 320 * 224];
    original[0] = Color::rgb(18, 32, 46);
    original[320 * 224 - 1] = Color::rgb(60, 72, 84);
    compositor.compose_authored(&original).unwrap();
    let (pixels, size) = compositor.frame();
    assert_eq!((size.width, size.height), (800, 448));
    assert_eq!(pixels[112 * 800 + 240], original[0]);
    assert_eq!(pixels[335 * 800 + 559], original[320 * 224 - 1]);
    assert_eq!(pixels[0], Color::BLACK);
    assert_eq!(pixels[112 * 800 + 239], Color::BLACK);
}
#[test]
#[ignore = "explicit private-ROM frozen palette/cache/restore proof"]
fn palette_only_even_checkpoint_restore_rebuilds_saved_scene_without_live_ui_or_world_steps() {
    for revision in tje_rom::Revision::ALL {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../roms")
            .join(match revision {
                tje_rom::Revision::Rev00 => "rev00.bin",
                tje_rom::Revision::Rev02 => "rev02.bin",
            });
        let rom = tje_rom::Rom::import(&std::fs::read(path).unwrap()).unwrap();
        let memory = AddressSpace::new(rom.into_bytes(), revision);
        let resources = ResourceCatalog::load(&memory).unwrap();
        let mut source = fixture(&resources);
        let mut vram = vec![0x11; 0x10000];
        for upload in resources.static_uploads() {
            let length = usize::from(upload.words) * 2;
            let start = upload.source as usize;
            vram[usize::from(upload.destination)..usize::from(upload.destination) + length]
                .copy_from_slice(&memory.rom()[start..start + length]);
        }
        let original = vec![Color::BLACK; 320 * 224];
        let mut runtime = PublicationRuntime::new();
        let cram = [0; 64];
        let before =
            bincode::encode_to_vec((&source.scene, &source.world), bincode::config::standard())
                .unwrap();
        runtime
            .refresh_source(&source, &resources, &memory, &vram, &cram, &original)
            .unwrap();
        let dark = runtime.frame().0.to_vec();
        source.live_even_ui = Overlay::Inventory;
        runtime
            .refresh_source(&source, &resources, &memory, &vram, &cram, &original)
            .unwrap();
        assert_eq!(runtime.frame().0, dark);
        let bright = [0x0eee; 64];
        runtime
            .refresh_source(&source, &resources, &memory, &vram, &bright, &original)
            .unwrap();
        let expected = runtime.frame().0.to_vec();
        assert_ne!(expected, dark);
        assert_eq!(
            before,
            bincode::encode_to_vec((&source.scene, &source.world), bincode::config::standard())
                .unwrap()
        );
        runtime.evict_resources();
        runtime.invalidate();
        runtime
            .refresh_source(&source, &resources, &memory, &vram, &bright, &original)
            .unwrap();
        assert_eq!(runtime.frame().0, expected);
        assert_eq!(
            before,
            bincode::encode_to_vec((&source.scene, &source.world), bincode::config::standard())
                .unwrap()
        );
        source.scene.actors[0].frame = FrameId::new(resources.frame_count()).unwrap();
        assert!(
            runtime
                .refresh_source(&source, &resources, &memory, &vram, &bright, &original)
                .is_err()
        );
        assert_eq!(runtime.frame().0, expected);
    }
}

struct AuthoredOnly<'a> {
    config: &'a SessionConfig,
    clock: &'a PhaseClock,
}
impl PublicationSource for AuthoredOnly<'_> {
    fn config(&self) -> &SessionConfig {
        self.config
    }
    fn clock(&self) -> &PhaseClock {
        self.clock
    }
    fn world_visible(&self) -> bool {
        false
    }
    fn world(&self) -> &WorldState {
        panic!("authored publication must not read the world")
    }
    fn players(&self) -> &CoopState {
        panic!("authored publication must not read heroes")
    }
    fn hero_actor(&self, _: SeatId) -> Option<ActorId> {
        panic!("authored publication must not read hero bodies")
    }
    fn services(&self) -> &PlayerServices {
        panic!("authored publication must not read resource services")
    }
    fn population(&self) -> &PopulationState {
        panic!("authored publication must not read populations")
    }
    fn terrain_attributes(&self, _: SeatId) -> u16 {
        panic!("authored publication must not read terrain")
    }
    fn geometry_epoch(&self, _: LevelId) -> Result<u64> {
        panic!("authored publication must not read geometry")
    }
    fn hero_style(&self, _: SeatId) -> Result<VisualStyle> {
        panic!("authored publication must not read hero styles")
    }
    fn companion_visual(&self, _: ActorId) -> Result<DynamicVisual> {
        panic!("authored publication must not read companions")
    }
    fn effect_visual(&self, _: EffectId) -> Result<DynamicVisual> {
        panic!("authored publication must not read effects")
    }
    fn props(&self) -> impl Iterator<Item = Prop> + '_ {
        std::iter::empty()
    }
    fn speech(&self, _: SeatId) -> Result<Option<SpeechView<'_>>> {
        Ok(None)
    }
    fn patches(&self) -> impl Iterator<Item = PriorityPatch> + '_ {
        std::iter::empty()
    }
    fn discovery(&self, _: LevelId) -> Result<&dyn DiscoveryView> {
        panic!("authored publication must not read discovery")
    }
    fn item_appearance(&self, _: &PlacedItem) -> Result<ResourceKey> {
        panic!("authored publication must not read items")
    }
}

#[test]
#[ignore = "explicit private-ROM four-seat data setup and authored publication proof; not gameplay"]
fn real_first_floor_setup_and_authored_publication_keep_all_four_heroes_frozen() {
    use tje_simulation::{
        Admission,
        loading::LoadingSources,
        session::{Session, SetupTransaction, SimulationState},
        world_bridge::{
            BridgeViews, GeometryView, capture_actor_bodies, capture_effects, capture_heroes,
        },
    };
    for revision in tje_rom::Revision::ALL {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../roms")
            .join(match revision {
                tje_rom::Revision::Rev00 => "rev00.bin",
                tje_rom::Revision::Rev02 => "rev02.bin",
            });
        let rom = tje_rom::Rom::import(&std::fs::read(path).unwrap()).unwrap();
        let memory = AddressSpace::new(rom.into_bytes(), revision);
        let resources = ResourceCatalog::load(&memory).unwrap();
        let sha1 = revision.sha1();
        let world = WorldConfig {
            root_seed: 123,
            rom_sha1: std::array::from_fn(|index| {
                u8::from_str_radix(&sha1[index * 2..index * 2 + 2], 16).unwrap()
            }),
            size: LevelSize::Original,
            density: Density::Original,
        };
        let original = match revision {
            tje_rom::Revision::Rev00 => tje_world::original::OriginalRevision::Rev00,
            tje_rom::Revision::Rev02 => tje_world::original::OriginalRevision::Rev02,
        };
        let mut config = SessionConfig::stock(
            world,
            tje_four_player::Config::four_seats(123),
            original,
            &[],
        )
        .unwrap();
        config.catalogue_identity = resources.identity();
        config.frame_count = resources.frame_count();
        config.animations = resources.animation_contracts();
        let mut state = SimulationState::new(&config).unwrap();
        let level = LevelId::new(1).unwrap();
        let terrain = tje_world::rom::RomResources::new(&memory).unwrap();
        let items =
            tje_simulation::resources::RomItemDefinitions::new(memory.rom(), revision).unwrap();
        let sources = LoadingSources {
            terrain: &terrain,
            items: &items,
            catalog: resources.definitions(),
            stock_kind: |level, id| resources.stock_kind(level, id),
        };
        state.loading.prepare_floor(&state.world, level).unwrap();
        for tick in 0..10000 {
            state
                .loading
                .advance_sources(
                    &mut state.world,
                    &mut state.services,
                    &mut state.population,
                    &sources,
                    tick,
                    128,
                )
                .unwrap();
            if state.world.floor(level).is_some() {
                break;
            }
        }
        assert!(state.world.floor(level).is_some());
        let mut ready = false;
        for tick in 10000..20000 {
            if state
                .loading
                .advance_initial_positions(&state.world, &state.players, &terrain, tick, 128)
                .unwrap()
            {
                ready = true;
                break;
            }
        }
        assert!(ready);
        let heroes = capture_heroes(&state.players, &state.services);
        let effects = capture_effects(&state.services.resources);
        let actors = capture_actor_bodies(&state.population, resources.definitions()).unwrap();
        let geometry = GeometryView::frozen(
            &state.world,
            &state.loading,
            &terrain,
            &actors,
            BridgeViews {
                heroes: &heroes,
                effects: &effects,
            },
        )
        .unwrap();
        let arrivals = state
            .loading
            .prepare_initial_arrivals(
                &state.world,
                &state.players,
                &state.services,
                &state.population,
                &geometry,
            )
            .unwrap();
        state.loading.commit_arrivals(
            arrivals,
            &mut state.world,
            &mut state.players,
            &mut state.services,
            &mut state.population,
        );
        let mut session: Session = SetupTransaction::prepare(config, state, vec![])
            .unwrap()
            .finish_setup()
            .unwrap();
        assert_eq!(*session.world.active(), [Some(level); 4]);
        assert!(
            session
                .players
                .players
                .iter()
                .all(|hero| hero.life == LifeState::Alive
                    && hero.inventory.iter().flatten().count() == 2)
        );
        let Admission::Step(stamp) = session
            .clock
            .admit(
                Phase::PresentationA,
                tje_four_player::input::InputFrame::neutral(20000),
            )
            .unwrap()
        else {
            panic!("first publication must be admitted")
        };
        let frozen = bincode::encode_to_vec(
            (
                &session.world,
                &session.players,
                &session.services,
                &session.population,
                &session.loading,
            ),
            bincode::config::standard(),
        )
        .unwrap();
        {
            let (refs, target) = session.publication_access();
            PublicationBuilder::new(target)
                .freeze(
                    &AuthoredOnly {
                        config: refs.config,
                        clock: refs.clock,
                    },
                    &resources,
                    stamp,
                )
                .unwrap();
        }
        session.publish(stamp).unwrap();
        session.validate().unwrap();
        assert_eq!(session.published.kind, PublishedKind::Authored);
        assert_eq!(session.published.stamp, Some(stamp));
        assert_eq!(
            frozen,
            bincode::encode_to_vec(
                (
                    &session.world,
                    &session.players,
                    &session.services,
                    &session.population,
                    &session.loading
                ),
                bincode::config::standard()
            )
            .unwrap()
        );
    }
}
