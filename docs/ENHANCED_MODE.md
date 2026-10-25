# Enhanced mode

Enhanced mode is an explicit, default-off simulation profile. Ordinary
`Machine::step` / `play_frame` do not consult its registry. The compiled cartridge
remains the fallback for every unselected entry. Enhanced code is not a timing
proof, and does not enter the proven/readable production tables.

## Resumable overrides, sidecars and input (format 3)

Design agreed with the snapshot owner before implementation, 2026-10-25.
Faithful machine snapshot format **2** and its exact bytes/hash remain unchanged.
Enhanced profiles use an enhanced-only **3** envelope, returned as the same
`MachineSnapshot` type so local saves, rewind and rollback hashing retain one
atomic unit. The existing header retains its ROM SHA1, frame count, CPU and
profile identity; the hardware payload is followed by a versioned enhanced
payload (schema 2) and its length, covered by the existing whole-image checksum. A plain
machine restore rejects this envelope: callers must use `EnhancedMode::restore`.
That method decodes and validates *all* extra state before restoring hardware,
then commits the decoded sidecars/scheduler without another fallible operation.
Faithful/enhanced/profile/sidecar-schema/config mismatches are errors, not
best-effort conversions. Snapshots are private local data, never public assets.

### Authoring and scheduling

Existing `Translation` and `FramedTranslation` callbacks remain bounded,
synchronous Atomic/Functional RTS replacements. They cannot suspend inside their
Rust call stack. `EnhancedProfile::with_resumable` adds explicit
`ResumableTranslation` entries; these may replace classified TimeAware RTS
functions as well. Their function takes `&mut dyn enhanced::Context` and
`&mut Continuation`, returning `OverrideStep`:

- `Return(Option<u32>)`: ordinary synthetic RTS, optionally setting D0.
- `Wait(Wait::Frames(n))`: suspend for at least `n` completed hardware frames.
- `Wait(Wait::Interrupt)`: suspend until an unmasked interrupt is acknowledged
  and its real handler has returned to this invocation.
- `Wait(Wait::Poll)`: yield one bounded clock quantum before checking a polling
  condition again. A VBlank RAM-counter loop reads the actual counter, yields,
  then rechecks; no readiness bit is fabricated.
- `CallOriginal { function, arguments }`: materialize an actual emulated call
  frame, run the compiled original in the frame scheduler, then resume the Rust
  state machine with its register effects intact. The original root's exact
  PC/SP stays bypassed, including polling backedges to its entry; nested calls
  at another SP still consult the enhanced registry.
- `ContinueOriginal`: a bounded before-only observer; enter the current compiled
  original once without keeping a callback/return obligation. Its deferred
  bypass is snapshot-safe even if an interrupt arrives before the first opcode.
  Use this for roots whose callees may reset or perform a nonlocal transfer.

The continuation is explicit serializable data (`stage` plus sixteen `u32`
locals), not a host future, closure, thread, pointer or borrowed Rust frame.
Larger per-invocation state belongs in a sidecar. Each callback invocation must
terminate. A positive clock charge after each transition prevents all-Rust
loops from monopolizing the frontend. Waiting advances real devices, admits and
runs real interrupts, and returns to the frontend at each hardware frame.
Original calls are likewise stepped, never synchronously drained across frames.
Suspended interrupt handlers may themselves enter resumable overrides.

Only classified RTS roots may be selected: RTE/reset/nonlocal-transfer roots are
not override entries. Managed `CallOriginal` transitions require an actual RTS;
use `ContinueOriginal` when no return obligation is appropriate. Synchronous
legacy callbacks cannot call a resumable override; use an explicit
`CallOriginal` transition instead. Nesting is bounded at 32 active invocations;
legacy original calls retain their 1,000,000-step/32-depth operational budget. Failure
latches the driver, with no fallback/retry of partially applied effects; only a
successful atomic restore or reset clears it.

### Typed sidecars

An extra defines a `Sidecar` type with bincode 2 `Encode` / `Decode<()>`, `Send`,
a stable unique `KEY`, and a nonzero schema `VERSION`. Register its initial state through
`mode.sidecars_mut().register(initial_state)` before playback. Both host and
callback can use `sidecars().get::<T>()` / `sidecars_mut().get_mut::<T>()`.
Missing registrations and duplicate keys fail closed. The registry is ordered
by key, not hash-table iteration or host type identifiers. Every entry's key,
version, canonical initial-state SHA1 and complete current value are serialized.
Restore requires the same registered schema and initial-state identity. For
immutable course/configuration fields, override
`Sidecar::configuration_identity() -> &[u8]` (at most 128 bytes; empty by default).
Its trusted registered value is compared against decoded/current state before
restore or capture; even a checksum-rewritten archive cannot replace the
configuration by retaining the old initial-identity claim. A stored digest must
be authenticated against the normalized immutable data by `validate`.
Reset restores registered initial values. At most 64 keys (1–128 bytes each) and
4 MiB of enhanced snapshot payload are supported; no arbitrary unregistered
state is silently initialized. Optional `Sidecar::validate` checks invariants
when registering, capturing and restoring a value.

All deterministic state belongs here: extra seats, course data and progress,
extra RNGs, pending audio requests, publication state and gameplay settings.
Codecs must be deterministic and round-trip their values. Do not encode host
pointers, wall time, unordered-map iteration or platform-specific native sizes.
The complete enhanced image (including pending continuations and input phase)
is the rewind/save/netplay divergence hash. Existing frontend netplay remains
faithful-only until its enhanced-profile transport is separately enabled.

### Four-pad routing and integration

`EnhancedInputs { pads: [GenesisController; 4] }` carries four complete controller
states; `From<GenesisInputs>` supplies disconnected seats three/four. Enhanced
`step` / `play_frame` accept either. Only the first two pads reach original
hardware, unchanged. All four are available through `enhanced::Context::inputs`
and are included in enhanced snapshots; faithful mode ignores extra seats and
delegates exactly to the original two-pad API. This foundation does not add a
four-seat frontend controller-binding UI or change original input RAM.

Use `mode.snapshot(engine)`, `snapshot_into(engine, image)` and
`restore(engine, image)` for enhanced saves/rewind/rollback. Native and browser
adapters use these APIs whenever built with enhanced mode, including when the
selection is off (where they delegate to exact faithful format 2). Profile IDs
must change when override behavior changes. Immutable configuration/course data
must also participate in registered initial-state identity.
Rollback adapters also use the atomic capture/reusable-capture/restore/hash
boundary (and the selected driver for advancement); none drops an enhanced
envelope to obtain a bare hardware hash. The existing faithful-only connection
admission guard remains in place.
