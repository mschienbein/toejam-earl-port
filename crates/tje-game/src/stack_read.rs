//! Shared declarations for readable-stack failures and cited original bugs.
//!
//! A declaration is not permission to borrow a callee's spills. Backends must
//! validate current owner/frame/lifetime and profile-required input evidence;
//! paired checks additionally require the actual original semantic witness.
use crate::{FunctionId, call_frame::FrameState};
use tje_mem::written_ram::canonical_ram_byte;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalWidth {
    Byte,
    Word,
    Long,
}
impl LocalWidth {
    pub const fn bytes(self) -> u8 {
        match self {
            Self::Byte => 1,
            Self::Word => 2,
            Self::Long => 4,
        }
    }
}

/// One deliberate original uninitialized-local read, relative to its OWN entry.
/// The citation must identify the matching source/listing offset and width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UninitializedLocalRead {
    pub owner: FunctionId,
    pub frame: FrameState,
    pub offset: i32,
    pub width: LocalWidth,
    pub citation: &'static str,
}
impl UninitializedLocalRead {
    pub fn address(self) -> u32 {
        self.frame.entry_sp.wrapping_add_signed(self.offset)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reader {
    RustMemory,
    HardwareRam,
    VdpDma,
    ExplicitLocal,
    RootEntry,
    DmaBoundary,
}
impl Reader {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RustMemory => "rust_memory",
            Self::HardwareRam => "hardware_ram",
            Self::VdpDma => "vdp_dma",
            Self::ExplicitLocal => "explicit_uninitialized_local",
            Self::RootEntry => "root_entry",
            Self::DmaBoundary => "owned_dma_boundary",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnsupportedReason {
    Unwritten,
    ExplicitReadUnavailable,
    InvalidExplicitLocal,
    AlreadyWritten,
    InterveningNonOwnWrite,
    UnwindUnavailable,
    ObserverUnavailable,
    InvalidRootStack,
    ScopeExhausted,
    MissingOriginalFrameWitness,
    PairedInputMismatch,
    UncitedInputDependence,
    GuardCapacityExhausted,
    PendingOwnedDma,
    DiscardedOwnedDma,
    UnknownDmaOrigin,
    StaleDmaLease,
}
impl UnsupportedReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unwritten => "unwritten_stack_read",
            Self::ExplicitReadUnavailable => "explicit_local_unavailable",
            Self::InvalidExplicitLocal => "invalid_explicit_local",
            Self::AlreadyWritten => "explicit_local_already_written",
            Self::InterveningNonOwnWrite => "intervening_non_own_write",
            Self::UnwindUnavailable => "unwind_capability_unavailable",
            Self::ObserverUnavailable => "ram_dma_observer_unavailable",
            Self::InvalidRootStack => "invalid_root_stack",
            Self::ScopeExhausted => "scope_identity_exhausted",
            Self::MissingOriginalFrameWitness => "missing_original_frame_witness",
            Self::PairedInputMismatch => "paired_explicit_input_mismatch",
            Self::UncitedInputDependence => "uncited_overlapping_input_dependence",
            Self::GuardCapacityExhausted => "guard_capacity_ceiling",
            Self::PendingOwnedDma => "pending_owned_dma",
            Self::DiscardedOwnedDma => "discarded_owned_dma",
            Self::UnknownDmaOrigin => "unknown_dma_origin",
            Self::StaleDmaLease => "stale_dma_lease",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unsupported {
    pub root: FunctionId,
    pub entry_sp: u32,
    pub raw_address: u32,
    pub address: u32,
    pub width: u8,
    pub reader: Reader,
    pub reason: UnsupportedReason,
}
impl std::fmt::Display for Unsupported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Unsupported readable stack: root={} entry_sp={:06X} reader={} address={:06X} raw={:06X} width={} reason={}",
            self.root.metadata().name,
            self.entry_sp,
            self.reader.as_str(),
            self.address,
            self.raw_address,
            self.width,
            self.reason.as_str()
        )
    }
}
impl std::error::Error for Unsupported {}
impl Unsupported {
    pub fn capability(root: FunctionId, entry_sp: u32, reason: UnsupportedReason) -> Self {
        Self::new(root, entry_sp, entry_sp, 0, Reader::RootEntry, reason)
    }
    pub fn unavailable(read: UninitializedLocalRead) -> Self {
        Self::new(
            read.owner,
            read.frame.entry_sp,
            read.address(),
            read.width.bytes(),
            Reader::ExplicitLocal,
            UnsupportedReason::ExplicitReadUnavailable,
        )
    }
    pub fn paired(
        root: FunctionId,
        entry_sp: u32,
        read: UninitializedLocalRead,
        reason: UnsupportedReason,
    ) -> Self {
        Self::new(
            root,
            entry_sp,
            read.address(),
            read.width.bytes(),
            Reader::ExplicitLocal,
            reason,
        )
    }
    fn new(
        root: FunctionId,
        entry_sp: u32,
        raw: u32,
        width: u8,
        reader: Reader,
        reason: UnsupportedReason,
    ) -> Self {
        Self {
            root,
            entry_sp,
            raw_address: raw & tje_mem::ADDRESS_MASK,
            address: canonical_ram_byte(raw).unwrap_or(raw & tje_mem::ADDRESS_MASK),
            width,
            reader,
            reason,
        }
    }
}

mod guard;
pub use guard::{DmaMetrics, DmaRetirement, GUARD_CAPACITY, Guard, Namespace, OriginalSegment};

#[cfg(test)]
mod api_tests {
    use super::*;
    use crate::{Context, Dispatcher, Error, Platform, ids};
    use tje_mem::{Bus, Revision};

    struct NoMemory(Option<Error>);
    impl Bus for NoMemory {
        fn read_u8(&self, _: u32) -> u8 {
            panic!("default residue API must never read RAM")
        }
        fn write_u8(&mut self, _: u32, _: u8) {
            panic!("default residue API must never write RAM")
        }
    }
    impl tje_mem::Platform for NoMemory {
        fn write_vdp_control(&mut self, _: u16) {
            panic!("no device effects")
        }
        fn write_vdp_data(&mut self, _: u16) {
            panic!("no device effects")
        }
    }
    impl Platform for NoMemory {
        fn read_device_u8(&mut self, _: u32) -> u8 {
            panic!("no device reads")
        }
        fn read_device_u16(&mut self, _: u32) -> u16 {
            panic!("no device reads")
        }
        fn write_device_u8(&mut self, _: u32, _: u8) {
            panic!("no device writes")
        }
        fn write_device_u16(&mut self, _: u32, _: u16) {
            panic!("no device writes")
        }
        fn check_mmio_contract(&self) -> crate::Result<()> {
            self.0.clone().map_or(Ok(()), Err)
        }
    }
    impl Context for NoMemory {
        fn revision(&self) -> Revision {
            Revision::Rev02
        }
        fn data_register(&self, _: usize) -> u32 {
            panic!("no register reads")
        }
        fn set_data_register(&mut self, _: usize, _: u32) {
            panic!("no register writes")
        }
        fn address_register(&self, _: usize) -> u32 {
            panic!("no register reads")
        }
        fn set_address_register(&mut self, _: usize, _: u32) {
            panic!("no register writes")
        }
        fn status_register(&self) -> u16 {
            panic!("no register reads")
        }
        fn set_status_register(&mut self, _: u16) {
            panic!("no register writes")
        }
        fn dispatcher(&self) -> Dispatcher {
            panic!("no dispatch")
        }
        fn call_original(&mut self, _: FunctionId, _: &[u32]) -> crate::Result<u32> {
            panic!("no original fallback")
        }
    }
    const fn request(width: LocalWidth) -> UninitializedLocalRead {
        FrameState::new(0xffff00).uninitialized_local(
            ids::GET_RANDOM_NUMBER,
            -8,
            width,
            "authored own-frame offset/width declaration",
        )
    }

    #[test]
    fn descriptor_retains_width_owner_frame_offset_and_citation_without_permission() {
        for (width, bytes) in [
            (LocalWidth::Byte, 1),
            (LocalWidth::Word, 2),
            (LocalWidth::Long, 4),
        ] {
            let read = request(width);
            assert_eq!(read.width.bytes(), bytes);
            assert_eq!(read.owner, ids::GET_RANDOM_NUMBER);
            assert_eq!(read.frame, FrameState::new(0xffff00));
            assert_eq!(read.offset, -8);
            assert_eq!(read.address(), 0xfffef8);
            assert_eq!(read.citation, "authored own-frame offset/width declaration");
        }
    }

    #[test]
    fn unsupported_default_never_fetches_any_operand_or_changes_state() {
        for width in [LocalWidth::Byte, LocalWidth::Word, LocalWidth::Long] {
            let read = request(width);
            let error = NoMemory(None).read_uninitialized_local(read).unwrap_err();
            let failure = error.unsupported_stack_read().unwrap();
            assert_eq!(failure.root, read.owner);
            assert_eq!(failure.entry_sp, read.frame.entry_sp);
            assert_eq!(failure.raw_address, read.address());
            assert_eq!(failure.address, read.address());
            assert_eq!(failure.width, width.bytes());
            assert_eq!(failure.reader, Reader::ExplicitLocal);
            assert_eq!(failure.reason, UnsupportedReason::ExplicitReadUnavailable);
        }
    }

    #[test]
    fn typed_failure_preserves_frame_and_never_mints_control_transfer() {
        let expected = Unsupported::unavailable(request(LocalWidth::Word));
        let error = Error::from(expected);
        assert!(error.is_failed_execution());
        assert!(error.preserves_call_frame());
        assert_eq!(error.unsupported_stack_read(), Some(expected));
        assert!(!matches!(error, Error::ControlTransfer(_)));
        assert!(
            error
                .to_string()
                .contains("reader=explicit_uninitialized_local")
        );
    }

    #[test]
    fn default_api_preserves_an_earlier_fatal_error() {
        let expected = Error::from(Unsupported::unavailable(request(LocalWidth::Byte)));
        let error = NoMemory(Some(expected.clone()))
            .read_uninitialized_local(request(LocalWidth::Long))
            .unwrap_err();
        assert_eq!(error, expected);
    }
}
