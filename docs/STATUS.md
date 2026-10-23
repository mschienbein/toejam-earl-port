# Status (current entry first)

## 2026-10-23 (evening): engine integrated; 507 of 648 functions proven

Branch `integrate/p0`.

Translation (generated in [FUNCTIONS](FUNCTIONS.md)): 507 functions are proven in both
revisions and registered, 135 are written but held with an exact blocker each, and 6 are
not yet written (low-level audio mailbox and sound-driver routines). Most held functions
are correct but run in zero time while the original takes real time: frame counters,
interrupts landing mid-function, Z80 timing. They wait on two contracts in progress:
exact engine timing for Rust bodies and an interrupt commutation rule.

Check mode gained, since the last entry:

- TimeAware projection of interrupt-visible effects;
- typed exits (RTE, terminal, reset, tail call) with banked stacks;
- a settled fixture boundary;
- dead-stack replay discharge (finite per-state evidence);
- a root-frame audit for nested original calls;
- lazy framed entries for functions that keep arguments on the stack;
- a dependent-equivalence mode for reads that genuinely depend on stack residue;
- an opt-in continue-after-failure corpus mode.

Recorded sessions: 50 input-only power-on recordings, about 1.5 million frames, each
replaying exactly between two independent reference runs. Together they reach 621 of
the 648 functions. Coverage includes every present type in both revisions, all 25
Earthling kinds on REV 02 (18 on REV 00), and all services on REV 02. A natural
all-ten-pieces campaign is still in progress; the ending is so far reached only through
the original pause secret, and those recordings are labelled as such.

Cycle-exact engine (`tje-engine-*`, `tje-native-platform`): integrated. It compiles the
user's ROM at first launch (Cranelift on desktop, generated WebAssembly in the browser)
and matches the reference at every instruction: 1.58 billion instructions over 128,000
frames through the IR, and all 27 then-committed sessions (663,575 frames) natively on
Windows. Audio uses one canonical scalar resampler on every target. Open: macOS ARM64
audio differs in the last bits (platform math library); the whole-corpus native and
browser campaigns over all 50 sessions; the frontends switching from the hybrid host to
the engine.

## 2026-10-23: readable layer in progress; cycle-exact engine under construction

Branch `integrate/p0` (not yet merged to `main`).

What runs now:

- `tje-native` and `tje-web` load the user's ROM (REV 02 or REV 00, checked by hash) and
  play the game. Proven Rust translations replace the original functions; everything
  else still runs on the vendored jgenesis 68000 (the hybrid host). The shipped game
  will not contain that 68000 (see [PLAN](PLAN.md)).
- `tje-host` check mode runs each translated function next to the original on real
  game states and compares every write, read and register the original's ABI exposes.
- `tje-session` records and replays input-only sessions; `sessions/` holds the
  committed recordings.

Translation progress (of 648 catalogued functions):

| | Count |
| --- | ---: |
| Proven in both revisions and registered for production | 305 |
| Written but not yet proven (kept out of production, blocker noted per function) | 109 |
| Not yet written | 234 |

Per subsystem, production / pending: audio 7/0, camera 9/13, collision 5/0, core 12/11,
cutscenes 42/15, earthlings 47/9, elevator 16/11, Jam Out 9/8, map 29/0, platform 13/1,
player 7/0, presents 41/17, runtime 6/0, ship 5/8, speech 5/3, sprites 36/3, UI 16/10.
More bodies are in lane branches waiting on shared host work.

Shared work these are waiting on:

- Time-aware projection of interrupt-visible effects in check mode. Most pending bodies
  wait on this.
- Host exits for functions that never return normally: RTE, terminal loops and reset.
- The settled fixture boundary.
- The open-bus prefetch contract for MMIO reads.
- Word-index addressing helpers.

The call-frame API (`tje_game::call_frame`) landed on 2026-10-23 and unblocks the
frame-dependent functions.

The cycle-exact engine is being written as separate crates:

- `tje-engine-core` (decoder, cycle model, IR);
- the runtime;
- the Cranelift backend (desktop) and the WebAssembly generator (browser);
- the verification harness.

None of it is integrated yet.

Release workflows (`release-native.yml`, `release-web.yml`) exist and run only on tags
or by hand. Nothing has been published.

## 2026-10-23: repository created

The decisions and the plan are in [PLAN](PLAN.md). Nothing is built yet. Next: phase 0
(foundations) and phase 1 (the jgenesis reference and the hybrid host).
