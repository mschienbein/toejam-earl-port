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

impl<B: Bus + ?Sized> crate::land::PathResources for RomResources<'_, B> {
    fn path_shape(&self, metatile: u8) -> Result<Option<u8>> {
        for shape in 0..16 {
            let base = self.shape(Shape::Path, shape, false)?;
            let pointer = self.pointer(
                rom::tile_alternative_pointers(self.revision).address() + u32::from(base) * 4,
            )?;
            let count = self.byte(pointer)?;
            if count == 0 || count > 64 {
                return Err(Error::Resource("path alternative count"));
            }
            self.range(pointer + 1, usize::from(count))?;
            for index in 0..u32::from(count) {
                if self.byte(pointer + 1 + index)? == metatile {
                    return Ok(Some(shape));
                }
            }
        }
        Ok(None)
    }
    fn edge_boundary(&self, boundary: u8, alternate: bool) -> Result<u8> {
        if !(16..=19).contains(&boundary) {
            return Err(Error::Resource("path edge boundary"));
        }
        let value = self.byte(
            rom::map_generation_lookup_tables(self.revision).address()
                + if alternate { 36 } else { 16 }
                + u32::from(boundary),
        )?;
        if value >= 98 {
            return Err(Error::Resource("path edge shape"));
        }
        Ok(value)
    }
}

pub struct SpecialRomSource<'r, 'a, B: Bus + ?Sized> {
    resources: &'r RomResources<'a, B>,
    kind: crate::loading::SpecialFloor,
}
impl<B: Bus + ?Sized> RomResources<'_, B> {
    pub fn special_source(
        &self,
        kind: crate::loading::SpecialFloor,
    ) -> SpecialRomSource<'_, '_, B> {
        SpecialRomSource {
            resources: self,
            kind,
        }
    }
    pub fn fixed_seed(&self, level: LevelId) -> Result<u16> {
        if level.get() >= 26 {
            return Err(Error::Bounds("fixed campaign seed floor"));
        }
        self.word(rom::fixed_world_seeds(self.revision).address() + u32::from(level.get()) * 2)
    }
}
impl<B: Bus + ?Sized> SpecialRomSource<'_, '_, B> {
    fn raw_word(&self, index: usize) -> Result<u16> {
        use crate::loading::SpecialFloor;
        if index >= crate::Dimensions::ORIGINAL.area() {
            return Err(Error::Bounds("special floor cell"));
        }
        let row = index / 19;
        let column = index % 19;
        let resources = self.resources;
        let revision = resources.revision;
        Ok(match self.kind {
            SpecialFloor::Zero if (12..17).contains(&row) && (9..13).contains(&column) => {
                u16::from(resources.byte(
                    rom::level0_map_data(revision).address() + ((row - 12) * 4 + column - 9) as u32,
                )?)
            }
            SpecialFloor::Zero => 34,
            SpecialFloor::One => {
                u16::from(resources.byte(rom::level1_map_data(revision).address() + index as u32)?)
            }
            SpecialFloor::Final => {
                resources.word(rom::funkatron_map_data(revision).address() + index as u32 * 2)?
            }
        })
    }
}
impl<B: Bus + ?Sized> crate::loading::CellSource for SpecialRomSource<'_, '_, B> {
    fn cell(&self, index: usize) -> Result<crate::loading::SourceCell> {
        let word = self.raw_word(index)?;
        Ok(crate::loading::SourceCell {
            cell: Cell::from_word(word & 0x1fff)?,
            directions: 0,
            marked: word & 0x4000 != 0,
            alternate: word & 0x8000 != 0,
        })
    }
}

impl<B: Bus + ?Sized> crate::original::resources::OriginalResources for RomResources<'_, B> {
    fn original_revision(&self) -> crate::original::OriginalRevision {
        self.revision.into()
    }
    fn original_shape(&self, bank: crate::original::resources::ShapeBank, index: u8) -> Result<u8> {
        use crate::original::resources::ShapeBank;
        let tables = rom::map_generation_lookup_tables(self.revision);
        let (address, limit) = match bank {
            ShapeBank::Path => (tables.tiles().address(), 16),
            ShapeBank::EdgePlain => (tables.edge_plain().address(), 20),
            ShapeBank::EdgeAlternate => (tables.edge_alternate().address(), 20),
            ShapeBank::FilledPlain => (tables.filled_plain().address(), 32),
            ShapeBank::FilledAlternate => (tables.filled_alternate().address(), 32),
            ShapeBank::BoundaryPlain => {
                (rom::map_boundary_shapes_plain(self.revision).address(), 20)
            }
            ShapeBank::BoundaryAlternate => (
                rom::map_boundary_shapes_alternate(self.revision).address(),
                20,
            ),
        };
        if index >= limit {
            return Err(Error::Resource("OriginalV1 shape table index"));
        }
        let value = self.byte(address + u32::from(index))?;
        if value >= 98 {
            return Err(Error::Resource("OriginalV1 shape identity"));
        }
        Ok(value)
    }
    fn original_turn(&self, direction: u8, choice: u8) -> Result<u8> {
        if ![1, 2, 4, 8].contains(&direction) || !(1..=3).contains(&choice) {
            return Err(Error::Resource("OriginalV1 turn lookup"));
        }
        let value = self.byte(
            rom::map_path_turn_directions(self.revision).address()
                + u32::from(direction)
                + u32::from(choice),
        )?;
        if ![1, 2, 4, 8].contains(&value) {
            return Err(Error::Resource("OriginalV1 turn direction"));
        }
        Ok(value)
    }
    fn original_transition_contains(&self, shape: u8, descriptor: u8) -> Result<bool> {
        if !(1..=16).contains(&descriptor) {
            return Err(Error::Resource("OriginalV1 transition descriptor"));
        }
        let address = rom::map_transition_descriptors(self.revision).address()
            + u32::from(descriptor - 1) * 10;
        let shapes = self.pointer(address + 2)?;
        let count = self.byte(shapes)?;
        if count == 0 || count > 64 {
            return Err(Error::Resource("OriginalV1 transition shape count"));
        }
        self.range(shapes + 1, usize::from(count))?;
        for index in 0..u32::from(count) {
            if self.byte(shapes + 1 + index)? == shape {
                return Ok(true);
            }
        }
        Ok(false)
    }
    fn original_special_word(
        &self,
        floor: crate::loading::SpecialFloor,
        index: usize,
    ) -> Result<u16> {
        self.special_source(floor).raw_word(index)
    }
    fn original_level1_item(&self, index: u8) -> Result<(i32, i32)> {
        if index >= 12 {
            return Err(Error::Resource("OriginalV1 first floor item index"));
        }
        let position = rom::level1_present_locations(self.revision).at(i32::from(index));
        Ok((
            i32::from(self.word(position.x().address())? as i16),
            i32::from(self.word(position.y().address())? as i16),
        ))
    }
    fn original_present_weight(&self, index: u8) -> Result<u8> {
        if index >= 27 {
            return Err(Error::Resource("OriginalV1 present weight index"));
        }
        self.byte(rom::present_weight_table(self.revision).address() + u32::from(index))
    }
    fn original_large_excluded(&self, word: u16) -> Result<bool> {
        for index in 0..23 {
            if i16::from(
                self.byte(rom::large_item_excluded_terrain_ids(self.revision).address() + index)?
                    as i8,
            ) == (word & 127) as i16
            {
                return Ok(true);
            }
        }
        Ok(false)
    }
}
