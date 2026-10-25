//! ROM-free access regressions. Controlled callees isolate caller alias handling;
//! these tests are not original-ROM or replay-envelope proof evidence.
use super::*;
use crate::{
    Dispatcher, Error, FunctionId, Translation,
    call_frame::FrameState,
    execution::{CallCeilingScope, ExecutionClass, RamWidth, WaitCondition, WaitRequest},
};
use std::{
    cell::RefCell,
    collections::{BTreeMap, VecDeque},
};
use tje_mem::{AddressSpace, Bus, Revision};

#[derive(Debug, Clone, PartialEq, Eq)]
enum Access {
    Read(u8, u32),
    Write(u8, u32, u32),
    /// A tagged discarded read (e.g. the MOVEM terminal word before UNLK): never a semantic read.
    Discard(u8, u32),
    Wait(WaitCondition),
    Yield,
}
struct Machine {
    memory: AddressSpace,
    trace: RefCell<Vec<Access>>,
    scripted: RefCell<BTreeMap<u32, VecDeque<u8>>>,
    data: [u32; 8],
    address: [u32; 7],
    sr: u16,
    sp: u32,
    frame: FrameState,
    class: ExecutionClass,
    controlled: bool,
    yields_allowed: usize,
    /// Controlled environment effects, intentionally not main-thread bus writes.
    yield_changes: VecDeque<(u32, u16)>,
}
impl Machine {
    fn new(r: Revision) -> Self {
        Self {
            memory: AddressSpace::new(vec![0; 0x100000], r),
            trace: RefCell::new(Vec::new()),
            scripted: RefCell::new(BTreeMap::new()),
            data: [0; 8],
            address: [0; 7],
            sr: 0x2000,
            sp: 0xff7f00,
            frame: FrameState::new(0xff7f00),
            class: ExecutionClass::TimeAware,
            controlled: false,
            yields_allowed: 0,
            yield_changes: VecDeque::new(),
        }
    }
    fn clear(&self) {
        self.trace.borrow_mut().clear();
    }
    fn reads(&self) -> Vec<(u8, u32)> {
        self.trace
            .borrow()
            .iter()
            .filter_map(|a| match a {
                Access::Read(w, p) => Some((*w, *p)),
                _ => None,
            })
            .collect()
    }
    fn seed_args(&mut self, args: &[u32]) {
        for (i, &arg) in args.iter().enumerate() {
            self.frame.argument_slot(i as i32).write(self, arg);
        }
    }
}
impl Bus for Machine {
    fn read_u8(&self, p: u32) -> u8 {
        self.trace.borrow_mut().push(Access::Read(1, p));
        self.scripted
            .borrow_mut()
            .get_mut(&p)
            .and_then(VecDeque::pop_front)
            .unwrap_or_else(|| self.memory.read_u8(p))
    }
    fn read_u16(&self, p: u32) -> u16 {
        self.trace.borrow_mut().push(Access::Read(2, p));
        self.memory.read_u16(p)
    }
    fn read_u32(&self, p: u32) -> u32 {
        self.trace.borrow_mut().push(Access::Read(4, p));
        self.memory.read_u32(p)
    }
    fn write_u8(&mut self, p: u32, v: u8) {
        self.trace
            .borrow_mut()
            .push(Access::Write(1, p, u32::from(v)));
        self.memory.write_u8(p, v);
    }
    fn write_u16(&mut self, p: u32, v: u16) {
        self.trace
            .borrow_mut()
            .push(Access::Write(2, p, u32::from(v)));
        self.memory.write_u16(p, v);
    }
    fn write_u32(&mut self, p: u32, v: u32) {
        self.trace.borrow_mut().push(Access::Write(4, p, v));
        self.memory.write_u32(p, v);
    }
}
impl tje_mem::Platform for Machine {
    fn write_vdp_control(&mut self, value: u16) {
        panic!("unexpected VDP control {value:04x}");
    }
    fn write_vdp_data(&mut self, value: u16) {
        panic!("unexpected VDP data {value:04x}");
    }
}
impl crate::Platform for Machine {
    fn poll_yield(&mut self) -> Result<()> {
        self.trace.borrow_mut().push(Access::Yield);
        if self.yields_allowed == 0 {
            return Err(Error::Platform(
                "isolated unit context has no live progress".into(),
            ));
        }
        self.yields_allowed -= 1;
        if let Some((address, value)) = self.yield_changes.pop_front() {
            self.memory.write_u16(address, value);
        }
        Ok(())
    }
    fn wait_until(&mut self, request: WaitRequest) -> Result<()> {
        self.trace
            .borrow_mut()
            .push(Access::Wait(request.condition));
        let WaitCondition::Ram {
            address,
            width: RamWidth::Byte,
            mask,
            value,
            comparison,
        } = request.condition
        else {
            return Err(Error::Platform("unexpected unit wait predicate".into()));
        };
        loop {
            let actual = u32::from(self.read_u8(address));
            if comparison.satisfied(actual, mask, value) {
                return Ok(());
            }
            crate::Platform::poll_yield(self)?;
        }
    }
    fn discard_u8(&mut self, address: u32) -> crate::Result<()> {
        self.trace.borrow_mut().push(Access::Discard(1, address));
        let _ = self.memory.read_u8(address);
        Ok(())
    }
    fn discard_u16(&mut self, address: u32) -> crate::Result<()> {
        self.trace.borrow_mut().push(Access::Discard(2, address));
        let _ = self.memory.read_u16(address);
        Ok(())
    }
    fn read_device_u8(&mut self, address: u32) -> u8 {
        panic!("unexpected device byte read {address:08x}");
    }
    fn read_device_u16(&mut self, address: u32) -> u16 {
        panic!("unexpected device word read {address:08x}");
    }
    fn write_device_u8(&mut self, address: u32, value: u8) {
        panic!("unexpected device byte write {address:08x}={value:02x}");
    }
    fn write_device_u16(&mut self, address: u32, value: u16) {
        panic!("unexpected device word write {address:08x}={value:04x}");
    }
}
impl Context for Machine {
    fn revision(&self) -> Revision {
        self.memory.revision()
    }
    fn data_register(&self, i: usize) -> u32 {
        self.data[i]
    }
    fn set_data_register(&mut self, i: usize, v: u32) {
        self.data[i] = v;
    }
    fn address_register(&self, i: usize) -> u32 {
        self.address[i]
    }
    fn set_address_register(&mut self, i: usize, v: u32) {
        self.address[i] = v;
    }
    fn status_register(&self) -> u16 {
        self.sr
    }
    fn set_status_register(&mut self, v: u16) {
        self.sr = v;
    }
    fn stack_pointer(&self) -> Result<u32> {
        Ok(self.sp)
    }
    fn set_stack_pointer(&mut self, v: u32) -> Result<()> {
        self.sp = v;
        Ok(())
    }
    fn frame_state(&self) -> Option<FrameState> {
        Some(self.frame)
    }
    fn set_frame_state(&mut self, v: FrameState) -> Result<()> {
        self.frame = v;
        Ok(())
    }
    fn execution_class(&self) -> ExecutionClass {
        self.class
    }
    fn set_execution_class(&mut self, v: ExecutionClass) -> Result<()> {
        self.class = v;
        Ok(())
    }
    fn enter_call_ceiling(&mut self, _: FunctionId) -> Result<CallCeilingScope> {
        Ok(CallCeilingScope::not_required())
    }
    fn leave_call_ceiling(&mut self, _: CallCeilingScope) -> Result<()> {
        Ok(())
    }
    fn dispatcher(&self) -> Dispatcher {
        if self.controlled {
            Dispatcher::hybrid_with_overrides(CONTROLLED_CALLEES)
        } else {
            Dispatcher::hybrid()
        }
    }
    fn call_original(&mut self, id: FunctionId, _: &[u32]) -> Result<u32> {
        Err(Error::Untranslated(id))
    }
}

#[test]
fn metatile_is_loaded_before_either_camera_cursor_in_both_revisions() {
    for r in Revision::ALL {
        for camera in [0, 1] {
            let mut c = Machine::new(r);
            let map = if camera == 0 {
                ram::g_player1_level_map(r)
            } else {
                ram::g_player2_level_map(r)
            };
            let row = if camera == 0 {
                ram::g_camera1_tile_row_cursor(r)
            } else {
                ram::g_camera2_tile_row_cursor(r)
            };
            let column = if camera == 0 {
                ram::g_camera1_tile_column_cursor(r)
            } else {
                ram::g_camera2_tile_column_cursor(r)
            };
            row.write(&mut c, 1000); // offscreen path makes the regression independent of callees
            c.seed_args(&[camera, 5, 5]);
            c.clear();
            map_animate_metatile_path_change(&mut c).unwrap();
            let data: Vec<_> = c
                .reads()
                .into_iter()
                .filter(|(_, p)| {
                    [
                        *p == map.at(5).at(5).address(),
                        *p == row.address(),
                        *p == column.address(),
                    ]
                    .into_iter()
                    .any(|v| v)
                })
                .collect();
            assert_eq!(
                data,
                vec![
                    (2, map.at(5).at(5).address()),
                    (2, row.address()),
                    (2, column.address())
                ]
            );
        }
    }
}
#[test]
fn direction_branch_reads_remain_separate_and_short_circuit() {
    for r in Revision::ALL {
        for (direction, expected_reads, expected_row, expected_column) in
            [(2, 1, 5, 8), (8, 2, 5, 6), (1, 3, 4, 7), (4, 3, 6, 7)]
        {
            let mut c = Machine::new(r);
            let column = Address::<i16>::new(0xff7000);
            column.write(&mut c, 7);
            ram::g_land_path_traversal_direction(r).write(&mut c, direction);
            c.clear();
            let mut row = 5;
            advance_marked_position(&mut c, &mut row, column);
            assert_eq!((row, column.read(&c)), (expected_row, expected_column));
            assert_eq!(
                c.reads()
                    .iter()
                    .filter(|&&(width, p)| width == 1
                        && p == ram::g_land_path_traversal_direction(r).address())
                    .count(),
                expected_reads
            );
        }
    }
}
#[test]
fn changing_direction_samples_are_not_replaced_by_one_cached_value() {
    for r in Revision::ALL {
        let mut c = Machine::new(r);
        let column = Address::<i16>::new(0xff7000);
        column.write(&mut c, 7);
        c.scripted.borrow_mut().insert(
            ram::g_land_path_traversal_direction(r).address(),
            VecDeque::from([4, 8]),
        );
        let mut row = 5;
        advance_marked_position(&mut c, &mut row, column);
        assert_eq!((row, column.read(&c)), (5, 6));
    }
}
#[test]
fn bank_comparison_reads_player_two_before_player_one() {
    for r in Revision::ALL {
        let c = Machine::new(r);
        assert!(marked_banks_share_level(&c));
        assert_eq!(
            c.reads(),
            vec![
                (1, ram::g_player2_map_level(r).address()),
                (1, ram::g_player1_map_level(r).address())
            ]
        );
    }
}
#[test]
fn fall_predicate_retains_two_sprite_reads_only_in_revision_two() {
    for r in Revision::ALL {
        for state in [0, 78, 79] {
            let mut c = Machine::new(r);
            let object = ram::g_player_info(r).at(0);
            let terrain = ram::g_player1_terrain_underfoot(r);
            terrain.write(&mut c, 0x90);
            object.sprite_state().write(&mut c, state);
            c.clear();
            assert_eq!(
                path_change_can_fall(&c, terrain, object),
                r == Revision::Rev00 || state == 0
            );
            let n = c
                .reads()
                .iter()
                .filter(|&&(w, p)| w == 1 && p == object.sprite_state().address())
                .count();
            assert_eq!(
                n,
                if r == Revision::Rev00 {
                    0
                } else if state == 78 {
                    1
                } else {
                    2
                }
            );
        }
    }
}

// Test-only callee effects isolate the caller's escaping local alias, not a
// replay proof or a production fallback. Every invocation records a real effect.
const CALLS: Address<u16> = Address::new(0xff6f00);
const OBSERVED: Address<u16> = Address::new(0xff6f02);
fn observe_reservation(c: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    Address::<u16>::new(0xff6f04).write(c, 1);
    Ok(None)
}
fn consume_alias(c: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let count = CALLS.read(c);
    CALLS.write(c, count + 1);
    if count == 0 {
        Address::<i16>::new(c.address_register(6) - 2).write(c, 99);
        Ok(Some(1))
    } else {
        Ok(Some(u32::MAX))
    }
}
fn observe_animation(c: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    OBSERVED.write(c, args[2] as u16);
    Ok(None)
}
fn observe_free(c: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let p = Address::<u16>::new(0xff6f06);
    p.write(c, p.read(c) + 1);
    Ok(None)
}
static CONTROLLED_CALLEES: &[Translation] = &[
    Translation {
        id: ids::ALLOCATE_ENTITY_WITH_EVICTION,
        function: allocate_wide_word_index,
    },
    Translation {
        id: ids::ALLOCATE_ENTITY_VIEWPORT_SPRITES,
        function: observe_wrapped_viewport_object,
    },
    Translation {
        id: ids::ENTITY_START_LAND_PATH_ANIMATION,
        function: capture_effect_frame,
    },
    Translation {
        id: ids::MAP_QUEUE_VISIBLE_METATILE_ROW,
        function: capture_queue_frame,
    },
    Translation {
        id: ids::ENTITY_RESERVE_LAND_PATH_ANIMATIONS,
        function: observe_reservation,
    },
    Translation {
        id: ids::MAP_CONSUME_PATH_CELL_DIRECTIONS,
        function: consume_alias,
    },
    Translation {
        id: ids::MAP_ANIMATE_METATILE_PATH_CHANGE,
        function: observe_animation,
    },
    Translation {
        id: ids::FREE_ENTITY,
        function: observe_free,
    },
];
#[test]
fn animation_arguments_reload_the_column_slot_after_consuming_callee_aliases_it() {
    for r in Revision::ALL {
        let mut c = Machine::new(r);
        c.controlled = true;
        ram::g_player1_map_level(r).write(&mut c, 1);
        ram::g_player2_map_level(r).write(&mut c, 2);
        c.seed_args(&[0, 5, 7]);
        map_animate_marked_land_paths(&mut c).unwrap();
        assert_eq!(OBSERVED.read(&c), 99);
        assert_eq!(CALLS.read(&c), 2);
        assert_eq!(Address::<u16>::new(0xff6f06).read(&c), 8);
    }
}

#[test]
fn queued_writes_reload_a_camera_parameter_aliased_by_the_first_upload_store() {
    for r in Revision::ALL {
        for column_queue in [false, true] {
            let mut c = Machine::new(r);
            let start = if column_queue {
                ram::g_camera_column_upload_start_row(r)
            } else {
                ram::g_camera_row_upload_start_column(r)
            };
            let initial = c
                .frame
                .argument_byte(4)
                .address()
                .wrapping_sub(start.address()) as i16;
            let coordinate = if column_queue { 0 } else { 5 };
            let changed = ((initial as u16 & 0xff00) | coordinate as u16) as i16;
            let args = [30, 0, 5, 0, i32::from(initial) as u32, 0];
            c.seed_args(&args);
            ram::g_camera_column_upload_height(r).write(&mut c, 32);
            c.clear();
            // This isolated prefix deliberately stops at the real scheduler
            // requirement. It proves alias/store semantics, not a fake IRQ.
            let result = if column_queue {
                crate::map::map_queue_visible_metatile_column(&mut c)
            } else {
                crate::map::map_queue_visible_metatile_row(&mut c)
            };
            assert!(result.is_err());
            let expected = if column_queue {
                ram::g_camera_column_upload_words(r)
                    .at(i32::from(changed))
                    .at(0)
            } else {
                ram::g_camera_row_upload_words(r)
                    .at(i32::from(changed))
                    .at(0)
            };
            assert!(
                c.trace
                    .borrow()
                    .iter()
                    .any(|entry| *entry == Access::Write(2, expected.address(), 0x8000))
            );
            assert_eq!(c.frame.argument_word(4).read(&c), changed);
        }
    }
}

fn capture_effect_frame(c: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let entry_sp = c.frame_state().unwrap().entry_sp;
    Address::<u32>::new(0xff6f08).write(c, entry_sp);
    Ok(None)
}
fn capture_queue_frame(c: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    let entry = c.frame_state().unwrap();
    Address::<u32>::new(0xff6f0c).write(c, entry.entry_sp);
    assert_eq!(args, [30, 48, 96, 0, 0, 96]);
    for (index, expected) in [0, 776, 408].into_iter().enumerate() {
        assert_eq!(entry.argument_slot(6 + index as i32).read(c), expected);
    }
    Ok(None)
}

#[test]
fn metatile_transfer_keeps_the_effect_arguments_under_the_queue_argument_area() {
    for r in Revision::ALL {
        let mut c = Machine::new(r);
        c.controlled = true;
        c.seed_args(&[0, 5, 5]);
        ram::g_player1_level_map(r).at(5).at(5).write(&mut c, 30);
        ram::g_camera1_tile_row_cursor(r).write(&mut c, 48);
        ram::g_camera1_tile_column_cursor(r).write(&mut c, 96);
        ram::g_camera_column_upload_height(r).write(&mut c, 1);
        ram::g_land_path_traversal_direction(r).write(&mut c, 4);
        map_animate_metatile_path_change(&mut c).unwrap();
        assert_eq!(
            Address::<u32>::new(0xff6f08).read(&c) - Address::<u32>::new(0xff6f0c).read(&c),
            24
        );
        assert_eq!(c.sp, 0xff7f00);
    }
}

fn allocate_wide_word_index(c: &mut dyn Context, _: &[u32]) -> Result<Option<u32>> {
    let counter = Address::<u16>::new(0xff6f10);
    counter.write(c, counter.read(c) + 1);
    Ok(Some(512))
}
fn observe_wrapped_viewport_object(c: &mut dyn Context, args: &[u32]) -> Result<Option<u32>> {
    let object = ram::g_player_info(c.revision()).at(0);
    assert_eq!(
        args[0],
        object.address(),
        "ASL.W entity512 must wrap before the base addition"
    );
    object.draw_flags().write(c, 6);
    Ok(None)
}
#[test]
fn reservation_wraps_the_short_entity_offset_before_adding_the_object_base() {
    for r in Revision::ALL {
        let mut c = Machine::new(r);
        c.controlled = true;
        c.seed_args(&[0, 0, 0]);
        entity_reserve_land_path_animations(&mut c).unwrap();
        assert_eq!(Address::<u16>::new(0xff6f10).read(&c), 8);
        assert_eq!(ram::g_player_info(r).at(0).width().read(&c), 32);
        assert_eq!(c.sp, 0xff7f00);
    }
}

#[test]
fn marked_land_clr_bytes_keep_their_discarded_reads_separate_from_semantic_reads() {
    for r in Revision::ALL {
        let mut c = Machine::new(r);
        c.controlled = true;
        c.seed_args(&[0, 5, 7]);
        ram::g_player1_map_level(r).write(&mut c, 1);
        ram::g_player2_map_level(r).write(&mut c, 2);
        for n in 0..26 {
            ram::g_player_info(r)
                .at(n)
                .frame_duration()
                .write(&mut c, (n + 1) as i8);
        }
        c.clear();
        map_animate_marked_land_paths(&mut c).unwrap();
        for n in 0..26 {
            let p = ram::g_player_info(r).at(n).frame_duration().address();
            let events: Vec<_> = c
                .trace
                .borrow()
                .iter()
                .filter(|event| match event {
                    Access::Read(_, a) | Access::Discard(_, a) | Access::Write(_, a, _) => *a == p,
                    Access::Wait(_) | Access::Yield => false,
                })
                .cloned()
                .collect();
            assert_eq!(
                &events[..3],
                &[
                    Access::Read(1, p),
                    Access::Discard(1, p),
                    Access::Write(1, p, 0)
                ]
            );
            assert_eq!(events.last(), Some(&Access::Write(1, p, (n + 1) as u32)));
        }
        for n in 0..8 {
            let p = ram::g_land_path_animation_entities(r).at(n).address();
            let events: Vec<_> = c
                .trace
                .borrow()
                .iter()
                .filter(|event| match event {
                    Access::Read(_, a) | Access::Discard(_, a) | Access::Write(_, a, _) => *a == p,
                    Access::Wait(_) | Access::Yield => false,
                })
                .cloned()
                .collect();
            assert_eq!(
                events,
                vec![
                    Access::Read(1, p),
                    Access::Discard(1, p),
                    Access::Write(1, p, 0)
                ]
            );
        }
    }
}

#[test]
fn fixed_pending_wait_owns_only_byte_polls_including_its_final_zero() {
    for r in Revision::ALL {
        let mut c = Machine::new(r);
        let flag = ram::g_sprite_graphics_upload_pending(r).cast::<i8>();
        c.scripted
            .borrow_mut()
            .insert(flag.address(), VecDeque::from([0x80, 1, 0]));
        c.yields_allowed = 2;
        super::super::waits::wait_pending_byte(&mut c, flag).unwrap();
        assert_eq!(
            *c.trace.borrow(),
            vec![
                Access::Wait(WaitRequest::ram_byte_equal(flag.address(), 0xff, 0).condition),
                Access::Read(1, flag.address()),
                Access::Yield,
                Access::Read(1, flag.address()),
                Access::Yield,
                Access::Read(1, flag.address()),
            ]
        );
    }
}

#[test]
fn reservation_compound_polls_short_circuit_without_duplicate_row_samples() {
    for r in Revision::ALL {
        let mut c = Machine::new(r);
        c.controlled = true;
        c.seed_args(&[0, 0, 0]);
        let row = ram::g_camera_row_upload_pending(r).at(0).address();
        let column = ram::g_camera_column_upload_pending(r).at(0).address();
        c.scripted
            .borrow_mut()
            .insert(row, VecDeque::from([1, 0, 0]));
        c.scripted
            .borrow_mut()
            .insert(column, VecDeque::from([1, 0]));
        c.yields_allowed = 2;
        c.clear();
        entity_reserve_land_path_animations(&mut c).unwrap();
        let polls: Vec<_> = c
            .trace
            .borrow()
            .iter()
            .filter(|event| {
                matches!(event, Access::Read(1, p) if *p == row || *p == column)
                    || **event == Access::Yield
            })
            .cloned()
            .collect();
        assert_eq!(
            polls,
            vec![
                Access::Read(1, row),
                Access::Yield,
                Access::Read(1, row),
                Access::Read(1, column),
                Access::Yield,
                Access::Read(1, row),
                Access::Read(1, column)
            ]
        );
        assert!(
            !c.trace
                .borrow()
                .iter()
                .any(|event| matches!(event, Access::Wait(_)))
        );
    }
}

#[test]
fn dynamic_queue_poll_reloads_camera_after_live_progress_changes_its_slot() {
    for r in Revision::ALL {
        for column_queue in [false, true] {
            let mut c = Machine::new(r);
            c.seed_args(&[30, 0, 0, 0, 0, 1000]); // clipped geometry: no tile callees
            let camera_word = c.frame.argument_word(4).address();
            let flags = if column_queue {
                ram::g_camera_column_upload_pending(r)
            } else {
                ram::g_camera_row_upload_pending(r)
            };
            c.yields_allowed = 1;
            c.yield_changes.push_back((camera_word, 1));
            c.clear();
            if column_queue {
                crate::map::map_queue_visible_metatile_column(&mut c).unwrap();
            } else {
                crate::map::map_queue_visible_metatile_row(&mut c).unwrap();
            }
            let trace = c.trace.borrow();
            let start = trace
                .iter()
                .position(|event| *event == Access::Write(1, flags.at(0).address(), 1))
                .unwrap()
                + 1;
            assert_eq!(
                &trace[start..start + 5],
                &[
                    Access::Read(2, camera_word),
                    Access::Read(1, flags.at(0).address()),
                    Access::Yield,
                    Access::Read(2, camera_word),
                    Access::Read(1, flags.at(1).address()),
                ]
            );
            assert!(!trace.iter().any(|event| matches!(event, Access::Wait(_))));
        }
    }
}
