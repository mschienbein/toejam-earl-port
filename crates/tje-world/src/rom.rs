//! Read-only, checked runtime imports from the user's cartridge. Nothing here
//! copies ROM tables into a public asset or an authoritative world sidecar.
use crate::{Cell, Error, LevelId, Resources, Result, Shape};
use tje_mem::{AddressSpace, Bus, Revision, rom};

pub struct RomResources<'a, B: Bus + ?Sized = AddressSpace> {
    bus: &'a B,
    revision: Revision,
    rom_len: usize,
}
impl<'a> RomResources<'a> {
    pub fn new(memory: &'a AddressSpace) -> Result<Self> {
        Self::checked(memory, memory.revision(), memory.rom().len())
    }
}
impl<'a, B: Bus + ?Sized> RomResources<'a, B> {
    /// Enhanced callbacks run only on a verified retail cartridge. Reads are
    /// confined to that revision's ROM window, never RAM or a device register.
    pub fn from_verified_bus(bus: &'a B, revision: Revision) -> Result<Self> {
        Self::checked(bus, revision, revision.rom_size())
    }
    fn checked(bus: &'a B, revision: Revision, rom_len: usize) -> Result<Self> {
        let value = Self {
            bus,
            revision,
            rom_len,
        };
        for (address, length) in [
            (
                rom::level_metatile_maps(value.revision).address(),
                122 * 128 * 2,
            ),
            (
                rom::overlay_metatile_maps(value.revision).address(),
                15 * 128 * 2,
            ),
            (
                rom::player_tile_property_lookup(value.revision).address(),
                398,
            ),
            (
                rom::map_generation_lookup_tables(value.revision).address(),
                196,
            ),
            (
                rom::tile_alternative_pointers(value.revision).address(),
                98 * 4,
            ),
            (
                rom::map_transition_descriptors(value.revision).address(),
                16 * 10,
            ),
        ] {
            value.range(address, length)?;
        }
        Ok(value)
    }
    fn range(&self, address: u32, length: usize) -> Result<()> {
        let end = (address as usize)
            .checked_add(length)
            .ok_or(Error::Resource("ROM range overflow"))?;
        if end <= self.rom_len {
            Ok(())
        } else {
            Err(Error::Resource("ROM table/pointer outside cartridge"))
        }
    }
    fn byte(&self, address: u32) -> Result<u8> {
        self.range(address, 1)?;
        Ok(self.bus.read_u8(address))
    }
    fn word(&self, address: u32) -> Result<u16> {
        self.range(address, 2)?;
        Ok(self.bus.read_u16(address))
    }
    fn pointer(&self, address: u32) -> Result<u32> {
        self.range(address, 4)?;
        let pointer = self.bus.read_u32(address);
        self.range(pointer, 1)?;
        Ok(pointer)
    }
}
impl<B: Bus + ?Sized> Resources for RomResources<'_, B> {
    fn parameter(&self, level: LevelId, parameter: u8) -> Result<(u8, u8)> {
        if !level.ordinary() || parameter >= 8 {
            return Err(Error::Resource("ordinary generation parameter"));
        }
        let row = self.byte(
            rom::map_generation_range_row(self.revision).address() + u32::from(level.get() - 2),
        )?;
        if row >= 8 {
            return Err(Error::Resource("generation range row"));
        }
        let address = rom::map_generation_range_pairs(self.revision).address()
            + u32::from(row) * 16
            + u32::from(parameter) * 2;
        let minimum = self.byte(address)?;
        let maximum = self.byte(address + 1)?;
        if minimum > maximum {
            return Err(Error::Resource("generation range reversed"));
        }
        Ok((minimum, maximum))
    }
    fn shape(&self, kind: Shape, corners: u8, alternate: bool) -> Result<u8> {
        if corners > 15 {
            return Err(Error::Resource("shape corner/direction mask"));
        }
        let offset = match kind {
            Shape::Path => 0,
            Shape::Edge => {
                if alternate {
                    36
                } else {
                    16
                }
            }
            Shape::Filled => {
                if alternate {
                    88
                } else {
                    56
                }
            }
        };
        let shape = self.byte(
            rom::map_generation_lookup_tables(self.revision).address()
                + offset
                + u32::from(corners),
        )?;
        if shape >= 98 {
            return Err(Error::Resource("unresolved generation shape"));
        }
        Ok(shape)
    }
    fn alternative(&self, shape: u8, draw: u32) -> Result<Cell> {
        if shape >= 98 {
            return Err(Error::Resource("alternative shape index"));
        }
        let pointer = self.pointer(
            rom::tile_alternative_pointers(self.revision).address() + u32::from(shape) * 4,
        )?;
        let count = self.byte(pointer)?;
        if count == 0 || count > 64 {
            return Err(Error::Resource("alternative count must be 1..=64"));
        }
        self.range(pointer + 1, usize::from(count))?;
        Cell::new(self.byte(pointer + 1 + draw % u32::from(count))?, 0)
    }
    fn transition(&self, shape: u8, descriptor: u8, draw: u32) -> Result<Option<Cell>> {
        if descriptor >= 16 {
            return Err(Error::Resource("transition descriptor"));
        }
        let address =
            rom::map_transition_descriptors(self.revision).address() + u32::from(descriptor) * 10;
        let bank = self.byte(address)?;
        let shapes = self.pointer(address + 2)?;
        let alternatives = self.pointer(address + 6)?;
        let count = self.byte(shapes)?;
        if count == 0 || count > 64 {
            return Err(Error::Resource("transition list must be 1..=64"));
        }
        self.range(shapes + 1, usize::from(count))?;
        self.range(alternatives, usize::from(count) * 2)?;
        for index in 0..u32::from(count) {
            if self.byte(shapes + 1 + index)? == shape {
                let overlay = if index >= u32::from(count) / 2 && bank & 15 != 15 {
                    bank.checked_add(7)
                        .ok_or(Error::Resource("transition overlay overflow"))?
                } else {
                    bank
                };
                let base = self.byte(alternatives + index * 2 + (draw & 1))?;
                return Cell::new(base, overlay).map(Some);
            }
        }
        Ok(None)
    }
    fn base_tile(&self, base: u8, row: u8, column: u8) -> Result<u16> {
        if base & 0x7f >= 122 || row >= 8 || column >= 16 {
            return Err(Error::Resource("base tile coordinates"));
        }
        let mirrored = base & 0x80 != 0;
        let column = if mirrored { 15 - column } else { column };
        let address = rom::level_metatile_maps(self.revision).address()
            + u32::from(base & 0x7f) * 256
            + u32::from(row) * 32
            + u32::from(column) * 2;
        Ok(self.word(address)? ^ if mirrored { 0x0800 } else { 0 })
    }
    fn overlay_tile(&self, overlay: u8, row: u8, column: u8) -> Result<u16> {
        if overlay == 0 {
            return Ok(0);
        }
        if overlay > 31 || overlay == 16 || row >= 8 || column >= 16 {
            return Err(Error::Resource("overlay coordinates"));
        }
        let mirrored = overlay & 16 != 0;
        let column = if mirrored { 15 - column } else { column };
        let address = rom::overlay_metatile_maps(self.revision).address()
            + u32::from((overlay & 15) - 1) * 256
            + u32::from(row) * 32
            + u32::from(column) * 2;
        let tile = self.word(address)?;
        Ok(if tile == 0 {
            0
        } else {
            tile ^ if mirrored { 0x0800 } else { 0 }
        })
    }
    fn property(&self, tile: u16) -> Result<u8> {
        if tile & 0x7ff >= 398 {
            return Err(Error::Resource("terrain property tile index"));
        }
        self.byte(
            rom::player_tile_property_lookup(self.revision).address() + u32::from(tile & 0x7ff),
        )
    }
}
