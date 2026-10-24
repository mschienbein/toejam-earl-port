//! Metadata-only observer lease for non-strict forks. Existing owners receive
//! their complete prefix plus actual new observations on every Result path.
use genesis_core::{GenesisBus, native};

pub(crate) struct JournalLease {
    cpu: Option<Vec<native::CpuAccess>>,
    ram: Option<Vec<native::RamWrite>>,
    chips: Option<Vec<native::TimedWrite>>,
    installed_timing_observer: bool,
}
impl JournalLease {
    pub(crate) fn install(bus: &mut GenesisBus) -> Self {
        let installed_timing_observer = bus.timing_survey.is_none();
        let cpu = bus.native_cpu_accesses.replace(Vec::new());
        let ram = bus.native_ram_writes.replace(Vec::new());
        let chips = bus.native_writes.take();
        bus.enable_native_write_trace();
        Self {
            cpu,
            ram,
            chips,
            installed_timing_observer,
        }
    }

    pub(crate) fn restore(self, bus: &mut GenesisBus) {
        fn forward<T>(previous: Option<Vec<T>>, current: Option<Vec<T>>) -> Option<Vec<T>> {
            previous.map(|mut previous| {
                if let Some(current) = current {
                    previous.extend(current);
                }
                previous
            })
        }
        bus.native_cpu_accesses = forward(self.cpu, bus.native_cpu_accesses.take());
        bus.native_ram_writes = forward(self.ram, bus.native_ram_writes.take());
        bus.native_writes = forward(self.chips, bus.native_writes.take());
        // Never rewind an actual policy mutation. The passive observer installed
        // here had no policy; a policy-bearing state belongs to its actual setter.
        if self.installed_timing_observer
            && bus
                .timing_survey
                .as_ref()
                .is_some_and(|survey| survey.policy.is_none())
        {
            bus.timing_survey = None;
        }
    }
}
