# Status (current entry first)

## 2026-10-24 (night): replay proof tier in preparation; both natural endings recorded

Branch `integrate/p0`. 460 of 648 functions are proven and registered; 188 are written and held.

The held functions mostly finish in zero time where the original takes real time, so interrupts
and the Z80 see a different machine state partway through. A new check-only proof tier, the
**replayed environment**, is being built for them:

- A function's own ordered bus accesses must match the original's exactly, with reads served from
  the original's recording.
- Calls into other catalogued functions are satisfied by the original's actual call, from JSR to
  RTS, after the inputs that call consumes are checked. Every callee is proven on its own.
- Stack bookkeeping is checked where it is observed: locals read back, bytes a callee reads, and
  registers at exit.
- Timing stays proven by the engine's whole-session replays.

To prepare, every held body was compared with the matching compiler output at the level of
individual reads and writes. This covered read order and width, repeated reads, the discarded
read in each CLR, and the exact polling loops. The audit also found and fixed real semantic
differences, each with a regression test:

- the generic enemy's chance comparisons were unsigned where the original's are signed;
- an elevator step restored a saved byte where the original passes the player pointer in D3;
- the SEGA logo had an invented wait in its palette loop;
- part 11 of the intro used the wrong sequence pointer;
- the player falling check used the wrong local-variable offsets;
- the startup code polled a word bit where the original polls a byte bit.

New shared support: discarded bus reads that keep their purpose tag, RAM wait predicates that own
every poll, a yield marker for compound polling loops, exact-width host writes, and a register and
condition-code dependency analysis from the engine's decoder.

Recorded sessions: 83 input-only recordings, 4,152,667 frames. Both revisions now have a natural
campaign that collects all ten ship pieces across all 27 levels and plays the ending and complete
credits, with no cheats or pauses. Each recording replays exactly between two independent
reference runs. They reach 638 of the 648 functions in each revision. The two audio retry paths
were never reached in a bounded search of 48,384 forked runs; labelled fixtures exercise them on
real states by setting the Z80 busy byte for 112 Z80 ticks.

Engine: native and browser replay the first 82 recordings (3,634,662 frames) with no differences;
the REV 00 campaign is being checked. The release workflow builds Windows, universal macOS and
Linux packages. The packaged Linux and Windows builds were tested with both ROMs, and one defect
(two frames run after a fatal error) is fixed.

Next:

- the replayed-environment core, then one proof campaign over the whole corpus;
- a stronger profile for the interrupt, reset and Z80-bus routines;
- promotion of the ten functions in the current confirmation run;
- release builds and final reviews.

## 2026-10-24: timing bridge integrated; 461 of 648 functions proven, all 648 written

Branch `integrate/p0`.

Every catalogued function now has a readable Rust body: 461 are proven and registered, and 187
are written but held with an exact blocker each. The proven count fell because an independent
review found sixteen Presents functions that had entered production without a whole-corpus
pass; the candidate corpus showed ten of them failing, so all sixteen went back to pending. The
review's second finding was closed: a proof registry can no longer run held bodies in
replacement mode.

Most held functions finish in zero time where the original takes real time, so an interrupt
lands at a different instruction. The **timing bridge** now lets a readable Rust body consume
the engine's reference-phase timing through a portable `TimingSource`, so the body spends
exactly the original's time. Its first proofs:

- `WaitForNextVBlank` matches on the canonical path in both revisions.
- Four platform routines (both controller reads, the VRAM word copy including its 65,536-word
  zero-count case, and the CRAM palette copy) pass the strict clocked comparison: 11,056 checks
  per revision, no mismatches.

Support for timed bodies that call other functions is next. It covers 165 of the 187 held
functions.

Check mode also gained, since the last entry:

- typed execution contracts in every subsystem, with measured operational ceilings for the long
  searches (twice the largest exact cost over the corpus);
- a shared call clock;
- a future-frame provenance watch;
- auditing of DMA reads from work RAM;
- passive capture of interrupt frames;
- a strict zero-count DMA contract with a canonical proof in both revisions.

Recorded sessions: 81 input-only recordings, 3,114,838 frames, each replaying exactly between
two independent reference runs. They reach 638 of the 648 functions in each revision, all 25
Earthling kinds, all 11 services and ranks 0 to 8. The ten never reached are eight static or
exceptional entries and two audio retry paths. The ending is still reached only through the
disclosed pause secret; the natural all-ten-pieces campaign stands at six pieces on REV 00 and
eight on REV 02.

Engine: native and browser replay 73 of these recordings (2,776,462 frames, 35.1 billion
instructions) with no differences. Both frontends offer native NTSC or square pixels, integer
or fit scaling, and default to integer scaling at the native aspect ratio. Release packages
carry complete third-party notices. The public web preview now runs the engine build.

Next:

- timing-bridge support for calls into other functions;
- timed bodies for the held functions, subsystem by subsystem, each admitted only after a
  whole-corpus pass in both revisions;
- the natural ending campaign;
- crash and bug reports from the frontends;
- release builds.

## 2026-10-23 (night): whole-corpus gate; 482 of 648 functions proven

Branch `integrate/p0`.

Every production translation was checked against the whole committed corpus: all 50
sessions, in both revisions, including cross-revision replays. 25 functions that had
passed their fixture proofs failed on longer content sessions and are back in pending
with an exact blocker each. No failure was masked or waived.

| Cause | Functions |
| --- | ---: |
| An interrupt fires inside the original call; the zero-time Rust body misses it | 18 |
| Z80 progress differs when the function reads sound-driver RAM | 2 |
| Controller sampling shifts across an input edge (`WaitForNextVBlank`) | 1 |
| A finite search runs past the 1,000,000-instruction per-call cap | 3 |
| The original stalls on the VDP after its last port write | 1 |

The three searches are level-transition chains and Earthling spawn placement. Each
takes 1.2 to 1.7 million instructions over 78 to 136 frames, while the VBlank and
HBlank handlers keep running. They wait on reviewed per-function operational ceilings:
twice the measured maximum, failing closed, never treated as a bound.

Translation now stands at 482 proven, 160 held and 6 not yet written. A function now
enters production only after passing the whole corpus in both revisions; 16 candidates
wait for the next pass.

Since the last entry:

- Check mode has an open-bus operand-source contract for partially driven MMIO reads.
- The browser build runs the engine generated from the user's ROM and contains no 68000
  emulation. It replays 24,000 frames byte-exact in Chromium and holds 60 fps on the
  WebGL2 fallback. The ROM and the verified engine are remembered across reloads;
  Forget clears both.
- The native engine cache is written as one buffered file: a 20 ms cold store on Windows,
  where the first launch had timed out after 180 s.
- Audio initialisation uses pure-Rust maths throughout, so macOS ARM64 no longer
  depends on the platform maths library. The four audio goldens are unchanged on Linux,
  Windows and Chrome; confirmation on macOS CI is pending.
- The release and audit tool tests pass on Windows.

Next:

- the interrupt commutation rule and engine-timed Rust bodies (most held functions);
- the operational ceilings;
- the native frontend switching to the engine;
- the whole-corpus engine campaigns (native and browser);
- the six audio mailbox routines;
- a natural all-ten-pieces campaign.

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
