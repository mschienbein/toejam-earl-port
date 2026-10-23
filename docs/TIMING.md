# Original timing and the native frame schedule

## Scope and reproducibility

Measured on **REV 02**, using the original 68000 instruction/timing loop in the
vendored standalone jgenesis reference, not replacement/check-mode timings. These
are emulator measurements, not logic-analyzer measurements of a physical console.
The reference pin and local changes are in
[`vendor/jgenesis/NOTICE.md`](../vendor/jgenesis/NOTICE.md).

The additive `tje-timing` binary loads the private function inventory at runtime.
It never needs a compiled translation and never changes reference RAM, controller
input, or CPU timing to reach a scene. The committed examples contain **only
controller holds**. The elevator and Jam Out navigation was adapted from the
Sessions lane's controller-only recordings; the tool independently observed their
entry points below. JSON measurement outputs stay in private scratch, not Git.

All seven runs together execute **96,000 reference video frames**, of which
**88,650** are in the specified measurement windows. The twelve independent
retiming forks per selected frame add **54,000** experimental video frames.

| Run | Frames executed | Measured interval `[start,end)` | Observed coverage |
| --- | ---: | --- | --- |
| Attract | 18,000 | 0–18,000 | Title at 475; `IntroVideo` at 1,682; all eleven intro parts, from part 1 at 1,693 to part 11 at 10,152 |
| Menu | 9,000 | 906–9,000 | `NewGameMainMenu` / `GeneralMenuHandler`; remains at player selection |
| 1P stationary | 18,000 | 1,228–18,000 | Original main loop, ToeJam, one controller, normal music and idle RNG |
| 2P separation | 18,000 | 1,500–18,000 | `EnterSplitScreen` at 1,601; 3,259 split-line ISR entries; later falls/merges are included, not mislabelled as continued split-screen |
| Map | 9,000 | 1,800–9,000 | `DrawGameMap` at 1,803; `MapUploadCoverAnimationFrame` at 1,813; `DrawMapCharacters` at 1,814; map remains open |
| Elevator/navigation | 12,000 | 950–12,000 | Map at 1,863; `ElevatorAdvanceRidersToNextLevel` at 3,255; ride scene at 3,256; `ElevatorRideVBlank` at 3,261; end-ride routine at 3,421 |
| Jam Out | 12,000 | 966–12,000 | `StartJamOut` at 966; `JamOutVBlank` at 967; four-pair layout at 1,317, nine-pair at 1,662; both pads exercise instruments |

The 2P retiming window, 1,800–2,400, is entirely in split-screen. The full
separation run is deliberately reported as a **session**, not 16,500 uninterrupted
split-screen frames. The intro supplies the requested reachable cutscene; the
ending and random-sample Jam Out callback are not claimed as covered. REV 00,
PAL, and modes not visited here have no measured equivalence claim.

### Exact commands

From the port repository, with the user's private ROM and inventory present:

```sh
P=/absolute/path/to/toejam-earl-port
W="$P/scratch/TimingSurvey/wt"       # or the integrated repository
O="$P/scratch/TimingSurvey"
I="$P/scratch/Inventory/functions.json"
export CARGO_TARGET_DIR="$O/target"
export CARGO_BUILD_JOBS=4
# Working directory for the following commands is $W.
cargo build -p tje-host --bin tje-timing --release
T="$CARGO_TARGET_DIR/release/tje-timing"
E=vdp-control,vdp-data,vdp-all,ym2612,psg,sound-all

"$T" --rom "$P/roms/rev02.bin" --inventory "$I" --frames 18000 \
  --experiment "$E" --experiment-start 6000 --experiment-frames 600 \
  --output "$O/attract-18000.json"
"$T" --rom "$P/roms/rev02.bin" --inventory "$I" \
  --input crates/tje-host/examples/timing-menu.json --frames 9000 --measure-start 906 \
  --experiment "$E" --experiment-start 1800 --experiment-frames 600 \
  --output "$O/menu-9000.json"
"$T" --rom "$P/roms/rev02.bin" --inventory "$I" \
  --input crates/tje-host/examples/start-game.json --frames 18000 --measure-start 1228 \
  --experiment "$E" --experiment-start 1400 --experiment-frames 600 \
  --output "$O/one-player-18000.json"
"$T" --rom "$P/roms/rev02.bin" --inventory "$I" \
  --input crates/tje-host/examples/timing-split.json --frames 18000 --measure-start 1500 \
  --experiment "$E" --experiment-start 1800 --experiment-frames 600 \
  --output "$O/split-18000.json"
"$T" --rom "$P/roms/rev02.bin" --inventory "$I" \
  --input crates/tje-host/examples/timing-map.json --frames 9000 --measure-start 1800 \
  --experiment "$E" --experiment-start 1800 --experiment-frames 600 \
  --output "$O/map-9000.json"
"$T" --rom "$P/roms/rev02.bin" --inventory "$I" \
  --input crates/tje-host/examples/timing-elevator.json --frames 12000 --measure-start 950 \
  --experiment "$E" --experiment-start 3000 --experiment-frames 900 \
  --output "$O/elevator-12000.json"
"$T" --rom "$P/roms/rev02.bin" --inventory "$I" \
  --input crates/tje-host/examples/timing-jam.json --frames 12000 --measure-start 966 \
  --experiment "$E" --experiment-start 1000 --experiment-frames 600 \
  --output "$O/jam-12000.json"
```

Omit `--experiment` for passive measurement only. `--measure-end` bounds an
observation window without changing inputs or emulation. `--fixed-scanline`
selects the second experimental target; the measured default is 112. Output keys
are `issuer/CPU/device/read-or-write/active-or-vblank/irqN/scanline`, with counts
and minimum/maximum intra-line master-clock positions. `entries` and `first_entry`
provide independently observed scene-entry evidence. `work_ticks`, idle counts,
non-idle VBlanks, ISR entry/return lines and elapsed clocks, register-number
histograms, and the comparison statistics are also exported. No RAM, VRAM, pixels,
PCM samples, ROM content, or chip-register payload arrays are exported.

## Clock and observation definitions

The NTSC reference uses **53,693,175 master clocks/s**, **3,420 clocks/line** and
**262 lines/frame**: 63.695246 microseconds/line, 16.688155 ms/frame, or
**59.922743 Hz**, not exactly 60 Hz. The 224 active lines leave 38 vertical-blank
lines, 2.420419 ms. One video period is 128,005.714 nominal 68000 clocks at the
reference's divide-by-seven clock. A frontend must retain fractional audio/sample
and presentation timing instead of rounding a frame to 1/60 second.

- **Active** means live VDP line 0–223; **VBlank** means line 224–261. A write while
  the display-enable bit is clear still has an active *vertical phase*: it is not
  evidence of a visible raster effect. No status-register read is injected to
  classify phase.
- IRQ context is tracked from CPU interrupt acceptance through exception return,
  including nested interrupts. It is independent of vertical phase. In particular,
  a VBlank handler can still be executing on active line 64 or later.
- The PC saved before a 68000 instruction identifies its hardware accesses,
  including deferred end-of-instruction stores. Half-open address ranges from all
  648 inventory entries supply names. **Zero measured 68000 hardware accesses were
  unattributed.** Z80 accesses retain their own `Z80Program@PC` identity, not the
  interrupted 68000 function. The inventory's Z80 supplement identifies bootstrap
  and sound-driver programs; it does not invent 68000 function ranges for them.
- Both processors' Z80 RAM accesses are included, including Z80 instruction and
  operand fetches. A 68000 mapped access is counted once, not again as a Z80-origin
  access. DMA starts are actual inactive-to-active transitions, not guesses based
  on control-port words. Long port stores are their two original word transfers.
- 68000 reads are observed at the existing read dispatch and writes at the
  existing post-instruction dispatch. Z80 phase uses the Z80 clock: the VDP has not
  yet caught up while the reference runs Z80 ticks for the current 68000 step.
  This preserves the reference's instruction-level model, including its existing
  VDP status-read adjustment; it does not promise sub-instruction physical-bus
  accuracy. The largest reference clock advance observed in a step was 1,225
  master clocks (22.815 microseconds), including optimized DMA waits.
- Host frame numbers count `FrameRendered` events, not line-zero crossings or the
  game's resettable `gFrameCounter`. Logic-tick elapsed clocks include interrupts,
  refresh waits and bus stalls; they are **not** isolated C instruction costs.

The passive-observer ROM test forks an uninstrumented reference and compares
3,600 frames: full work RAM, all pixels, ordered VDP/YM/PSG writes, f64 stereo
sample bits, CPU PC and elapsed clock all match. This is an observation-invariance
check, not a claim that the experimental policies preserve the game.

## Hardware-access classification

Totals over the measured windows above. **A** is active phase outside IRQ4;
**H** is inside IRQ4, including its few VBlank accesses; **V** is VBlank outside
IRQ4. The JSON retains both phase and IRQ so this disjoint presentation loses no
ability to distinguish the exceptional H-in-VBlank cases. CPU origins are merged
only in this table, not in the output.

| Device / operation | A | H | V |
| --- | ---: | ---: | ---: |
| VDP control write | 35,041 | 166,471 | 1,474,689 |
| VDP control/status read | 1 | 0 | 1 |
| VDP data write | 308,688 | 59,923 | 511,272 |
| VDP data read | 0 | 0 | 0 |
| VDP H/V counter read / debug write | 0 | 0 | 0 |
| Actual DMA start | 1,089 | 0 | 54,516 |
| YM2612 write, including address latch | 9,819,443 | 0 | 2,112,082 |
| YM2612 read | 1,431,485 | 0 | 2,308,251 |
| PSG write | 990 | 0 | 321 |
| Z80 bus-request write | 29,705 | 0 | 314,496 |
| Z80 bus-acknowledge read | 7 | 0 | 938 |
| Z80 reset write | 6 | 0 | 4 |
| Z80 RAM read, including fetches | 1,261,871,399 | 0 | 194,031,496 |
| Z80 RAM write | 28,346,732 | 0 | 4,501,797 |
| Z80 ROM-bank write | 1,883 | 0 | 8,377 |
| Controller-port read | 18,898 | 0 | 267,762 |
| Controller-port write | 28,090 | 0 | 401,894 |

PSG reads and reset-port reads were not observed. Zero-count categories are not
silently treated as tested output-insensitive operations.

Representative active-display attribution, aggregating these sessions:

| Issuer | Observed active accesses | Meaning |
| --- | ---: | --- |
| `ClearUpperVramTilemap` | 84,787 data writes | Scene/menu uploads, often with visible display disabled |
| `MoveDataToVRAM` | 78,489 data writes | Main-thread loading, not confined to VBlank |
| `InitializeTitleTransitionTilemap` | 45,310 data writes | Title/scene preparation |
| `DrawStarfieldPlane` | 10,861 control + 10,851 data writes | Main-thread background work |
| `DmaCopyWordsToVram` | 8,681 control writes | Includes active-phase DMA programming |
| `JamOutDrawPackedBackgroundBlock` | 9,845 data writes | Jam Out layout/background changes |
| `HandleHBlank` and viewport-chain helpers | See next section | The split-screen raster transaction |
| `SetupYM2612` / `AudioWriteFmChannelReleaseMask` | IRQ6 writes extending into active display | VBlank *context* is not a promise that the bus writes happen during VBlank |
| Z80 sound driver | Millions of active-phase YM writes; exact issuing PCs in JSON | PCM/DAC playback remains clocked throughout scanout |
| `GetCtrl1Input` / `GetCtrl2Input` | Both active and VBlank accesses | Includes Z80 mailbox/DAC pacing compensation, not just gamepad reads |

In the stationary 1P run alone, IRQ6 YM writes/reads extended through active
line 62 and controller accesses through line 64. In the elevator/navigation run,
IRQ6 VDP data writes reached line 78, YM traffic line 122 and controller traffic
line 124. Moving all work labelled “VBlank” to one atomic blank-period callback is
therefore not faithful even before considering split-screen.

## HBlank / line-interrupt work

The source evidence is G's `src/core/startup_and_main_loop.c` and
`src/asm/interrupts.s`; the rows below are dynamic observations. Lines are the
VDP's live physical scanline, not a nominal “split is at 112” label.

| Context | Observed ISR entry lines | Return lines | Entry to just before RTE, 68000-clock equivalents |
| --- | --- | --- | --- |
| 1P gameplay | 181: 16,737; 182: 33; startup exception 246: 1 | 182: 6,938; 183: 9,832; 247: 1 | 532–544, median 541 |
| 2P separation, split part | 111: 3,243; 112: 16 | 113: 3,259 | Full split transaction 924–946 |
| 2P separation, shared/transition part | 181: 13,205; 182: 22 | 182/183 | Shared transaction 532–544; disabled-handler path 148–150 |
| Open map | 181: 7,200 | 182: 7,199; 183: 1 | Mostly 148–160; first four graphical calls 532 |

These durations include wrapper saves/restores and any reference wait states but
exclude the RTE instruction itself. A full split transaction spans about
120.5–123.3 microseconds, nearly **two scanlines**, not one instantaneous HBlank.

In all 3,259 split transactions in the separation session:

- `SelectSecondaryViewportSprites`: four control words and two data words per
  call, on line 112; this changes the original sprite-chain data, rather than
  simply switching an abstract camera.
- `HandleHBlank`: ten control words and four data words per call. It writes plane
  A base register **R2**, updates the lower view's VSRAM and horizontal-scroll
  words, includes the source's 112-pixel vertical adjustment, and finally writes
  **R10** to disable further graphical line work for that frame.
- The four handler data writes occur on **line 113**, not all on line 112. The
  plane-base/control transaction straddles lines 112–113.

The stationary 1P handler instead calls `TerminatePrimarySpriteChain` (two control
words plus one data word) and writes R10; it does not install lower-view scrolling.
Map display can keep taking IRQ4 while its graphics gate makes the handler return
without any device access. Title, menus and this Jam Out run had no IRQ4 entries.
Inventory-open paths and exceptional startup entries are retained in the full
session measurements rather than hidden by a nominal-line filter.

## Which active accesses change output?

### Experimental method and limits

For each selected reference frame, clone the **identical complete starting
machine state** into twelve forks: six classes times targets 224 and 112. Move
the immutable ROM between sequential forks instead of copying it. Each fork runs
original CPU instructions and the same inputs. It queues active-phase writes in
the selected class until the next target-line crossing, maintaining queued order.
Target 224 is the next VBlank; target 112 is the next occurrence of that fixed
line **after issue**, not a prediction of future writes.

VDP bursts obey the real FIFO/DMA backpressure and stall 68000 issue while their
released queue drains. They are not illegal instantaneous uploads. Deferred control
and data are tested independently and together; separate-class tests deliberately
expose the VDP address/command latch dependency. Sound tests include both address
and data ports. Compare every returned framebuffer pixel, the effective ordered
sound-port write stream, f64 stereo sample bits, sample counts, and work RAM.

Every fork is discarded after that one video interval. Thus differences cannot be
explained by many frames of accumulated RNG drift. However, altered waits, YM busy
polls, DMA source-read timing and CPU progress **within** the interval can differ;
`ram_different_frames` reports this. A result is the effect of the complete retiming
policy, not proof of the isolated visual effect of one register bit. Writes queued
after line 112 can remain queued beyond the comparison boundary: zero differences
are not a proof of next-frame or whole-session equivalence. Nonzero audio-frame
counts can include changed sample counts with zero amplitude error.

### Video results

Changed frames, **target 224 / target 112**. Each row compares 600 independent
reference frames except elevator/navigation, which compares 900.

| Window | Control writes only | Data writes only | All VDP writes |
| --- | ---: | ---: | ---: |
| Intro, 6,000–6,600 | 9 / 8 | 2 / 18 | 9 / 8 |
| Menu, 1,800–2,400 | 11 / 11 | 0 / 35 | 11 / 11 |
| 1P, 1,400–2,000 | 0 / 0 | 0 / 0 | 0 / 0 |
| Split-screen, 1,800–2,400 | **600 / 600** | **600 / 600** | **600 / 600** |
| Map entry/display, 1,800–2,400 | 3 / 3 | 3 / 3 | 3 / 3 |
| Elevator/navigation, 3,000–3,900 | 260 / 260 | 259 / 259 | 260 / 259 |
| Jam Out, 1,000–1,600 | 9 / 9 | 3 / 16 | 10 / 10 |

In split-screen, deferring all 12,000 selected active writes changes all 600
frames, up to **33,063 pixels/frame**. Data-only changes reach **34,010 pixels**
out of 71,680 (47.45%). Reference hardware attribution in that window identifies
the line handler and secondary viewport-chain helper as the active VDP issuers.
The same retiming has zero changed pixels in the stationary 1P window, whose
active VDP work only terminates the unused chain and disables subsequent line work.
This is an observed *context-specific* non-effect, not permission to defer all
active VDP writes in 1P inventory, map, loading or scene transitions.

DMA-start commands belong to the control/all-VDP experiments; they are not an
independent “start bit only” operation. Active DMA starts were observed and the
retiming tests include their bus-ownership consequences. Controller, bus-request,
reset and mailbox writes are classified, not retimed: they participate in software
handshakes and cannot be equated with a replaceable video/sound register stream.

### Sound results

Selected examples; counts are 224 / 112. “Stream” compares ordered **untimed**
YM/PSG port writes; the PCM comparison also catches timing changes when that
stream remains identical. Maximum error is the largest absolute normalized f64
channel-sample difference in the corresponding window.

| Window / class | Active writes queued per policy | PCM-different frames | Stream-different frames | Maximum sample error, 224 / 112 |
| --- | ---: | ---: | ---: | ---: |
| 1P / YM2612 | 4,082 | 36 / 36 | **0 / 0** | 0.164530 / 0.238132 |
| Split / YM2612 | 4,142 | 43 / 43 | **0 / 0** | 0.219574 / 0.254580 |
| Elevator / YM2612 | 125,229 | 520 / 520 | 4 / 508 | 0.272909 / 0.288569 |
| Elevator / PSG | 539 | **204 / 195** | 1 / 134 | 0.113656 / 0.123543 |
| Elevator / all sound | 125,768 | 724 / 715 | 5 / 642 | 0.272909 / 0.288569 |
| Jam Out / YM2612 (also all sound here) | 61,356 | 252 / 252 | 5 / 252 | 0.314282 / 0.342143 |

The 1P YM test changes **23,813 stereo samples** despite identical ordered port
writes; the split test changes 28,731. Jam Out changes 175,753 stereo samples.
The elevator all-sound policy changes 437,708 samples at 224 and 435,069 at 112.
PSG was genuinely exercised in the elevator window; its zero writes in the 1P,
split and Jam Out windows are not claimed as PSG equivalence tests.

**Result:** batching sound writes once per frame fails, even if an untimed
register-stream check passes. Preserve time and order, including YM address-latch
writes, busy polling effects and Z80 DAC/PSG pacing. The Z80 must continue running
between main-thread and interrupt callbacks.

## Main-loop work, idle draws and lag

The tool detects the original loop head and its RNG wait, using the inventory's
`FurtherInitialisation` base plus the measured internal-label offsets. It counts
an idle RNG call only if its actual stack return address is the idle-call return
site. Calls made by gameplay, scenes, or music are not idle draws.

| Session | Completed measured loop iterations | Elapsed CPU-clock equivalents, min / mean / max | Idle draws per video interval, min / mean / max | Non-idle VBlank boundaries |
| --- | ---: | --- | --- | ---: |
| 1P | 16,772 | 17,238 / 47,976.87 / 93,640 | 0 / 27.009 / 41 | 2 |
| 2P separation | 16,421 | 12,522 / 48,145.15 / 8,628,202 | 0 / 27.265 / 43 | 941 |
| Elevator/navigation | 9,898 | 12,486 / 71,854.20 / 91,374,540 | 0 / 20.847 / 42 | 1,368 |

The huge latter maxima are **not millions of cycles of normal per-frame world
simulation**: these are source loop iterations enclosing modal inventory/map,
level generation and elevator waits. The longest spans are 67 and 713 host video
intervals. The map run has only three completed ordinary iterations after its
measurement start; it then deliberately waits inside the map display. Its 7,197
non-idle VBlanks include **7,187 explicit `WaitForNextVBlank` waits**. These must not
be counted as 7,197 gameplay lag frames. The exported field is intentionally named
`nonidle_vblanks`, not `lag_frames`.

The stationary 1P work alternates strongly. Four consecutive iteration phases,
indexed from its first loop entry, have mean costs **22,201.52 / 75,667.88 /
17,996.43 / 76,041.65** CPU-clock equivalents. The source alternates camera/sprite
work with the two different simulation groups; it does not run every subsystem
once every video frame. Idle-draw counts cannot be converted using one fixed
“cycles per RNG call”: the signed arithmetic/runtime path, interrupts and bus
waits vary. The direct elapsed-clock measurements above accompany the counts for
that reason.

The 1P non-idle boundaries are frame 1,228 during initial
`UpdateAndQueueEntitySprites`, and frame 17,961 at `lmult`; the corresponding
iterations finish in the following host intervals. Even a work cost below the
whole 128,006-clock frame budget can cross VBlank because previous interrupt/audio
work consumed part of the available interval.

In the entirely split-screen window 1,800–2,400, **152 of 600** VBlank boundaries
arrive while main work is still running, principally in viewport-sprite and entity
animation work. Its 600 completed iterations cost 23,390–173,486 clocks (mean
72,049.58). These are genuine busy boundaries, not map-wait false positives. This
is a substantial **25.33%** of that window, not a rare startup exception. Busy
boundaries do not imply exactly one lost logic iteration each: a late iteration
can skip its idle wait and start the next one within the same video interval.
Check mode must use actual logic boundaries, not a presumed one-to-one video/tick
index.

## Proposed native schedule

**[INFERENCE / proposal]** Keep the current hybrid reference as the timing oracle
until each translated scheduler/platform path can emit the same timestamped
transactions. The native schedule should be a master-clock event schedule, not
`update(); vblank(); render(); audio();` once per nominal 60-Hz frame.

1. **Continuous device time.** Maintain an integer master-clock timeline. Advance
   VDP scanlines and rendering, Z80, YM2612 and PSG to each due event, preserving
   upstream clock divisors and sample accumulation. Presentation and audio-buffer
   delivery never change emulated event timestamps.
2. **Main-thread logic.** Preserve the original alternating work groups and
   waits. A translated tick may compute quickly on the host, but publish its RAM
   and hardware effects at its modeled/reference times. Explicit map/menu/ride
   waits remain continuations, not synchronous loops that block device clocks.
   Replay uses recorded idle-draw counts and busy/logic-boundary metadata. Full
   native play needs translated-path timing/coroutine boundaries where IRQ work
   can observe partially completed main work; these measurements do not justify
   treating every function or loop iteration as atomic.
3. **VBlank at line 224.** Take the original pending interrupt, preserving its
   enable/mask ordering. Run the ordinary graphics service or installed callback
   (`ElevatorRideVBlank`, `JamOutVBlank`, scene callback), then sequencer/input work
   in source order. Schedule the resulting bus operations at their individual
   offsets, including those extending into active display. Do not force the
   entire callback into the 38 blank lines, or publish every operation at line 224.
4. **Line interrupt.** Preserve R10 counter/reload behavior, pending-interrupt
   latency and the source graphics gates. In measured split-screen the interrupt
   is entered around line 111/112, the chain/control writes occur on 112, and
   scroll data on 113. Emit that ordered transaction as a timed continuation.
   Do not replace it with a single lower-camera swap at 112: both fixed-line
   experiments changed every split-screen frame. Shared-screen termination and
   inventory-window branches need their own source paths, not a hard-coded split
   transaction on every IRQ4.
5. **Timestamp sound writes.** Each write carries integer master-clock time,
   processor origin and stable issue order for ties. Advance chip/Z80 state to
   the write before applying it. Keep YM address/data pairs, Z80 mailbox/bus
   ownership, controller DAC compensation and sample playback order intact. Use
   the retained chip models' clock quanta, not host wall time, audio callback
   cadence, or VBlank-level timestamps. Verification compares the ordered stream
   **and** its timestamps/sample output; this survey's untimed-stream counter
   alone is explicitly insufficient.
6. **Lag and RNG policy.** Faithful replay reproduces the recorded continuation
   boundaries and exact idle counts, with the documented reference lag-frame
   comparison policy from [PLAN.md](PLAN.md). An interactive deterministic policy
   can instead choose a versioned fixed idle count; **27 per ordinary video tick**
   is a measured-mean candidate, not an implemented or proven-equivalent choice.
   Here the observed 0–43 minus that fixed 27 gives -27 to +16 draws
   per interval; no universal bound is established. Modal callbacks must not
   receive ordinary idle draws. A native implementation must document its chosen
   policy rather than silently inherit host CPU speed.

### Differences and bounds

**[INFERENCE]** None of the demonstrated video or audio corruption is unavoidable
when removing the 68000: those differences are consequences of a bad schedule,
not approved fidelity exceptions. Keeping the same devices and event timeline
permits zero intended pixel or sound-register differences. This survey does not
prove a complete native timing model or whole-game equivalence.

The explicitly allowed differences remain original lag presentation and the
chosen idle-draw policy. The measured split window bounds one important case:
152 busy boundaries in 600 frames, not a single global “at most one lag frame”
allowance. Advancing a displayed result by one NTSC frame changes its presentation
by **16.688155 ms**; it must be listed per replay, not hidden by a blanket tolerance.
Idle-count bounds above apply only to these measured windows; later RNG-dependent
state divergence is not numerically bounded by the draw-count difference.

The existing reference's instruction-level dispatch quantization is inherited,
not a new native exception. A sound or H-int scheduler must not add another whole
line (63.695 microseconds) or whole frame of jitter on top. In the experiments,
released writes were delayed by up to 768,173 master clocks, about 14.307 ms,
with large measured PCM and lower-screen differences; that is evidence against
coarse batching, not an acceptable error budget.
