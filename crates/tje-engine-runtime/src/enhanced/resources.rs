//! Immutable, rebuildable ROM resources. Only their authenticated manifest is
//! authoritative; decoded frames/catalogues are never copied into a snapshot.
use super::{Sidecar, context::error};
use bincode::{Decode, Encode};
use sha1::{Digest, Sha1};
use std::any::Any;
use tje_game::Result;

pub trait Resource: Any + Send + Sync {
    const KEY: &'static str;
    const VERSION: u32;
    /// Authenticate actual normalized imported resources and ROM/config identity.
    fn identity(&self) -> [u8; 32];
    /// Validate decoded hardware and sidecars together before either commits.
    fn validate_restore(
        &self,
        _states: &super::PreparedSidecars,
        _hardware: &super::SnapshotView<'_>,
    ) -> Result<()> {
        Ok(())
    }
}
struct Entry {
    key: &'static str,
    version: u32,
    identity: [u8; 32],
    value: Box<dyn Any + Send + Sync>,
    validate_restore:
        fn(&dyn Any, &super::PreparedSidecars, &super::SnapshotView<'_>) -> Result<()>,
}
#[derive(Default)]
pub struct Resources {
    entries: Vec<Entry>,
}
impl Resources {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn register<T: Resource>(&mut self, resource: T) -> Result<()> {
        if T::KEY.is_empty() || T::KEY.len() > 128 || T::VERSION == 0 || self.entries.len() >= 64 {
            return Err(error(
                "enhanced resources need bounded unique keys and nonzero versions",
            ));
        }
        let index = self
            .entries
            .binary_search_by_key(&T::KEY, |entry| entry.key)
            .err()
            .ok_or_else(|| error("enhanced resource key is already registered"))?;
        let identity = resource.identity();
        fn validate<T: Resource>(
            value: &dyn Any,
            states: &super::PreparedSidecars,
            hardware: &super::SnapshotView<'_>,
        ) -> Result<()> {
            value
                .downcast_ref::<T>()
                .ok_or_else(|| error("immutable resource type changed"))?
                .validate_restore(states, hardware)
        }
        self.entries.insert(
            index,
            Entry {
                key: T::KEY,
                version: T::VERSION,
                identity,
                value: Box::new(resource),
                validate_restore: validate::<T>,
            },
        );
        Ok(())
    }
    pub fn get<T: Resource>(&self) -> Result<&T> {
        self.entries
            .binary_search_by_key(&T::KEY, |entry| entry.key)
            .ok()
            .and_then(|index| self.entries[index].value.downcast_ref::<T>())
            .ok_or_else(|| error("enhanced immutable resource is not registered with this type"))
    }
    pub(super) fn validate_restore(
        &self,
        states: &super::PreparedSidecars,
        hardware: &super::SnapshotView<'_>,
    ) -> Result<()> {
        for entry in &self.entries {
            (entry.validate_restore)(&*entry.value, states, hardware)?;
        }
        Ok(())
    }
    pub(super) fn manifest(&self) -> Result<Manifest> {
        if self.entries.is_empty() {
            return Err(error("cannot install an empty enhanced resource set"));
        }
        let entries = self
            .entries
            .iter()
            .map(|entry| Identity {
                key: entry.key.into(),
                version: entry.version,
                identity: entry.identity,
            })
            .collect();
        let mut manifest = Manifest {
            entries,
            digest: [0; 20],
        };
        manifest.digest = manifest.calculate();
        manifest.validate()?;
        Ok(manifest)
    }
}
#[derive(Encode, Decode)]
struct Identity {
    key: String,
    version: u32,
    identity: [u8; 32],
}
#[derive(Encode, Decode)]
pub(super) struct Manifest {
    entries: Vec<Identity>,
    digest: [u8; 20],
}
impl Manifest {
    fn calculate(&self) -> [u8; 20] {
        let mut hash = Sha1::new();
        hash.update(b"tje/enhanced/resources/1\0");
        for entry in &self.entries {
            hash.update((entry.key.len() as u16).to_be_bytes());
            hash.update(entry.key.as_bytes());
            hash.update(entry.version.to_be_bytes());
            hash.update(entry.identity);
        }
        hash.finalize().into()
    }
}
impl Sidecar for Manifest {
    const KEY: &'static str = "tje/enhanced/resources";
    const VERSION: u32 = 1;
    fn validate(&self) -> Result<()> {
        if self.entries.is_empty()
            || self.entries.len() > 64
            || self
                .entries
                .windows(2)
                .any(|pair| pair[0].key >= pair[1].key)
            || self
                .entries
                .iter()
                .any(|entry| entry.key.is_empty() || entry.key.len() > 128 || entry.version == 0)
            || self.calculate() != self.digest
        {
            return Err(error("invalid enhanced immutable-resource manifest"));
        }
        Ok(())
    }
    fn configuration_identity(&self) -> &[u8] {
        &self.digest
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Example(u8);
    impl Resource for Example {
        const KEY: &'static str = "test/immutable";
        const VERSION: u32 = 1;
        fn identity(&self) -> [u8; 32] {
            [self.0; 32]
        }
    }
    #[test]
    fn resource_manifest_authenticates_actual_identity_without_resource_payload() {
        let mut resources = Resources::new();
        resources.register(Example(7)).unwrap();
        assert_eq!(resources.get::<Example>().unwrap().0, 7);
        assert!(resources.register(Example(8)).is_err());
        let manifest = resources.manifest().unwrap();
        let encoded = bincode::encode_to_vec(&manifest, super::super::sidecar::codec()).unwrap();
        let (mut decoded, _): (Manifest, _) =
            bincode::decode_from_slice(&encoded, super::super::sidecar::codec()).unwrap();
        decoded.entries[0].identity[0] ^= 1;
        assert!(decoded.validate().is_err());
    }
    #[derive(Encode, Decode)]
    struct Count(u32);
    impl Sidecar for Count {
        const KEY: &'static str = "test/count";
        const VERSION: u32 = 1;
    }
    #[test]
    fn state_borrow_is_disjoint_and_changed_resource_restore_rejects() {
        fn registry(value: u8) -> super::super::Sidecars {
            let mut resources = Resources::new();
            resources.register(Example(value)).unwrap();
            let mut state = super::super::Sidecars::default();
            state.register(Count(0)).unwrap();
            state.install_resources(resources).unwrap();
            state
        }
        let mut first = registry(7);
        let (count, resource) = first.state_and_resource::<Count, Example>().unwrap();
        count.0 += u32::from(resource.0);
        assert_eq!(first.get::<Count>().unwrap().0, 7);
        let second = registry(8);
        assert!(second.prepare(first.images().unwrap()).is_err());
        assert_eq!(second.get::<Count>().unwrap().0, 0);
        assert_eq!(second.resource::<Example>().unwrap().0, 8);
        let mut resources = Resources::new();
        resources.register(Example(7)).unwrap();
        assert!(first.install_resources(resources).is_err());
    }

    #[test]
    fn catalogue_manifest_mismatch_rejects_before_real_hardware_restore() {
        use crate::{Backend, Bus, CpuState, Engine, StepResult};
        use genesis_config::GenesisEmulatorConfig;
        use std::convert::Infallible;
        use tje_mem::{AddressSpace, Revision};
        use tje_native_platform::DeviceRuntime;
        // Authored clock workload; this is an atomic-envelope test, not gameplay.
        struct Clock;
        impl Backend for Clock {
            type Error = Infallible;
            fn execute(
                &mut self,
                cpu: &mut CpuState,
                bus: &mut dyn Bus,
            ) -> std::result::Result<StepResult, Infallible> {
                cpu.d[0] = cpu.d[0].wrapping_add(1);
                bus.write16(0xff0100, cpu.d[0] as u16);
                Ok(StepResult {
                    cycles: 44,
                    ..StepResult::default()
                })
            }
        }
        struct HardwareGuard;
        impl Resource for HardwareGuard {
            const KEY: &'static str = "test/hardware-guard";
            const VERSION: u32 = 1;
            fn identity(&self) -> [u8; 32] {
                [19; 32]
            }
            fn validate_restore(
                &self,
                _: &super::super::PreparedSidecars,
                hardware: &super::super::SnapshotView<'_>,
            ) -> Result<()> {
                if hardware.ram[0x200] != 0 {
                    return Err(error("authored hardware resource invariant"));
                }
                Ok(())
            }
        }
        fn set(value: u8) -> Resources {
            let mut resources = Resources::new();
            resources.register(Example(value)).unwrap();
            resources.register(HardwareGuard).unwrap();
            resources
        }
        for revision in [Revision::Rev00, Revision::Rev02] {
            let mut bytes = vec![0; 512];
            bytes[..4].copy_from_slice(&0xffff00_u32.to_be_bytes());
            bytes[4..8].copy_from_slice(&0x100_u32.to_be_bytes());
            bytes[0x100..0x102].copy_from_slice(&0x4e71_u16.to_be_bytes());
            let mut engine = Engine::from_devices(
                DeviceRuntime::new(
                    AddressSpace::new(bytes, revision),
                    &GenesisEmulatorConfig::default(),
                ),
                Clock,
            );
            let mut source = super::super::EnhancedMode::new(super::super::DEBUG_PROFILE, revision)
                .unwrap()
                .with_resources(set(7))
                .unwrap();
            source
                .play_frame(&mut engine, super::super::EnhancedInputs::default())
                .unwrap();
            let image = source.snapshot(&engine).unwrap();
            let mut target = super::super::EnhancedMode::new(super::super::DEBUG_PROFILE, revision)
                .unwrap()
                .with_resources(set(8))
                .unwrap();
            let hardware = engine.snapshot().unwrap();
            let before = target.snapshot(&engine).unwrap();
            assert!(target.restore(&mut engine, &image).is_err());
            assert_eq!(engine.snapshot().unwrap().as_bytes(), hardware.as_bytes());
            assert_eq!(
                target.snapshot(&engine).unwrap().as_bytes(),
                before.as_bytes()
            );
            let mut same = super::super::EnhancedMode::new(super::super::DEBUG_PROFILE, revision)
                .unwrap()
                .with_resources(set(7))
                .unwrap();
            engine.devices_mut().memory_mut().ram_mut()[0x200] = 1;
            let bad_hardware = source.snapshot(&engine).unwrap();
            engine.devices_mut().memory_mut().ram_mut()[0x200] = 0;
            let before = same.snapshot(&engine).unwrap();
            assert!(same.restore(&mut engine, &bad_hardware).is_err());
            assert_eq!(
                same.snapshot(&engine).unwrap().as_bytes(),
                before.as_bytes()
            );
            same.restore(&mut engine, &image).unwrap();
            assert!(source.with_resources(set(7)).is_err());
        }
        assert!(
            super::super::EnhancedMode::default()
                .with_resources(set(7))
                .is_err()
        );
    }
}
