//! Public, conservative ownership inventory and the pre-mutation admission gate.
//! A complete inventory is deliberately distinct from implemented gameplay.
use crate::{BehaviorId, Error, Result};
use bincode::{Decode, Encode};
use sha2::{Digest, Sha256};
use tje_game::{FunctionId, exits::ExitContract};
use tje_mem::Revision;

#[path = "ownership_generated.rs"]
mod generated;
pub use generated::IDENTITY;
pub const BODY_MANIFEST_VERSION: u16 = 1;
pub const FUNCTION_COUNT: usize = 648;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OriginalAbi {
    Registers,
    ExceptionFrame,
    CheckedAudioResource,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    ModeOwned(&'static str),
    ReadOnly(OriginalAbi),
    DeviceAudio(OriginalAbi),
}
#[derive(Debug)]
pub struct Node {
    pub function: FunctionId,
    pub body_version: u16,
    pub disposition: Disposition,
    pub references: &'static [FunctionId],
    /// Conservative all-function edge, never a license to execute an unknown address.
    pub indirect: bool,
}

pub fn nodes() -> &'static [Node] {
    generated::NODES
}
pub fn roots() -> &'static [FunctionId] {
    generated::ROOTS
}
pub fn irq_roots() -> &'static [FunctionId] {
    generated::IRQ_ROOTS
}
pub fn node(function: FunctionId) -> Option<&'static Node> {
    nodes().iter().find(|entry| entry.function == function)
}
fn index(function: FunctionId) -> Result<usize> {
    nodes()
        .iter()
        .position(|entry| entry.function == function)
        .ok_or(Error::Ownership("callee has no explicit disposition"))
}

/// Validates metadata, callback-reference and IRQ coverage before any live state.
pub fn validate_manifest() -> Result<()> {
    if nodes().len() != FUNCTION_COUNT || tje_game::functions().count() != FUNCTION_COUNT {
        return Err(Error::Ownership("function catalogue extent changed"));
    }
    for (position, (entry, metadata)) in nodes().iter().zip(tje_game::functions()).enumerate() {
        if entry.function != metadata.id
            || entry.body_version == 0
            || nodes()[..position]
                .iter()
                .any(|previous| previous.function == entry.function)
        {
            return Err(Error::Ownership(
                "manifest identities or body versions are invalid",
            ));
        }
        for &callee in metadata.callees.iter().chain(entry.references) {
            index(callee)?;
        }
        match entry.disposition {
            Disposition::ModeOwned("") => {
                return Err(Error::Ownership("mode-owned function has no service owner"));
            }
            Disposition::ReadOnly(_) if metadata.subsystem != "runtime" => {
                return Err(Error::Ownership("unreviewed read-only original boundary"));
            }
            Disposition::DeviceAudio(OriginalAbi::ExceptionFrame)
                if tje_game::exits::contract(entry.function) != ExitContract::ExceptionReturn =>
            {
                return Err(Error::Ownership(
                    "IRQ wrapper does not have an RTE contract",
                ));
            }
            Disposition::DeviceAudio(OriginalAbi::CheckedAudioResource)
                if metadata.subsystem != "audio" =>
            {
                return Err(Error::Ownership(
                    "device/audio original consumes an unowned subsystem",
                ));
            }
            _ => {}
        }
    }
    for &root in roots().iter().chain(irq_roots()) {
        index(root)?;
    }
    let reachable = closure(roots())?;
    let irq = closure(irq_roots())?;
    if !reachable.iter().all(|&present| present) || !irq.iter().all(|&present| present) {
        return Err(Error::Ownership(
            "conservative callback/IRQ closure is incomplete",
        ));
    }
    Ok(())
}

/// Admission-only bounded work; no heap allocation and no pointer chasing in play.
pub fn closure(roots: &[FunctionId]) -> Result<[bool; FUNCTION_COUNT]> {
    let mut reachable = [false; FUNCTION_COUNT];
    for &root in roots {
        reachable[index(root)?] = true;
    }
    loop {
        let mut changed = false;
        for (position, entry) in nodes().iter().enumerate() {
            if !reachable[position] {
                continue;
            }
            if entry.indirect {
                reachable.fill(true);
                return Ok(reachable);
            }
            for &target in entry
                .function
                .metadata()
                .callees
                .iter()
                .chain(entry.references)
            {
                let destination = index(target)?;
                changed |= !reachable[destination];
                reachable[destination] = true;
            }
        }
        if !changed {
            return Ok(reachable);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Encode, Decode)]
pub enum CallbackFamily {
    EntityMotion,
    EarthlingAbility,
    EarthlingContact,
    VBlank,
    Music,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Encode, Decode)]
pub struct TypedCallback {
    pub family: CallbackFamily,
    pub behavior: BehaviorId,
}
impl TypedCallback {
    pub fn function(self) -> Result<FunctionId> {
        if !self.behavior.valid() {
            return Err(Error::Ownership("invalid typed behavior identity"));
        }
        let entry = nodes()
            .get(usize::from(self.behavior.get()))
            .ok_or(Error::Ownership("behavior is not in the body manifest"))?;
        if !callback_allowed(self.family, entry) {
            return Err(Error::Ownership(
                "callback family and service owner disagree",
            ));
        }
        Ok(entry.function)
    }
}
fn callback_allowed(family: CallbackFamily, entry: &Node) -> bool {
    matches!(
        (family, entry.disposition),
        (
            CallbackFamily::Music,
            Disposition::DeviceAudio(OriginalAbi::CheckedAudioResource)
        ) | (
            CallbackFamily::EarthlingAbility | CallbackFamily::EarthlingContact,
            Disposition::ModeOwned("earthlings"),
        ) | (
            CallbackFamily::VBlank,
            Disposition::ModeOwned("core" | "elevator" | "cutscenes" | "jamout"),
        ) | (
            CallbackFamily::EntityMotion,
            Disposition::ModeOwned(
                "player"
                    | "earthlings"
                    | "sprites"
                    | "presents"
                    | "elevator"
                    | "ship"
                    | "cutscenes"
                    | "jamout",
            ),
        )
    )
}

/// Importer boundary: exact entry addresses only. Interior/unknown callbacks fail.
/// Addresses are resolved once and never serialized in authoritative state.
pub fn resolve_callback(
    revision: Revision,
    family: CallbackFamily,
    address: u32,
) -> Result<TypedCallback> {
    let function = tje_game::function_at(revision, address)
        .ok_or(Error::Ownership("unknown or interior ROM callback target"))?;
    let behavior = BehaviorId::new(index(function)? as u16).ok_or(Error::Ownership(
        "callback identity exceeds the finite catalogue",
    ))?;
    let callback = TypedCallback { family, behavior };
    callback.function()?;
    if tje_game::exits::contract(function) != ExitContract::Rts {
        return Err(Error::Ownership("ROM callback does not return by RTS"));
    }
    Ok(callback)
}

/// Hashes the actual imported, sorted callback set, not a caller-supplied digest.
pub fn resource_identity(rom_sha1: [u8; 20], callbacks: &[TypedCallback]) -> Result<[u8; 32]> {
    if callbacks.len() > 4096 || callbacks.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(Error::Ownership(
            "callback catalogue is not bounded/unique/canonical",
        ));
    }
    let mut hash = Sha256::new();
    hash.update(b"tje-simulation/resources/1\0");
    hash.update(IDENTITY);
    hash.update(rom_sha1);
    for &callback in callbacks {
        callback.function()?;
        hash.update([callback.family as u8]);
        hash.update(callback.behavior.get().to_be_bytes());
    }
    Ok(hash.finalize().into())
}

/// Reviewed service receipts are separate from the inventory. An empty S1
/// registry is intentional and cannot be supplied/replaced by untrusted input.
const IMPLEMENTED_SERVICES: &[(&str, u16)] = &[];

/// This gate has no machine, registry, RAM, reset or mutable sidecar argument.
/// Therefore a rejected ownership request cannot partially install a profile.
pub fn admit(manifest_identity: [u8; 32], callbacks: &[TypedCallback]) -> Result<()> {
    if manifest_identity != IDENTITY {
        return Err(Error::Ownership("ownership manifest identity mismatch"));
    }
    validate_manifest()?;
    for &callback in callbacks {
        callback.function()?;
    }
    for entry in nodes() {
        if let Disposition::ModeOwned(service) = entry.disposition
            && !IMPLEMENTED_SERVICES.contains(&(service, entry.body_version))
        {
            return Err(Error::Unimplemented {
                function: entry.function,
                service,
            });
        }
    }
    Err(Error::Ownership(
        "S6 native/browser admission evidence is not installed",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tje_game::ids;

    #[test]
    fn complete_transitive_and_irq_inventory_does_not_admit_gameplay() {
        validate_manifest().unwrap();
        assert!(closure(roots()).unwrap().iter().all(|&value| value));
        assert!(closure(irq_roots()).unwrap().iter().all(|&value| value));
        assert!(matches!(
            admit(IDENTITY, &[]),
            Err(Error::Unimplemented { .. })
        ));
        assert!(admit([0; 32], &[]).is_err());
    }

    #[test]
    fn resolves_both_revisions_without_serializing_executable_pointers() {
        for revision in [Revision::Rev00, Revision::Rev02] {
            let address = ids::HANDLE_V_BLANK.metadata().address(revision);
            let callback = resolve_callback(revision, CallbackFamily::VBlank, address).unwrap();
            assert_eq!(callback.function().unwrap(), ids::HANDLE_V_BLANK);
            assert!(resolve_callback(revision, CallbackFamily::VBlank, address + 2).is_err());
            assert!(resolve_callback(revision, CallbackFamily::Music, address).is_err());
            assert!(resolve_callback(revision, CallbackFamily::EntityMotion, 0xffa25a).is_err());
            assert!(
                resolve_callback(
                    revision,
                    CallbackFamily::VBlank,
                    ids::LEVEL6_ISR.metadata().address(revision)
                )
                .is_err()
            );
        }
    }

    #[test]
    fn imported_catalogue_identity_is_canonical_and_revision_bound() {
        let callback = resolve_callback(
            Revision::Rev02,
            CallbackFamily::VBlank,
            ids::HANDLE_V_BLANK.metadata().address(Revision::Rev02),
        )
        .unwrap();
        assert_ne!(
            resource_identity([1; 20], &[callback]).unwrap(),
            resource_identity([2; 20], &[callback]).unwrap()
        );
        assert!(resource_identity([1; 20], &[callback, callback]).is_err());
    }
}
