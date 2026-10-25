# Larger, denser levels

Status: **backend and S2 mode-owned geometry/transition services, native admission closed**, 2026-10-25. Independent size/density settings, bounded ExpandedV1 generation, persistent LevelId records, terrain/land services and saved transition machines are implemented in `crates/tje-world`; this is **not a playable larger-level mode**. Wider view is not itself a larger world or a population-cap increase. Faithful defaults remain unchanged. No HD artwork is involved.

`G` is the [matching decompilation](https://github.com/mschienbein/toejam-earl-decomp), researched at commit `57fc83fc54781af8e992bbc3b32afe788c1dbd01`; `G/...` paths are relative to it, and `P/...` paths are relative to this repository. REV 02 addresses come from `G/config/rev02/names.json`.

## Implemented phase 1 contract

`tje-world` is an independent, safe-Rust library. No engine/frontend path depends
on it and no original selection table changes. `LevelSize` (`Original`, `Expanded`
28×39, experimental `Large` 38×52, custom 8–64 per axis) and `Density` (`Original`,
`Low`, `Normal`, `Busy`) are independent immutable configuration fields.
`admit_native` / enhanced `configure_native` accept only `Original` + `Original`
and otherwise return the missing-closure error **before profile registration,
sidecar insertion, reset or any RAM mutation**. The accepted default does nothing;
it does not replace the faithful original generator with ExpandedV1.

The shared [mode-owned simulation plan](mode-owned-simulation.md), led by the
four-player work, inventories functions, shared interfaces and implementation-size
estimates for terrain/render, collision, player, population, inventory/phone, map,
and elevator transitions. Four-player and larger worlds use the same `LevelId`
(validated 0–31), not independently invented bank IDs. ExtendedView's current
stock-bank renderer is intentionally not modified here; its future map-source
input must borrow `LevelGeometry` without copying expanded cells into stock RAM.
Maker's stock-size admission remains separate; its validated future 8–64-cell
format can supply the same geometry source. No extra obtains a native admission
exception merely because another extra can display more pixels.

### Generator identity, work and publication

- `GenerationSpec` includes ROM SHA1, root seed, generator version, size/density,
  ordinary floor ID and a measured stock/control population baseline. Special
  floors 0/1/26 are not generated or stretched. Large/custom remain experimental.
- Configuration identity is SHA-256 over `tje-world/config/1\0`, version as BE u16,
  root seed as BE u64, the 20 ROM-digest bytes, size tag, width/height as BE u16,
  and density tag. Generation identity adds `tje-world/generation/1\0`, that
  digest, BE floor ID, baseline walkable tiles (BE u32), ordinary item/Earthling
  counts (BE u16 each), reserved-item count and two one-byte service flags.
- Candidate geometry has a separate SHA-256 identity over
  `tje-world/geometry/1\0`, BE version, BE root seed, ROM SHA1, BE width/height
  and BE floor ID. Changing density or its measured population baseline does
  **not** reseed the geometry candidate for an attempt. Population capacity can
  still determine whether that candidate is accepted.
- A stage starts from SHA-256 of `tje-world/stage/1\0`, BE version, the appropriate
  identity digest (geometry for geometry stages; full generation for population,
  loot and AI), BE floor ID, zero-based attempt byte, BE u16 stage-label length,
  and label bytes. Its first eight digest bytes are BE u64, reduced modulo
  2147483646, then incremented. Park–Miller/Schrage keeps the full 31-bit state;
  draws expose its low 16 bits and use explicit modulo reduction. No faithful
  RNG/cache, randomized host hash or idle-loop draw is involved.
- Stages are `anchors`, `paths`, `regions`, `features`, `shapes`, `alternatives`,
  `terrain`, `connectivity`, `validation`, `population` and `runtime-ai`; `loot`
  is reserved but does not yet choose actual item definitions. The code fixes
  loop/neighbor/draw order. Anchor candidates draw X then Y; accepted anchors
  then draw their alternate flag. Routes draw an axis only when both axes
  differ, then reduce Manhattan distance. Regions draw frontier index then
  direction. Feature candidates use the imported parameter-4 percentage capped
  at 35%, reserve anchor neighborhoods, and mark bounded 3×3 interior patches.
  Alternatives draw one choice; mixed-region transition candidates inspect at
  most 16 descriptors, each with at most 64 shapes. Populations use their own
  stream; actor AI states are allocated from `runtime-ai`.
- Separated anchors reserve 3×3-cell arrival/exit/objective regions. A closed
  chain connects them, then bounded iterative frontier growth, interior feature
  marking and imported ROM edge/filled/path shape, alternative and transition
  resolution produce final `Cell` values. Full ordinary land uses the edge bank;
  filled-terrain shapes are separate features, not a synonym for solid ground.
  Region scratch labels and final cells are distinct types. This is a new
  versioned pipeline, **not original seed/draw-order equivalence**.
- Connectivity uses actual 8×8-tile terrain, including mirrored bases and
  overlay precedence. A fixed 128-tile local flood propagates boundary bits
  through a circular worklist containing at most one entry per metatile. Coarse
  labels alone cannot validate a required route. Arrival/exit/objective 4×2-tile
  footprints and all declared region anchors must be reachable.
- Each `advance` performs 1–1024 deterministic work units. One unit is a bounded
  candidate/cell, never an elapsed-time slice. Anchor sampling is capped at
  `area*8`, growth at `area*32`, tile-connectivity queue work at `area*128`,
  placement at `area*4` candidates per requested record. Exactly **64 attempts**
  (0–63) may run. Exhaustion retains the request/settings/reason and never
  publishes the last invalid map, substitutes stock geometry or changes a seat.
- The enhanced `generate` body processes 256 work units, restores its job to
  the typed sidecar, then yields `Wait::Frames(1)`. It is deliberately **not
  bound to `MapLoadPlayerLevel` or any native profile**: returning from such an
  override into original consumers would violate closed admission. A future
  mode-owned loader may select it once the shared closure is complete.

### Persistence, placement and limits

`WorldState` owns sorted LevelId-keyed floors, mutable cell/path edits, dirty-cell
bits, shared coarse 320×256 discovery, stable pickup/removal bits, dropped items,
per-floor runtime RNG and unpublished loading work. Reentry does not regenerate a
stored floor. `FloorSpec` distinguishes ExpandedV1, runtime-imported stock/special
geometry and DesignedV1 provenance without duplicating source populations after
publication. The aggregate simulation owns transition continuations; it must
commit hero positions together with `WorldState::commit_cohort` before queries.

Density budgets use **measured reachable walkable tile area**, not map envelope,
viewport count or width. `Original` retains supplied original counts; `Low`,
`Normal`, `Busy` apply 75/100/125% to the stock-area ratio with integer rounding.
Reservations are allocated first, then totals cap at 128 items and 64 Earthlings
per floor. Placement uses checked terrain footprints, stable IDs, explicit
spacing, arrival safety and a four-hostile local cap; impossible placement
rejects that attempt rather than silently dropping records. These records carry
placement **roles**, not fabricated cartridge actor/item definitions; resolving
roles, loot composition and actual gameplay behavior belongs to the shared
mode-owned population cutover. Promotion scaling is recorded as a rational
area ratio, not applied to original rank RAM in this phase.

Four hero-active floors retain independent records. Individual format limits are
128 items/64 Earthlings per floor, 128 global drops, 64 effects and 60 companion
reservations; they are not a promise that every maximum fits simultaneously.
The S1 session reservation accounts for progression props as well: its complete
four-floor maximum request is 1052 actors and rejects before mutation against
1024. A negotiated 112-item/floor expanded profile reserves 988 actors; stock
four-disjoint-floor capacity reserves 592, not the earlier 256 proposal.
The canonical codec is fixed-width little-endian bincode with a 4 MiB decoding
ceiling. Schema/configuration/extent/work invariants validate before standalone
restore and in enhanced format 3; the changed standalone world sidecar schema is
version 2, with no version-1 shim. Nothing allocates extra Genesis RAM, VRAM,
CRAM or hardware sprite entries. Native/profile admission remains closed.
Actual imported sprite-piece residency admission is still part of the closed
native boundary, not something these placement primitives claim to prove.
The encoded maximum-dimension regression measured **12,353 bytes** of geometry
and **4,297 bytes** of population per floor; 23 stored maximum-size floors plus
a pending maximum-size generation encoded to **588,504 bytes**. The test also
reserves all remaining slots in the 32-floor 32+16 KiB envelope and 64 KiB for
other enhanced owners, still below the 4 MiB ceiling.

Tests cover fixed PRNG/SHA vectors; deterministic success/failure across the
dimension/seed corpus and work quanta; exact 64-attempt exhaustion for
coarse-connected/tile-disconnected geometry; bounds/caps; every loading-phase
round-trip; stable bits beyond 31; atomic failed publication; multi-floor model
transitions and persistent deltas; default-off admission; and logical resource
budgets. `world-proof` privately imports each supported revision and emits
determinism/round-trip hashes and clearly labeled **terrain-overview** PNGs.
It does not emit a gameplay, campaign, native/wasm parity or balance claim.

```sh
cargo run -p tje-world --features private-proof --example world-proof -- \
  /private/path/to/user-rom.bin /private/output/world-overviews
```

## S2 shared geometry, loading and transitions

The source algorithms are the readable `tje-game` map terrain/placement, land
consumption, elevator geometry/transitions, falling/level relocation and
telephone/doorway bodies. The original point query's overlay omission and unsafe
negative indexing are not exported as new-world contracts: shared terrain and
footprint queries use checked coordinates and actual base/overlay/mirror support.
`terrain::position_in_cell` preserves TerrainValidation's cyclic row/column
search, draw order, mirror output and full-metatile anchor rules with validated
dimensions. `TerrainCache` samples a separate 5×5 neighborhood for any seat.
Elevator barriers expose collision, rider entry/exit and placement exclusions
without camera banks. Placement work counts rejected/marked cells, has ten random
trials followed by a finite exhaustive scan, and never consumes uninitialized
coordinates on failure.

`land::LandState` keeps path marks/alternates separate from final cell words and
consumes one cell per authoritative land step in up/right/down/left preference.
It retains traversal/RNG/dirty state through capture; graphics allocation and
animation completion cannot gate land mutation. An unsupported path/isolated
marked edge rejects rather than importing the original stale-register bug.
Discovery retains the original translated `(x-72,y-8)` bounds and 320×256 bins,
including their last boundary pixels; a shared bit awards once per level.

`loading::ImportJob` stages at most 1024 cells per advance and authenticates a
versioned rolling source digest before publication. Cancellation, source-digest
failure and generation's exact 64-attempt failure leave live floors and seats
unchanged. `RomResources::special_source` reads floor 0's inset, floor 1 and floor
26's authored maps directly from the user's ROM at runtime, always at original
dimensions. These imports retain ordinary population/removal/item/drop deltas
through the same shared records; they do not invoke an original RAM loader.

`transitions::Journey` is a saved finite cohort machine: join window, departure,
bounded loading, safe placement, ready-to-commit, arrival and completion/failure.
Four disjoint journeys may coexist in the aggregate; its owner rejects
overlapping cohort masks. Destination positions and every cohort member validate
before a single atomic level-ID commit, and support is rechecked at that boundary.
Elevator joins name seats explicitly. Falls accept authored links, with the
campaign adapter explicitly handling 1→0, 26→25 and 0→highest visited floor.
Doorways relocate within their own level and retain per-seat crossing latches.
Ship entry 25→26 requires the supplied shared objective mask; no ordinary
elevator can bypass it. Special destinations must be staged through the explicit
special import adapter rather than accidentally invoking ExpandedV1.

The original telephone is **not a teleport**. Its saved six-ring/60-tick cadence
and four-seat answer arbitration emit a seat-local temporary map-reveal request.
The UI closes that continuation explicitly; it never moves a hero or awards
exploration score. Transition timers are explicit enhanced simulation ticks,
not faithful frame/cycle-equivalence claims.

ROM-free semantic tests compare TerrainValidation outputs/RNG and both original
5×5 terrain caches against observations captured from the readable Rust bodies
over authored synthetic resources in REV 00 and REV 02. Lifecycle tests cover
four independent maximum-bound floors, all persistent deltas, copies/swaps,
cohort atomicity, loading/placement cancellation/failure, special/ship paths,
land traversal and saved phone/doorway continuations. These are service tests,
not a four-pad native/browser campaign receipt. The aggregate phase bridge,
player/AI/UI/render integrations and complete admission evidence remain S6's gate.

The original design and uncompleted shipping acceptance requirements follow.


## Original generation and limits

| Ref | G source / established behavior |
| --- | --- |
| L1 | [G `include/terrain.h:4–30`](../../../toejam-earl-decomp/include/terrain.h#L4), [G `include/ram_map.h:6–31`](../../../toejam-earl-decomp/include/ram_map.h#L6): 19 columns × 26 rows of word metatiles, two active banks and one saved bank; packed active path nibbles and saved alias, 26 level seeds. REV 02 map bases `0xFF81AA`, `0xFF8586`, `0xFF8962`; each bank is 988 bytes. Map rows and final-level ID both happen to equal 26; they are not the same concept. |
| L2 | [G `src/map/level_generation_and_changes.c:418–467,482–519,536–674`](../../../toejam-earl-decomp/src/map/level_generation_and_changes.c#L418): `GenerateLevelSeeds` (`0x004658`) copies fixed seeds or draws 26 words; `ChangeLevel` (`0x004A1A`) scopes RNG, reuses/swaps banks, places anchors, connects/grows regions, checks connectivity, refines shapes/features/transition chains and chooses alternatives. Successful generation restores the prior RNG; rejection does not. `MapLoadPlayerLevel` (`0x004DFE`) stores a replacement seed and retries. Level 0/1/final maps are authored special paths, not this ordinary generator. |
| L3 | [G `src/map/random_regions_and_cleanup.c:9–84,86–108`](../../../toejam-earl-decomp/src/map/random_regions_and_cleanup.c#L9), [G `docs/subsystems/map.md:108–163`](../../../toejam-earl-decomp/docs/subsystems/map.md#L108): Park–Miller/Schrage full 31-bit state, low 16-bit public result, 59-word cache without advancing external state; ROM per-level ranges and direction/shape tables; recursive region/path search and stage-dependent flags. The `19*26` fill calculation and many bounds are literals. |
| L4 | [G `src/map/terrain_at_position.c:17–118`](../../../toejam-earl-decomp/src/map/terrain_at_position.c#L17): a cell spans 128×64 world pixels, made of 16×8 eight-pixel tiles. Nonzero map coordinates are decremented to zero-based array indices. Out-of-map is water for levels 1/26, void otherwise; negative input coordinates are not safely rejected in the original point query. Two 5×5 player terrain caches include overlay/doorway logic. |
| L5 | [G `include/map.h:23–79`](../../../toejam-earl-decomp/include/map.h#L23), [G `include/assets_map.h`](../../../toejam-earl-decomp/include/assets_map.h), [G `docs/subsystems/map.md:52–76,165–194`](../../../toejam-earl-decomp/docs/subsystems/map.md#L52): generation-class/visit/region-label flags differ from final cell base/mirror/overlay flags; 122 base metatile maps, fifteen overlay maps, sixteen transition descriptors and ROM terrain/alternative/range tables. Live path destruction changes geometry, animations and fall checks. |
| L6 | [G `src/earthlings/spawning_and_banks.c:100–175,190–269,426–473`](../../../toejam-earl-decomp/src/earthlings/spawning_and_banks.c#L100), [G `include/earthlings.h:6–8,40–84`](../../../toejam-earl-decomp/include/earthlings.h#L6): two twenty-record persistent banks, eighteen bytes/record; seeded choices, special ship/mailbox insertion; ten random terrain-valid placement attempts then a full cyclic scan; hard-coded `%26`, `%19`, stride 19. Camera rectangle/parity controls instantiated objects, not initial whole-map population. |
| L7 | [G `src/presents/level_item_setup.c:151–220,324–397`](../../../toejam-earl-decomp/src/presents/level_item_setup.c#L151), [G `include/ram_presents.h:8–30`](../../../toejam-earl-decomp/include/ram_presents.h#L8): two 32-item banks, original item count 12/16 plus level growth, capped at 28 ordinary slots, four reserved large-item slots; 32-bit collected masks; 32 dropped items; marked-tile rejection can fail to consume attempts. More land alone does not increase food/presents or safely support more than 32 items. |
| L8 | [G `src/core/startup_and_main_loop.c:347–362,677–705`](../../../toejam-earl-decomp/src/core/startup_and_main_loop.c#L347), [G `include/ram_map.h:28–31`](../../../toejam-earl-decomp/include/ram_map.h#L28): exploration score uses `(x-72)/320`, `(y-8)/256`, fixed 2560×1792 bounds and seven bytes per **level**, not one bit per metatile. The visited table's 26-level extent is explicitly inferred in its header. The main loop consumes RNG while waiting for frame change, so a level seed is not a complete runtime replay. |
| L9 | [G `docs/subsystems/camera.md:30–49,97–155`](../../../toejam-earl-decomp/docs/subsystems/camera.md#L30), [G `include/ram_camera.h:13–57`](../../../toejam-earl-decomp/include/ram_camera.h#L13): two cameras; row buffers `[2][45]`, column buffers `[2][32]`; X clamp 8–2360, Y 8–1560 shared/1672 split; 64×32 VRAM map wrapping and VBlank upload ownership. The world cannot be enlarged by changing a renderer scissor alone. |
| L10 | [G `include/entities.h:7–24,60–107`](../../../toejam-earl-decomp/include/entities.h#L7), [G `include/ram_sprites.h:6–43,56–79`](../../../toejam-earl-decomp/include/ram_sprites.h#L6), [G `docs/subsystems/sprites.md:72–122,149–196`](../../../toejam-earl-decomp/docs/subsystems/sprites.md#L72): two heroes, forty 128-byte objects, twenty dynamically allocated gameplay objects, 54 viewport slots, 80 hardware pieces, 640 sprite-pool tiles and nominal 112-tile frame budget; motion/visibility/allocation are coupled. |

Memory/hardware baseline: 64 KiB work RAM ([G `src/asm/interrupts.s:10–25`](../../../toejam-earl-decomp/src/asm/interrupts.s#L10)); 64 KiB VRAM, 64 CRAM words/four sixteen-color slots ([G `src/asm/vdp.s:6–49,150–166`](../../../toejam-earl-decomp/src/asm/vdp.s#L6)); ResetVDP chooses plane A `A000`, window `D000`, plane B `E000`, sprite table `F800` and H40 ([G `src/core/startup_and_main_loop.c:33–54`](../../../toejam-earl-decomp/src/core/startup_and_main_loop.c#L33)). H40's twenty entries/forty sprite tiles per scanline come from [P vendored VDP `registers.rs:72–104`](vendor/jgenesis/backend/genesis-components/src/vdp/registers.rs#L72), not G. The original sprite pool alone occupies 640×32 = 20 KiB before terrain/HUD/maps; there is no spare high-resolution framebuffer in VRAM.

## Concrete world model

Two settings, each off/default-stock unless the player explicitly starts an enhanced world:

1. **Level size**: `Original` 19×26; proposed `Expanded` **28 columns × 39 rows** (1092 cells, about 2.21× original area); `Large` 38×52 (4× area); custom/Maker bounds 8–64 cells per axis. Large/custom are experimental until generation and pacing tests pass.
2. **Density**: explicit walkable-area-scaled item and Earthling budgets with named low/normal/busy presets, independent of camera width and size. Original setting retains original caps/composition. Density never implicitly doubles because a fourth camera is open.

Ordinary campaign levels 2–25 can use the expanded generator. Original authored level 0/1/final scenes remain original-sized and centered in the containing world; do not stretch/reinterpret their ROM maps, ending or elevator staging. A separately authored Maker course can replace their roles under its own ruleset. The mode manifest records where each generator applies.

Use `LevelGeometry { width:u16, height:u16, cells, path_state, anchors }` in an owned enhanced sidecar, not resized 68000 RAM arrays. Final cell representation separates base metatile/mirror, overlay/mirror and live path state; intermediate generation-region labels have a separate scratch type and cannot accidentally be rendered as final cells. ROM-resolved base/overlay definitions and terrain properties remain imported resources, not copied tables in P.

Retain the original coordinate convention as a useful compatibility policy: first cell starts at world `(128,64)`; one border-cell band surrounds it. Define world envelope `((width+2)*128, (height+2)*64)`. For the stock dimensions this yields 2688×1792, and camera maximum `world_size - viewport_size - 8` reproduces the documented 2360/1560/1672 clamps. New queries first reject negative/out-of-range coordinates, then do checked index arithmetic; this safe enhanced policy is not a fix silently applied to the faithful original bug. At 64×64 the envelope is 8448×4224 pixels. Store positions/24.8 accumulators in checked i32 and squared distances/intermediates in i64, even though many final positions fit i16.

### Generation contract

- `Original` generator identity remains revision-bound and original-sized, using the user's ROM tables/draw order. The unchanged faithful path stays available without the enhanced profile. When debugging enhanced compatibility, record both requested and accepted replacement seeds; do not claim the rejected seed regenerated the accepted floor.
- `ExpandedV1` is an explicit new generator/ruleset version. It generalizes the readable anchor/path/region/shape pipeline rather than running original functions with a wider stride. Persist root seed, generator version, dimensions, selected rules and accepted attempt index; stage seeds derive deterministically from that identity plus stage label. Specify the integer PRNG and draw order in implementation tests; no host hash randomization or idle-loop RNG consumption.
- Use the original Park–Miller recurrence as the specified per-stage generator, normalized to 1…2147483646; seed derivation uses SHA-256 of the versioned identity/stage bytes, interpreted in a defined byte order then reduced into that interval. Different streams cover geometry, population, loot and runtime AI. No changes to faithful `gRNGState`/cache.
- Parameterize every loop/stride/bound/anchor range and area percentage. Scale minimum anchor separation and route lengths with dimensions; preserve valid arrival, exit and ship-piece footprint regions. Connectivity is a tile-terrain property, not merely reciprocal coarse cell labels.
- Replace recursive large-grid flood/transition searches with bounded iterative worklists. Preallocate at most width×height entries (and separately bound descriptor search work); generate at a resumable loading-phase boundary, with device/audio progress still live. Yield by deterministic work count, not elapsed milliseconds. No spin-wait generation or VBlank loop inside an Atomic override.
- Validate anchors, exit/required collectible reachability, terrain-valid spawnable area and capacity before publication. Maximum 64 generation attempts; each attempt seed is versioned/derived from the same identity and attempt number. On exhaustion, reject the requested world with seed/settings/reason, leaving current state intact. Do not silently substitute an easy stock map or publish the last invalid map.
- Level state is keyed by LevelId and preserves live path edits, pickups, dropped items, actor removals and fog across visits. Four players may occupy four different floors; a fixed A/B/C map cache is not authoritative state. Cache immutable geometry separately from mutable deltas if profiling justifies it, but start with bounded full small maps rather than introducing streaming chunks unnecessarily.

## Density, activity and resource limits

Starting upper bounds: 64 persistent Earthlings, 128 placed items (including reserved services/large items), 128 global dropped items, 64 shared projectile/effect slots; up to four hero-active floors. Increase the enhanced logical entity arena from the four-player stock proposal of 256 to **1024** for this profile, accounting for all simultaneously active floors, items, companions, mandatory actors and effects. Derive actual reserved capacity from the imported definition piece counts before starting; a host draw/cache shortage must not decide AI movement or despawn a required object.

Collected/removed/failure tracking becomes stable-ID bitsets, not original 32-bit masks. Separate persistent population from transient render instances, and visual interest from simulation interest. Actor simulation uses a fixed player-centered activity envelope plus required transition/objective actors, independent of view size, layout, display refresh or director camera. Deactivated persistent actors retain state; activation order is stable by LevelId/entity ID. Define whether dormant AI timers pause (recommended initially) as part of `ExpandedV1`, rather than accidentally advancing only whatever the renderer visits.

Budget calculation should use measured walkable/spawnable area, not the number of water/void cells. Start items at approximately the original item count × walkable-area ratio × a visible resource-density multiplier, capped at 128; Earthlings similarly, capped at 64. Preserve reserved ship/mailbox/service slots first. Use explicit spacing and local threat caps so a dense map cannot place all hostiles at an arrival. Reject incompatible density/cap settings before play, rather than allocating until failure and accepting a different generated seed on different machines.

Memory [INFERENCE]: one maximum map's word cells are 8192 bytes; path bytes add 4096; dynamic discovery/collection/removed bitsets are small. Allow 32 KiB geometry/path/fog per floor, 16 KiB actor/item persistent state per floor, at most 32 floors for the shared Maker envelope, plus a 1024-entity arena and generation scratch. A conservative **4 MiB authoritative state** cap remains feasible with these bounds; allocate exactly bounded reusable structures, not 4 MiB every frame. Allow 64 MiB derivative decoded-resource/GPU caches initially. State-size calculations must be checked against actual encoded widths at implementation; these are host budgets, not proof of Genesis fit.

## Overrides and shared rendering

Use EnhancedMode's separate non-default `EnhancedProfile(name, &[Translation], &[FramedTranslation])`, existing `Context` physical-frame ABI and selected nested-call interception. Atomic/Functional replacements with synthetic RTS charges are enhanced, not timing-proven faithful Rust. Reuse the dispatch convention of `NativeCompiledOriginalCallees` ([P `docs/ENGINE.md:437–506`](docs/ENGINE.md#L437)); do not modify faithful selection tables or add a second opcode interpreter.

EnhancedMode now supplies typed sidecars, resumable overrides and format-3 atomic envelopes (see [ENHANCED_MODE.md](../ENHANCED_MODE.md)). The still-missing part is the shared mode-owned world/render/transition closure in [mode-owned-simulation.md](mode-owned-simulation.md); larger levels do **not** require four-player balance/UI, but they cannot bypass that common storage-ownership boundary.

Replace these ownership boundaries together:

- Seed/generation/loading: `GenerateLevelSeeds`, `MapLoadPlayerLevel`, `ChangeLevel`, `MapGenerateSeparatedAnchors`, `MapConnectAnchorPositions`, `MapGetRandomGenerationParameter` (`0x004F50`), region/path/shape/transition helpers and bank copy/swap routines. New dimensions must not reach unmodified literal-bound generation leaves.
- Terrain/geometry: `GetMapTerrainAtWorldPosition` (`0x00D9DC`), `UpdateLandAroundPlayer` (`0x00DAD6`), `TerrainValidation`, placement/player-level positioning, Earthling/player bounds/collision callbacks and doorway support queries.
- Persistence/population: Earthling retain/release/spawn/load/dispatch and all per-level failure/removal logic; `LoadLevelItemBank`, item placement/update/pickup/drop, teleport/telephone and elevator/ship anchor setup. An original two-bank lookup cannot remain downstream of a sidecar LevelId.
- Live geometry/discovery: `ProcessLand` (`0x01F6D0`), path consumption/animation, falling support, `ProcessInput` exploration award, `DrawGameMap`/cover/markers. Generalize the seven-byte coarse discovery grid and preserve no-double-score behavior. Keep 320×256-world-unit discovery blocks initially with dynamic bounds; do not reveal an entire metatile because a wide view can draw it.
- View/render: camera position/follow/move/build helpers, viewport visibility/loading, sprite queue/animation-motion, world/HUD/map packet boundaries. Use host terrain/metasprite instances resolved from imported tables; no enlarged VDP frame, per-view hardware sprite allocation or guessed upload-buffer slack. Simulation animation/motion must still advance when a resource is not visually resident.

The closure is broad even for two players. Inventory/phone/map/elevator/cutscene blocking paths must become mode-owned resumable transitions before admission. Validate all callback/data-table targets, and do not let host pointers or new dimensions leak into compiled-original RAM-array consumers. Original bounded pure resource decoding/math and sound/device routines may remain compiled.

ExtendedView's first 400×224 (400×112 original split) presentation and experimental ≤512 width remain the camera envelope, not a license to display the entire enlarged floor. For four players use the co-op 2×2 400×224 view composition or optional shared view; the level geometry is independent of the composition. Character palette swaps follow co-op's runtime ROM-derived per-instance palettes, not extra global CRAM banks. Generation only uses existing metatile art decoded locally; no ROM table payload, generated map snapshot, tiles or original text is checked into P.

Snapshots atomically combine faithful format-2 `TJESTATE` (actual ROM SHA1, CPU/devices/timing/input/audio) with enhanced profile/body-manifest/configuration and the map/population/RNG/delta sidecar through EnhancedMode's format-3 envelope. Enhanced inputs use a separate versioned record; faithful two-pad recordings and Netplay packets are not repurposed. Peer-to-peer enhanced play must compare generator identity, dimensions, accepted attempts and sidecar hashes before input simulation; no hosted sharing is needed.

## Balance and risks

- Larger maps lengthen travel/search/fall recovery even at the same density. Expose estimated area/density in lobby; retain multiple reachable loops and a bounded required-object route distance. More food/presents, telephone locations and mobility resources can scale with route length; do not simply multiply enemy count by area and leave supplies at 28.
- Density settings change composition/count/spacing and loot, not original character hitbox/speed. Per-hero local threat caps and arrival safety take precedence over filling the requested count. If a generation contract cannot satisfy a minimum population, report it rather than silently changing rules mid-session.
- Keep exploration points per newly visited coarse block, once per shared discovery state; decide resource/promotion scaling explicitly for named presets so large maps are not free rank inflation. Recommended expanded preset scales promotion thresholds by walkable-area ratio with integer rounding, while stock mode keeps original thresholds.
- Risk: recursion/area/coordinate overflow; use bounded worklists and wider arithmetic. Risk: original-generation stages use transient flags; separate types. Risk: view-dependent activation changes RNG/fairness; fixed interest. Risk: fall/backtrack loses deltas; level-ID persistence. Risk: stock authored special floors/ending no longer align; leave their content envelope intact.

## Phase estimates and acceptance tests

Estimates [INFERENCE], engineering days, excluding shared sidecar/phase/render F-A/F-B costs already itemized in the co-op document; not a measured schedule.

| Phase | Observable exit | Estimate |
| --- | --- | --- |
| L-A | Bounded geometry/query/bank-state cutover; stock-dimension enhanced control and original faithful unchanged | 4–6 days |
| L-B | Versioned expanded pipeline, iterative searches, deterministic retry/validation and seed identity | 5–8 days |
| L-C | Density/item/Earthling/mask migration, dynamic discovery, map/UI/camera bounds and loading transitions | 5–8 days |
| L-D | Native/wasm determinism, stress/campaign progression, pacing/resource tuning | 4–7 days |

Required tests before shipping:

1. Complete existing REV 00/REV 02 engine session corpus, native/browser frame **and audio hashes** unchanged with size/density defaults and extras absent/off. A presentation-only 400px setting must not select larger geometry/population rules.
2. Stock-dimension enhanced control versus original geometry/anchor/population observations for accepted seeds; document intended enabled-mode timing differences, do not relabel enhanced frame hashes faithful. Distinguish requested from replacement seed on rejection.
3. Deterministic property corpus across dimensions 8×8, 19×26, 28×39, 38×52, 64×64 and edge/asymmetric grids: valid cells/indices/flags, bounded work, legal footprints, connected required route, deterministic success/failure/attempt count and byte-identical encoded geometry on native/wasm. Validate actual tile terrain, not just coarse anchors.
4. Negative/border/extreme coordinates, camera every edge, all mirrored base/overlay combinations, doorway support, sinking/water/void/falling, collapsing paths and visible dirty-cell updates. Empty/all-marked/no-spawnable maps fail finitely instead of invoking original unbounded placement bugs.
5. Maximum persistent populations on four different active floors, large item/dropped/projectile pools, fixed-ID bitsets beyond bit 31, collectible/service reservations and forced graphics cache misses. No simulation result depends on draw budget or viewport count.
6. Leave/reenter/fall through multiple levels after terrain mutation/collection/removal; all deltas and fog survive cache changes and snapshot/restore at every generation/loading phase. A partial generation never replaces the playable level.
7. Same world/input manifest under 320/400/512 presentation, shared/quadrant views, fast-forward and variable presentation cadence: authoritative state/RNG identical. Discovery and promotion updates occur once, not per view.
8. Whole short and full campaign progression with expanded ordinary levels, original special floors and all hero counts. Playtest route-time, resource starvation, rank pace and threat density before promoting a preset from experimental.

Phase-1 tests and private evidence are recorded in the worker report. The full shipping acceptance list above remains open; no playable expanded campaign or native admission is claimed.
