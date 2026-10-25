//! Source-requirement fixtures, not audio execution/admission evidence.
//! G/src/asm/z80bus.s:10-39, sound_pcm.s:10-131, sound_psg.s:9-45.
//! ROM symbols/sample bytes remain runtime inputs; none are copied here.
use super::super::*;
use super::{delay, device};
use crate::ids;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryForm {
    Ordinary,
    LazyFramed,
    ExistingOwnerFragment(FunctionId),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Path {
    UploadAcceptedInclusiveRomCopyAndTwoMulu,
    UploadTimeoutRetainsBusRequest,
    PendingAcceptedPacketAndRamConsumption,
    PendingZeroHeaderStillCommandAndCooldownFive,
    PendingTimeoutRetainsPcmPointer,
    ImmediateCooldownSkipsAllLaterReads,
    ImmediateInhibitSkipsAllBusReads,
    ImmediatePsgActiveSkipsArgument,
    ImmediateTimeoutSkipsArgument,
    ImmediateAcceptedPreservesAllRegistersAndPendingRam,
    PsgAcceptedCommandThenStatusFifteen,
    PsgTimeoutConsumesOnlyPendingEffect,
    RetryReleaseDelayYieldAndReacquire,
    FragmentFallsThroughExistingOwnerFrame,
}
struct RequirementFixture {
    entry: FunctionId,
    source: &'static str,
    form: EntryForm,
    busack_owner: FunctionId,
    busack_offset: i32,
    preserved_data: u8,
    preserved_address: u8,
    retry: Option<RetryDelayKind>,
    paths: &'static [Path],
}
const PCM_PATHS: &[Path] = &[
    Path::PendingAcceptedPacketAndRamConsumption,
    Path::PendingZeroHeaderStillCommandAndCooldownFive,
    Path::PendingTimeoutRetainsPcmPointer,
    Path::RetryReleaseDelayYieldAndReacquire,
];
const PSG_PATHS: &[Path] = &[
    Path::PsgAcceptedCommandThenStatusFifteen,
    Path::PsgTimeoutConsumesOnlyPendingEffect,
    Path::RetryReleaseDelayYieldAndReacquire,
];
const FIXTURES: &[RequirementFixture] = &[
    RequirementFixture {
        entry: ids::LOAD_SOUND_DRIVER_TO_Z80,
        source: "src/asm/z80bus.s:10-39",
        form: EntryForm::Ordinary,
        busack_owner: ids::LOAD_SOUND_DRIVER_TO_Z80,
        busack_offset: 0x1a,
        preserved_data: 0xfc,
        preserved_address: 0x7c,
        retry: None,
        paths: &[
            Path::UploadAcceptedInclusiveRomCopyAndTwoMulu,
            Path::UploadTimeoutRetainsBusRequest,
        ],
    },
    RequirementFixture {
        entry: ids::AUDIO_RETRY_PENDING_PCM_MAILBOX,
        source: "src/asm/sound_pcm.s:10-15",
        form: EntryForm::ExistingOwnerFragment(ids::AUDIO_SUBMIT_PENDING_PCM_SAMPLE),
        busack_owner: ids::AUDIO_SUBMIT_PENDING_PCM_SAMPLE,
        busack_offset: 0x18,
        preserved_data: 0xf8,
        preserved_address: 0x7e,
        retry: Some(RetryDelayKind::Pcm24),
        paths: &[
            Path::FragmentFallsThroughExistingOwnerFrame,
            Path::RetryReleaseDelayYieldAndReacquire,
            Path::PendingTimeoutRetainsPcmPointer,
            Path::PendingZeroHeaderStillCommandAndCooldownFive,
        ],
    },
    RequirementFixture {
        entry: ids::AUDIO_SUBMIT_PENDING_PCM_SAMPLE,
        source: "src/asm/sound_pcm.s:21-68",
        form: EntryForm::Ordinary,
        busack_owner: ids::AUDIO_SUBMIT_PENDING_PCM_SAMPLE,
        busack_offset: 0x18,
        preserved_data: 0xf8,
        preserved_address: 0x7e,
        retry: Some(RetryDelayKind::Pcm24),
        paths: PCM_PATHS,
    },
    RequirementFixture {
        entry: ids::AUDIO_TRY_SUBMIT_SEQUENCER_PCM_SAMPLE,
        source: "src/asm/sound_pcm.s:74-131",
        form: EntryForm::LazyFramed,
        busack_owner: ids::AUDIO_TRY_SUBMIT_SEQUENCER_PCM_SAMPLE,
        busack_offset: 0x44,
        preserved_data: 0xff,
        preserved_address: 0x7f,
        retry: Some(RetryDelayKind::Pcm24),
        paths: &[
            Path::ImmediateCooldownSkipsAllLaterReads,
            Path::ImmediateInhibitSkipsAllBusReads,
            Path::ImmediatePsgActiveSkipsArgument,
            Path::ImmediateTimeoutSkipsArgument,
            Path::ImmediateAcceptedPreservesAllRegistersAndPendingRam,
            Path::RetryReleaseDelayYieldAndReacquire,
        ],
    },
    RequirementFixture {
        entry: ids::AUDIO_RETRY_PENDING_PSG_MAILBOX,
        source: "src/asm/sound_psg.s:9-14",
        form: EntryForm::ExistingOwnerFragment(ids::AUDIO_SUBMIT_PENDING_INDEXED_COMMAND),
        busack_owner: ids::AUDIO_SUBMIT_PENDING_INDEXED_COMMAND,
        busack_offset: 0x18,
        preserved_data: 0xf8,
        preserved_address: 0x7f,
        retry: Some(RetryDelayKind::Psg124),
        paths: &[
            Path::FragmentFallsThroughExistingOwnerFrame,
            Path::RetryReleaseDelayYieldAndReacquire,
            Path::PsgTimeoutConsumesOnlyPendingEffect,
        ],
    },
    RequirementFixture {
        entry: ids::AUDIO_SUBMIT_PENDING_INDEXED_COMMAND,
        source: "src/asm/sound_psg.s:20-45",
        form: EntryForm::Ordinary,
        busack_owner: ids::AUDIO_SUBMIT_PENDING_INDEXED_COMMAND,
        busack_offset: 0x18,
        preserved_data: 0xf8,
        preserved_address: 0x7f,
        retry: Some(RetryDelayKind::Psg124),
        paths: PSG_PATHS,
    },
];

/// Every original publication is a byte and CMD is LAST. These are protocol
/// addresses/roles, not extracted sample/driver payload or fabricated service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PacketValue {
    AddressLow,
    AddressMiddle,
    AddressHigh,
    LengthLow,
    LengthHigh,
    RedundantAddressHigh,
    EffectIdLow,
    Command(u8),
}
const PCM_PACKET: &[(u32, PacketValue)] = &[
    (0xa00044, PacketValue::AddressLow),
    (0xa00045, PacketValue::AddressMiddle),
    (0xa00046, PacketValue::AddressHigh),
    (0xa00048, PacketValue::LengthLow),
    (0xa00049, PacketValue::LengthHigh),
    (0xa00042, PacketValue::RedundantAddressHigh),
    (0xa00040, PacketValue::Command(2)),
];
const PSG_PACKET: &[(u32, PacketValue)] = &[
    (0xa00042, PacketValue::EffectIdLow),
    (0xa00040, PacketValue::Command(1)),
];
const RETRY_MANIFEST: &str = "sessions/audio-retry-perturbation-fixtures.json";
const RETRY_LABEL: &str =
    "labelled perturbation of a real state; not observed in natural play within a bounded search";
const RETRY_RECIPES: &[(Revision, FunctionId, &str)] = &[
    (
        Revision::Rev02,
        ids::AUDIO_SUBMIT_PENDING_PCM_SAMPLE,
        "rev02-audio-pcm-busy-window-owner-1",
    ),
    (
        Revision::Rev02,
        ids::AUDIO_SUBMIT_PENDING_INDEXED_COMMAND,
        "rev02-audio-psg-busy-window-owner-1",
    ),
    (
        Revision::Rev00,
        ids::AUDIO_SUBMIT_PENDING_PCM_SAMPLE,
        "rev00-audio-pcm-busy-window-owner-1",
    ),
    (
        Revision::Rev00,
        ids::AUDIO_SUBMIT_PENDING_INDEXED_COMMAND,
        "rev00-audio-psg-busy-window-owner-1",
    ),
];
const GUARDS: &[(&str, FunctionId)] = &[
    (
        "immediate/gates/cooldown-1/inhibit-0",
        ids::AUDIO_TRY_SUBMIT_SEQUENCER_PCM_SAMPLE,
    ),
    (
        "immediate/gates/cooldown--1/inhibit-0",
        ids::AUDIO_TRY_SUBMIT_SEQUENCER_PCM_SAMPLE,
    ),
    (
        "immediate/gates/cooldown--32768/inhibit-0",
        ids::AUDIO_TRY_SUBMIT_SEQUENCER_PCM_SAMPLE,
    ),
    (
        "immediate/gates/cooldown-0/inhibit-1",
        ids::AUDIO_TRY_SUBMIT_SEQUENCER_PCM_SAMPLE,
    ),
    (
        "immediate/gates/cooldown-0/inhibit--1",
        ids::AUDIO_TRY_SUBMIT_SEQUENCER_PCM_SAMPLE,
    ),
    (
        "immediate/actual-z80-psg-active/invalid-unused-sample",
        ids::AUDIO_TRY_SUBMIT_SEQUENCER_PCM_SAMPLE,
    ),
    ("upload/reset-denies-grant", ids::LOAD_SOUND_DRIVER_TO_Z80),
    (
        "pcm/reset-denies-19-polls",
        ids::AUDIO_SUBMIT_PENDING_PCM_SAMPLE,
    ),
    (
        "psg/reset-denies-19-polls",
        ids::AUDIO_SUBMIT_PENDING_INDEXED_COMMAND,
    ),
    (
        "immediate/reset-denies-19-polls",
        ids::AUDIO_TRY_SUBMIT_SEQUENCER_PCM_SAMPLE,
    ),
];

#[test]
fn every_audio_entry_keeps_real_assembly_abi_entry_form_and_revision_source() {
    assert_eq!(FIXTURES.len(), 6);
    let dispatcher = crate::Dispatcher::hybrid_with_overrides_and_frames(
        crate::audio::PENDING_TRANSLATIONS,
        crate::audio::PENDING_FRAMED_TRANSLATIONS,
    );
    for (index, fixture) in FIXTURES.iter().enumerate() {
        assert!(
            !FIXTURES[..index]
                .iter()
                .any(|other| other.entry == fixture.entry)
        );
        let metadata = fixture.entry.metadata();
        assert!(metadata.abi.strict_registers); // All D/A and CCR still compared, not only preservation masks.
        assert_eq!(metadata.abi.preserved_data, fixture.preserved_data);
        assert_eq!(metadata.abi.preserved_address, fixture.preserved_address);
        assert!(!fixture.source.is_empty());
        assert!(!fixture.paths.is_empty());
        let selected = dispatcher
            .resolve_entry(fixture.entry)
            .unwrap()
            .expect("canonical production or pending audio entry")
            .entry();
        match fixture.form {
            EntryForm::LazyFramed => {
                assert_eq!(metadata.abi.argument_count, 1);
                assert!(matches!(selected, crate::TranslatedEntry::Framed(_)));
            }
            EntryForm::ExistingOwnerFragment(owner) => {
                assert_eq!(metadata.kind, "fragment");
                assert!(
                    fixture
                        .paths
                        .contains(&Path::FragmentFallsThroughExistingOwnerFrame)
                );
                for revision in [Revision::Rev02, Revision::Rev00] {
                    assert_eq!(
                        metadata.address(revision) + 14,
                        owner.metadata().address(revision)
                    );
                }
            }
            EntryForm::Ordinary => {
                assert_eq!(metadata.abi.argument_count, 0);
                assert!(matches!(selected, crate::TranslatedEntry::Values(_)));
            }
        }
        for revision in [Revision::Rev02, Revision::Rev00] {
            let source = ControlSite {
                entry: fixture.busack_owner,
                byte_offset: fixture.busack_offset,
            };
            assert!(source.validate_shape().is_ok());
            assert!(source.address(revision) >= fixture.busack_owner.metadata().address(revision));
        }
    }
}
#[test]
fn six_requirements_use_only_device_controls_and_separate_typed_delay_not_cpu_privilege() {
    for fixture in FIXTURES {
        for revision in [Revision::Rev02, Revision::Rev00] {
            let take = device(fixture.entry, false, 0x100);
            assert_eq!(take.validate_shape(revision), Ok(()));
            if fixture.entry == ids::LOAD_SOUND_DRIVER_TO_Z80 {
                assert_eq!(fixture.retry, None);
                assert!(
                    fixture
                        .paths
                        .contains(&Path::UploadTimeoutRetainsBusRequest)
                );
                for value in [0, 0x100] {
                    assert_eq!(
                        device(fixture.entry, true, value).validate_shape(revision),
                        Ok(())
                    );
                }
            } else {
                let kind = fixture.retry.unwrap();
                let event = delay(fixture.entry, kind);
                assert_eq!(event.validate_shape(revision), Ok(()));
                assert!(
                    fixture
                        .paths
                        .contains(&Path::RetryReleaseDelayYieldAndReacquire)
                );
                // The phase1.5 scheduler yield is NOT a typed delay or retirement.
                assert_eq!(
                    event.span.retirement_end - event.span.retirement_start,
                    kind.retirements()
                );
            }
        }
    }
}
#[test]
fn bounded_busack_and_symbolic_packet_requirements_preserve_width_and_store_order() {
    let mut counter = 20u16;
    let mut reads = 0;
    loop {
        counter -= 1;
        if counter == 0 {
            break;
        }
        reads += 1;
    }
    assert_eq!(reads, 19); // Decrement BEFORE each full16-bit, explicitly sourced word read.
    assert_eq!(
        PCM_PACKET.last(),
        Some(&(0xa00040, PacketValue::Command(2)))
    );
    assert_eq!(
        PSG_PACKET.last(),
        Some(&(0xa00040, PacketValue::Command(1)))
    );
    assert_eq!(PCM_PACKET[5], (0xa00042, PacketValue::RedundantAddressHigh));
    assert_eq!(PCM_PACKET.len(), 7);
    assert_eq!(PSG_PACKET.len(), 2);
    // Header-zero remains a distinct accepted-path requirement, not a claim
    // that evaluating an authored equation executes the audio body.
    assert!(PCM_PATHS.contains(&Path::PendingZeroHeaderStillCommandAndCooldownFive));
}
#[test]
fn real_guard_and_authored_retry_references_do_not_become_natural_or_stronger_proofs() {
    assert_eq!(GUARDS.len(), 10);
    assert_eq!(
        GUARDS
            .iter()
            .filter(|(_, id)| *id == ids::AUDIO_TRY_SUBMIT_SEQUENCER_PCM_SAMPLE)
            .count(),
        7
    );
    assert_eq!(
        RETRY_MANIFEST,
        "sessions/audio-retry-perturbation-fixtures.json"
    );
    assert_eq!(
        RETRY_LABEL,
        "labelled perturbation of a real state; not observed in natural play within a bounded search"
    );
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut count = 0;
        let mut owners = 0;
        for &(r, owner, _) in RETRY_RECIPES {
            if r != revision {
                continue;
            }
            count += 1;
            owners |= if owner == ids::AUDIO_SUBMIT_PENDING_PCM_SAMPLE {
                1
            } else if owner == ids::AUDIO_SUBMIT_PENDING_INDEXED_COMMAND {
                2
            } else {
                4
            };
        }
        assert_eq!(count, 2);
        assert_eq!(owners, 3);
    }
    // Consume session SHA1/complete typed receipts from the manifest at proof
    // time. No source payload, fabricated immediate-retry recipe or snapshot is
    // copied here. Guard-only10/revision and authored original-positive4 are NOT
    // Rust success/retry/control-profile admission. Upload's separate corpus
    // proof does not approve the five mailbox entries or weaken these requirements.
    let dispatcher = crate::Dispatcher::hybrid_with_overrides_and_frames(
        crate::audio::PENDING_TRANSLATIONS,
        crate::audio::PENDING_FRAMED_TRANSLATIONS,
    );
    assert!(
        FIXTURES
            .iter()
            .all(|fixture| dispatcher.resolve_entry(fixture.entry).unwrap().is_some())
    );
}
