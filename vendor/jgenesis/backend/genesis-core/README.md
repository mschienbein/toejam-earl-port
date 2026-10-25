# genesis-core

Sega Genesis / Mega Drive emulation core.

The Genesis contains the following components, some of which were reused from the Master System:
* 68000 CPU clocked at 7.67 MHz
* Z80 CPU clocked at 3.58 MHz, meant primarily for audio processing
* VDP (video display processor) that is based on the Master System VDP but adds yet another more advanced graphics mode
  * Can render in either 256x224 or 320x224; PAL consoles additionally support 256x240 and 320x240
  * Supports 9-bit RGB color with up to 64 colors onscreen simultaneously
  * Supports shadowing and highlighting to create additional colors beyond the 512 colors of 9-bit RGB
  * Supports two background layers with per-pixel priority
  * Supports per-scanline and per-16px-column background scrolling
  * Supports up to 80 sprites per frame and up to 20 sprites per scanline, with sprites ranging in size from 8x8 to 32x32
* YM2612, an FM synthesis sound chip
  * Contains 6 fully configurable FM synthesis channels, each with 4 operators that can be arranged in 1 of 8 different configurations
  * One of the 6 channels can be swapped out for a raw DAC channel which directly outputs a PCM sample
  * Output frequency of 53.267 KHz
* SN76489, a PSG lifted directly from the Master System
* 64KB of working RAM for the 68000
* 8KB of working/audio RAM for the Z80
* 64KB of VRAM
* 128 bytes of CRAM (color RAM for storing palettes)
* 80 bytes of VSRAM (vertical scroll RAM for storing per-column V-scroll values)

This crate contains code for the following:
* Genesis VDP
* Genesis FM sound chip
* Genesis memory map
* SVP chip, an auxiliary processor used by _Virtua Racing_
* Genesis main loop

## TJE reference proof hooks

`reference-proof-hooks` is an explicitly enabled development feature. It is
default-off and forbidden by the native/frontend shipping dependency-graph test,
including the actual `wasm32-unknown-unknown` browser graph. It implies the
reference emulator; it is not a peripheral-only shipping feature.

`GenesisEmulator::create_with_reference_proof` installs passive original-opcode
provenance before the existing power-on reset. It neither arms a perturbation nor
adds a reset charge or device-clock advance. Ordinary `create` retains its prior
behavior. The single `reference_proof` module owns this development-only bridge;
normal source timing and pending-write order are unchanged.

The authored audio fixture lane accepts only a pinned retail revision and a
session/SHA-1/frame/original-instruction binding at a real pending PCM/PSG owner
invocation. It authenticates the preceding original sequencer JSR, the retained
caller frame and CPU exit state, and the actual cartridge identity. At that
owner's committed word BUSREQ take, it physically writes only Z80 RAM byte
`0x41` from its required initial zero to one. Initial command byte `0x40` must also
be zero. From the first original retry-release commit, it counts exactly 112
actually invoked Z80 ticks that are released, non-reset and not VDP-halted, then
restores the saved zero at that exact tick tail.

This is two one-shot physical writes, not a held value, read overlay, readiness
override, guessed 68000 delay or clock change. A physical writer journal catches
all target-byte aliases and same-value writes. Intervening writes are preserved
and logged; an unexpected writer, opaque mutable RAM view, missing journal,
non-original execution, reset or diagnostic-ceiling excess fails permanently before any
successful receipt. Failure never reasserts one or silently restores zero.

Typed receipts retain the original commit/access/instruction locators, every
eligible tick and every physical target-byte event. Their event ordinals define
chronology; clock fields retain the reference model's respective processor
counters, not a claim of sub-instruction physical-console bus accuracy. A receipt
can be completed only after the original owner returns through its real RTS and
the existing caller frame. All such receipts are explicitly `natural: false`;
they do not admit a Rust body or establish natural reachability.

Live-origin authority is private and never serialized. Decoding or speculative
cloning invalidates it; archival receipts and state-shaped copies cannot mint a
live provider. Proof tooling must replay the input-only session from power-on and
verify its source hash. Never publish machine snapshots, cartridge content or
prefetch/open-bus payloads.

Audio source authority is irreversibly invalidated by non-original execution even
before arming, any public mutable CPU/bus escape (including same-value use), and
public state/configuration mutation or import. The provider uses private internal
access; tools drain only observer metadata through the dedicated read-only drain.
Completion also requires a fresh actual owner RTS exit, identical CPU checkpoint
and unchanged execution epoch. Unarmed archival clones may execute passive
analysis, but cannot arm or seal; active mutating clones remain fail-closed.
The 1,024-original-instruction limit is an operational diagnostic ceiling only,
not a proved source bound, completion guarantee or extra runtime allowance.

### Passive reset input

The same explicit proof constructor also observes the actual reset window. The
independent reset lane records the raw line, ordered physical vector and prefetch
reads (including their purposes and open-bus transitions), immutable before/after
CPU and bus/clock fields, and the actual returned CPU charge. Constructor and
soft-reset code continue to ignore that charge: no device/master-clock advance
is invented. These are baseline emulator observations, not a claim about console
reset accuracy or work performed by the game's `_start` function.

`reference_reset_observation` exposes archival facts only.
`take_fresh_reset_input` consumes a separate opaque, nonserializable, one-shot
input at the immediate genuine entry. It is not root admission, reset/RTE
execution authority, or production eligibility. The host must consume it in its
single pre-fork admission; analysis branches may receive only privately scoped
immutable facts. Progress expires freshness. Public mutable CPU/bus escape,
non-original execution and imports irreversibly invalidate reset live-instance
lineage, even for same-value mutations. Clone/Decode cannot copy it, and a real
reset on an analysis copy cannot revive it.

Genuine soft/hard resets can record new reset inputs only on an independently
trusted live instance. Hard reset moves only that reset lineage into the actual
replacement; it does not revive the audio fixture's power-on origin. Archives
remain untrusted. Bounded capture overflow, incomplete/exceptional reset and
changed entry fail closed without changing reference execution. Authored
ROM-free regressions cover physical-state/timing inertness, one-shot freshness,
stale re-entry, same-value escape, archival copies and reset-on-clone denial.
