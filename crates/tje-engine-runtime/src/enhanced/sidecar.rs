use super::context::error;
use bincode::{Decode, Encode};
use sha1::{Digest, Sha1};
use std::any::Any;
use tje_game::Result;

pub(super) const MAX_STATE_BYTES: usize = 4 * 1024 * 1024;

pub(super) fn codec() -> impl bincode::config::Config {
    bincode::config::standard()
        .with_fixed_int_encoding()
        .with_little_endian()
        .with_limit::<MAX_STATE_BYTES>()
}

/// Deterministic extra-owned data. Bump VERSION when its canonical codec changes.
/// Include immutable configuration/course identity in the registered initial value.
pub trait Sidecar: Any + Send + Encode + Decode<()> {
    const KEY: &'static str;
    const VERSION: u32;

    /// Validate invariants after decoding, before any live state is changed.
    fn validate(&self) -> Result<()> {
        Ok(())
    }

    /// Immutable configuration digest, checked against the trusted registered
    /// initial value even for a checksum-rewritten snapshot. Empty means this
    /// sidecar has no immutable fields. Validate must authenticate the digest
    /// against its encoded configuration (not merely trust a stored digest).
    fn configuration_identity(&self) -> &[u8] {
        &[]
    }
}

trait Erased: Send {
    fn any(&self) -> &dyn Any;
    fn any_mut(&mut self) -> &mut dyn Any;
    fn encode(&self) -> Result<Vec<u8>>;
    fn configuration_identity(&self) -> &[u8];
}
impl<T: Sidecar> Erased for T {
    fn any(&self) -> &dyn Any {
        self
    }
    fn any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        bincode::encode_to_vec(self, codec()).map_err(|cause| error(&cause.to_string()))
    }
    fn configuration_identity(&self) -> &[u8] {
        Sidecar::configuration_identity(self)
    }
}

type DecodeState = fn(&[u8]) -> Result<Box<dyn Erased>>;
struct Entry {
    key: &'static str,
    version: u32,
    identity: [u8; 20],
    configuration: Vec<u8>,
    initial: Vec<u8>,
    state: Box<dyn Erased>,
    decode: DecodeState,
}

#[derive(Encode, Decode)]
pub(super) struct Image {
    key: String,
    version: u32,
    identity: [u8; 20],
    state: Vec<u8>,
}

/// A sorted, explicitly registered registry. No unregistered state is restored.
#[derive(Default)]
pub struct Sidecars {
    entries: Vec<Entry>,
    resources: Option<super::Resources>,
}
impl Sidecars {
    pub fn register<T: Sidecar>(&mut self, initial: T) -> Result<()> {
        if T::KEY.is_empty() || T::KEY.len() > 128 || T::VERSION == 0 {
            return Err(error(
                "sidecar needs a nonempty bounded key and nonzero version",
            ));
        }
        let index = match self
            .entries
            .binary_search_by_key(&T::KEY, |entry| entry.key)
        {
            Ok(_) => return Err(error("sidecar key is already registered")),
            Err(index) => index,
        };
        initial.validate()?;
        let configuration = Sidecar::configuration_identity(&initial);
        if configuration.len() > 128 {
            return Err(error(
                "sidecar immutable configuration identity exceeds 128 bytes",
            ));
        }
        let configuration = configuration.to_vec();
        let bytes = Erased::encode(&initial)?;
        let total: usize = self.entries.iter().map(|entry| entry.initial.len()).sum();
        if bytes.len().saturating_add(total) > MAX_STATE_BYTES || self.entries.len() >= 64 {
            return Err(error("enhanced sidecar registry exceeds its size limit"));
        }
        fn decode<T: Sidecar>(bytes: &[u8]) -> Result<Box<dyn Erased>> {
            let (state, used): (T, _) = bincode::decode_from_slice(bytes, codec())
                .map_err(|cause| error(&cause.to_string()))?;
            if used != bytes.len() {
                return Err(error("sidecar has trailing data"));
            }
            state.validate()?;
            Ok(Box::new(state))
        }
        // Validate the registered reset value now, not during a later reset.
        let decoded = decode::<T>(&bytes)?;
        if decoded.encode()? != bytes || decoded.configuration_identity() != configuration {
            return Err(error("sidecar initial codec is not canonical"));
        }
        self.entries.insert(
            index,
            Entry {
                key: T::KEY,
                version: T::VERSION,
                identity: Sha1::digest(&bytes).into(),
                configuration,
                initial: bytes,
                state: Box::new(initial),
                decode: decode::<T>,
            },
        );
        Ok(())
    }

    pub fn get<T: Sidecar>(&self) -> Result<&T> {
        self.entries
            .binary_search_by_key(&T::KEY, |entry| entry.key)
            .ok()
            .and_then(|index| self.entries[index].state.any().downcast_ref())
            .ok_or_else(|| error("sidecar type is not registered"))
    }

    pub fn get_mut<T: Sidecar>(&mut self) -> Result<&mut T> {
        let index = self
            .entries
            .binary_search_by_key(&T::KEY, |entry| entry.key)
            .map_err(|_| error("sidecar type is not registered"))?;
        self.entries[index]
            .state
            .any_mut()
            .downcast_mut()
            .ok_or_else(|| error("sidecar key belongs to another type"))
    }

    pub(super) fn install_resources(&mut self, resources: super::Resources) -> Result<()> {
        if self.resources.is_some() {
            return Err(error("enhanced immutable resources are already installed"));
        }
        let manifest = resources.manifest()?;
        self.register(manifest)?;
        self.resources = Some(resources);
        Ok(())
    }
    pub fn resource<T: super::Resource>(&self) -> Result<&T> {
        self.resources
            .as_ref()
            .ok_or_else(|| error("enhanced immutable resources are absent"))?
            .get::<T>()
    }
    /// Split disjoint storage without cloning authoritative state or resources.
    pub fn state_and_resource<T: Sidecar, R: super::Resource>(&mut self) -> Result<(&mut T, &R)> {
        let resource = self
            .resources
            .as_ref()
            .ok_or_else(|| error("enhanced immutable resources are absent"))?
            .get::<R>()?;
        let index = self
            .entries
            .binary_search_by_key(&T::KEY, |entry| entry.key)
            .map_err(|_| error("sidecar type is not registered"))?;
        let state = self.entries[index]
            .state
            .any_mut()
            .downcast_mut()
            .ok_or_else(|| error("sidecar key belongs to another type"))?;
        Ok((state, resource))
    }
    pub fn state_and_resources<T: Sidecar, A: super::Resource, B: super::Resource>(
        &mut self,
    ) -> Result<(&mut T, &A, &B)> {
        let resources = self
            .resources
            .as_ref()
            .ok_or_else(|| error("enhanced immutable resources are absent"))?;
        let a = resources.get::<A>()?;
        let b = resources.get::<B>()?;
        let index = self
            .entries
            .binary_search_by_key(&T::KEY, |entry| entry.key)
            .map_err(|_| error("sidecar type is not registered"))?;
        let state = self.entries[index]
            .state
            .any_mut()
            .downcast_mut()
            .ok_or_else(|| error("sidecar key belongs to another type"))?;
        Ok((state, a, b))
    }
    pub(super) fn validate_restore(
        &self,
        prepared: &Prepared,
        hardware: &super::SnapshotView<'_>,
    ) -> Result<()> {
        if let Some(resources) = &self.resources {
            resources.validate_restore(prepared, hardware)?;
        }
        Ok(())
    }

    pub(super) fn images(&self) -> Result<Vec<Image>> {
        let mut total = 0_usize;
        self.entries
            .iter()
            .map(|entry| {
                if entry.state.configuration_identity() != entry.configuration {
                    return Err(error("sidecar immutable configuration changed"));
                }
                let state = entry.state.encode()?;
                total = total.saturating_add(state.len());
                if total > MAX_STATE_BYTES {
                    return Err(error("enhanced sidecars exceed their snapshot size limit"));
                }
                Ok(Image {
                    key: entry.key.into(),
                    version: entry.version,
                    identity: entry.identity,
                    state,
                })
            })
            .collect()
    }

    pub(super) fn prepare(&self, images: Vec<Image>) -> Result<Prepared> {
        if images.len() != self.entries.len() {
            return Err(error("snapshot sidecar registry does not match"));
        }
        let states = images
            .into_iter()
            .zip(&self.entries)
            .map(|(image, entry)| {
                if image.key != entry.key
                    || image.version != entry.version
                    || image.identity != entry.identity
                {
                    return Err(error(
                        "snapshot sidecar schema or initial configuration does not match",
                    ));
                }
                let state = (entry.decode)(&image.state)?;
                if state.configuration_identity() != entry.configuration {
                    return Err(error(
                        "snapshot sidecar immutable configuration does not match",
                    ));
                }
                Ok(state)
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Prepared(states))
    }

    pub(super) fn commit(&mut self, prepared: Prepared) {
        for (entry, state) in self.entries.iter_mut().zip(prepared.0) {
            entry.state = state;
        }
    }

    pub(super) fn reset(&mut self) {
        for entry in &mut self.entries {
            entry.state =
                (entry.decode)(&entry.initial).expect("registered canonical sidecar reset value");
        }
    }
}

pub struct Prepared(Vec<Box<dyn Erased>>);
impl Prepared {
    pub fn get<T: Sidecar>(&self) -> Result<&T> {
        self.0
            .iter()
            .find_map(|state| state.any().downcast_ref::<T>())
            .ok_or_else(|| error("prepared sidecar type is not registered"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Encode, Decode)]
    struct CourseState {
        course: [u8; 4],
        progress: u32,
    }
    impl Sidecar for CourseState {
        const KEY: &'static str = "authored-immutable-course";
        const VERSION: u32 = 1;
        fn configuration_identity(&self) -> &[u8] {
            &self.course
        }
    }

    #[test]
    fn forged_initial_identity_cannot_replace_immutable_configuration() {
        let mut registry = Sidecars::default();
        registry
            .register(CourseState {
                course: [1, 2, 3, 4],
                progress: 0,
            })
            .unwrap();
        let mut images = registry.images().unwrap();
        // Keep the genuine initial-identity claim but change the decoded course.
        images[0].state = bincode::encode_to_vec(
            CourseState {
                course: [4, 3, 2, 1],
                progress: 99,
            },
            codec(),
        )
        .unwrap();
        assert!(registry.prepare(images).is_err());
        assert_eq!(registry.get::<CourseState>().unwrap().course, [1, 2, 3, 4]);
        assert_eq!(registry.get::<CourseState>().unwrap().progress, 0);
        registry.get_mut::<CourseState>().unwrap().progress = 99;
        assert!(registry.images().is_ok());
        registry.get_mut::<CourseState>().unwrap().course[0] = 5;
        assert!(registry.images().is_err());
        registry.reset();
        assert_eq!(registry.get::<CourseState>().unwrap().course, [1, 2, 3, 4]);
    }
}
