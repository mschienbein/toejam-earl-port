# Controller sessions and differential replay

`tje-session` records **inputs, never machine snapshots**. Sessions in [`sessions/`](../sessions/)
are safe to publish: their contents are controller state and descriptive metadata only.
The ROM remains a user-supplied private input. Reports contain execution counts and
content observations; do not publish RAM/VRAM, pixels, PCM, or runtime ROM resources.

## Format 1

A `.tje.json` session has these fields:

- `format`: `1`.
- `revision`: `rev02` or `rev00`; a mismatching cartridge is rejected.
- `start`: `{ "kind": "power_on" }` for every initial committed session.
- `frames`: exact total completed video frames, not the resettable `gFrameCounter`.
- `tool`: writer version (`tje-session/0.2.0`; initial recordings used `0.1.0`).
- `description`: human-readable provenance/purpose.
- `runs`: `{ "frames": duration, "pads": [pad1, pad2] }` records, in order from power-on frame zero.

Pad bits 0 through 7 mean **Up, Down, Left, Right, A, B, C, Start**, respectively.
Bit 8 means connected; zero means unplugged. Pads are always three-button pads.
These are session bits, **not** the game's different A/B/C bit order in work RAM.
Each frame uses one state for both physical controller ports. Durations must be
positive; adjacent equal states must be combined; their checked sum must equal the
header count. Unknown fields, invalid masks, disconnected pads with pressed buttons,
unsupported versions, and overflowing counts are errors. The library also builds
without the optional emulator/CLI dependencies, including for WebAssembly.

An explicitly authored deeper start is a `recipe`, not a snapshot:

```json
{"kind":"recipe","description":"Explain the experiment and every intervention",
 "writes":[{"before_frame":1200,"address":16744448,"width":2,"value":1,
            "reason":"Explain this specific synthetic intervention"}]}
```

Its inputs still begin at power-on. Writes apply **before** their named video
frame on every runner, including both sides of a comparison. Only canonical work
RAM, widths 1/2/4, fitting values, even wider addresses, documented reasons, sorted
frame order and frames inside the recording are accepted. The example is schema
illustration, not a game setup. No initial session uses any RAM write. Recipes
must document preconditions and cannot be described as unmodified gameplay.

## Native recorder

**F9** (configurable as `hotkeys.record`) starts/stops recording. Starting performs
a hard power-on reset, so a recording begun during play does not silently depend
on unrecorded history. The reset reconstructs the console with the same cartridge.
Paused host scheduler ticks are not emulated frames and are not recorded.

Stopping writes a compact, uniquely named `.tje.json` to `sessions/` under the OS
user-data directory printed at startup (`--data-dir` overrides it). Reset,
cartridge import and normal application exit also stop/save. Starting without a
cartridge prints a diagnostic; stopping an empty recording creates no file.
Only the final bound physical pad states passed to the backend are recorded:
keyboard, injected keyboard scripts and gamepads all take the same path.

## Replay and coverage

From the repository root, build once:

```sh
export CARGO_TARGET_DIR="$PWD/scratch/session-target"
cargo build -p tje-session --features runner --release
SESSION_TOOL="$CARGO_TARGET_DIR/release/tje-session"
"$SESSION_TOOL" replay --rom roms/rev02.bin \
  --session sessions/rev02-one-player.tje.json sessions/rev02-two-player.tje.json \
  --mode reference --report scratch/rev02-coverage.json
"$SESSION_TOOL" replay --rom roms/rev02.bin \
  --session sessions/rev02-one-player.tje.json \
  --mode check --report scratch/rev02-check.json
"$SESSION_TOOL" replay --rom roms/rev02.bin \
  --session sessions/rev02-one-player.tje.json \
  --mode replace --report scratch/rev02-replace.json
"$SESSION_TOOL" union --reports scratch/rev02-coverage.json scratch/rev00-coverage.json \
  --out scratch/coverage-union.json
```

By default, check/replacement use `HookRegistry::translated(revision)`, not a duplicate
registry. Untranslated functions continue in the original. Check mode is fail-closed
and reports the host's exact function error; it is not replacement mode with
warnings suppressed. Coverage should be taken from **reference** (or check, whose
live CPU still executes the original), never presented as reference coverage when
measured in replacement mode.

`--function func_00059e` (repeatable, also accepting catalogue names) narrows the
checked/replaced roots through the registry's existing root filter. It never
changes the production callee availability. An explicit `--pending --mode check
--function …` instead uses the owners' canonical pending value/framed bodies as
strict proof roots, composed with production and only those selected candidates.
Pending selection requires explicit roots, rejects missing/ambiguous bodies, and
cannot enable replacement. It changes no game status or production table.

### Modular replay-environment campaigns

The `replayed-environment` Cargo feature binds the frozen
`HookRegistry::proof_replayed_environment(revision, pending, framed, functions)`
and `Host::finalize_replayed_campaign()` API and enables Host's
`reference-proof-hooks` actual-read journal capability. This is check-only
analysis; default/shipping runner graphs leave that capability off. A binary
without the feature reports unavailable and cannot silently fall back to strict
checks or approve zero mismatches.

The durable corpus driver builds from a clean committed source tree, records the
actual binary SHA-256 and source/body-availability identities, then runs every
committed `sessions/*.tje.json` in both revisions:

```sh
. /home/michael/.local/share/tje-dev/env.sh
uv run python tools/replay_campaign.py build \
  --target-dir "$PWD/scratch/replay-target" \
  --out "$PWD/scratch/replay-build.json"
uv run python tools/replay_campaign.py run \
  --build-receipt "$PWD/scratch/replay-build.json" \
  --rom-rev00 roms/rev00.bin --rom-rev02 roms/rev02.bin \
  --all-pending --workers 10 --out "$PWD/scratch/replay-campaign"
```

Use repeatable `--function func_...` instead of `--all-pending` for an explicit
requested set. The worker preflights each requested root, records constructor
Unsupported exclusions, and constructs the supported batch with the complete
candidate availability. Any exclusion keeps the original outer scope incomplete,
even if the supported Host subset completes. An empty supported set never becomes
an empty-filter request for all roots. Metadata errors fail closed.

Each session report preserves `checks.replayed_environment` schema 1 verbatim.
Counts distinguish entered/reference-completed/checked/own-matched/passed/failed,
Unsupported, open and provisional cases; only Host-issued dependency-closed
`passed` proofs count as passed. Exact first-failure indices/packets, truncation
and each callee obligation's entry witness, sealed body/form/origin/version,
effective contract, required outputs and actual `discharged_by` packet remain
linked by report hash and JSON pointer. These process-local descriptions are
never compared across jobs to mint or import proof authority.

Reports and logs are fsynced before an atomic completion receipt; the receipt's
temporary file is flushed before replacement. POSIX also fsyncs the parent
directories. Windows reopens reports with non-truncating write access for
`os.fsync` (`_commit`/`FlushFileBuffers`); it skips directory fsync because Python
does not expose a portable Windows directory-flush descriptor. This is not a
claim of POSIX-equivalent power-loss durability for directory metadata. Windows
launches native Cargo/workers directly from `PATH`, without a POSIX environment
script; POSIX retains the sourced `--env-script`. Worker log handles stay owned
by the parent through process completion and fsync; unrelated descriptors are
not inherited.

Restart the same command to reuse intact matching receipts; damaged,
unparsable or identity-mismatched jobs are not done. Recorded failures are retained
without confirmation retries. Source/binary/ROM/root-scope changes require a new
output directory. No more than ten workers run, and every zero-checked requested
function is listed separately for each revision.

An uncalled root in one session may be covered elsewhere in the same corpus,
revision and source/body manifest. All entered cases must nevertheless be closed,
checked and passed, with no failed/unsupported/open/provisional case or unresolved
obligation. Per-session finalizer errors stay visible. The driver writes no
production or `PROVEN_REPLAYED` tables; Main alone changes status.

### Strict check failure collection and coverage

For an explicitly counted corpus investigation, `--mode check
--continue-after-mismatch` selects the host's `ContinueOriginal` policy. Each
failed root hook is disabled only for the remaining session; the unchanged live
original continues within the same frame, not by retrying or replaying a partial
frame. All failures remain counted. The runner emits a structured `check_failure`
JSON line to stderr after the frame and writes bounded evidence under
`<report>.failures/session-NNNN/`, with the session/frame, trace index, exact detail,
VDP diagnostics, registered per-function counters and wait/IRQ summaries.
Final reports retain `checks.failures`, `checks.failure_policy` and
`disabled_functions`. Any counted failure yields `status: "failed"` and a
nonzero exit **after** the report is saved, even if every input frame completed.
`union` preserves that failed status and exits nonzero as well. This flag is
rejected outside check mode; default check behavior remains fail-fast. It is a
diagnostic collection mode, never a passing equivalence result.

The denominator is exactly `tje_game::functions()`'s **648 inventory entries**,
including its three fragments, in each revision. Every JSON report lists each
inventory ID/name with an executable-PC entry count, including zero counts.
Counting occurs at the instruction-ready boundary before dispatch, not by sampling
one PC per frame or guessing from the call graph. Tail transfers and fragment
entries are counted as entries, not falsely called ordinary calls. Reports include
per-session hit totals and union counts, plus separate content unions per revision.
This runtime tooling needs neither the private inventory JSON nor a decomp checkout.

Content evidence is deliberately stricter than a function merely being available:

- Title/menu: observed original entry milestones.
- 1P and split 2P: live player-selection and split-screen RAM, after gameplay starts.
- Walking: an active player's observed world position changes by over 24 units.
- Inventory: the live per-player inventory-open view is nonzero.
- Presents: successful `SetupSelectedPresent` entry, not an attempted/open-rejected
  present. `OpenPresent`'s selected inventory byte is sampled at entry, so mystery
  (26) and bonus hi-tops (27) retain their storage identity as well as the resolved
  effect. The randomizer's separate `RandomizePresents` path counts type 18.
- Earthlings: a live bank record has a nonzero allocated object and viewport-visible
  bits 2/4. Merely generating an offscreen enemy does not count an encounter. The
  checklist includes all 25 definition identities, including ship/special kinds.
- Elevator and map: `ElevatorBeginRideScene` and `DrawGameMap` entry, respectively.
- Ship pieces: after a `ShipPieceAbility` entry, exactly one initialized assignment
  byte changes from a positive level to zero. Power-on/reset clears of all entries
  are never mistaken for collected pieces.
- Jam Out: `StartJamOut` entry. Ending-start and scrolling credits are separate
  milestones; neither is a claim that the entire ending has completed.
- Friendly services: wizard/opera require their paid active RAM states; wiseman
  identification requires the actual identify-call entry and a two-buck decrease,
  not merely opening its menu. Santa requires the drop ability, a valid Santa
  record in dropping state and a changed dropped-present insertion index. Telephone
  answering requires map entry with the source's reveal mode set, not just ringing.
  Hula/hot-tub/clap use their actual player poses; lemonade uses its granted-reward
  state. A life request needs accepted consent plus revival and the donor's original
  counter effect. Failed prompts, declines and idle handler calls do not count.

First-observed frame values in a union are evidence from one contributing session,
not a shared timeline. Use the per-session records to identify that timeline.

An execution failure writes `--report` before the failing host is dropped and
still exits nonzero with the original error. Its `status: "failed"` diagnostic
contains the exact session/input frame, host frame/PC/context, earlier completed
session reports, structured check counters (including TimeAware IRQ/poll summaries
and nested-original counts) and wait-frame measurements. It does not export raw
TimeAware traces or fabricate a completed coverage report; `union` rejects this
failure schema. If the diagnostic cannot be written, both that I/O error and the
original execution error remain visible. Input/ROM parse failures occur before a
host exists and therefore have no invented host counters.
Completed check/replacement reports also retain `checks.by_function`, including
the exact registered production set with zero-hit entries, per-function checked/
replacement/mismatch counts and the host's structured diagnostics. Reference-only
reports omit this Rust-check data rather than inventing a checked registration.

## Determinism and first divergence

```sh
# Two independent power-on reference instances; compare every byte each frame.
"$SESSION_TOOL" compare --rom roms/rev02.bin \
  --session sessions/rev02-one-player.tje.json --second reference \
  --boundary frame --report scratch/determinism.json
# Same inputs, completed game-logic boundaries, original versus replacements.
"$SESSION_TOOL" compare --rom roms/rev02.bin \
  --session sessions/rev02-one-player.tje.json --second replace \
  --boundary tick --report scratch/replacement-divergence.json
```

`--boundary frame` compares all 64 KiB of RAM **including dead stack**, VDP
VRAM/CRAM/VSRAM and registers, rendered pixels and dimensions, ordered VDP/YM2612/PSG
writes, IEEE-754-bit-exact stereo PCM, idle-draw counts and lag events. There are no
hash-only passes and no ignored-byte masks. This is the deterministic-replay test.
It does not skip lag frames: two timed original instances must match those too.

`--boundary tick` snapshots RAM when the main loop reaches `main_wait` after one
busy iteration, **before idle RNG draws**. Ordinals align completed iterations even
when video-frame numbers differ. A VBlank crossing while the reference iteration
was busy marks a documented lag tick; its paired ordinal is skipped and recorded
as `(tick, reference_frame, second_frame)`. A lag reported only by the candidate is
not an exemption. No other RAM range is ignored. The first difference reports the
contiguous first differing RAM range, tick, both video frames and PCs, plus each
runner's last entered function as execution context. That function is **not** a
claim that it caused the write. Hook-level check failures provide exact callee
attribution. A growing 120-tick alignment backlog is rejected, not accumulated
without a bound; unmatched final ticks fail as well.

A divergence writes the report and exits nonzero. It is expected that untimed
replacement of RNG/other functions can change idle draw counts and dead compiler
stack bytes before the whole game is native; the tool reports that honestly. It
never silently calls such a replacement replay equivalent. Idle draws are measured
per frame in the replay report, not guessed from the seed or normalized away.

## Regenerating inputs with the bot

```sh
"$SESSION_TOOL" bot --rom roms/rev02.bin --scenario one-player --frames 12000 \
  --out sessions/rev02-one-player.tje.json --report scratch/rev02-one-player.json
# Additional goals: campaign, explore, coop, level-zero, game-over, jam-all, intro, menus.
"$SESSION_TOOL" export-legacy --session sessions/rev02-one-player.tje.json \
  --out scratch/one-player-holds.json
```

The bot runs the reference and records the actual pad decisions, so later replay
does not run the bot or depend on planner changes. Menu function entries determine
selection timing. Gameplay reads `tje-mem` views for active players, inventory,
map banks, elevators and Earthling records. It attempts real inventory opens,
opens/closes the map with C, and uses weighted shortest paths on the live RAM map,
with walkability decoded from runtime user-ROM metatile/overlay/property resources.
Terrain decoding reuses the game's proven generic camera lookup helpers, including
the original word-index wrap for nonzero overlay selector 16; it does not replace
that selector with bare ground or substitute the overlay-ignoring point query.
Land routes must reach a walkable goal cell: proximity is not permission to walk
blindly across the final water/space gap. Live doorway entities contribute the
original conditional 4-by-3 ground footprint, and centered upward input opens a
closed doorway. Campaign planning carries real mobility presents for disconnected
terrain instead of spending them on unrelated encounter targets.
Diagonal land steps require support on both intervening orthogonal cells, in both
route planning and reachable-goal searches. This conservative controller policy
prevents waypoint tolerance from clipping a void corner, holding an original lean
pose and eventually triggering its short fall; it does not change game motion.
Ship collection targets the nearest reachable boundary of the cartridge's actual
contact rectangle, not a solid or unwalkable sprite-center tile. Mobility demand is
recomputed after level changes. Ordinary walking never silently becomes swimming;
a real active innertube explicitly enables water routing.
An explicit fall journey can also cross upper-level water when that real tube is
active. If a walking fall route is inaccessible, the controller considers held
mobility instead of waiting indefinitely with unused wings or a tube.
Macro-map connectivity includes hidden roads. Before spending mobility to escape
a disconnected component, the campaign targets reachable ground inside a live
`0x4000` marked metatile with an adjacent terrain class at least `0x50`. This is
the original grounded `ProcessLand` trigger; only the game's ensuing path animation
changes terrain, and the planner then replans. No invisible road is fabricated.
For a pending zero-direction edge cell, the navigator also observes the original
first-cell replacement rule and all cartridge-backed random alternatives. It
avoids standing where any such replacement would remove support. This separate
footing constraint never changes the reported current terrain, consumes no RNG
and writes no RAM; it is not a fabricated map update.
Missing neighbor directions (the original residual-register bug) and zero-length
choice sets are unknown/unsafe, never vacuously safe. Requiring future terrain
class at most `0x50` is a conservative bot policy; the original post-redraw fall
test specifically checks void `0x90` together with animation and height gates.
On later campaign levels, existing super-hitops presents can be opened for real
speed. Running requires a five-by-five stable-ground corridor through the next
32 world pixels, so it is not used to edge-hop around narrow path corners.
The bot also releases A near interaction/elevator targets rather than treating
the speed power as a grant.
If a path animation severs the route to an older trigger, the planner abandons
that stale trigger and considers actual inventory mobility instead of idling.
An observed road reveal, doorway teleport or level change invalidates the old ship
contact and clears that level's inaccessible-item/doorway exclusions. A failed pickup on
the old component must not permanently hide real mobility or food after the route
changes; completed friendly interactions remain excluded.
Campaign reserves are tracked by present kind: possessing an innertube does not
suppress collecting real wings or doorways. An unreachable ship target yields to
needed pickups instead of starving its own mobility supply. Real extra lives,
promotions and later-level speed presents are used through the inventory; a live
doorway may be reused if another route becomes inaccessible. Mystery/randomizer
fallbacks remain actual present opens, not fabricated effects or grants.
An unreachable old doorway is excluded until the route changes or a real spare
doorway is opened, so it cannot starve held wings, doorways or mystery presents.
Extra-life reserves are used up to the original limit of eight lives, not an
artificial five-life ceiling.
The campaign also collects up to two healing presents in advance and does not
open that reserve at full health merely because the inventory is nearly full.
The reserve includes genuine rootbeer as well as food and fudge sundaes. Critical
selection prefers a guaranteed sundae or rootbeer heal before ordinary food,
whose original effect can be yucky, rather than blindly opening the first slot.
A held real healing present preempts a long route when health is at most the
largest single Earthling hit in the runtime cartridge, once the player is
grounded on walkable terrain, including the original negative quicksand height.
That bound is observed and cached, not copied from
a retail resource table. A fixed very-low-health band misses lethal two-hit
sequences that jump past it. The inventory is not opened in mid-flight, and
health changes still come only from the real present.
Healing presents also use a short controller-settle interval before reopening
the menu for a second held heal; the longer unrelated-present pause must not
leave another one-hit-health gap between two actual food uses.
At that same one-hit bound, if food is exhausted and an awake allocated hazard
is nearby, a held real wing present is used to escape instead of being hoarded
for a terrain failure. Food still wins when both are available. Wing takeoff
uses the short settle interval and a nearby danger can request launch even for
a close destination; an already-flying player still stops flapping to land at
the real target. No replacement wing, extra flight time or health is granted.
Already-active wings satisfy a stale mobility demand: a spare innertube, doorway
or mystery present must not cancel the real flight. A successful elevator route
clears the prior demand before choosing more inventory actions.
A healthy campaign already near a real departure does not abandon it for a
distant proactive reserve once that level's ship piece is collected. Actual
uncollected pieces and critical healing still take precedence over that exit.
Sleeping players use the content planner's four-frame cycle of distinct real
A/direction inputs, following the original changed-input wake rule instead of
repeating one A-only phase. Reading the book still waits for its real countdown;
the controller never writes a wake state or fabricates a wake-effect entity.

Flight steers toward the world-space destination, with a velocity-based braking
estimate, rather than chasing individual eight-pixel land waypoints. Wings still
use the original directional acceleration, coasting, A flaps and finite timer;
the controller model does not alter the game's physics or grant flight time.
For a distant flight destination, takeoff uses neutral/released and A-only inputs
until the original flying pose is observed. An unrelated nearby pickup cannot
suppress that launch or cause a grounded walk into space before the next flap.
An existing lean first receives the opposite direction to step back onto support:
neutral/A does not cancel that original pose. Automatic flaps preserve this
back-out input instead of replacing it with an A-only command.
The same wing controller handles deliberate falls toward level zero; it does not
reintroduce a separate eight-pixel waypoint flap policy for that journey.
A campaign resumed from a genuine level-zero prefix also leaves the island after
its observed lemonade and hot-tub interactions: the original fall returns it to
the highest reached ordinary level instead of leaving it idle without an elevator.
A resumed `level-zero` excursion requires a bath witnessed by its current
controller before returning complete. Old aggregate service coverage from a
failed earlier game cannot end the excursion before it actually revisits zero.
Its service goals use the current bath and live lemonade-reward state; menus
clear the controller's prior bath state.
Blocking map-screen exits use the controller that requested the map, including
P2's telephone map; P1's C button cannot dismiss a P2-owned map wait.
No map, route, present table, or decoded resource is embedded in the crate or
recording. It prefers ship-piece targets, then matching-level departure elevators.
P2 initially stays behind to exercise split-screen, then joins the elevator target.
Jam Out plays both pads, exits through held Start, then selects random-sample mode.
Legacy export refuses recipes or connectivity changes it cannot represent.

The goal-directed planner minimizes edge exposure, accounts for active elevator
collision geometry and moving Earthlings, approaches the original doorway strip
before pressing Up, exits arrival elevators, chooses inventory slots through the
real cursor and holds actions through both controller-poll parities. It never
grants presents or health.
Nearby awake, allocated hazards shorten the replan interval rather than leaving
a moving-enemy cost map stale for two seconds. Costs use the live entity
position (the bank mirrors it only on alternating updates) and an eight-tick
velocity-sized safety margin. This is conservative controller routing, not an
exact enemy-AI prediction; it draws no randomness and changes no game state.
Threat surveys are rate-limited and skipped when a target/map/motion change
already requires replanning; they do not allocate or poll every far-away frame.
An elevator approached from its roof side first receives a lateral staging
target outside the original twenty-pixel crossing barrier, stays on that flank
until below the doorway, and only then turns inward to its front target. This
prevents a sideways/inward goal oscillation without crossing the closed roof.
Door fine-adjustment tests the real doorway coordinates, not the temporary
lateral waypoint, so reaching a flank cannot freeze the frontward staging leg.

Land routes never silently retry as swimming routes: an unreachable land goal is
reported to its caller, which must explicitly choose swimming or use a real
mobility present. This prevents inaccessible item goals from becoming drowning
instructions while preserving explicit `game-over` and travel goals.
If live terrain instead proves that a campaign player has already splashed into
water without active wings or an inner tube, the controller explicitly seeks
the closest reachable dry footing before resuming its old objective. This
shortest-cost shore search uses the existing navigation buffers, respects void
corners and real doorway ground, and stops at the shore rather than converting
the old objective into an unbounded swim. It does not change deliberate
`game-over` drowning, a deliberate fall, or an explicitly buoyant travel route.

`campaign` prioritizes uncollected ship-piece records; `campaign-two` runs the
same feedback policy with both players and pads, without an all-pieces shortcut.
`explore` also seeks missing presents and friendly interactions; `coop` separates
and rejoins the players before riding together; `game-over` deliberately drowns
the player through the last-life sequence. `jam-all` selects all eight original
Jam Out choices; `intro` watches the full natural intro; `menus` exercises controller
options, pause, the map and the real inventory drop path.

`content-one` and `content-two` use the separate content-directed planner with
one or two connected pads. It observes live item/Earthling/telephone banks,
prioritizes missing stored present identities and encounters, uses real purchase
and identification cursors, sneaks with logical A near Santa, answers ringing
phones, and chooses health/promotion goals from live resources. It shares the
same runtime terrain navigator and physical-controller conversion with the
campaign planner. These are goal selectors, not assertions that every generated
recording completes the checklist: only the replay's observed evidence counts.
The content policy can prepare a fresh two-player life-transfer route entirely
through controllers: genuinely eliminate P2, wake/request from the donor and
accept the transfer. It never discards an existing late-game input prefix to
replace it with that preparation. Services use reachable contact points and
original marked-road approaches; pending present unboxing completes before
following an elevator-waiting partner. Santa routes coordinate sneak/freeze
between both pads and collect repeated real gifts to supply missing presents,
promotions and spawnable Earthlings. These policies do not themselves certify
unobserved ranks or interactions; only the actual replay evidence does.

`content-stockpile` is a distinct single-player capture policy: it collects real
gifts through ordinary controller inputs instead of routinely unboxing them,
retaining the genuine inventory until the original full-inventory pickup check
is observed. That observed reaction, not a fabricated slot count, completes the
capture. It never synthesizes inventory or restores a snapshot, and does not
change the default `content-one` or `content-two` policies. Generated recordings
still require the same independent replay evidence as every other session.

```sh
"$SESSION_TOOL" bot --rom roms/rev02.bin --scenario content-one --frames 40000 \
  --out sessions/rev02-content-one.tje.json --report scratch/rev02-content-one.json
"$SESSION_TOOL" bot --rom roms/rev00.bin --scenario content-two --frames 40000 \
  --out sessions/rev00-content-two.tje.json --report scratch/rev00-content-two.json
```

`--prefix PATH` replays a previous **input-only** power-on session, then chooses
new inputs from its resulting live state. It never loads a snapshot. Prefixes
must match the cartridge revision, cannot contain recipe writes and cannot exceed
the total `--frames` limit. This makes route refinement reproduce the exact history
that acquired an item or reached a later level. Every resulting recording still
contains the complete input stream from power-on.

The planner observes actual success rather than treating a target as completed:
ship collection needs the original ability plus an assignment-byte transition,
friendly services require payment/service RAM evidence, and the campaign stops
only after all ten pieces and the real ending/final-level transition. Fixed World
keeps geometry reproducible; dynamic RNG/AI still require feedback. A missing route
is not permission to teleport, set completion flags or fabricate a late-game state.
An ending recorder does not stop merely when `MapEnterFinalLevel` begins: normal
`ProcessInput` must resume after that entry, graphics must be active and both
final-map banks must actually be loaded. The recorder observes the original
transition without inventing or rewriting either player's entity-level state.

`secret-ending` and its two-player variant `secret-ending-two` are separate,
explicitly labelled original-input routes: each enters
the hidden `GamePaused` sequence (`src/ui/pause.c:129-160`), which clears nine ship
assignments, collects the remaining actual piece, and watches the real ending.
It contains no RAM writes but **is not a ten-piece campaign**. The coverage
detector does not count the nine mass-cleared assignments as collected pieces.
This supplemental path covers the reachable secret and permits ending/credits
translation checks without misrepresenting full-campaign collection evidence.

## Initial recorded coverage (2026-10-23)

These are observed executable-entry counts, not estimates or static reachability.
All six streams begin at power-on, contain no setup writes and total **62,000
video frames**. Gameplay sessions selected Fixed World, confirmed by `gWorldType`.

| Revision | Session | Frames | Inventory entries reached |
| --- | --- | ---: | ---: |
| REV 02 | `rev02-one-player.tje.json` | 12,000 | 378 / 648 |
| REV 02 | `rev02-two-player.tje.json` | 12,000 | 386 / 648 |
| REV 02 | `rev02-jam-out.tje.json` | 7,000 | 144 / 648 |
| REV 00 | `rev00-one-player.tje.json` | 12,000 | 395 / 648 |
| REV 00 | `rev00-two-player.tje.json` | 12,000 | 307 / 648 |
| REV 00 | `rev00-jam-out.tje.json` | 7,000 | 144 / 648 |

Unions: **REV 02: 413 / 648; REV 00: 427 / 648; both: 428 / 648**.
The full per-entry counts and per-session checklist are generated by the replay
commands above; they are not embedded game data.

Both revisions reached title (frame 475), menus (606), solo gameplay (958), real
inventory opens, walking, the map, an elevator ride and level 2. The solo map and
ride entries were at 1863 and 3256. Both two-player sessions reached split-screen
at 2635; REV 02 also rode together (3785). REV 00's two-player planner remained
blocked on level 1, so its solo session supplies that revision's elevator coverage.
Jam Out starts at 966; the sessions exercise both ordinary and random-sample
VBlank handlers and all 18 Jam Out subsystem entries.

Present evidence is limited to super hi-tops (effect 7) and the bonus-hi-tops
storage identity (27). Visible Earthling definition identities 0, 1 and 9 were
encountered (ship piece, devil, mailbox). REV 00 solo collected ship piece 0
at frame 9348; REV 02 collected none. No session reached the ending or credits.
Thus **220 / 648 entries**, the other 26 present storage identities, 22 Earthling
kinds, most ship pieces and levels beyond 2 remain uncovered. This is not a
whole-game fidelity claim. The goal-directed navigation/collection plan above
describes how to extend that coverage without introducing saved-state shortcuts.

Two independent reference instances matched every completed frame for every
stream: full RAM, VDP state, pixels, ordered chip writes and bit-exact PCM,
**62,000 / 62,000 frame comparisons, zero divergences**. The input streams were
unchanged when their wire records gained explicit `frames`/`pads` field names;
the final parser rejects old unlabelled tuples rather than keeping a compatibility
format. Function checking and replacement are separate modes, not a claim that
untimed replacements already match the original's whole-game timing.

## Expanded deterministic checkpoint

The original six recordings plus the following paired streams total **18 sessions,
179,351 video frames**. Every stream is input-only from power-on; none contains RAM
writes or a stored state. Independent reference/reference comparisons matched every
frame, including all RAM, VDP state, pixels, ordered sound-chip writes and PCM:
**179,351 / 179,351 comparisons, zero divergences, no skipped lag frames**.

| Added scenario | REV 02 frames / entries | REV 00 frames / entries |
| --- | ---: | ---: |
| Full natural intro | 11,467 / 160 | 11,467 / 160 |
| All eight Jam Out choices | 24,047 / 144 | 24,047 / 144 |
| Controller/pause/map/inventory menus | 2,746 / 280 | 2,746 / 280 |
| Split/rejoin, health clap, shared elevator | 3,754 / 343 | 3,740 / 346 |
| Natural last life and game over | 5,527 / 269 | 5,527 / 269 |
| Explicit original secret-ending input | 11,071 / 394 | 11,212 / 402 |

Independent reference entry coverage is **REV 02: 509 / 648; REV 00: 517 / 648;
combined: 517 / 648**. Reports retain all 648 counts, including zeroes. These
numbers cover only the committed 18-stream checkpoint, not private bot experiments
or subsequently added content recordings.

Both revisions now exercise complete intro playback, all eight Jam Out modes,
controller options, inventory drop, pause, actual two-player separation/rejoin and
health clap, both elevator ends, last-life exit, game over, the ending, scrolling
credits and final level 26. The ending recordings explicitly use the original
hidden pause-controller sequence: they are **not** evidence that all ten pieces
were collected naturally. Only the actual remaining pickup is counted.

The whole-corpus check-mode replay is a separate gate; reference determinism does
not establish Rust-translation correctness. The natural ten-piece campaign,
level-zero services, remaining present identities/Earthlings and paid interactions
remain separate coverage work. No missing normal entry is labelled unreachable
merely because this checkpoint did not execute it.

## Final-level menu continuations

`rev02-final-menus.tje.json` (12,403 frames, 397 entries) and
`rev00-final-menus.tje.json` (12,544 frames, 405 entries) continue the explicitly
labelled secret-ending controller history. They exercise the original final-level
pause choices: select Reset then Continue, resume play, reopen the menu and perform
a real reset. They contain no RAM writes and do not claim natural ten-piece
collection. All **24,947** additional reference/reference frame comparisons pass
with zero divergences or skipped frames.

Combining the 18-stream checkpoint, nine content recordings and these two streams
produces **29 sessions / 688,522 frames**: **REV 02 593 / 648, REV 00 591 / 648,
combined 596 / 648**. The two new inventory entries are `DrawPauseChoicePanel`
and `RunFinalLevelPauseMenu`. The nine content recordings' complete comparison
reports were inspected, and every committed controller history was matched
exactly to its compared input history, including the two renamed recordings.
This is a measured corpus checkpoint, not a claim that the remaining natural
campaign, all content interactions or the settled check-mode gate are complete.

## Unreachable and exceptional entries

The report keeps the full **648** denominator and lists every zero-count entry
with its source location and a disposition. A zero count alone is **not** evidence
of dead code. The complete paired inventory and a source/header reference search
show these five entries have no callers, interrupt vectors, typed function-pointer
data references, code-pointer assignments, music callbacks or external entry roots:

| Entry | REV 02 | REV 00 | Definition |
| --- | --- | --- | --- |
| `ReloadActiveCameraViews` | `000BE2` | `000BE2` | `src/core/startup_and_main_loop.c:368` |
| `ShowRandomUncollectedShipPiece` | `020E2A` | `020DB2` | `src/ship/ship_screen_and_collection.c:652` |
| `NoOp` | `023428` | `0233B0` | `src/presents/noop.c:4` |
| `CopyLongwordRunToVramTile` | `026F98` | `026F20` | `src/asm/vdp.s:58` |
| `FillVdpDataBytes` | `026FD8` | `026F60` | `src/asm/vdp.s:90` |

`ErrorTrap` is different: exception vectors point to its terminal loop. It is not
dead, but deliberately provoking a machine exception is not successful normal
play. No ordinary uncovered function is relabelled unreachable to improve a score.

The original elevators also have no downward ride transaction: departure processing
calls `ElevatorAdvanceRidersToNextLevel`, which increments rider levels; arrival
processing only opens, closes and retires the arrival record. Coverage therefore
distinguishes departure, arrival and real falls to lower levels, rather than
inventing a downward elevator mode absent from the original.

## Latest content and rank checkpoint

This measured **79-recording / 3,110,038-video-frame** content checkpoint
supersedes the historical 29-session coverage summary above. It combines
50 inherited recordings (1,493,422 frames) with 29 new input-only recordings
(1,616,616 frames); private bot experiments are not included.

The 29 new recordings have independent power-on **reference/reference** comparisons
at every video frame: **1,616,616 compared frames**, no skipped lag ticks, and
`divergence: null` in every receipt. The
[public proof index](../sessions/content-reference-proofs.json) preserves each
actual compared session path and its receipt. Matching canonical controller
SHA-256 values bind renamed public files to the exact compared revision,
power-on start, frame count and RLE inputs; descriptions and tool notes are not
controller inputs.

| Reference entry coverage | Observed | Still zero |
| --- | ---: | ---: |
| REV 00 | 638 / 648 | 10 |
| REV 02 | 638 / 648 | 10 |
| Both revisions combined | 638 / 648 | 10 |

In each revision the aggregate replay checklist now observes all **28 present
storage identities**, all **25 Earthling kinds**, all **11 tracked friendly
interactions**, and every rank **0 through 8**. Actual ship-piece pickup coverage
remains **2 / 10 per revision** in this content corpus. Aggregate content and rank
witnesses are not a single-game natural campaign.

The four `rev00-content-final-display`, `rev02-content-final-display`,
`rev00-content-final-two` and `rev02-content-final-two` continuations retain their
explicitly disclosed original pause-secret ending prefixes. They are input-only,
but are **not natural ten-piece evidence**. The other twenty-five new content/rank
recordings use normal play.

`rev00-content-full-inventory.tje.json` adds a real full-inventory ground-pickup
attempt (`HasSpeakerSpeech`) in 46,535 frames. Its ordinary game-over/restart and
later play remain in the power-on controller history; no state is restored.
The proof index also carries the renewed comparisons of the previous 23 files
from the rebased runner; those controller files are byte-identical.

The REV 02 counterparts `rev02-content-cupid-bounce.tje.json` (31,424 frames)
and `rev02-content-singer-weak-target.tje.json` (26,669 frames) exercise the
original bouncing-damage and free-timed-effect-slot entries respectively.
The latter records funded P2 actually paying the singer near a weak native
Earthling, not merely witnessing its definition. Both complete frame
comparisons have `divergence: null`.

`rev00-content-whirlwind-reset.tje.json` (79,716 frames) uses a real rocket
present near the existing level-seven tornado and reaches the original saved
position restore. `rev00-content-allocation-pressure.tje.json` (64,158 frames)
uses real Hi-Tops, randomizers and clustered Earthling presents to enter both
the priority-release helper and its shared-tomato check. These are actual
executable entries, not injected allocation fixtures; the check's entry does
not claim that a shared tomato was successfully released. Both complete
frame comparisons have `divergence: null`.

The REV 02 counterpart `rev02-content-allocation-pressure.tje.json` (85,074
frames) now closes the remaining revision-specific graphics entries: two
original priority-release calls and one shared-tomato check. A separate
live-original call observer confirms that all three calls fully returned,
with no faulted entry opcodes or unfinished calls. Its complete independent
frame comparison also has `divergence: null`. The matched per-revision
decomp listings are identical except for the verified `FreeEntity` absolute
relocation; code identity is not substituted for either recording's execution.

Both revisions now witness every ordinary gameplay/graphics catalogue entry.
The ten global zeroes remain eight source-backed static/exceptional entries
and two explicitly unresolved audio retry fragments, not a reduced denominator.
This complete public-input union is the immutable measured 74-recording baseline
plus the five data-only follow-ons; retained baseline coverage is not described
as a new 79-recording replay.

These are observed reference executable-entry/content counts, not a claim that
the remaining entries are unreachable. Reference/reference determinism is a
separate evidence requirement and does not establish translated/native
equivalence. This checkpoint does **not** certify a natural all-ten-pieces
campaign, or a settled whole-corpus check-mode release gate.

## Map → missing-ship-piece summary → gameplay regression

`rev02-map-ship-exit.tje.json` and `rev00-map-ship-exit.tje.json` are
2,400-frame, power-on controller recordings for the reported web-preview exit
path. They use the existing fixed-world solo menu prefix through frame 1,039,
then remain neutral until gameplay. There are no RAM writes or snapshots.

The recorded P1 intervals are half-open video-frame ranges:

| Frames | Input | Intended transition |
| --- | --- | --- |
| 1,800–1,808 | C | Gameplay → map |
| 1,900–1,908 | A | Map → missing-ship-piece summary |
| 2,050–2,066 | C | Acknowledge/release → restore gameplay |
| 2,300–2,316 | Right | Continue playing after the exit |

Default web/native keyboard equivalents are **C**, **Z**, **C**, then
**Right Arrow**. On the map, A or B opens the ship summary; C instead returns
directly to gameplay. The ship summary accepts any A/B/C press and its release,
or times out after 240 game ticks. Start is not its acknowledgement button.
The selecting player's inventory must be closed before opening the map.

This is the missing-pieces overview (`ShowShipPieces(-1, player)`), not the
piece-collection animation, a collected-piece claim, or the completed-ship
ending. The proof receipt in `sessions/map-ship-exit-proofs.json` keeps the
reference/reference and native-engine comparisons separate from frontend
no-crash observations; determinism alone does not prove a translated hybrid
replacement correct.

## Natural ten-piece campaigns: REV 00 and REV 02

Both recordings are input-only, single-game Fixed World campaigns from power-on.
Each genuinely visits level 0 and every level 1 through 26, collects **all ten
actual ship pieces**, plays the original ending and complete scrolling credits,
and continues through the settled final level.

| Recording | Video frames | Actual ship pieces | Visited levels |
| --- | ---: | ---: | ---: |
| `rev00-natural-campaign.tje.json` | 518,005 | 10 / 10 | 27 / 27 |
| `rev02-natural-campaign.tje.json` | 519,824 | 10 / 10 | 27 / 27 |

Neither recording contains a startup RAM recipe, restored machine snapshot,
RAM writes, or original pause-secret mass collection. Independent Reference
replays observe neither GAME OVER nor an in-game pause in either history.

| Natural campaign witness | REV 00 frame | REV 02 frame |
| --- | ---: | ---: |
| Genuine level 0 | 209,307 | 273,888 |
| Tenth actual ship-piece pickup | 507,801 | 512,968 |
| Ending sequence | 508,477 | 513,643 |
| Scrolling credits | 510,504 | 515,670 |
| Credits complete / final-map entry | 513,436 | 518,602 |
| Normal final-level display/gameplay | 513,457 | 518,623 |
| Recording end | 518,005 | 519,824 |

All ten pickup identities in each revision have separate executed collection
witnesses. Their frames are preserved in the
[campaign proof index](../sessions/campaign-reference-proofs.json), alongside
the actual independent power-on Reference/Reference receipts: **518,005 REV 00
and 519,824 REV 02 video frames**, **1,037,829 total**, each with
`divergence: null` and zero skipped lag ticks. The comparisons include full
RAM/dead stack, VDP state, pixels, ordered hardware writes and bit-exact PCM;
there are no ignored-byte masks.

This is tool-assisted normal play. Read-only state observations inform ordinary
controller directions and inventory timing; actual presents, randomizers, wings,
inner tubes, ground/doorway footing and the level-zero bath do the work inside
the original game. Both complete controller histories replay independently
without the generator or any live observations. The previously disclosed
pause-secret ending recordings are retained, but are not counted as either
natural campaign's collection evidence.

The measured public Reference union now contains **83 recordings / 4,152,667
video frames**: the retained 79-input content checkpoint, two fresh map-exit
replays and both fresh natural campaign replays. The individual natural
campaigns execute **519 / 648** catalogue entries in REV 00 and **546 / 648**
in REV 02. Per-revision and combined union entry counts remain:

| Public Reference union | Observed entries | Still zero | Actual pickup identities |
| --- | ---: | ---: | ---: |
| REV 00 | 638 / 648 | 10 | 10 / 10 |
| REV 02 | 638 / 648 | 10 | 10 / 10 |
| Both revisions combined | 638 / 648 | 10 | 10 / 10 |

Retained source measurements are not relabelled as a fresh 83-recording replay.
Failed development branches and private per-level input checkpoints are not
included in this public corpus. These recordings establish the natural
campaigns in both revisions; they do not certify translated/native equivalence
or the separate whole-corpus check-mode release gate.

