//! The game's 68000 address space, not host-native C structs.
//!
//! All pointer values remain 32-bit addresses. Generated record types are markers:
//! `ObjectState::SIZE` describes the emulated layout, never `size_of::<ObjectState>()`.
//! Views own only an address and never borrow bytes, so overlapping aliases, original
//! out-of-bounds accesses, and pointers into either ROM or RAM remain representable.

use std::marker::PhantomData;

pub use tje_rom::Revision;

pub mod arithmetic;
pub mod examples;
mod generated;
pub use generated::*;

pub const RAM_START: u32 = 0xff0000;
pub const RAM_SIZE: usize = 0x10000;
pub const ADDRESS_MASK: u32 = 0x00ff_ffff;

/// Byte primitives for shared-emulator adapters; override wider operations if the
/// adapter also maps hardware whose access width has observable side effects.
pub trait Bus {
    fn read_u8(&self, address: u32) -> u8;
    fn write_u8(&mut self, address: u32, value: u8);

    fn read_i8(&self, address: u32) -> i8 {
        self.read_u8(address) as i8
    }
    fn read_u16(&self, address: u32) -> u16 {
        u16::from_be_bytes([self.read_u8(address), self.read_u8(address.wrapping_add(1))])
    }
    fn read_i16(&self, address: u32) -> i16 {
        self.read_u16(address) as i16
    }
    fn read_u32(&self, address: u32) -> u32 {
        u32::from_be_bytes([
            self.read_u8(address),
            self.read_u8(address.wrapping_add(1)),
            self.read_u8(address.wrapping_add(2)),
            self.read_u8(address.wrapping_add(3)),
        ])
    }
    fn read_i32(&self, address: u32) -> i32 {
        self.read_u32(address) as i32
    }
    fn write_i8(&mut self, address: u32, value: i8) {
        self.write_u8(address, value as u8);
    }
    fn write_u16(&mut self, address: u32, value: u16) {
        for (offset, byte) in value.to_be_bytes().into_iter().enumerate() {
            self.write_u8(address.wrapping_add(offset as u32), byte);
        }
    }
    fn write_i16(&mut self, address: u32, value: i16) {
        self.write_u16(address, value as u16);
    }
    fn write_u32(&mut self, address: u32, value: u32) {
        for (offset, byte) in value.to_be_bytes().into_iter().enumerate() {
            self.write_u8(address.wrapping_add(offset as u32), byte);
        }
    }
    fn write_i32(&mut self, address: u32, value: i32) {
        self.write_u32(address, value as u32);
    }
}

/// ROM is immutable, RAM is exactly 64 KiB. The Genesis decodes RAM throughout
/// E00000..FFFFFF, mirroring its low 16 address bits. The canonical names use FF0000.
/// Storage operations deliberately allow unaligned accesses; instruction-level
/// address-error exceptions and non-memory devices belong to the CPU/platform host.
#[derive(Clone)]
pub struct AddressSpace {
    rom: Vec<u8>,
    ram: Box<[u8; RAM_SIZE]>,
    revision: Revision,
}

impl AddressSpace {
    /// Also accepts synthetic ROMs for ROM-free tests; production imports go through tje-rom.
    pub fn new(rom: Vec<u8>, revision: Revision) -> Self {
        assert!(
            rom.len() <= 0x400000,
            "cartridge exceeds the Genesis ROM window"
        );
        Self {
            rom,
            ram: vec![0; RAM_SIZE].into_boxed_slice().try_into().unwrap(),
            revision,
        }
    }
    pub fn rom(&self) -> &[u8] {
        &self.rom
    }
    pub fn ram(&self) -> &[u8; RAM_SIZE] {
        &self.ram
    }
    pub fn ram_mut(&mut self) -> &mut [u8; RAM_SIZE] {
        &mut self.ram
    }
    pub fn revision(&self) -> Revision {
        self.revision
    }
}

impl Bus for AddressSpace {
    fn read_u8(&self, address: u32) -> u8 {
        let address = address & ADDRESS_MASK;
        if address >= 0xe00000 {
            self.ram[(address & 0xffff) as usize]
        } else if let Some(value) = self.rom.get(address as usize) {
            *value
        } else {
            panic!("unmapped memory read at {address:#08x}; use a platform bus for devices")
        }
    }
    fn write_u8(&mut self, address: u32, value: u8) {
        let address = address & ADDRESS_MASK;
        if address >= 0xe00000 {
            self.ram[(address & 0xffff) as usize] = value;
        } else if address < 0x400000 {
            // Ordinary ROM cartridge writes have no storage effect (no SRAM in this game).
        } else {
            panic!("unmapped memory write at {address:#08x}; use a platform bus for devices")
        }
    }
}

/// Size of an emulated C type. This is independent of the host ABI and pointer size.
pub trait Target {
    const SIZE: u32;
}

/// A typed address into emulated storage. Offsets intentionally do not check a C
/// array bound; crossing that bound must access the adjacent bytes, not host UB.
#[repr(transparent)]
pub struct Address<T> {
    value: u32,
    marker: PhantomData<fn() -> T>,
}
impl<T> Copy for Address<T> {}
impl<T> Clone for Address<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> PartialEq for Address<T> {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}
impl<T> Eq for Address<T> {}
impl<T> std::fmt::Debug for Address<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Address({:#010x})", self.value)
    }
}
impl<T> Address<T> {
    pub const fn new(value: u32) -> Self {
        Self {
            value,
            marker: PhantomData,
        }
    }
    pub const fn address(self) -> u32 {
        self.value
    }
    pub const fn cast<U>(self) -> Address<U> {
        Address::new(self.value)
    }
    pub const fn byte_offset(self, bytes: i32) -> Self {
        Self::new(self.value.wrapping_add(bytes as u32))
    }
}
impl<T: Target> Address<T> {
    pub const fn offset(self, count: i32) -> Self {
        self.byte_offset((count as u32).wrapping_mul(T::SIZE) as i32)
    }
}

/// A pointer *value* read from storage, not a Rust reference. Null is representable;
/// dereferencing it simply addresses ROM at zero, as it did on this cartridge.
#[repr(transparent)]
pub struct Ptr<T>(Address<T>);
impl<T> Copy for Ptr<T> {}
impl<T> Clone for Ptr<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> PartialEq for Ptr<T> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl<T> Eq for Ptr<T> {}
impl<T> std::fmt::Debug for Ptr<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
impl<T> Ptr<T> {
    pub const fn new(value: u32) -> Self {
        Self(Address::new(value))
    }
    pub const fn address(self) -> u32 {
        self.0.address()
    }
    pub const fn target(self) -> Address<T> {
        self.0
    }
}
impl<T> Target for Ptr<T> {
    const SIZE: u32 = 4;
}

/// Void and function pointers stay numeric; calling a function requires the host's
/// address dispatcher rather than converting a cartridge word into a Rust fn pointer.
pub struct Opaque;
pub struct Function;
impl Target for Opaque {
    const SIZE: u32 = 1;
}
impl Target for Function {
    const SIZE: u32 = 1;
}

/// An incomplete outer bound does not assert a capacity or a record stride.
pub struct OpenArray<T>(PhantomData<T>);
impl<T: Target, const N: usize> Target for [T; N] {
    const SIZE: u32 = T::SIZE * N as u32;
}
impl<T: Target, const N: usize> Address<[T; N]> {
    pub const fn at(self, index: i32) -> Address<T> {
        self.cast::<T>().offset(index)
    }
    pub const fn declared_len(self) -> usize {
        N
    }
}
impl<T: Target> Address<OpenArray<T>> {
    pub const fn at(self, index: i32) -> Address<T> {
        self.cast::<T>().offset(index)
    }
}

pub trait Scalar: Target + Copy {
    fn read(bus: &(impl Bus + ?Sized), address: u32) -> Self;
    fn write(self, bus: &mut (impl Bus + ?Sized), address: u32);
}
impl<T: Scalar> Address<T> {
    pub fn read(self, bus: &(impl Bus + ?Sized)) -> T {
        T::read(bus, self.value)
    }
    pub fn write(self, bus: &mut (impl Bus + ?Sized), value: T) {
        value.write(bus, self.value);
    }
}
macro_rules! scalar {
    ($ty:ty, $size:expr, $read:ident, $write:ident) => {
        impl Target for $ty {
            const SIZE: u32 = $size;
        }
        impl Scalar for $ty {
            fn read(bus: &(impl Bus + ?Sized), address: u32) -> Self {
                bus.$read(address)
            }
            fn write(self, bus: &mut (impl Bus + ?Sized), address: u32) {
                bus.$write(address, self);
            }
        }
    };
}
scalar!(u8, 1, read_u8, write_u8);
scalar!(i8, 1, read_i8, write_i8);
scalar!(u16, 2, read_u16, write_u16);
scalar!(i16, 2, read_i16, write_i16);
scalar!(u32, 4, read_u32, write_u32);
scalar!(i32, 4, read_i32, write_i32);
impl<T> Scalar for Ptr<T> {
    fn read(bus: &(impl Bus + ?Sized), address: u32) -> Self {
        Self::new(bus.read_u32(address))
    }
    fn write(self, bus: &mut (impl Bus + ?Sized), address: u32) {
        bus.write_u32(address, self.address());
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldLayout {
    pub name: &'static str,
    pub offset: u32,
    pub size: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StructLayout {
    pub name: &'static str,
    pub source: &'static str,
    pub size: u32,
    pub alignment: u32,
    pub fields: &'static [FieldLayout],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RamSymbol {
    pub name: &'static str,
    pub address: u32,
    pub c_type: &'static str,
    /// Header declaration, a structural alias, or explicitly opaque storage.
    pub source: &'static str,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RomDataUnit {
    pub name: &'static str,
    pub kind: &'static str,
    pub address: u32,
    pub size: u32,
}

impl Address<RenderFrame> {
    /// The bounded C access views are not ROM strides: the actual tail is count * 10 bytes.
    pub fn byte_len(self, bus: &(impl Bus + ?Sized)) -> u32 {
        RenderFrame::SIZE + u32::from(self.piece_count().read(bus)) * RenderPiece::SIZE
    }
    pub const fn piece(self, index: i32) -> Address<RenderPiece> {
        Address::new(self.value.wrapping_add(RenderFrame::SIZE)).offset(index)
    }
}

/// Width-preserving VDP port writes. A 68000 longword command is two ordered
/// control-port word writes; implementations may override it while preserving that trace.
pub trait Platform {
    fn write_vdp_control(&mut self, value: u16);
    fn write_vdp_data(&mut self, value: u16);
    fn write_vdp_command(&mut self, value: u32) {
        self.write_vdp_control((value >> 16) as u16);
        self.write_vdp_control(value as u16);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory() -> AddressSpace {
        AddressSpace::new(vec![0x12, 0x34, 0x80, 0xff], Revision::Rev02)
    }

    #[test]
    fn endian_signed_widths_and_immutable_rom() {
        let mut m = memory();
        assert_eq!(m.read_u16(0), 0x1234);
        assert_eq!(m.read_i8(2), -128);
        assert_eq!(m.read_i16(2), -32513);
        m.write_u32(0, 0);
        assert_eq!(m.read_u32(0), 0x123480ff);
        m.write_u32(RAM_START + 1, 0x89abcdef);
        assert_eq!(&m.ram()[1..5], &[0x89, 0xab, 0xcd, 0xef]);
        assert_eq!(m.read_i32(RAM_START + 1), 0x89abcdef_u32 as i32);
        m.write_i16(RAM_START + 2, -2);
        m.write_i8(RAM_START + 4, -1);
        assert_eq!(m.read_u32(RAM_START + 1), 0x89fffeff);
        m.write_i32(RAM_START + 8, i32::MIN);
        assert_eq!(m.read_u32(RAM_START + 8), 0x80000000);
    }

    #[test]
    fn physical_address_and_ram_mirrors() {
        let mut m = memory();
        m.write_u16(0xff_fffffe, 0xbeef);
        assert_eq!(m.read_u16(0xe0fffe), 0xbeef);
        m.write_u8(0xffff_ffff, 0xa5);
        assert_eq!(m.ram()[65535], 0xa5);
        assert_eq!(m.read_u16(0x00ff_ffff), 0xa512);
    }

    #[test]
    fn records_aliases_and_original_out_of_bounds_are_flat() {
        let mut m = memory();
        let player = ram::g_player_info(Revision::Rev02).at(1);
        player.level().write(&mut m, -1);
        assert_eq!(ram::g_earl_level(Revision::Rev02).read(&m), -1);
        assert_eq!(
            player.address() - ram::g_player_info(Revision::Rev02).at(0).address(),
            128
        );
        let bank = ram::g_earthling_banks(Revision::Rev02).at(0);
        bank.at(20).kind().write(&mut m, 17);
        assert_eq!(
            ram::g_earthling_banks(Revision::Rev02)
                .at(1)
                .at(0)
                .kind()
                .read(&m),
            17
        );
        assert_eq!(bank.at(-1).address(), bank.address() - EarthlingState::SIZE);
    }

    #[test]
    fn pointers_remain_big_endian_rom_or_ram_addresses() {
        let mut m = memory();
        let field = ram::g_player_info(Revision::Rev02).at(0).sequence();
        field.write(&mut m, Ptr::new(2));
        assert_eq!(field.read(&m).target().read(&m), -128);
        assert_eq!(m.read_u32(field.address()), 2);
        field.write(&mut m, Ptr::new(RAM_START + 8));
        m.write_i8(RAM_START + 8, -3);
        assert_eq!(field.read(&m).target().read(&m), -3);
        field.write(&mut m, Ptr::new(0));
        assert_eq!(field.read(&m).target().read(&m), 0x12);
    }

    #[test]
    fn variable_frames_do_not_use_bounded_view_size_as_stride() {
        let mut m = memory();
        let frame = Address::<RenderFrame>::new(RAM_START + 0x100);
        frame.piece_count().write(&mut m, 3);
        frame.piece(2).tile_data().write(&mut m, Ptr::new(2));
        assert_eq!(frame.byte_len(&m), 34);
        assert_eq!(frame.piece(2).address(), frame.address() + 24);
        assert_eq!(frame.piece(2).tile_data().read(&m).target().read(&m), -128);
        assert_eq!(RenderFramePieceView::SIZE, 124);
        assert_eq!(RenderFrameFourPieceView::SIZE, 44);
    }

    #[test]
    #[should_panic(expected = "unmapped memory read")]
    fn unmapped_devices_are_not_fake_zero_memory() {
        memory().read_u8(0xc00004);
    }
}
