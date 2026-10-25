# Mode-owned simulation: the common cutover for world-changing extras

Status: gated S1 typed service/ownership foundation, with subsystem integration under implementation, 2026-10-25. **This is not an admitted playable mode.** Larger-level, generalized Maker, four-player and Overlord gameplay admission remain closed until the complete subsystem integration and S6 evidence below exist. Faithful mode stays the untouched default.

This document consolidates the requirements of [four-player co-op](four-player.md), [larger/denser levels](larger-denser-levels.md), [Overlord](overlord.md), [Maker](maker.md), and the [extended-view renderer](../EXTENDED_VIEW.md). These are not five new engines. They need one generalized world representation and one set of mode-owned simulation services, selected through the existing enhanced driver. Features select rules/capabilities over that world, not independent copies of player/AI code.

## 1. What exists, and what does not

The default faithful engine still runs compiled original code and its original RAM layout. Its two-player tables, 19×26 map banks, forty object records, two viewport bits and 80 hardware sprites must not be resized, aliased or reinterpreted for extras.

Available foundations:

- `tje-engine-runtime::enhanced`: explicit default-off profiles, live `Context`, resumable callbacks, scheduled original calls, typed deterministic sidecars, four complete controller states, and atomic format-3 enhanced snapshots. See [ENHANCED_MODE.md](../ENHANCED_MODE.md). Faithful snapshots remain format 2.
- `tje-game`: readable Rust bodies, metadata, classifications and physical call ABI. Availability, proof and production selection are **different facts**; some bodies remain pending/check-only. A readable function is a source for generalization, not automatic permission to select it or claim a faithful timing proof.
- `tje-world`: shared bounded `LevelId`, dimensions, final-cell/geometry/resource interfaces. LargerLevels owns deterministic generation, mutable LevelId-keyed world/persistence, stable-ID collection/removal state and bounded population-placement records. Those placement records are **not** cartridge AI/loot definitions or event dispatch; model transitions do not authorize original player/elevator RAM calls. Its new-world native admission remains fail-closed. Do not create a parallel four-player map format.
- `tje-four-player`: four typed seat/character identities, validated per-seat state and level-ID arrival bookkeeping, stable source routing, explicit wire-to-game button conversion, a distinct enhanced input-frame schema, runtime palette transforms and fixed quadrant rectangles. With `enhanced-mode`, `CoopState` is a validated sidecar and requests to enable gameplay reject before mutation. This is data/codec functionality, **not** movement, inventory effects, transitions or rendered co-op.
- ExtendedView2's presenter retains the original center and adds side strips using two stock map banks and two published view submissions. It can decode runtime ROM metasprite pieces and terrain; it does not own a four-view simulation or lift hardware allocation limits for the original game.

Missing is the **complete world ownership closure** below, together with a sidecar-driven host publication/composition path. There is deliberately no `FOUR_PLAYER_PROFILE` filled with partial replacements. A rejected request cannot silently become two-player play, an invisible/invulnerable extra actor, a debug profile, or a movement-only demo.

## 2. Selection is an ownership boundary, not a patch list

Preserve the original cartridge boot/device/audio scheduling and enhanced driver's bounded execution. Replace whole selected world subsystems once they contain new state. Original `player != 0`, `player ^ 1`, `PLAYER_BOTH == 2`, signed actor sentinels, two map-bank lookups and viewport masks cannot remain downstream of sidecar-owned actors or maps.

Each profile has an immutable, versioned **ownership manifest** containing:

1. Profile/ruleset and generalized body-manifest versions; required capabilities and supported hero/role masks.
2. Selected entry `FunctionId`s and adapter/body versions, grouped by subsystem.
3. Every direct callee and installed ROM/data-table callback reachable after cutover, with an explicit disposition: mode-owned typed service, bounded read-only resource/math service, or approved device/audio original.
4. The allowed original call boundary and ABI for each retained leaf. No `SeatId`, host pointer, host entity ID, extended dimension or sidecar level reference may enter an original player/object/map ABI.
5. Configuration/course/generator identity and the bounded state/resource budgets being admitted.

Build the transitive closure from function metadata **and** descriptor/dispatch tables. Metadata's direct `callees` alone is not a callback audit. Translate recovered callback addresses to finite typed behavior/animation IDs when importing ROM definitions; never serialize function pointers. Unknown callbacks or missing dispositions reject admission before changing the current machine/profile/sidecars. Include interrupt-reachable graphics/HUD work, input/menu paths, special scenes and nonlocal exits, not just calls made by `ProcessPlayers`.

Profiles compose through a single owner for a shared root. Unioning Jam Out and co-op tables, or larger-level and Maker loading tables, is not sufficient: duplicate selections are already rejected by the enhanced registry, and two independent owners would disagree even without duplicate addresses. A composed adapter must invoke the relevant typed services in the manifest's order. Ordinary Jam Out remains a separate two-seat mode until its state/effect ownership is explicitly generalized.

### Setup and frame bridge

The original setup roots (`SetupPlayer`, `SetupTheMainGame`, initial inventory/Earthlings/elevators, seed setup and positioning) become one new-session transaction:

- Validate immutable settings, resource definitions, all callback dispositions and capacity reservations first.
- Prepare the bounded sidecars and initial shared levels off the live state; do not publish half a setup.
- Install the complete profile and initial sidecars together at the load/reset boundary.
- Mark setup complete before admitting any world phase; do not let a later original setup root clear part of the new world.

Keep the original four-phase relative schedule initially: player/input work and Earthling/elevator/item work alternate, with movement/animation and presentation at their existing relative cadence. Record hardware frame ID, phase, input latch and each phase's last admission in authoritative state. Repeated entry to a phase in the same frame, redraws, audio polls and restored continuations must not advance it twice. A world step is not a render call.

Blocking inventory/map/phone/elevator/death/ship sequences become finite state machines with explicit saved continuations and timers. Use `Wait::Frames`, `Interrupt` or `Poll` for real device progress; scheduled original calls must be proven to return with the required RTS contract. An Atomic callback cannot drain a main loop, wait on a video counter, or call a TimeAware leaf synchronously. `ContinueOriginal` is only a bounded observer for a safe original; it cannot make an unowned world consumer safe. RTE/reset/nonlocal roots are not selectable RTS callbacks. Audit the original IRQ's graphics leaves separately while retaining real audio/input/device progression; never fabricate readiness or suppress an unsupported wait.

### S1 executable service boundary

`tje-simulation` owns the shared `ActorId`, `EffectId`, logical resource IDs, `PhaseClock`, capacity reservation, and aggregate `Session`/`SetupTransaction`. It reuses `tje-world::LevelId`/`WorldState` and `tje-four-player::SeatId`/`CoopState`, rather than inventing another map or roster representation. One aggregate sidecar authenticates immutable configuration and validates hero identities, current floors, resources, capacities and publication references together before registration/capture/restore.

`tools/generate_ownership.py --decomp PATH` emits the auditable `crates/tje-simulation/src/ownership.generated.json` and its allocation-free Rust lookup table. The inventory expands all 648 public metadata identities through direct callees, recovered C/assembly function references, potential indirect call sites, both IRQ roots, and both revisions' descriptor/dispatch schemas. An indirect site conservatively reaches **every catalogued function**; this is an explicit over-approximation, not a claim that metadata alone identifies the actual ROM callback set. Source identities, per-function ownership dispositions, retained-original ABI boundaries and descriptor field offsets are auditable. No ROM table values, graphics, palettes or extracted maps are emitted.

Runtime import resolves actual callback addresses only at exact catalogued RTS entries, checks the typed callback family/service owner, and records finite behavior IDs instead of executable pointers. Unknown/interior/ABI-incompatible targets reject. The inventory does not grant implementation authority: built-in implemented-service receipts and S6 native/browser admission evidence are separate gates, and S1 deliberately installs no gameplay profile or selector.

The saved bridge starts with `PresentationA (1) → Population (2) → PresentationB (3) → Players (0)`, matching the original loop's preincrement. Movement/animation follow the player/population phases; land/view publication follow the odd phases. Every phase admits at most once per increasing hardware frame with an immutable input latch. Service preflight rejects before admission; an execution failure latches the session rather than retrying partial effects. Original setup reentries only acknowledge the already committed new-session transaction and never clear live world state.

### Explicit generator/configuration and recording prerequisites

`SessionConfig` schema 2 authenticates `tje-world::GeneratorId::{OriginalV1,ExpandedV1,DesignedV1}` separately from dimensions/density. Stock co-op requires **OriginalV1**: the revision-bound original seeds, draw order, geometry and items, proven against original-engine generated map RAM for the same seeds/levels across floors 0–26 in both revisions. ExpandedV1 remains the larger-world generator. A synthetic 19×26 ExpandedV1 fixture is explicitly not an OriginalV1 receipt. Completed floors and pending generation/import work must agree with the selected generator. DesignedV1 additionally authenticates a bounded immutable per-level course/source/definition manifest before aggregate restore commits hardware.

The controller-only enhanced recording format is separate from faithful `tje-session`: eight-byte `TJEINPUT` magic, little-endian u16 version 1 and zero u16 flags, u32 body length, SHA-256 of the body, then canonical fixed-width little-endian bincode. The typed body contains complete normalized session configuration, initial stable input routing, compressed four-pad/action runs, and frame/epoch-ordered explicit source assignments. It contains no RAM recipe, snapshot, ROM resource data or executable pointers. Payloads are bounded at 16 MiB, 262144 runs and 65536 assignments; unknown versions/flags, bad checksums/lengths, stale assignment epochs, disabled/unassigned input, trailing bytes and config/ROM/generator mismatches reject. SHA-256 is corruption/identity checking, not author authentication.

Replay's seek index is derivative and rebuilt once; rewinding seeks by the restored hardware frame, never a serialized host iterator. Disconnect records neutral input without silently transferring a seat. The aggregate latches hero input only at the Players phase, including a first hardware frame after boot/intro; it does not reuse the primitive codec's consecutive-from-zero bookkeeping helper for gameplay cadence. Recording primitives do not install a playable profile, add transport, or supply the still-required actual native/browser four-pad transition evidence.
## 3. Subsystems that move together

The names below identify ownership roots, not a complete whitelist. The implementation must expand them through all leaves/callbacks before admission.

| Subsystem | Original boundaries / starting bodies | Mode-owned responsibility and exit condition |
| --- | --- | --- |
| Players and input | `GetPlayerInput`, `ProcessInput`, button checks, `ProcessPlayers`, `ProcessMovement`, terrain-motion callbacks, `SetSprite`, falling/relocation/death/life helpers | Explicit `SeatId`, archetype and roles; 24.8 motion; health/rank/lives/resources; every status counter; four independent 5×5 terrain caches; fall cohorts, hurt/respawn and identified donation targets. No original two-element helper below this boundary. All four heroes are authoritative, collidable and damageable, not just drawn. |
| Presents, items and shared resources | Initial inventory, `LoadLevelItemBank`, update/pickup/drop, all present dispatch/effect/projectile/expiry handlers, decoys/telephones/services | Sixteen slots per hero; all 28 wrapper/effect identities, shared randomization/identification; explicit owner/generation on effects; bounded pools/quotas; atomic claims/purchases/drops; rotating seat tie priority; one shared ship-part state. No original first-four-projectile/two-decoy/two-telephone or 32-bit-mask assumptions. |
| AI and persistent populations | Earthling retain/release/load/allocate/spawn/dispatch; nearest-target steering; generic and kind-specific thinkers; contacts/mole/Santa/services | Persistent actors distinct from draw instances; up to four hero-active floors; four-seat eligibility/contact latches; original decoy priority and deterministic higher-seat equal-distance tie; protected states and kind-specific terrain/damage; stable-ID collection/removal/failure tracking; original twenty-Earthling default independent of optional density. |
| Geometry, land and discovery | `GetMapTerrainAtWorldPosition`, `UpdateLandAroundPlayer`, `TerrainValidation`, placement/bounds, `ProcessLand` and path animation/consumption | One shared level record; bounds/stride from dimensions; base/overlay/mirror terrain semantics; checked coordinates; mutable paths and dirty cells; dynamic coarse 320×256-world-unit discovery/no-double-score. Original-sized, ExpandedV1 and DesignedV1 feed the same query/mutation services. |
| Loading and level transitions | `GenerateLevelSeeds`, `MapLoadPlayerLevel`, `ChangeLevel`, copies/swaps; elevator setup/process/ride, fall/arrival/doorway/phone/ship helpers | Level-ID persistence rather than camera banks; bounded resumable generation/loading; accepted attempt identity; safe anchors; atomic cohort destination commit before new-level queries; independent floors; preserve edits, consumed resources, removals and drops on return. Original special floors 0/1/26 and final ship progress have explicit adapters. |
| UI and interaction | HUD, inventory/mail/identify, map covers/markers, speech/choices, life requests, pause and transitions | Separate seat UI state/draw packets; overlays neutralize only that hero's locomotion and do not grant immunity or stop other actors. Cooperative pause is an explicit all-participant agreement. No original inventory entities 26–39 or blocking map/menu loops. Requests name recipient/service IDs. |
| Animation and rendering | Camera follow/build, split mode, entity viewport/update/queue, animation-motion, shadows, HUD/inventory/map/speech publication | Simulation animation and authored motion callbacks run without GPU residency. Publish frozen logical entity/piece/view packets; host terrain/metasprite composition supports four independent views and exceeds hardware sprite/SAT budgets. Stock authored scenes remain explicitly 320-pixel pillarboxed until an adapter is approved. |

### Starting from readable Rust, without importing the RAM layout

For each service:

1. Inventory the relevant readable bodies and their proof/status, source semantics, call order, widths, wrap behavior and known original bugs. Reuse the proven algorithms where available; do not claim all readable leaves are already proven.
2. Factor the **logical operation** into a bounded function over typed world/seat/entity views. Parameterize player enumeration, partner/donor selection, level geometry, contact masks, entity capacity and resource lookup. Preserve original archetype animation/stat dimensions and intended fixed-point/counter semantics.
3. Keep narrow original-facing adapters only at enhanced dispatch boundaries. Decode genuine ABI arguments there and resolve to typed services. Never pass sidecar pointers into compiled originals or simulate four seats by swapping fabricated players through original RAM records.
4. Replace callbacks with typed dispatch over the imported definition catalogue. Keep read-only ROM tables, correctly bounded resource decoders/math and audio/device routines where their contracts are safe. Keep synthetic RTS timing out of faithful proof receipts.
5. Test the generalized operation at two-seat/original-size settings against recorded semantic observations, then test the added domain (four seats, separate floors, larger bounds, all effects). Enabled-mode frame/cycle equivalence is not promised; faithful-off frame/audio equivalence is mandatory whenever engine/frontend paths change.

This is one generalized implementation, not copied and independently maintained `ProcessPlayers4`, `ExpandedProcessPlayers`, `MakerProcessPlayers` and `OverlordProcessPlayers` trees.

## 4. Sidecar layout and capacity contract

Authoritative state uses ordered, bounded storage; allocate session capacity once and reuse scratch. Integers have fixed widths, positions are signed 24.8 with checked i32/i64 intermediates, and IDs never alias original signed-byte sentinels.

| State owner | Contents | Lifetime / invariant |
| --- | --- | --- |
| Session identity / phase bridge | Ruleset/body manifest, ROM identity, course/config/generator digest, root seed/independent RNG streams, hardware frame/phase/input admission, mutation queue, shared objectives, publication epoch | Immutable identity authenticated against normalized configuration; phase/mutation/RNG state fully saved. No wall-time or idle-loop randomness. |
| `tje-four-player::CoopState` | Fixed `[PlayerState; 4]`, `SeatId`, independent archetype/palette, stats/inventory/counters/terrain/UI, stable input source map and assignment epoch, four input/action words, seat-local per-LevelId arrival history and claim priority | The current primitive has no simulated effects, entity ownership handles or transition implementation; these are added with the owning services. Array position must match seat, roster/settings must match immutable identity, disabled seats cannot acquire input/history by arrival. |
| `tje-world` shared persistent world | Sorted LevelId records: dimensions/cells/path edits/dirty flags, fog/discovery, anchors/links/elevators, population/items and stable collected/removed/failure state, accepted generation attempts and loading work | One mutable record per visited level. Original campaign 0–26; shared Maker envelope at most 32 floors, at most four hero-active floors. Cache/camera slot is never a level identity. |
| Runtime entity/effect arena | Generation-checked IDs, kind/owner/source/level, motion/behavior/status/contact data; quotas and reserved progression resources; global dropped/projectile/effect pools | Both stock four-player and expanded profiles reserve a **1024-entry** logical arena. Stock four-floor density uses 20 Earthlings and 32 placed items per floor; the maximum four-floor expanded profile uses 64 Earthlings and **112 placed items per floor**, plus 128 global drops and 64 effects. Individual shared-world format limits of 128 items/floor are not simultaneously admissible on four active floors with every other pool full. These are explicit joint profile caps, not unconditional counts or extra original RAM. |
| Per-seat / role state | Overlay/choice/release/pause latches; elevator/fall cohort; effect IDs, donor/recipient and service references; Overlord role/budget/commands when enabled | Explicit IDs/masks; no `player ^ 1`. Four-pad input does not imply four hero bodies in a 3-vs-1 ruleset. |
| Rebuildable presentation | Decoded ROM tiles/pieces/palettes, GPU bindings, view transforms, UI layout, visible draw lists and clip scratch | Not authoritative. Rebuild after restore; eviction or skipped presentation must not alter AI, motion, collisions, discovery, RNG or allocation success. |

Keep **participating input seats**, **hero-body seats** and **network ownership** as distinct masks in the generalized role manifest. They coincide only in ordinary co-op. The current co-op-only `Config::participants` deliberately couples its hero/input masks; Overlord must version and extend that configuration/validation before accepting director input with no hero body. It cannot route a director by merely disabling player four or assigning that seat an original player index.

Worst-case admission arithmetic (Main-approved S1 contract):

- Stock four-player: `4 heroes + 4×20 Earthlings + 4×32 items + 28 progression actors + 60 companions + 64 effects + 128 drops = 492` logical actors. The former 256-entry stock proposal cannot guarantee four disjoint floors.
- Maximum four-floor expanded profile: `4 + 4×64 + 4×112 + 28 + 60 + 64 + 128 = 988`, within 1024.
- Requesting all individual maxima instead gives `4 + 4×64 + 4×128 + 28 + 60 + 64 + 128 = 1052`, exceeding 1024. Reject that request **before mutation**; never silently trim items, despawn required actors, or let allocation/render order choose a different world.
- Reserve all simultaneously live authoritative storage as well: at most 32 floors × 48 KiB geometry/persistence, 512 KiB pending bounded generation, 1024 × 512 bytes runtime actor allowance, and 128 KiB aggregate/control/input/config/continuation allowance = **2.625 MiB**. This conservative reservation remains below the shared 4 MiB format-3 cap; actual codec bounds are also enforced at registration/capture/restore. The separate 64 MiB presentation allowance cannot authorize simulation allocation.

Budgets [INFERENCE, to be measured against the actual codecs]: co-op seat data under 16 KiB; maximum geometry/path/fog about 32 KiB/floor plus 16 KiB persistent actors/items/floor; total authoritative enhanced payload at most **4 MiB**, shared with continuations/inputs/config, and derivative graphics/cache storage initially **64 MiB**. Do not use the whole sidecar allowance for a course and leave no runtime room. Sidecar registration/capture already enforce the combined payload bound; admission needs a worst-case reservation calculation as well.

Persistent world invariants must be checked together with cross-sidecar references at registration/capture/restore. The current per-sidecar `validate` hook alone cannot validate a reference into another sidecar. Prefer one registered aggregate world/session owner (with typed internal services) where that keeps the transaction simple, or add an explicit registry-level cross-reference validation boundary before admission/restore commit. Do not commit hardware and then discover a missing destination/actor.

## 5. Host sprite/terrain pipeline and multi-view publication

ExtendedView2 provides a useful resource-decoding/presentation starting point, **not** permission to expand its stock `Entity[40]` arrays or two bank selectors:

- Its published submissions are double-buffered before original sprite upload and selected at VBlank with committed cameras; **each** buffer contains all forty entities and both original view submissions, not one buffer per view. The new owner should publish a complete immutable packet at its phase boundary, with explicit frame/phase/epoch and per-view level references, not borrow a half-written world during rendering.
- Borrow mode-owned views, shared level geometry and logical entity descriptions. Runtime piece decoding must retain normal/mirrored offsets, RLE/interleaved/raw formats, palette bank, priority, tile reuse, masks, submerged clipping, shadows and level-keyed falling-priority patches.
- Emit stable per-view instances in the authored **sprite-layer order**: draw-late flag, descending ground Y, then stable entity/piece order (generation-checked IDs replace original record order for new actors). VDP tile priority resolves sprite-versus-terrain separately; it must **not** reorder world sprites. An earlier obscured low-priority sprite still occludes a later high-priority sprite. Keep HUD/world/shadow/inventory layer ordering explicit, clip each view independently and use bounded reusable submission/sort storage. Required visual objects cannot disappear because the original 54 viewport slots, 80 sprites or scanline caps were exhausted.
- Separate **simulation animation/motion** from presentation resource residency. The original pipeline couples motion callbacks to prepared frames/tile budgets; factor that coupling before replacing the allocator. A missing GPU tile can defer a draw, never movement/contact/AI or progression.
- Default co-op/director composition is four **400×224 native-density** views on **800×448**. Unused quadrants are neutral status/map panels, not duplicated simulated players. The small layout primitive is already in `tje-four-player::view`; a compositor is not.
- Runtime hero palettes are per instance. Preserve transparent index zero and neutral/dark outline colors; variant hue rotation must not rewrite one global CRAM slot. Keep seat number/icon indicators for color-blind readability. The current palette primitive resolves the user's four gameplay banks, respecting each ROM piece's selected bank; it ships no extracted palette table.
- Shared camera is deliberately deferred: fixed quadrants are the cheap, deterministic default. Add same-level/safe-rectangle/hysteresis policy only after it can be shown not to alter activation/fog and not to require zoom/tether/teleport. No HBlank trick supplies four cameras.
- The wider stock presenter keeps activity fixed by its selected **width/profile**, not by physical window size: its Earthling/elevator X interest expands with the configured margin, so different selected widths are not claimed to simulate identically. The new co-op/world owner instead specifies a canonical hero-centered interest envelope independently of layout. Sidecar-driven world rendering is a second source adapter for a shared resource pipeline, not an implicit simulation-mode change when a window is resized.

The director's tactical camera/editor zoom can inspect authorized geometry but never activates dormant floors or broadens hero-centered simulation interest. Native/browser use the same deterministic packet construction; GPU/cache/backend differences stay outside authoritative state.

### S5 service boundary

`tje-mode-view` is the shared S5 consumer of `tje-simulation::{ActorId,
PublicationStamp}`. It installs no overrides and cannot admit a world profile.
`Publisher::publish` validates a complete four-view submission, then swaps its
preallocated immutable buffers. A packet freezes at most 1024 logical actors,
twelve pieces per actor, four level-keyed falling patches and four independent
HUD/inventory/map/speech packets. Duplicate actor slots (including different
generations), repeated epochs, non-publication phases and mismatched seat UI
reject without exposing staging. `invalidate` discards derivative publication
after restore; S6 republishes restored state without advancing another phase.

`WorldSource` borrows at most four ordered `LevelGeometry` records and device
pattern/palette data under the packet's exact frame/phase/epoch. Rendering a
different epoch rejects. No geometry or complete world is copied per redraw.
The compositor owns reusable 800×448 output, per-view terrain/sprite layers and a
4096-entry direct-mapped ROM piece cache (512 bytes per maximum-sized piece).
Cache collisions decode into the same slot rather than removing a draw object;
cache eviction and redraw cannot call simulation or consume RNG. Sprite
occupancy is resolved in the manifest's draw-late / descending-ground-Y /
stable-ID order before sprite-versus-terrain priority, with shadows behind world
bodies and UI above them. Every quadrant clips independently. Palette variants
use live CRAM words per instance, preserving neutral/outline/index-zero semantics
and adding seat-number indicators rather than rewriting global CRAM.

`animation::Program` imports bounded sequence/frame resources from the user's
ROM. `start` and `step` run only at simulation-animation admission; their saved
cursor records sequence index, remaining ticks, flip, held and started state.
They emit the first/next-frame motion callback exactly once, including legal
zero-period held frames, without any tile-cache/readiness/view predicate. S3/S4
retain typed movement/collision dispatch; an absent new callback is not a zero
velocity. Resource addresses remain derivative, never serialized callbacks.

Native `Input::local_controllers` / `backend::enhanced_inputs` and browser
`Inputs::pollLocal` / Rust `input::enhanced_inputs` are separate four-pad
boundaries. Native slots retain device UUIDs; browser enhanced routing reserves
browser identity plus index, neutralizes a disconnected source and requires
explicit reassignment to swap seats. Browser APIs cannot distinguish two
identical devices that change their exposed indices: users must explicitly
reassign them rather than guessing ownership. Extra bindings are default off,
and the existing faithful sampling/recording paths remain two-pad. Binding
preparation does not bypass S6's closed gameplay selector.

The private `view-proof` example captures runtime device resources from a
faithful recording, then renders explicitly authored dense/disjoint-ID and
four-overlay fixtures with the same compositor. Its images and frame/audio
observations must remain private. The fixture does not simulate extra heroes,
prove a transition, or satisfy S6's four-pad campaign evidence.


## 6. Determinism, saves, rewind, recordings and networking

- Every mutation has a fixed phase and stable iteration order: LevelId, entity ID, then recorded rotating seat priority where the rule calls for it. Same-tick pickup/part/purchase claims commit once. Life/health donation names both ends and conserves resources; elevator riders are an atomic mask/cohort, not pair bits.
- Activity is the fixed hero-centered gameplay envelope plus required transition/objective actors, independent of the physical window, selected layout, map overlay, director/editor camera, graphics cache and presentation cadence. Specify dormant timer policy in the ruleset; do not let draw visits advance it.
- RNG streams derive with versioned byte order/domain labels from session/level/purpose identity. Geometry, population, loot, AI and director commands do not consume each other's stream or the original idle-loop cache accidentally. Work-budget yields use deterministic counts, never elapsed milliseconds.
- Capture **only** through `EnhancedMode::snapshot`/`snapshot_into`; restore through `EnhancedMode::restore`. Format 3 already covers core device/CPU/timing/audio state, four hardware inputs, scheduler/continuations and every registered sidecar/config identity. Its complete image hash is the rewind/divergence unit. Never save only the seat struct or unwrap an enhanced image to a bare hardware hash.
- Immutable configs authenticate their digest against actual encoded values. Per-seat validation is not enough: reject bad roles, stale generations, impossible level references, over-budget pools, partial transitions or invalid course links before either half of restore mutates. Rebuild derivative graphics and refresh the active publication without running an extra simulation phase or replaying audio events.
- `tje-four-player::input::InputFrame` is a separate strict schema with frame ID, four existing session-order pad words and four action words; it is **not yet a complete enhanced recording/lobby event format**. Add versioned assignment/config/role/quantized director events when those frontends exist. Faithful `tje-session` remains exactly two-pad.
- Stable source IDs and recorded explicit reassignments determine seats; current poll order does not. Disconnect means neutral input, not transferring a hero. A/C swap is explicit and tested: wire directions/A/B/C/Start becomes gameplay directions/B/C/A/Start. Only pads zero/one reach original hardware.
- Current netplay stays **faithful-only**. A complete enhanced hash does not grant protocol compatibility. Enhanced networking must separately negotiate profile/body/rules/config/course/ROM identity, all seat/role owners, predictions/confirmations and disconnect policy. Four independent peers/topology/relay or hosted services require a later user decision; no transport changes, accounts or spending belong to this cutover.
- ROM-derived palettes/decoded tiles/maps/screenshots/PCM/snapshots remain private. Public fixtures are authored metadata/synthetic colors/opcodes only; never commit a generated map or a private contact sheet as a test golden.

## 7. Staged implementation and estimated size

Estimates below are **[INFERENCE] engineering days, not a measured schedule or commitment**. They include native/wasm source-side work but not user approvals for online topology, hosted sharing or new art. The functional slices overlap; do not add every original design's foundation estimates again. A stage can land gated primitives while admission remains closed; it cannot advertise an incomplete playable mode.

| Stage | Work / dependency | Observable exit | Estimated size |
| --- | --- | --- | --- |
| S0 — current foundation | Existing enhanced scheduler/sidecars/snapshot; shared bounded geometry/generation and co-op data/input/palette primitives; this plan | Default untouched; missing world closure rejects before mutation; primitive invariants/codec/rewind tests | Current deliveries; not charged again |
| S1 — typed services and complete manifest | Inventory transitive/callback/IRQ closure; typed actor/resource IDs; aggregate identity/state budget and cross-reference validation; bounded phase/setup adapter | A complete auditable ownership map and service contracts; no admitted gameplay yet | 5–8 days |
| S2 — geometry/persistence/load transitions | Generalize queries/collision/anchors/seeds/copies/land/path/discovery; level-ID persistence; bounded generator work; fall/elevator/phone/doorway/ship and special-floor state machines | Four independent level lifetimes retain edits/items/removals across every transition; deterministic loading cancellation/failure | 10–16 days |
| S3 — players, inventories and resource closure | Lift readable movement/status/damage/death/cooperation and all 28 present effects; effect ownership/quotas; atomic resource/ship/donation arbitration; four-seat UI intent | Each archetype in each seat has real effects/collision/damage/resources; no two-element callback fallthrough | 12–20 days |
| S4 — AI/population/activity closure | Generalize kind-specific thinkers, terrain, banks/contact/services/mole/Santa/decoys; stable-ID masks/reservations, fixed hero interest; density policies over shared world | Four same/disjoint-level populations simulate independently of renderer and capacity pressure; all original kinds remain valid | 7–11 days |
| S5 — host publication/render and local controls | Factor animation-motion from residency; shared ROM terrain/piece decoder/source adapters; bounded view packets/compositor; four HUD/inventory/map/speech overlays, palette instances and native/browser binding UI | Four full-density independent views beyond original hardware sprite limits, usable four local pads with reconnect; native/browser output available | 8–13 days |
| S6 — admitted playable local co-op | Integrate S1–S5 as ONE complete profile; full manifest admission, scripted enhanced recording/lobby events; first-level and then full-campaign determinism/resource/transition tests, balance/playability | Private four-pad runs through level transitions in both ROM revisions and native/browser; only then enable the session choice | 7–12 days |

Shared cutover envelope: approximately **49–80 engineering days [INFERENCE]** including the above work if done sequentially; parallelizable geometry/player/render slices shorten elapsed time only after their interfaces and shared commit owner are settled. This range deliberately reflects the broad ownership closure rather than calling a few overridden loops “four-player phase 1.” Admission is not lowered between stages.

LargerLevels supplied the underlying closure sizing: terrain/query/collision 4–6d; generation/load/bank migration 8–12d; live land/discovery 3–5d; population/masks 6–9d; player/UI blocking paths 5–8d; dynamic map/progression scaling 3–5d; renderer/camera source adaptation 5–8d; elevator/fall/ship/special scenes 4–6d. These **overlap S1–S6** and are not additive. Expanded random generation and density tuning can be developed against the shared services without waiting for four-player balance; larger two-player gameplay still waits for every consumer of its new geometry/populations to be owned. Maker's validated DesignedV1 data replaces generation, not the simulation services. The existing wider stock view does not need to wait for this cutover.

### Overlord after the shared closure

Overlord requires S1–S6's generalized hero/AI/terrain/resource/UI infrastructure, not four-pad input alone. Add a role manifest with heroes 0–2 and director 3 (no fourth hero body); ordered finite commands; integer energy/timer remainders; generation-checked possession with safe decision-boundary AI handoff; bounded validated spawns and non-topology-changing telegraphed traps; director view/UI; explicit objective/death/exit arbitration and snapshots. The director camera must not activate the world or reveal unauthorized hero state. Initial possession is limited to the capability list in [overlord.md](overlord.md), not every generic thinker.

Its original O-A..O-D estimate is a further **15–25 days [INFERENCE] after the shared closure**, including adversarial determinism and short/full-campaign playtests, with no four-peer transport implied. Maker editor/browser/share UX and the expanded generator's own corpus/tuning remain separately scoped work over the same services. New art, shared-camera churn, four-independent-peer networking and hosted sharing are not prerequisites for local four-player play and are not silently added to these stages.

## 8. Admission and release evidence

Before a gameplay selector is admitted:

1. The full static/callback/IRQ closure has no unowned actor/map/UI consumer; unknown resources/unsupported waits/configs reject transactionally.
2. Existing REV 00/REV 02 faithful native/browser recordings have unchanged frame **and audio** hashes whenever engine/frontend paths changed. Default config never constructs an enhanced gameplay profile. Existing faithful two-player netplay remains admitted and extras remain rejected there.
3. Four simultaneous distinct inputs, keyboard focus, explicit swaps/reconnect, independent overlays/choices, all-seat pause, role ownership and wire conversion are tested in actual frontends, not only a router unit test.
4. Both archetypes in every seat exercise all 28 presents/statuses; damage/death/donation/falling, same/different floors, phones/services/mole/Santa, elevators, part/ship objectives and return visits preserve resources with deterministic arbitration.
5. Snapshot/rewind/restore/resimulation covers every phase/loading/effect/transition/command state. Same native/wasm manifest+inputs yields identical authoritative/core/audio results; layout/frame skipping/cache eviction does not alter authoritative state.
6. Dense overlapping and disjoint four-view scenes exceed original sprite limits without disappearing heroes/required actors, changing collision winners or allocating per-frame full-world copies. Test worst-case persistent/population/effect capacities and ID generations beyond the original mask widths.
7. A private enhanced-input four-pad script completes the first levels and real level transitions in **both revisions, natively and in the browser**; then exercise full campaign objectives/special scenes before claiming campaign completeness. Keep screenshots, saves and audio private and label evidence narrowly. A palette contact sheet or bookkeeping test is not a four-player session.
8. Run the repository's fmt, default/enhanced clippy/tests, both wasm builds, function-trace check, tools unittests and public audit after integration. Report exact exercised evidence, known gaps and remaining capability admission, not inferred playability.

Current fail-closed primitives intentionally do not satisfy items 3–7's gameplay acceptance. No private four-player gameplay screenshot or level-transition run is claimed by this foundation delivery.

## S4 service increment: AI and persistent populations

`tje-simulation::population` owns the generalized, typed population services. It
does **not** install a gameplay profile or weaken the S6 admission gate.

- Floors are ordered by `LevelId`; up to four retained hero floors share one
  persistent population each. `PopulationId { level, local }` survives release,
  removal and collection. Stable failure/removal/collection sets cover all 64
  identities, not an original 32-bit bank mask. Logical body handles occupy the
  agreed generation-checked actor domain 32..352; draw instances are not actors.
- Ordinary starting count remains twenty. The shared walkable-area density
  policy is explicit and independent of hero/view count. Per-floor capacity is
  distinct from initial count; negotiating a denser profile requires its actual
  body/companion reservation. Whole activity changes preflight capacity before
  mutation; exhaustion never evicts a required actor, relocates a creature or
  changes population composition to make a draw fit.
- Canonical interest is inclusive ±480 X and ±336 Y world pixels around each
  same-floor hero, plus required actors on hero-active floors. Dormant AI and
  animation timers pause. No camera, window size, draw visit or graphics cache
  participates in activation, RNG, collision or allocation.
- All 25 original kinds import from the user's cartridge. Movement, thinker and
  touch pointers become finite typed dispatch; unknown callbacks reject.
  Definition parameters, animation descriptors and projectile aiming vectors
  are checked runtime resources, not public table copies or serialized pointers.
  Generic and horizontal thinkers, protected attacks, terrain restrictions,
  truck transitions, Cupid/chicken projectiles, mole theft latches, Santa and
  per-customer service continuations use explicit four-seat eligibility.
- Targeting considers eligible decoys before heroes; equal-distance candidates
  prefer the higher seat, extending the original two-seat tie. Contacts rotate a
  saved seat priority and re-read hero eligibility after each transaction.
  Inventory/map overlays are not an AI pause or damage-immunity input.
- `step` admits alternating-record AI/contact phases. `advance_motion` integrates
  all active bodies' continuous axes/elevation; `advance_animation` applies
  delegated authored motion only at logical frame boundaries. All three have
  separate saved duplicate-admission stamps. Animation sequence indices and
  resource-frame indices are distinct, and held/zero-period animations need no
  renderer residency.
- `ContactSink` is the aggregate's real player/economy/effect transaction adapter.
  It accepts or rejects hurt/squish/status, mole capture/theft/release, Santa
  drops, objectives, paid-service stages and fully specified projectile launches.
  Rejection must not partially charge or mutate a hero. Lemonade/hot-tub
  contacts invoke S3's automatic hero-state services rather than inventing paid
  menus. `Environment` borrows checked geometry/resources and S5's logical
  animation step. No host ID or pointer enters an original object/player ABI.
- `resolve` and `service` accept current body generations only; `known_body`
  recognizes previously issued generations for effects that outlive their
  source's activity. Aggregate capture/restore uses `validate_world` together
  with the session's player/effect/service reference checks.

`population::stock::prepare_stock_specials` supplies OriginalV1 item generation
with the seeded first-two-record mailbox/ship prefix and its original strict
340×200 large-item exclusion predicate. It shares S2's exact `OriginalRng` and
`TerrainValidation` footprint search, preserves mailbox mode draw order and
elevator placement exclusions, and does not confuse the later ambient ordinary
population RNG with the seeded item generator. Authored courses still supply
explicit records, not stock population injection.

Tests exercise same-floor and four-disjoint-floor lifetimes, every kind and
typed thinker/contact family, decoy ties and eligibility, contacts and services
for all seats, protected states, sleeping, terrain, motion/animation admission,
stable IDs beyond 31, snapshot/resimulation, bounded spawning and atomic pool
pressure, and four 64-actor floors in the negotiated body domain. The optional
private test imports every kind and its initial animation in both retail
revisions without publishing asset data. These are service/resource checks,
not a four-pad campaign recording, native/browser parity proof or balance claim.
OriginalV1 full-generator RAM parity and S6 frontend/campaign evidence remain
their respective owners' admission work. No engine/frontend selection or
faithful gameplay path changes in this service increment.

