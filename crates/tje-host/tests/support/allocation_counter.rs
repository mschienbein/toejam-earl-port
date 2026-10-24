//! Thread-local allocation counting shared by unit and integration test binaries.
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
};

thread_local! {
    pub(crate) static ALLOCATIONS: Cell<(bool, u64, u64)> = const { Cell::new((false, 0, 0)) };
}
struct Meter;
fn record(size: usize) {
    ALLOCATIONS.with(|counter| {
        let (enabled, calls, bytes) = counter.get();
        if enabled {
            counter.set((true, calls + 1, bytes + size as u64));
        }
    });
}
unsafe impl GlobalAlloc for Meter {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record(layout.size());
        // SAFETY: forward the allocator contract unchanged to System.
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record(layout.size());
        // SAFETY: forward the allocator contract unchanged to System.
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        record(size);
        // SAFETY: pointer/layout originate from this System-backed allocator.
        unsafe { System.realloc(pointer, layout, size) }
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: pointer/layout originate from this System-backed allocator.
        unsafe { System.dealloc(pointer, layout) }
    }
}
#[global_allocator]
static ALLOCATOR: Meter = Meter;
