//! ROM-free source-access regressions. Dependency callbacks below are test-only
//! observations, not original substitution or replay-proof evidence.
use super::*;
use crate::call_frame::FrameState;
use crate::execution::{
    CallCeiling, CallCeilingScope, CallClock, ExecutionClass, RamWidth, WaitComparison,
    WaitCondition, WaitRequest,
};
use crate::{Dispatcher, FunctionId, Platform};
use std::cell::RefCell;
use std::collections::VecDeque;
use tje_mem::{AddressSpace, Bus};

const ENTRY_SP: u32 = 0xff7000;

fn invoke(
    context: &mut Fixture,
    body: crate::FramedTranslationFn,
    arguments: &[u32],
) -> Result<Option<u32>> {
    for (index, &value) in arguments.iter().enumerate() {
        context
            .memory
            .write_u32(context.frame.entry_sp + 4 + index as u32 * 4, value);
    }
    body(context)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Access {
    Read(u32, u8, u32),
    Discard(u32, u8, u32),
    Write(u32, u8, u32),
    Wait(WaitCondition),
    Yield,
}

#[derive(Debug, Clone)]
struct Dependency {
    id: FunctionId,
    arguments: Vec<u32>,
    sp: u32,
    pc: u32,
    data: [u32; 8],
    address: [u32; 7],
    retained: [u32; 5],
}

struct Fixture {
    memory: AddressSpace,
    data: [u32; 8],
    address: [u32; 7],
    status: u16,
    sp: u32,
    frame: FrameState,
    class: ExecutionClass,
    clock: CallClock,
    accesses: RefCell<Vec<Access>>,
    dependencies: Vec<Dependency>,
    phases: VecDeque<i8>,
    poll_values: VecDeque<i8>,
    players: RefCell<VecDeque<i8>>,
    results: VecDeque<(FunctionId, u32)>,
    input_calls: usize,
    close_after: usize,
    on_dependency: fn(&mut Fixture, FunctionId, &[u32]),
}

impl Fixture {
    fn new(revision: Revision) -> Self {
        Self {
            memory: AddressSpace::new(vec![0; 0x100000], revision),
            data: std::array::from_fn(|index| 0x7654_ab00 | index as u32),
            address: std::array::from_fn(|index| 0xff1000 + index as u32 * 4),
            status: 0x201f,
            sp: ENTRY_SP,
            frame: FrameState::new(ENTRY_SP),
            class: ExecutionClass::TimeAware,
            clock: CallClock::default(),
            accesses: RefCell::default(),
            dependencies: Vec::new(),
            phases: VecDeque::new(),
            poll_values: VecDeque::new(),
            players: RefCell::default(),
            results: VecDeque::new(),
            input_calls: 0,
            close_after: usize::MAX,
            on_dependency: |_, _, _| {},
        }
    }

    fn calls(&self, id: FunctionId) -> Vec<&Dependency> {
        self.dependencies
            .iter()
            .filter(|call| call.id == id)
            .collect()
    }

    fn byte_reads(&self, address: u32) -> Vec<u8> {
        self.accesses
            .borrow()
            .iter()
            .filter_map(|access| match *access {
                Access::Read(at, 1, value) if at == address => Some(value as u8),
                _ => None,
            })
            .collect()
    }

    fn assert_no_frame_counter_read(&self) {
        let counter = ram::g_frame_counter(self.memory.revision()).address();
        assert!(!self.accesses.borrow().iter().any(|access| matches!(access,
            Access::Read(at, _, _) | Access::Discard(at, _, _) if *at == counter || *at == counter + 1)));
    }
}

impl Bus for Fixture {
    fn read_u8(&self, address: u32) -> u8 {
        let scripted =
            if address == ram::g_inventory_redraw_player(self.memory.revision()).address() {
                self.players.borrow_mut().pop_front()
            } else {
                None
            };
        let value = scripted.map_or_else(|| self.memory.read_u8(address), |value| value as u8);
        self.accesses
            .borrow_mut()
            .push(Access::Read(address, 1, u32::from(value)));
        value
    }
    fn read_u16(&self, address: u32) -> u16 {
        let value = self.memory.read_u16(address);
        self.accesses
            .borrow_mut()
            .push(Access::Read(address, 2, u32::from(value)));
        value
    }
    fn read_u32(&self, address: u32) -> u32 {
        (u32::from(self.read_u16(address)) << 16) | u32::from(self.read_u16(address + 2))
    }
    fn write_u8(&mut self, address: u32, value: u8) {
        self.accesses
            .borrow_mut()
            .push(Access::Write(address, 1, u32::from(value)));
        self.memory.write_u8(address, value);
    }
    fn write_u16(&mut self, address: u32, value: u16) {
        self.accesses
            .borrow_mut()
            .push(Access::Write(address, 2, u32::from(value)));
        self.memory.write_u16(address, value);
    }
    fn write_u32(&mut self, address: u32, value: u32) {
        self.write_u16(address, (value >> 16) as u16);
        self.write_u16(address + 2, value as u16);
    }
}
impl tje_mem::Platform for Fixture {
    fn write_vdp_control(&mut self, value: u16) {
        self.write_device_u16(0xc00004, value);
    }
    fn write_vdp_data(&mut self, value: u16) {
        self.write_device_u16(0xc00000, value);
    }
    fn write_vdp_command(&mut self, value: u32) {
        self.write_device_u16(0xc00004, (value >> 16) as u16);
        self.write_device_u16(0xc00006, value as u16);
    }
}
impl Platform for Fixture {
    fn read_device_u8(&mut self, address: u32) -> u8 {
        panic!("unexpected device byte read {address:x}");
    }
    fn read_device_u16(&mut self, address: u32) -> u16 {
        panic!("unexpected device word read {address:x}");
    }
    fn write_device_u8(&mut self, address: u32, value: u8) {
        self.accesses
            .borrow_mut()
            .push(Access::Write(address, 1, u32::from(value)));
    }
    fn write_device_u16(&mut self, address: u32, value: u16) {
        self.accesses
            .borrow_mut()
            .push(Access::Write(address, 2, u32::from(value)));
    }
    fn discard_u8(&mut self, address: u32) -> Result<()> {
        let value = self.memory.read_u8(address);
        self.accesses
            .borrow_mut()
            .push(Access::Discard(address, 1, u32::from(value)));
        Ok(())
    }
    fn discard_u16(&mut self, address: u32) -> Result<()> {
        let value = self.memory.read_u16(address);
        self.accesses
            .borrow_mut()
            .push(Access::Discard(address, 2, u32::from(value)));
        Ok(())
    }
    fn wait_until(&mut self, request: WaitRequest) -> Result<()> {
        self.accesses
            .borrow_mut()
            .push(Access::Wait(request.condition));
        let WaitCondition::Ram {
            address,
            width: RamWidth::Byte,
            mask,
            value,
            comparison: WaitComparison::Equal,
        } = request.condition
        else {
            panic!("unexpected polling request {request:?}");
        };
        let revision = self.memory.revision();
        assert!(
            address == ram::g_inventory_redraw_phase(revision).address()
                || address == ram::g_sprite_graphics_upload_pending(revision).address()
        );
        assert_eq!((mask, value), (0xff, 0));
        loop {
            if self.read_u8(address) == 0 {
                return Ok(());
            }
            let next = self.poll_values.pop_front().expect("unsatisfied RAM poll");
            self.memory.write_u8(address, next as u8);
        }
    }
    fn poll_yield(&mut self) -> Result<()> {
        self.accesses.borrow_mut().push(Access::Yield);
        let phase = self.phases.pop_front().expect("unexpected compound spin");
        ram::g_inventory_redraw_phase(self.memory.revision()).write(&mut self.memory, phase);
        Ok(())
    }
}
impl Context for Fixture {
    fn revision(&self) -> Revision {
        self.memory.revision()
    }
    fn data_register(&self, index: usize) -> u32 {
        self.data[index]
    }
    fn set_data_register(&mut self, index: usize, value: u32) {
        self.data[index] = value;
    }
    fn address_register(&self, index: usize) -> u32 {
        self.address[index]
    }
    fn set_address_register(&mut self, index: usize, value: u32) {
        self.address[index] = value;
    }
    fn status_register(&self) -> u16 {
        self.status
    }
    fn set_status_register(&mut self, value: u16) {
        assert_eq!(
            self.status & !0x1f,
            value & !0x1f,
            "inventory body must not change SR/IPL"
        );
        self.status = value;
    }
    fn stack_pointer(&self) -> Result<u32> {
        Ok(self.sp)
    }
    fn set_stack_pointer(&mut self, value: u32) -> Result<()> {
        self.sp = value;
        Ok(())
    }
    fn frame_state(&self) -> Option<FrameState> {
        Some(self.frame)
    }
    fn set_frame_state(&mut self, value: FrameState) -> Result<()> {
        self.frame = value;
        Ok(())
    }
    fn execution_class(&self) -> ExecutionClass {
        self.class
    }
    fn set_execution_class(&mut self, value: ExecutionClass) -> Result<()> {
        self.class = value;
        Ok(())
    }
    fn enter_call_ceiling(&mut self, id: FunctionId) -> Result<CallCeilingScope> {
        self.clock
            .enter(id, CallCeiling::DEFAULT)
            .map_err(crate::Error::Clock)
    }
    fn leave_call_ceiling(&mut self, scope: CallCeilingScope) -> Result<()> {
        self.clock
            .leave(scope)
            .map(|_| ())
            .map_err(crate::Error::Clock)
    }
    fn dispatcher(&self) -> Dispatcher {
        Dispatcher::hybrid_with(&[])
    }
    fn call_original(&mut self, id: FunctionId, _: &[u32]) -> Result<u32> {
        panic!("unframed dependency {}", id.metadata().name);
    }
    fn call_original_entry(&mut self, id: FunctionId, _: ExecutionClass) -> Result<u32> {
        let arguments: Vec<_> = (0..id.metadata().abi.argument_count)
            .map(|index| self.memory.read_u32(self.sp + 4 + u32::from(index) * 4))
            .collect();
        self.dependencies.push(Dependency {
            id,
            arguments: arguments.clone(),
            sp: self.sp,
            pc: self.memory.read_u32(self.sp),
            data: self.data,
            address: self.address,
            retained: std::array::from_fn(|index| {
                self.memory.read_u32(self.sp + 4 + index as u32 * 4)
            }),
        });
        if id == ids::GET_PLAYER_INPUT {
            self.input_calls += 1;
            if self.input_calls >= self.close_after {
                let revision = self.revision();
                ram::g_player_inventory_open(revision)
                    .at(0)
                    .write(&mut self.memory, 0);
                ram::g_player_inventory_open(revision)
                    .at(1)
                    .write(&mut self.memory, 0);
            }
        }
        (self.on_dependency)(self, id, &arguments);
        let value = if self
            .results
            .front()
            .is_some_and(|&(expected, _)| expected == id)
        {
            self.results.pop_front().unwrap().1
        } else {
            0
        };
        self.data[0] = value;
        self.sp += 4;
        Ok(value)
    }
}

#[test]
fn refresh_polls_only_the_source_byte_before_and_after_request() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = Fixture::new(revision);
        ram::g_inventory_redraw_phase(revision).write(&mut c.memory, 2);
        c.poll_values = [1, 0, 0].into();
        invoke(
            &mut c,
            crate::ui::waiting::inventory_refresh_contents_and_wait,
            &[1],
        )
        .unwrap();
        assert_eq!(
            c.byte_reads(ram::g_inventory_redraw_phase(revision).address()),
            [2, 1, 0, 1, 0]
        );
        let waits: Vec<_> = c
            .accesses
            .borrow()
            .iter()
            .filter_map(|access| match *access {
                Access::Wait(condition) => Some(condition),
                _ => None,
            })
            .collect();
        let expected =
            WaitRequest::ram_byte_equal(ram::g_inventory_redraw_phase(revision).address(), 0xff, 0)
                .condition;
        assert_eq!(waits, [expected, expected]);
        c.assert_no_frame_counter_read();
        assert_eq!(ram::g_inventory_redraw_player(revision).read(&c.memory), 1);
    }
}

#[test]
fn draw_inventory_keeps_compound_byte_poll_and_revision_idle_store() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for player in 0..2 {
            let mut c = Fixture::new(revision);
            let data = c.data;
            let address = c.address;
            ram::g_player_idle_ticks(revision)
                .at(player)
                .write(&mut c.memory, 0x1234);
            c.phases = [2, 1, 0].into();
            invoke(&mut c, draw_inventory, &[player as u32, 3]).unwrap();
            assert_eq!(
                c.byte_reads(ram::g_inventory_redraw_phase(revision).address()),
                [4, 4, 2, 2, 1, 0]
            );
            assert_eq!(
                c.byte_reads(ram::g_player_inventory_open(revision).at(player).address()),
                [0, 0, 3]
            );
            let phase = ram::g_inventory_redraw_phase(revision).address();
            let open = ram::g_player_inventory_open(revision).at(player).address();
            let spin: Vec<_> = c
                .accesses
                .borrow()
                .iter()
                .filter(|access| match **access {
                    Access::Read(address, 1, _) => address == open || address == phase,
                    Access::Write(address, 1, _) => address == open,
                    Access::Yield => true,
                    _ => false,
                })
                .copied()
                .collect();
            assert_eq!(
                spin,
                [
                    Access::Read(phase, 1, 4),
                    Access::Read(open, 1, 0),
                    Access::Read(phase, 1, 4),
                    Access::Yield,
                    Access::Read(phase, 1, 2),
                    Access::Read(open, 1, 0),
                    Access::Read(phase, 1, 2),
                    Access::Write(open, 1, 3),
                    Access::Yield,
                    Access::Read(phase, 1, 1),
                    Access::Read(open, 1, 3),
                    Access::Yield,
                    Access::Read(phase, 1, 0),
                ]
            );
            assert!(
                !c.accesses
                    .borrow()
                    .iter()
                    .any(|access| matches!(access, Access::Wait(_)))
            );
            c.assert_no_frame_counter_read();
            assert_eq!(
                ram::g_player_idle_ticks(revision)
                    .at(player)
                    .read(&c.memory),
                if revision == Revision::Rev02 {
                    0
                } else {
                    0x1234
                }
            );
            assert_eq!(&c.data[2..4], &data[2..4]);
            assert_eq!(&c.address[2..], &address[2..]);
            assert_eq!(c.sp, ENTRY_SP);
            let init = c.calls(ids::INVENTORY_INITIALIZE_DISPLAY_ENTITIES)[0];
            assert_eq!(init.sp, ENTRY_SP - 28);
            assert_eq!(
                init.pc,
                ids::DRAW_INVENTORY.metadata().address(revision)
                    + if revision == Revision::Rev02 {
                        0xaa
                    } else {
                        0x9a
                    }
            );
        }
    }
}

#[test]
fn refresh_wait_owns_initial_and_terminal_polls_for_every_byte_truth_case() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for initial in [0_i8, 1, i8::MIN, -1] {
            let mut c = Fixture::new(revision);
            let phase = ram::g_inventory_redraw_phase(revision);
            let player = ram::g_inventory_redraw_player(revision);
            phase.write(&mut c.memory, initial);
            c.poll_values = if initial == 0 {
                [0].into_iter().collect()
            } else {
                [initial, 0, 0].into_iter().collect()
            };
            invoke(
                &mut c,
                crate::ui::waiting::inventory_refresh_contents_and_wait,
                &[1],
            )
            .unwrap();
            let request = WaitRequest::ram_byte_equal(phase.address(), 0xff, 0).condition;
            let mut expected = vec![
                Access::Wait(request),
                Access::Read(phase.address(), 1, u32::from(initial as u8)),
            ];
            if initial != 0 {
                expected.extend([
                    Access::Read(phase.address(), 1, u32::from(initial as u8)),
                    Access::Read(phase.address(), 1, 0),
                ]);
            }
            expected.extend([
                Access::Write(player.address(), 1, 1),
                Access::Write(phase.address(), 1, 1),
                Access::Wait(request),
                Access::Read(phase.address(), 1, 1),
                Access::Read(phase.address(), 1, 0),
            ]);
            let actual: Vec<_> = c
                .accesses
                .borrow()
                .iter()
                .filter(|access| match **access {
                    Access::Read(address, _, _) | Access::Write(address, _, _) => {
                        address == phase.address() || address == player.address()
                    }
                    Access::Wait(_) | Access::Yield => true,
                    _ => false,
                })
                .copied()
                .collect();
            assert_eq!(actual, expected, "initial phase {initial}");
            assert!(c.poll_values.is_empty());
            c.assert_no_frame_counter_read();
        }
    }
}

#[test]
fn redraw_contents_reloads_player_for_each_source_argument() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = Fixture::new(revision);
        ram::g_inventory_redraw_phase(revision).write(&mut c.memory, 1);
        ram::g_player_inventory_open(revision)
            .at(0)
            .write(&mut c.memory, 3);
        *c.players.borrow_mut() = [0, 1, 0, 1].into();
        invoke(&mut c, inventory_service_redraw_phase, &[]).unwrap();
        assert_eq!(
            c.byte_reads(ram::g_inventory_redraw_player(revision).address()),
            [0, 1, 0, 1]
        );
        assert_eq!(c.calls(ids::LOAD_PRESENTS_INTO_INVENTORY)[0].arguments, [1]);
        assert_eq!(c.calls(ids::INVENTORY_DRAW_BUCKS)[0].arguments, [1]);
        assert_eq!(c.sp, ENTRY_SP);
    }
}

#[test]
fn redraw_stats_keeps_player_reload_and_all_five_outgoing_longs() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = Fixture::new(revision);
        let data = c.data;
        let address = c.address;
        ram::g_inventory_redraw_phase(revision).write(&mut c.memory, 2);
        ram::g_inventory_banner_mode(revision)
            .at(0)
            .write(&mut c.memory, 2);
        *c.players.borrow_mut() = [0, 0, 1].into();
        invoke(&mut c, inventory_service_redraw_phase, &[]).unwrap();
        assert_eq!(c.calls(ids::INVENTORY_DRAW_PLAYER_STATS)[0].arguments, [0]);
        let banner = c.calls(ids::INVENTORY_DRAW_ACTION_BANNER)[0];
        assert_eq!(banner.arguments, [1, 2, 1]);
        assert_eq!(banner.retained, [1, 2, 1, 0, 1]);
        let hud = c.calls(ids::DRAW_HUD)[0];
        assert_eq!(hud.sp, ENTRY_SP - 44);
        assert_eq!(hud.retained, [1, 2, 1, 0, 1]);
        assert_eq!(&c.data[2..], &data[2..]);
        assert_eq!(&c.address[2..], &address[2..]);
        assert_eq!(c.sp, ENTRY_SP);
    }
}

#[test]
fn redraw_background_uses_byte_map_and_ordered_word_ports_for_all_phases() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for player in [0, 1] {
            for phase in [3, 4] {
                let mut c = Fixture::new(revision);
                ram::g_inventory_redraw_player(revision).write(&mut c.memory, player);
                ram::g_inventory_redraw_phase(revision).write(&mut c.memory, phase);
                invoke(&mut c, inventory_service_redraw_phase, &[]).unwrap();
                let writes: Vec<_> = c
                    .accesses
                    .borrow()
                    .iter()
                    .filter_map(|a| match *a {
                        Access::Write(at, width, value) if (0xc00000..0xc00008).contains(&at) => {
                            Some((at, width, value))
                        }
                        _ => None,
                    })
                    .collect();
                assert_eq!(writes.len(), 7 * 42);
                assert_eq!(
                    writes.iter().filter(|&&(at, _, _)| at == 0xc00000).count(),
                    280
                );
                let start = i32::from(player) * 14 + if phase == 4 { 0 } else { 7 };
                let map = rom::font_background_byte_map(revision)
                    .at(start)
                    .at(0)
                    .address();
                assert_eq!(c.byte_reads(map), [0]);
                assert_eq!(
                    ram::g_inventory_redraw_phase(revision).read(&c.memory),
                    phase - 1
                );
            }
        }
    }
}

#[test]
fn purchase_reads_price_before_bucks_and_repeats_the_rmw_operands() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for (price, bucks, success) in [
            (3, 4, true),
            (4, 3, false),
            (-128, 127, true),
            (127, -128, false),
        ] {
            let mut c = Fixture::new(revision);
            let data = c.data;
            let address = c.address;
            ram::g_player_inventory(revision)
                .at(1)
                .at(0)
                .write(&mut c.memory, -1);
            ram::g_player_mailbox_prices(revision)
                .at(1)
                .at(0)
                .write(&mut c.memory, price);
            ram::g_player_mailbox_items(revision)
                .at(1)
                .at(0)
                .write(&mut c.memory, 9);
            ram::g_player_bucks(revision)
                .at(1)
                .write(&mut c.memory, bucks);
            invoke(&mut c, inventory_buy_selected_mail_order_present, &[1]).unwrap();
            let price_address = ram::g_player_mailbox_prices(revision).at(1).at(0).address();
            let bucks_address = ram::g_player_bucks(revision).at(1).address();
            let reads: Vec<_> = c
                .accesses
                .borrow()
                .iter()
                .filter_map(|a| match *a {
                    Access::Read(at, 1, _) if at == price_address || at == bucks_address => {
                        Some(at)
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(
                reads,
                if success {
                    vec![price_address, bucks_address, price_address, bucks_address]
                } else {
                    vec![price_address, bucks_address]
                }
            );
            assert_eq!(
                ram::g_player_bucks(revision).at(1).read(&c.memory),
                if success {
                    bucks.wrapping_sub(price)
                } else {
                    bucks
                }
            );
            assert_eq!(c.calls(ids::PLAY_SOUND).len(), usize::from(!success));
            assert_eq!(&c.data[2..], &data[2..]);
            assert_eq!(&c.address[2..], &address[2..]);
            assert_eq!(c.sp, ENTRY_SP);
        }
    }
}

#[test]
fn purchase_full_inventory_stops_before_slot_sixteen_and_starts_dialogue() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = Fixture::new(revision);
        invoke(&mut c, inventory_buy_selected_mail_order_present, &[0]).unwrap();
        assert!(
            c.byte_reads(ram::g_player_inventory(revision).at(0).at(16).address())
                .is_empty()
        );
        assert!(
            c.byte_reads(ram::g_player_bucks(revision).at(0).address())
                .is_empty()
        );
        let dialogue = c.calls(ids::START_DIALOGUE)[0];
        assert_eq!(dialogue.arguments, [25, 0]);
        assert_eq!(
            dialogue.pc,
            ids::INVENTORY_BUY_SELECTED_MAIL_ORDER_PRESENT
                .metadata()
                .address(revision)
                + 0x100
        );
    }
}

#[test]
fn clear_viewport_retains_head_across_build_and_never_repairs_a6() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for viewport in [0, 1, u32::MAX] {
            let mut c = Fixture::new(revision);
            let data = c.data;
            let address = c.address;
            c.memory
                .write_u32(ram::g_entity_draw_list_head(revision).address(), 0xff4320);
            c.on_dependency = |c, id, _| {
                if id == ids::BUILD_VIEWPORT_SPRITE_LINKS {
                    c.address[6] = 0x12345678;
                }
            };
            invoke(
                &mut c,
                crate::ui::inventory_animations::entity_clear_viewport_sprites,
                &[viewport],
            )
            .unwrap();
            let position = c.calls(ids::POSITION_ENTITY_SPRITES)[0];
            let build = c.calls(ids::BUILD_VIEWPORT_SPRITE_LINKS)[0];
            assert_eq!(position.sp, ENTRY_SP - 24);
            assert_eq!(position.arguments, [0xff4320]);
            assert_eq!(build.sp, position.sp);
            assert_eq!(build.retained[0], 0xff4320);
            assert_eq!(
                build.pc,
                ids::ENTITY_CLEAR_VIEWPORT_SPRITES
                    .metadata()
                    .address(revision)
                    + 0x64
            );
            let head = ram::g_entity_draw_list_head(revision).address();
            let head_reads: Vec<_> = c
                .accesses
                .borrow()
                .iter()
                .filter(|access| {
                    matches!(access,
                Access::Read(at, _, _) if *at >= head && *at < head + 4)
                })
                .cloned()
                .collect();
            assert_eq!(
                head_reads,
                [
                    Access::Read(head, 2, 0xff),
                    Access::Read(head + 2, 2, 0x4320)
                ]
            );
            assert_eq!(c.address[6], 0x12345678);
            assert_eq!(&c.data[2..], &data[2..]);
            assert_eq!(&c.address[2..6], &address[2..6]);
            assert_eq!(c.sp, ENTRY_SP);
        }
    }
}

#[test]
fn randomize_keeps_sixty_iterations_register_residue_and_retained_arguments() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for (lives, already_open) in [(-1, 0), (0, 0), (0, 1)] {
            let mut c = Fixture::new(revision);
            let data = c.data;
            let address = c.address;
            ram::g_player_lives(revision)
                .at(1)
                .write(&mut c.memory, lives);
            ram::g_player_inventory_open(revision)
                .at(1)
                .write(&mut c.memory, already_open);
            invoke(&mut c, crate::ui::input::randomize_presents, &[0]).unwrap();
            assert_eq!(c.calls(ids::RANDOMIZE_INVENTORY).len(), 60);
            let waits = c.calls(ids::WAIT_FOR_NEXT_V_BLANK);
            assert_eq!(waits.len(), 60);
            for (index, wait) in waits.iter().enumerate() {
                assert_eq!(wait.arguments, [(index / 5) as u32]);
                assert_eq!(wait.sp, ENTRY_SP - 32);
                assert_eq!(&wait.retained[..3], &[(index / 5) as u32, 1, 0]);
                assert_eq!(wait.data[3] as u16, index as u16);
            }
            assert_eq!(
                c.calls(ids::TOGGLE_INVENTORY).len(),
                usize::from(lives >= 0 && already_open == 0)
            );
            assert_eq!(&c.data[2..], &data[2..]);
            assert_eq!(&c.address[2..], &address[2..]);
            assert_eq!(c.sp, ENTRY_SP);
        }
    }
}

#[test]
fn process_a_edge_tests_only_the_returned_present_byte_and_real_opening_array() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = Fixture::new(revision);
        ram::g_player_inventory_open(revision)
            .at(0)
            .write(&mut c.memory, 1);
        ram::g_player_processed_input(revision)
            .at(0)
            .write(&mut c.memory, 0x40);
        ram::g_player_lives(revision).at(1).write(&mut c.memory, -1);
        c.results
            .push_back((ids::INVENTORY_GET_SELECTED_PRESENT, 0x1234_0012));
        invoke(&mut c, crate::ui::input::process_inventory, &[]).unwrap();
        assert_eq!(c.calls(ids::RANDOMIZE_PRESENTS).len(), 1);
        let coordination = c.calls(ids::CHECK_IF_OPEN_FOR_BOTH)[0];
        assert_eq!(coordination.arguments, [ENTRY_SP - 8]);
        assert_eq!(c.memory.read_u32(ENTRY_SP - 8), 0);
        assert_eq!(coordination.address[6], ENTRY_SP - 4);
        assert_eq!(c.sp, ENTRY_SP);
    }
}

#[test]
fn process_uses_current_a6_for_post_callee_opening_stores_and_pointer() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = Fixture::new(revision);
        ram::g_player_inventory_open(revision)
            .at(0)
            .write(&mut c.memory, 1);
        ram::g_player_processed_input(revision)
            .at(0)
            .write(&mut c.memory, 0x40);
        c.on_dependency = |c, id, _| {
            if id == ids::TOGGLE_INVENTORY {
                c.address[6] = ENTRY_SP - 0x104;
            }
        };
        invoke(&mut c, crate::ui::input::process_inventory, &[]).unwrap();
        assert_eq!(c.memory.read_u16(ENTRY_SP - 0x108), 1);
        assert_eq!(c.memory.read_u16(ENTRY_SP - 8), 0);
        assert_eq!(
            c.calls(ids::CHECK_IF_OPEN_FOR_BOTH)[0].arguments,
            [ENTRY_SP - 0x108]
        );
        assert_eq!(c.sp, ENTRY_SP - 0x100);
    }
}

#[test]
fn modal_counter_keeps_incoming_d2_word_wrap_and_compiled_save_frame() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for seed in [i16::MIN, -1, 0, 1, 2, i16::MAX] {
            let mut c = Fixture::new(revision);
            let initial = 0x1234_0000 | u32::from(seed as u16);
            c.data[2] = initial;
            ram::g_player_inventory_open(revision)
                .at(0)
                .write(&mut c.memory, 1);
            c.close_after = 4;
            invoke(&mut c, crate::ui::input::check_b_button, &[]).unwrap();
            let services: Vec<_> = c
                .dependencies
                .iter()
                .filter(|call| {
                    call.id == ids::UPDATE_AND_QUEUE_ENTITY_SPRITES
                        || call.id == ids::WAIT_FOR_NEXT_V_BLANK
                })
                .collect();
            assert_eq!(services.len(), 4);
            for (index, service) in services.iter().enumerate() {
                let word = seed.wrapping_add(index as i16 + 1);
                assert_eq!(
                    service.id,
                    if i32::from(word) % 3 == 0 {
                        ids::UPDATE_AND_QUEUE_ENTITY_SPRITES
                    } else {
                        ids::WAIT_FOR_NEXT_V_BLANK
                    }
                );
                assert_eq!(service.data[2], 0x1234_0000 | u32::from(word as u16));
            }
            assert_eq!(c.data[2], initial);
            assert_eq!(c.sp, ENTRY_SP);
            assert_eq!(c.calls(ids::PROCESS_INVENTORY)[0].sp, ENTRY_SP - 12);
        }
    }
}

#[test]
fn special_wrappers_save_actual_d2_slot_and_publish_player_residue() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for (menu, body, caller) in [
            (
                2,
                crate::ui::input::inventory_open_mail_order_menu as crate::FramedTranslationFn,
                ids::INVENTORY_OPEN_MAIL_ORDER_MENU,
            ),
            (
                3,
                crate::ui::input::inventory_open_identification_menu as crate::FramedTranslationFn,
                ids::INVENTORY_OPEN_IDENTIFICATION_MENU,
            ),
        ] {
            let mut c = Fixture::new(revision);
            c.data[2] = 0x1234_abcd;
            c.on_dependency = |c, id, _| {
                if id == ids::CHECK_B_BUTTON {
                    c.memory.write_u32(c.sp + 4, 0x8765_4321);
                }
            };
            invoke(&mut c, body, &[1]).unwrap();
            let pushes: Vec<_> = c
                .accesses
                .borrow()
                .iter()
                .filter_map(|access| match *access {
                    Access::Write(address, width, value) => Some((address, width, value)),
                    _ => None,
                })
                .take(6)
                .collect();
            assert_eq!(
                pushes,
                [
                    (ENTRY_SP - 2, 2, 0xabcd),
                    (ENTRY_SP - 4, 2, 0x1234),
                    (ENTRY_SP - 8, 2, 0),
                    (ENTRY_SP - 6, 2, menu),
                    (ENTRY_SP - 10, 2, 1),
                    (ENTRY_SP - 12, 2, 0),
                ],
                "MOVE.L pushes low first; the intervening PEA pushes high first"
            );
            assert_eq!(c.calls(ids::DRAW_INVENTORY)[0].arguments, [1, menu]);
            let input = c.calls(ids::CHECK_B_BUTTON)[0];
            assert_eq!(input.data[2], 0x1234_0001);
            assert_eq!(input.sp, ENTRY_SP - 8);
            assert_eq!(input.pc, caller.metadata().address(revision) + 0x3a);
            assert_eq!(c.data[2], 0x8765_4321);
            assert_eq!(
                ram::g_player_processed_input(revision)
                    .at(1)
                    .read(&c.memory),
                0x40
            );
            assert_eq!(
                ram::g_player_previous_processed_input(revision)
                    .at(1)
                    .read(&c.memory),
                0x40
            );
            assert_eq!(c.sp, ENTRY_SP);
        }
    }
}

#[test]
fn open_present_preserves_mystery_and_duplicate_compaction_reads() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for mystery in [false, true] {
            let mut c = Fixture::new(revision);
            let data = c.data;
            let address = c.address;
            for (slot, value) in [(0, if mystery { 26 } else { 4 }), (1, 2), (2, 3), (3, -1)] {
                ram::g_player_inventory(revision)
                    .at(0)
                    .at(slot)
                    .write(&mut c.memory, value);
            }
            if mystery {
                c.results.extend([
                    (ids::SELECT_RANDOM_PRESENT, 26),
                    (ids::SELECT_RANDOM_PRESENT, 4),
                ]);
            }
            assert_eq!(invoke(&mut c, open_present, &[0]).unwrap(), Some(5));
            assert_eq!(c.calls(ids::OPEN_SELECTED_PRESENT)[0].arguments, [0, 4]);
            assert_eq!(
                c.byte_reads(ram::g_player_inventory(revision).at(0).at(1).address()),
                [2, 2]
            );
            assert_eq!(
                c.byte_reads(ram::g_player_inventory(revision).at(0).at(2).address()),
                [3, 3]
            );
            assert_eq!(
                ram::g_present_info(revision)
                    .at(4)
                    .identified()
                    .read(&c.memory),
                i8::from(!mystery)
            );
            assert_eq!(&c.data[2..], &data[2..]);
            assert_eq!(&c.address[2..], &address[2..]);
            assert_eq!(c.sp, ENTRY_SP);
        }
    }
}

#[test]
fn coordination_keeps_short_opening_reads_exclusions_and_partner_calls() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        for present in [0, 8, 15, 12, 19, 23, 25] {
            let mut c = Fixture::new(revision);
            let pointer = 0xff6000;
            c.memory.write_u16(pointer, 1);
            ram::g_player_inventory_open(revision)
                .at(1)
                .write(&mut c.memory, 1);
            c.results.push_back((ids::OPEN_PRESENT, present + 1));
            invoke(&mut c, check_if_open_for_both, &[pointer]).unwrap();
            assert_eq!(c.calls(ids::TOGGLE_INVENTORY).len(), 1);
            assert_eq!(
                c.calls(ids::OPEN_SELECTED_PRESENT).len(),
                usize::from(present == 0)
            );
            let pointer_reads: Vec<_> = c
                .accesses
                .borrow()
                .iter()
                .filter(|a| {
                    matches!(a,
                Access::Read(at, 2, 1) if *at == pointer)
                })
                .cloned()
                .collect();
            assert_eq!(pointer_reads.len(), 2);
            assert_eq!(c.sp, ENTRY_SP);
        }
    }
}

#[test]
fn synchronous_redraw_uses_four_real_calls_and_source_byte_test() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = Fixture::new(revision);
        c.on_dependency = |c, id, _| {
            assert_eq!(id, ids::INVENTORY_SERVICE_REDRAW_PHASE);
            let phase = ram::g_inventory_redraw_phase(c.revision());
            let next = phase.read(&c.memory) - 1;
            phase.write(&mut c.memory, next);
        };
        invoke(&mut c, inventory_redraw_synchronously, &[1]).unwrap();
        assert_eq!(
            c.byte_reads(ram::g_inventory_redraw_phase(revision).address()),
            [4, 3, 2, 1, 0]
        );
        assert_eq!(c.dependencies.len(), 4);
        assert!(c.dependencies.iter().all(|call| {
            call.pc
                == ids::INVENTORY_REDRAW_SYNCHRONOUSLY
                    .metadata()
                    .address(revision)
                    + 0x18
        }));
        assert_eq!(c.sp, ENTRY_SP);
    }
}

#[test]
fn framed_inventory_reads_the_source_word_without_eager_slot_long_reads() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = Fixture::new(revision);
        ram::g_player_elevator_flags(revision)
            .at(1)
            .write(&mut c.memory, 1);
        ram::g_elevator_departure_in_progress(revision).write(&mut c.memory, 1);
        invoke(&mut c, toggle_inventory, &[0x1234_0001]).unwrap();
        let arguments: Vec<_> = c.accesses.borrow().iter().filter(|access|
            matches!(access, Access::Read(address, _, _) if *address >= ENTRY_SP + 4 && *address < ENTRY_SP + 8)
        ).cloned().collect();
        assert_eq!(arguments, [Access::Read(ENTRY_SP + 6, 2, 1)]);
        assert!(c.dependencies.is_empty());
        assert_eq!(c.sp, ENTRY_SP);
    }
}

#[test]
fn modal_counter_uses_the_actual_post_callee_d2_residue() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = Fixture::new(revision);
        c.data[2] = 0x1234_0000;
        ram::g_player_inventory_open(revision)
            .at(0)
            .write(&mut c.memory, 1);
        c.close_after = 1;
        c.on_dependency = |context, id, _| {
            if id == ids::PROCESS_INVENTORY {
                context.data[2] = 0x1234_0002;
            }
        };
        invoke(&mut c, crate::ui::input::check_b_button, &[]).unwrap();
        assert_eq!(c.calls(ids::UPDATE_AND_QUEUE_ENTITY_SPRITES).len(), 1);
        assert!(c.calls(ids::WAIT_FOR_NEXT_V_BLANK).is_empty());
        assert_eq!(c.data[2], 0x1234_0000);
        assert_eq!(c.sp, ENTRY_SP);
    }
}

#[test]
fn draw_clr_reads_are_discarded_and_have_the_source_widths() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = Fixture::new(revision);
        ram::g_split_screen_enabled(revision).write(&mut c.memory, 1);
        c.phases = [2, 0].into();
        invoke(&mut c, draw_inventory, &[0, 1]).unwrap();
        let discarded: Vec<_> = c
            .accesses
            .borrow()
            .iter()
            .filter_map(|access| match *access {
                Access::Discard(address, width, _) if address != ENTRY_SP => Some((address, width)),
                _ => None,
            })
            .collect();
        assert!(
            c.accesses
                .borrow()
                .iter()
                .any(|access| matches!(access, Access::Discard(ENTRY_SP, 2, _))),
            "MOVEM terminal word must be tagged discarded"
        );
        let mut expected = vec![
            (
                ram::g_player_info(revision)
                    .at(0)
                    .frame_duration()
                    .address(),
                1,
            ),
            (ram::g_inventory_banner_mode(revision).at(0).address(), 1),
        ];
        if revision == Revision::Rev02 {
            expected.push((ram::g_player_idle_ticks(revision).at(0).address(), 2));
        }
        assert_eq!(discarded, expected);
    }
}

#[test]
fn toggle_zero_argument_uses_clr_discard_words_before_writing() {
    for revision in [Revision::Rev02, Revision::Rev00] {
        let mut c = Fixture::new(revision);
        ram::g_player_inventory_open(revision)
            .at(0)
            .write(&mut c.memory, 1);
        invoke(&mut c, toggle_inventory, &[0]).unwrap();
        let accesses = c.accesses.borrow();
        let start = accesses.iter().position(|access|
            matches!(access, Access::Discard(address, 2, _) if *address == ENTRY_SP - 12)).unwrap();
        let actual: Vec<_> = accesses[start..start + 4]
            .iter()
            .map(|access| match *access {
                Access::Discard(address, width, _) => ("discard", address, width),
                Access::Write(address, width, _) => ("write", address, width),
                other => panic!("unexpected CLR transfer {other:?}"),
            })
            .collect();
        assert_eq!(
            actual,
            [
                ("discard", ENTRY_SP - 12, 2),
                ("discard", ENTRY_SP - 10, 2),
                ("write", ENTRY_SP - 12, 2),
                ("write", ENTRY_SP - 10, 2),
            ]
        );
        assert_eq!(
            c.calls(ids::FORCE_TEXT_BACKGROUND_COLOR_TO_BLACK)[0].arguments,
            [0]
        );
    }
}
