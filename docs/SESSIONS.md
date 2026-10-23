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
- `tool`: version of the writer, currently `tje-session/0.1.0`.
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

Check/replacement use `HookRegistry::translated(revision)`, not a local duplicate
registry. Untranslated functions continue in the original. Check mode is fail-closed
and reports the host's exact function error; it is not replacement mode with
warnings suppressed. Coverage should be taken from **reference** (or check, whose
live CPU still executes the original), never presented as reference coverage when
measured in replacement mode.

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

First-observed frame values in a union are evidence from one contributing session,
not a shared timeline. Use the per-session records to identify that timeline.

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
# Also: --scenario two-player, --scenario jam-out; use each revision's cartridge.
"$SESSION_TOOL" export-legacy --session sessions/rev02-one-player.tje.json \
  --out scratch/one-player-holds.json
```

The bot runs the reference and records the actual pad decisions, so later replay
does not run the bot or depend on planner changes. Menu function entries determine
selection timing. Gameplay reads `tje-mem` views for active players, inventory,
map banks, elevators and Earthling records. It attempts real inventory opens,
opens/closes the map with C, and uses breadth-first paths on the live RAM map,
with walkability decoded from runtime user-ROM metatile/overlay/property resources.
No map, route, present table, or decoded resource is embedded in the crate or
recording. It prefers ship-piece targets, then matching-level departure elevators.
P2 initially stays behind to exercise split-screen, then joins the elevator target.
Jam Out plays both pads, exits through held Start, then selects random-sample mode.
Legacy export refuses recipes or connectivity changes it cannot represent.

The bot is intentionally a bounded deterministic planner, not an assertion that
all generated sessions finish the game. Its observations and the replay report
state exactly what was reached. Remaining coverage needs a goal-directed planner:
replan around moving Earthlings and collision gates, target loose presents by RAM
item banks, retain per-level goals for missing kinds/ship pieces, and track
successful collection before choosing an elevator. Fixed World keeps level routes
reproducible; live RNG/AI still require feedback. The ending should be reached by
collecting all ten pieces through real interactions and then taking the real final
transition, not by writing completion flags or storing a late-game snapshot.

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
