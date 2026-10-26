# Overlord, three heroes versus one director — gated services and gameplay design

Status: **default-off, fail-closed service increment; no admitted playable mode**, 2026-10-25. `tje-overlord` supplies typed director rules over the shared simulation. The S6 profile, aggregate format-3 integration, real native/browser controls and campaign evidence remain admission prerequisites. This is not four entries being added to original RAM. Faithful play and ordinary co-op are unaffected. No hosted service, TURN relay, new character art or spending is included.

`G` is the [matching decompilation](https://github.com/mschienbein/toejam-earl-decomp), researched at commit `57fc83fc54781af8e992bbc3b32afe788c1dbd01`; `G/...` paths are relative to it, and `P/...` paths are relative to this repository. REV 02 addresses come from `G/config/rev02/names.json`.

## What the original provides

| Ref | G evidence | Use / restriction |
| --- | --- | --- |
| O1 | [G `include/earthlings.h:6–19,40–84`](../../../toejam-earl-decomp/include/earthlings.h#L6), [G `include/ram_earthlings.h:12–37`](../../../toejam-earl-decomp/include/ram_earthlings.h#L12): two twenty-record banks, eighteen-byte persistent records; 38-byte ROM definition with state animations, update/ability/touch pointers, dimensions, sprite/tile counts, contact damage and timing ranges; per-level 32-bit removal/failure masks. | Do not treat an Earthling as a player object. Separate persistent actor, behavior definition and instantiated render object. The original bank at REV 02 `0xFFDE72` is not space for an extra player or traps. |
| O2 | [G `src/earthlings/spawning_and_banks.c:100–175,190–269`](../../../toejam-earl-decomp/src/earthlings/spawning_and_banks.c#L100): `FindSpawnLocationForEarthling` (`0x0177CC`) tries ten random cells then a cyclic full-map scan, validates terrain/footprints and overlap; `EarthlingRetainLevelBank` (`0x017AC8`) uses level seeds, choices and special ship/mailbox insertion. | Original population is level-wide, **not** camera-relative. Director placement must use a new bounded validator, preserving terrain/clearance rules and reserving objectives. |
| O3 | [G `spawning_and_banks.c:426–473,579–640,871–885`](../../../toejam-earl-decomp/src/earthlings/spawning_and_banks.c#L426): `LoadInEarthlings` (`0x01832A`) activates by two pending cameras, rectangle X (-160,480), Y (-112,336), alternating slot parity; `EarthlingDispatchAbilities` (`0x018834`) invokes the type ability, `EarthlingDispatchPlayerContacts` (`0x0188EE`) tests two heroes. | A director's roaming camera must not activate the whole map. Keep camera visibility, authoritative activity, possession and contacts distinct. |
| O4 | [G `src/earthlings/generic_enemy.c:20–67,71–167`](../../../toejam-earl-decomp/src/earthlings/generic_enemy.c#L20): `EarthlingProcessSpecialThinkStates` owns kind-specific protected attack states; `AI_GenericEnemyThink` selects target/turn/cardinal movement and animation states, obeys timers and blocked motion. | Possession replaces decision input at a safe state boundary; it must not skip attack animation, sleep, damage or collision. Do not call the generic thinker and director movement in the same decision slot. |
| O5 | [G `src/earthlings/terrain_movement.c:19–119,139–185`](../../../toejam-earl-decomp/src/earthlings/terrain_movement.c#L19): movement uses frame motion multipliers, 24.8 position, kind-specific terrain, collision/decoy handling, two-bank lookup and visibility/menu gates; original bounds are hard-coded. | Reuse generalized readable terrain/collision logic, not direct position writes. Original callbacks cannot accept a host pointer or a third hero's index. |
| O6 | [G `src/earthlings/targeting.c:14–137`](../../../toejam-earl-decomp/src/earthlings/targeting.c#L14): two decoys precede two heroes, eligibility/elevator checks and kind-specific steering. [G `src/player/earthling_damage_reactions.c:7–17,38–119,151–172`](../../../toejam-earl-decomp/src/player/earthling_damage_reactions.c#L7): effect eligibility, hurt/knockback, deferred type-defined hurt damage versus direct squish damage. | Extend contacts to exactly the three hero seats. Preserve response/invulnerability semantics and type damage rather than granting the director arbitrary HP subtraction. |
| O7 | [G `include/earthling_types.h:6–32`](../../../toejam-earl-decomp/include/earthling_types.h#L6): 25 kinds including ship piece, services, hazards and hostile variants. [G `docs/subsystems/earthlings.md:204–263`](../../../toejam-earl-decomp/docs/subsystems/earthlings.md#L204): friendly interactions, life requests, sleep/wake, damage immunity, Santa and present-created Earthlings. | Ship pieces and friendly/service actors are never director-spawnable/possessable. Kind inclusion is an explicit capability list, not `kind != ship piece`. |
| O8 | [G `src/map/level_generation_and_changes.c:418–430,536–674`](../../../toejam-earl-decomp/src/map/level_generation_and_changes.c#L418), [G `include/ram_map.h:6–31`](../../../toejam-earl-decomp/include/ram_map.h#L6): seeded generation, two active maps plus a saved bank, 19×26 word cells, 26 seeds, shared path mutation. [G `src/core/startup_and_main_loop.c:677–705`](../../../toejam-earl-decomp/src/core/startup_and_main_loop.c#L677): multiple update phases per video cadence. | Director RNG and placement cannot consume original generation RNG or mutate path cells outside the authoritative mutation phase. A seed alone is not the whole simulation state. |
| O9 | [G `include/entities.h:7–24,60–107`](../../../toejam-earl-decomp/include/entities.h#L7), [G `include/ram_sprites.h:6–43,56–79`](../../../toejam-earl-decomp/include/ram_sprites.h#L6): two hero tables, forty 128-byte objects, twenty dynamic gameplay slots, two view masks, 80 hardware sprites, 54 viewport slots, 640 sprite-pool tiles and nominal 112 tiles/frame. | New enemies/traps must use the enhanced arena and host renderer. Do not evict a ship piece to admit a director effect or confuse visual residency with alive/spawn counts. |

The original has no recovered Overlord command/budget/trap system. Traps below are newly authored simulation rules and simple geometric markers, not a claim to have found hidden ROM trap assets. Work RAM is 64 KiB, VRAM 64 KiB and CRAM four sixteen-color banks ([G `src/asm/interrupts.s:10–25`](../../../toejam-earl-decomp/src/asm/interrupts.s#L10), [G `src/asm/vdp.s:6–49,150–166`](../../../toejam-earl-decomp/src/asm/vdp.s#L6)). H40 scanline caps are twenty entries/forty sprite tiles, from [P vendored VDP `registers.rs:72–104`](vendor/jgenesis/backend/genesis-components/src/vdp/registers.rs#L72), not G. More host views must not be forced into those budgets or four global CRAM slots.

## Match and controls

- Seats 0–2 are heroes; seat 3 is the Overlord, with **no fourth hero body**, inventory or life-donation eligibility. Role/seat assignments are fixed in the lobby and recorded. A later rematch can rotate the director without transferring live actor state.
- Presets use the same rules engine: a short three-floor course with specified objectives, or the original campaign's ship-part/final-destination objective. Course manifest names required collectibles/exit. Heroes win when their required objectives and exit condition commit; director wins when no hero can respawn/donate under the rules, or a predeclared optional tick limit expires. Default short preset has a fifteen-minute simulation clock; the full campaign has no timer. No unwinnable hidden time limit.
- No mid-round dynamic difficulty that reads human success. Fixed ruleset parameters, level seed and initial resources are in the manifest. Death, possession release and objective collection do not refund budget unless explicitly specified.
- Default local rendering is a 2×2 800×448 logical canvas: three 400×224 hero views and one director panel. The director panel toggles tactical map / a 400×224 possessed-body view. Native/browser each receive only local-owned view packets; the renderer need not change world activity to draw a tactical panel. A remote director can see allowed geometry, not hidden inventory identities or unrevealed hero UI state. Local shared-screen players can naturally see other quadrants; do not promise secrecy on one display.
- All four modern pads use stable seat bindings as in the co-op design. Director directions move a tile-snapped cursor; A confirms/selects, B cancels/releases, C cycles the palette/tool, Start requests cooperative pause. A modern secondary stick/shoulder shortcut is optional. All operations also work with one three-button logical pad. Pointer actions quantize to world tile coordinates before being recorded.
- Commands are finite tagged data: `SelectActor(id)`, `Possess(id)`, `Release`, `Move(direction)`, `UseAbility`, `PlaceSpawn(kind, level, x, y)`, `PlaceTrap(type, level, x, y)`, `CancelPlacement`. No arbitrary callback, RAM address, script or damage payload crosses input/network boundaries.

## Authoritative state and ordering

Use the co-op sidecar's `PlayerId`, generation-checked `EntityId`, `LevelId`, level mutation queue and frame/phase clock. Add a bounded `OverlordState`:

- Ruleset/version, hero-seat mask, director seat, match clock/result/objective progress and deterministic tie policy.
- Integer energy balance plus regeneration remainder; budget cooldowns/caps; cursor/tool/level; possessed entity ID and ability gate; command sequence number and last committed input frame.
- At most six director-spawned alive hostiles and eight active traps globally (three traps per level). Track owner/source/expiry independently of renderer instances and original failure/removal masks. A director spawn cannot reuse a hero present-created actor's ID or erase a stock enemy removal bit.
- Trap records: stable ID, type, level/position, arming/lifetime timer, hero-trigger mask, owner and current state (`telegraph`, `armed`, `triggered`, `expired`). Pending placement is not an active trap and consumes no world capacity.
- Separate director RNG seeded from session/ruleset/level identity; all derived random choices recorded through state, not host time. No direct draw from G's `gRNGState` or shared 59-word cache.

Commands are applied in frame/sequence order at the phase boundary **before** AI decisions; validate role, entity generation/state, placement and energy against one before-state. Commit cost + ID allocation + pending actor atomically, otherwise reject with an unchanged state and a local reason. Then perform director/AI decisions, movement, trap/contact resolution, objective/death resolution and result commit. At a tie where an exit and terminal hero death are proposed in one phase, the already validated exit/objective transaction wins; log the fixed order in the ruleset. Repeated presentation, rollback prediction correction or duplicate packets cannot charge energy twice.

Snapshot budget [INFERENCE]: less than 16 KiB additional authoritative state beyond co-op, with fixed arrays/reused validation scratch. Renderer/UI caches are derivative. Director input, pending commands, timer remainders, spawn ownership and result state must hash/restore with the hero/world state.

## Possession and safe placement

### Possession

First supported capability set: Devil, Dentist (including its existing variant), Lawnmower and Nerd Herd. They cover ordinary chase/contact/terrain-restricted behavior without promising controls for every special actor. Other existing hostile kinds remain AI-driven in the world. Exposing a new kind requires its explicit ability/state/terrain adapter and acceptance tests; the design does not infer identical controls from the generic definition pointer.

One possessed body at a time. Directional intent chooses the actor's existing legal locomotion states/speeds; an ability button requests its existing legal attack transition, not an immediate hit. Protected attack/sleep/stun/death states complete normally. The unpossessed AI continues running other actors; possession skips **only** that body's target/wander decision at the same decision boundary. Terrain, motion callbacks, contact range, decoys, hurt, sleep and hazards remain authoritative generalized algorithms. Release/death/out-of-interest changes restore AI safely with a legal state/timer; never restore an old position or health.

Possession cannot target ship pieces, mailboxes, wizard/wiseman/opera service interactions, Santa, level-zero services, sleeping/dying/hidden transitional bodies, heroes, or another owner. The body must be on a hero-active level and within the fixed gameplay-interest envelope; the director cannot carry it around a dormant floor and activate an army. If all heroes leave its level, release possession and retain the body's persistent world state.

### Placement

Reuse generalized `TerrainValidation`, Earthling footprint/radius and elevator overlap logic from O2/O5. Validate bounded coordinates, allowed terrain, collision, original population and extra-actor caps, same-level hero activity and objective/service safe zones. Iterate a bounded candidate set for auto-placement; cursor placement fails clearly rather than spinning or silently moving the trap elsewhere. Original present-created Earthling exhaustion paths include out-of-bank bugs ([G `docs/subsystems/earthlings.md:265–269`](../../../toejam-earl-decomp/docs/subsystems/earthlings.md#L265)); never reuse those writes as a capacity policy.

Use fixed hero-centered interest and spawn exclusion rectangles, independent of physical window, shared/quadrant layout or director camera. Proposed canonical visible exclusion is each hero's 400×224 box plus a 32-pixel margin; allow spawns only outside their union but within a 640-pixel placement radius of a hero. Require a terrain-reachable approach and preserve a 128-pixel safe zone around arrival/exit/required objectives. A spawn telegraphs for sixty video ticks before contact/AI activation. It cannot appear already overlapping an unseen hero, a landing site or a sleeping hero. Revalidate at activation; if it becomes illegal, cancel and refund the unactivated spawn once.

Traps may be placed on visible ground but telegraph for sixty ticks, cannot arm under a hero, and never change base terrain/connectivity. Proposed newly authored first tools:

- **Snare pad:** radius 24 pixels, slows ordinary grounded movement to 3/4 for 120 video ticks after one valid trigger; no stacking, HP damage, attack immunity or effect on forced/falling/elevator motion.
- **Bump pad:** one telegraphed twelve-pixel displacement from its center, only if the destination is legal supported ground and outside elevator/objective safe zones. It cannot directly push into void/water or bypass the existing hurt/effect eligibility rules. No damage and no recursive re-trigger in the same phase.

Both expire after 600 video ticks or the first successful trigger; a rejected trigger does not repeatedly damage/freeze a hero. Markers are newly authored circles/arrows in the host UI layer; no extracted trap graphics are needed. More lethal terrain-changing traps are a separately balanced future tool, not silently enabled by this proposal.

## Starting balance sheet

Proposed short-match baseline, to be tuned by playtests rather than claimed balanced:

| Lever | Initial value / invariant |
| --- | --- |
| Energy | 40 starting, 100 cap, +1 per 60 video ticks; integer remainder saved. |
| Spawn | Devil 20, Dentist/variant 25, Lawnmower 30, Nerd Herd 30; fifteen-second global spawn cooldown; six live director spawns maximum. |
| Possession | Ten energy on entry, one per second held; five-second entry cooldown; cannot reset an attack timer by release/repossess. |
| Traps | Snare 10, bump 15; five-second placement cooldown; eight global/three-per-level active cap. |
| Hero fairness | No HP/damage boosts for possessed bodies; ordinary hero present effects still work; arrival/respawn safety and nonblocking menus follow the enhanced co-op rules. |
| Objective safety | No spawning/destruction/replacement of required parts, exits, services or inventory; no permanent topology edits. |

Cap counts include telegraphing actors so placement cannot oversubscribe a pool before activation. Lifetimes and cooldowns use the authoritative video tick, not wall time or AI-callback count. Fast-forward advances both teams equally; pause stops both. Match presets may change costs/population/hero resources/time limit, but all peers/replays must agree on a digest before starting. Full campaign pacing needs different regeneration/caps, exposed as a named preset rather than unrecorded scaling.

## Enhanced override plan and dependencies

Use EnhancedMode's separate opt-in `EnhancedProfile(name, &[Translation], &[FramedTranslation])` and existing `Context` physical-call ABI, not the faithful dispatcher table. Selected nested calls reenter the registry; unselected routines run compiled originals. Current Atomic/Functional callbacks use synthetic RTS charging and supply **no** sidecar or timing proof. The opt-in `NativeCompiledOriginalCallees` path in [P `docs/ENGINE.md:437–506`](docs/ENGINE.md#L437) is the precedent; its proof authority does not transfer to altered gameplay.

Build on the complete co-op closure, then add:

1. `ProcessInput` (`0x000A6E`) role dispatch and enhanced command latch; hero actions use the three-seat mask, director actions never call `SpriteChecks` or inventory code as player 3.
2. `ProcessEarthlings` (`0x019110`), `EarthlingDispatchAbilities` and the generic/type-specific decision boundaries (`AI_GenericEnemyThink`, `EarthlingProcessSpecialThinkStates`): controlled decision path for exactly the possessed actor, preserving protected state transitions.
3. `EarthlingRetainLevelBank`, `FindSpawnLocationForEarthling`, `LoadInEarthlings` and allocator/release/damage paths: persistent director ownership, separate caps, fixed hero interest and no capacity-dependent progression loss.
4. Generalized `EarthlingDispatchPlayerContacts`, `PlayerCanReceiveEarthlingEffect`, `Player_Youch`/`Player_Squish`, `ProcessMovement` and effect handlers: three heroes, original type contact semantics, newly authored trap effect transactions. Do not fabricate a ROM Earthling record/kind just to invoke a trap.
5. `ProcessPlayers`, level/elevator/ship collection and death/life-request boundaries: round result/progress arbitration and no director donation/respawn body.
6. Co-op host view/HUD/map packet boundaries: director tactical view, possession indicator, telegraphs and energy display. No `HandleHBlank` four-way trick and no director-camera-based activation.

Requires the future typed sidecar/context/phase bridge and host entity rendering from co-op; no global state or guessed RAM padding. Core snapshot format 1 has no enhanced fields: wrap the `TJESTATE` core state with profile/ruleset/body-manifest/config/course digest and complete sidecar, restore transactionally, and retain private ROM-derivative restrictions. New enhanced recordings include all four seat inputs and quantized commands. Current two-seat rollback cannot host this until generalized seat ownership/sidecar hashing/packet validation is implemented. Local/peer-to-peer only; no hosted uploads/matchmaking.

Hero variants reuse co-op's runtime ROM-derived palette transforms and per-instance host palettes; director selection/telegraphs use newly authored geometric UI. Four CRAM banks do not constrain the number of host palette instances, but faithful VDP behavior remains unchanged when the mode is off.

## Tests, risks and implementation sizes

Estimates [INFERENCE], engineering days **after** the shared co-op foundation and full three/four-seat world closure; those costs are not counted again here.

| Phase | Observable exit | Estimate |
| --- | --- | --- |
| O-A | Role manifest/commands, integer budget/result machine, safe-zone/placement validator and snapshots | 3–5 days |
| O-B | Four supported-kind possession adapters, AI handoff, sleep/stun/death/terrain/decoy behavior | 4–7 days |
| O-C | Both trap tools, telegraphs, director map/body UI and native/browser controls | 4–6 days |
| O-D | Adversarial determinism, complete short/full-campaign progression and balance playtests | 4–7 days |

Required tests:

- Existing REV 00/REV 02 faithful native/browser session frame **and audio** hashes unchanged with extra absent/off, and regular enhanced co-op unchanged without this ruleset.
- Table-driven each command/state/cost/cap; no role escalation, negative energy, stale entity generation, duplicate cost, cap bypass by pending telegraphs, malformed coordinate or extra director hero. One failed placement leaves no partial IDs/cost/map change.
- Every supported hostile kind, state boundary and facing through possession/release, collisions, attack cooldown, hurt/sleep, hero decoys and death. A pressed ability cannot skip animation/cooldown; another body's AI is unaffected. Unsupported kinds are rejected before mutation.
- All three heroes on separate levels; director camera pans over dormant levels; level activation/RNG/contact does not change. Heroes crossing a telegraph, arriving/respawning, riding/falling or standing in a menu cannot be unfairly trapped at activation.
- Trap nonstacking and lifecycle, supported-destination checks, exact finite lifetime/cooldowns, simultaneous triggers in deterministic seat order, and preservation of routes/required objectives under maximum tool use.
- Exit versus final death/time expiry, part collection, donation exhaustion and full campaign result. No impossible terminal state with an available donated life or a destroyed exit.
- Native/wasm same input manifest yields identical sidecar/core/audio hashes; snapshot/restore every lifecycle state; rollback/resimulation does not duplicate spawns, costs, refunds, telegraphs or sound events. Renderer refresh rate/layout cannot affect outcomes.
- Dense four-view scene beyond original VDP sprite limits; capped pools return defined failures, required heroes/ship/elevator resources remain available. Public audit finds no ROM palette/graphics/audio/text tables in documents, tools, fixtures or share payload examples.

Risks: possession adapters need kind-specific semantics; shared-display information cannot be secret; menu policy changes original pause behavior; controller-driven tactical placement must be usable; fun/balance cannot be established from source inspection. User choices worth a later playtest are harmless versus lethal trap tools, short-match versus full-campaign default, and any four-peer online work. Recommended defaults above let implementation proceed without inventing hosted infrastructure.

## Gated director service boundary

`tje-overlord` reuses `tje-simulation::session::Roles`, `ActorId`,
`population::PopulationState`, `tje-world` geometry/footprints and the S3
player geometry/motion services. It installs no engine override or frontend
selector. `request_gameplay` always rejects; a partial mode cannot fall back to
co-op or create an invisible fourth hero.

- The versioned configuration authenticates the shared session identity,
  three-body/four-input role manifest, distinct ownership mask, director seed,
  named short-course/campaign objective and exit policy. The full campaign has
  no hidden timer; the three-floor preset declares its fifteen-minute clock.
- A fixed 32-command queue accepts only ordered finite tagged data from the
  director owner. Identical queued duplicates do not reapply; committed
  sequences cannot charge twice. Command receipts are local UI data. Rejection
  consumes its ordered envelope but does not spend energy, allocate IDs, draw
  RNG or partially mutate the world.
- Integer video-tick counters, regeneration/possession remainders, input edges,
  command admission, pause vote, tool/cursor/view choice, all six spawn receipts,
  eight trap lifecycles, snare timers, objective progress and terminal result
  derive deterministic codecs. Their immutable configuration digest must match
  the shared aggregate before capture/restore. These codecs are not a separate
  save format: production capture remains the complete format-3 S6 image.
- The shared director population service checks exact body generations and
  excludes companions/unsupported kinds. Controlled input replaces only the
  expired generic decision slot after protected-state/blocked-motion gates;
  ordinary movement, terrain, decoys, contacts, sleep and damage remain shared.
  Dentist/variant has the existing directional attack animation. Devil,
  Lawnmower and Nerd Herd have passive type-defined contact attacks, not a new
  manual damage ability; unsupported ability requests reject.
- A spawn reserves a real hidden, inert body before charging. It revalidates
  the footprint, fixed hero exclusion, reachable approach and 128-pixel
  objective/service/arrival zones after sixty video ticks. Illegal unactivated
  reservations cancel/refund once. Activated death/release never refunds.
  Six live/pending reservations are global. The shared finite persistent
  identity budget is at most 64 total declared records per floor across stock,
  original holes, presents and director spawns over that floor's lifetime.
  Exhaustion rejects; it never recycles a stock/present identity or clears an
  original removal bit. This bound is explicit, not an unbounded tombstone list.
- Snare and bump markers never mutate terrain. Arming cannot occur under any
  hero; trigger eligibility additionally protects menus, arrival/respawn,
  airborne/forced/service/elevator states and shared effect immunity. Snare
  timers do not stack; shared voluntary motion alone is scaled by 3/4. Bumps
  commit only through the shared supported-ground/collision transaction.
- The tactical publication contains only authorized geometry, supplied public
  hero position markers, director budgets/tools and geometric telegraphs. Its
  type has no hero inventory/health/status UI. Rendering cannot retain or
  activate a floor, advance animation, mutate the world or draw RNG. Possessed
  body composition uses S5 rather than a second actor renderer.
- Shared objective/exit transactions arbitrate before terminal death and clock
  expiry, so a validated winning exit wins the same-phase tie. Seat three never
  enters S3's hero inventory, donation, respawn or life-exhaustion enumeration.

The ROM-free `director-proof` example and `tests/wasm-parity.mjs` compare the
same authored authoritative fixture on native and wasm. They are not faithful
hash receipts, a core/audio parity proof, an original-generator receipt, a
four-pad frontend run or a balance playtest. Synthetic fixtures and codec
round-trips must not be presented as completion of O-D.

Before admission, S6 must integrate the ordered command/decision/motion/contact/
result boundaries, aggregate reference validation and complete enhanced
snapshot; version the existing recording for role/quantized command events;
connect native/browser binding, selection and possessed-body publication; then
exercise private short campaigns and full progression in both revisions on
both backends. The complete section-8 evidence in
[mode-owned-simulation.md](mode-owned-simulation.md#8-admission-and-release-evidence)
still applies. No session choice opens on service tests alone.
