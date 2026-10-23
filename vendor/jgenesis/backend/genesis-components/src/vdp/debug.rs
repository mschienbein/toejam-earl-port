use crate::debug::{CopySpriteAttributesResult, CramEntry, SpriteAttributeEntry};
use crate::vdp;
use crate::vdp::colors::ColorTables;
use crate::vdp::registers::{DebugRegister, Registers};
use crate::vdp::render::PatternGeneratorRowArgs;
use crate::vdp::{
    CachedSpriteData, ColorModifier, ControlWriteFlag, Cram, DataPortLocation, DataPortMode,
    SpriteData, Vdp, Vram, Vsram, colors, render, sprites,
};
use jgenesis_common::debug::{DebugBytesView, DebugMemoryView, DebugWordsView, Endian};
use jgenesis_common::frontend::Color;
use jgenesis_common::num::U16Ext;
use std::ops::Range;

/// TJE hybrid diagnostics are cold, bounded observations, never an equality mask.
pub const HYBRID_VDP_DIFFERENCE_LIMIT: usize = 16;
const HYBRID_VDP_VALUE_LIMIT: usize = 256;

#[derive(Debug, Clone, Copy)]
pub struct VdpDifferenceLocation {
    pub component: &'static str,
    pub field: &'static str,
    pub registers: &'static [u8],
    pub index: Option<usize>,
    /// Byte address within VRAM/CRAM/VSRAM, not a CPU-bus address.
    pub address: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct VdpStateDifference {
    pub location: VdpDifferenceLocation,
    pub original: String,
    pub translated: String,
}

#[derive(Debug, Clone)]
pub struct VdpStateDifferences {
    pub differences: Vec<VdpStateDifference>,
    pub truncated: bool,
}

impl Default for VdpStateDifferences {
    fn default() -> Self {
        Self { differences: Vec::with_capacity(HYBRID_VDP_DIFFERENCE_LIMIT), truncated: false }
    }
}

pub(super) fn difference_location(
    component: &'static str,
    field: &'static str,
) -> VdpDifferenceLocation {
    VdpDifferenceLocation { component, field, registers: &[], index: None, address: None }
}

/// Stream Debug into a bounded value; never allocate a complete rendering buffer
/// or an arbitrarily large deferred-write queue merely to describe a mismatch.
fn bounded_debug(value: &impl std::fmt::Debug) -> String {
    struct Bounded {
        value: String,
        truncated: bool,
    }
    impl std::fmt::Write for Bounded {
        fn write_str(&mut self, text: &str) -> std::fmt::Result {
            let available = HYBRID_VDP_VALUE_LIMIT - self.value.len();
            if text.len() <= available {
                self.value.push_str(text);
                return Ok(());
            }
            let mut end = available;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            self.value.push_str(&text[..end]);
            self.truncated = true;
            Err(std::fmt::Error)
        }
    }
    let mut output =
        Bounded { value: String::with_capacity(HYBRID_VDP_VALUE_LIMIT), truncated: false };
    let _ = std::fmt::write(&mut output, format_args!("{value:?}"));
    if output.truncated {
        output.value.push_str(" [truncated]");
    }
    output.value
}

impl VdpStateDifferences {
    pub(super) fn compare<T: PartialEq + std::fmt::Debug>(
        &mut self,
        location: VdpDifferenceLocation,
        original: &T,
        translated: &T,
    ) -> bool {
        if original == translated {
            return true;
        }
        if self.differences.len() == HYBRID_VDP_DIFFERENCE_LIMIT {
            self.truncated = true;
            return false;
        }
        self.differences.push(VdpStateDifference {
            location,
            original: bounded_debug(original),
            translated: bounded_debug(translated),
        });
        true
    }

    pub(super) fn compare_slice<T: PartialEq + std::fmt::Debug>(
        &mut self,
        location: VdpDifferenceLocation,
        original: &[T],
        translated: &[T],
        address_stride: Option<u32>,
    ) -> bool {
        if !self.compare(
            VdpDifferenceLocation { field: "length", ..location },
            &original.len(),
            &translated.len(),
        ) {
            return false;
        }
        for (index, (a, b)) in original.iter().zip(translated).enumerate() {
            if !self.compare(
                VdpDifferenceLocation {
                    index: Some(index),
                    address: address_stride.map(|stride| stride * index as u32),
                    ..location
                },
                a,
                b,
            ) {
                return false;
            }
        }
        true
    }

    fn compare_registers(&mut self, component: &'static str, a: &Registers, b: &Registers) -> bool {
        macro_rules! fields {
            ($registers:expr; $($field:ident),+ $(,)?) => {
                $(if !self.compare(
                    VdpDifferenceLocation {
                        registers: $registers,
                        ..difference_location(component, stringify!($field))
                    },
                    &a.$field,
                    &b.$field,
                ) { return false; })+
            };
        }
        fields!(&[0]; h_interrupt_enabled, hv_counter_stopped);
        fields!(&[1]; display_enabled, v_interrupt_enabled, dma_enabled,
            vertical_display_size, mode_4, vram_size);
        fields!(&[2]; scroll_a_base_nt_addr);
        fields!(&[3]; window_base_nt_addr);
        fields!(&[4]; scroll_b_base_nt_addr);
        fields!(&[5]; sprite_attribute_table_base_addr);
        fields!(&[7]; background_palette, background_color_id);
        fields!(&[10]; h_interrupt_interval);
        fields!(&[11]; vertical_scroll_mode, horizontal_scroll_mode);
        fields!(&[12]; horizontal_display_size, shadow_highlight_flag, interlacing_mode);
        fields!(&[13]; h_scroll_table_base_addr);
        fields!(&[15]; data_port_auto_increment);
        fields!(&[16]; vertical_scroll_size, horizontal_scroll_size);
        fields!(&[17]; window_horizontal_mode, window_x_position);
        fields!(&[18]; window_vertical_mode, window_y_position);
        fields!(&[19, 20]; dma_length);
        fields!(&[21, 22, 23]; dma_source_address);
        fields!(&[23]; dma_mode);
        true
    }
}

impl Vdp {
    /// Describe only fields belonging to the selected existing equality scope.
    /// No access, clock, transaction completion or comparison decision is made.
    /// The first difference and at most fifteen further differences are retained;
    /// every value is bounded independently.
    #[cold]
    pub fn hybrid_state_differences(&self, other: &Self, logical: bool) -> VdpStateDifferences {
        let mut output = VdpStateDifferences::default();
        macro_rules! fields {
            ($component:literal, $a:expr, $b:expr; $($field:ident),+ $(,)?) => {
                $(if !output.compare(
                    difference_location($component, stringify!($field)),
                    &$a.$field,
                    &$b.$field,
                ) { return output; })+
            };
        }
        macro_rules! group {
            ($field:ident) => {
                if !output.compare(
                    difference_location(stringify!($field), "state"),
                    &self.$field,
                    &other.$field,
                ) {
                    return output;
                }
            };
        }
        macro_rules! array {
            ($field:ident) => {
                if !output.compare_slice(
                    difference_location(stringify!($field), "element"),
                    self.$field.as_ref(),
                    other.$field.as_ref(),
                    None,
                ) {
                    return output;
                }
            };
        }
        fields!("control_port", self.control_port, other.control_port;
            mode, location_bits, location, control_address, data_port_address, write_flag, dma_active);
        if !logical {
            fields!("internal", self.state, other.state;
                v_interrupt_pending, h_interrupt_pending, v_interrupt_enabled_latch,
                h_interrupt_enabled_latch, h_interrupt_counter, latched_hv_counter,
                v_border_forgotten, top_border, last_scroll_b_palettes, scanline,
                scanline_mclk_cycles, pixel, in_vblank);
            if !output.compare_slice(
                difference_location("internal", "pending_writes"),
                &self.state.pending_writes,
                &other.state.pending_writes,
                None,
            ) {
                return output;
            }
            fields!("internal", self.state, other.state;
                display_enable_pending, data_port_read_wait, vram_fill_data,
                vram_copy_odd_slot, interlaced_frame, interlaced_odd, frame_h_resolution);
            if !self.fifo.hybrid_differences(&other.fifo, &mut output) {
                return output;
            }
            group!(dma_latency);
            if !output.compare(
                difference_location("pending_fifo_writes", "length"),
                &self.pending_fifo_writes.len(),
                &other.pending_fifo_writes.len(),
            ) {
                return output;
            }
            for (index, (a, b)) in
                self.pending_fifo_writes.iter().zip(&other.pending_fifo_writes).enumerate()
            {
                if !output.compare(
                    VdpDifferenceLocation {
                        index: Some(index),
                        ..difference_location("pending_fifo_writes", "entry")
                    },
                    a,
                    b,
                ) {
                    return output;
                }
            }
            if !self.sprite_state.hybrid_differences(&other.sprite_state, &mut output) {
                return output;
            }
        }
        if !output.compare_registers("registers", &self.registers, &other.registers) {
            return output;
        }
        fields!("debug_register", self.debug_register, other.debug_register;
            display_disabled, forced_plane);
        if !logical {
            if !output.compare_registers(
                "latched_registers",
                &self.latched_registers,
                &other.latched_registers,
            ) {
                return output;
            }
            group!(latched_h_scroll);
            group!(latched_full_screen_v_scroll);
            array!(cached_sprite_attributes);
            array!(latched_sprite_attributes);
            for (component, a, b) in [
                (
                    "bg_buffers.plane_a_pixels",
                    &self.bg_buffers.plane_a_pixels,
                    &other.bg_buffers.plane_a_pixels,
                ),
                (
                    "bg_buffers.plane_b_pixels",
                    &self.bg_buffers.plane_b_pixels,
                    &other.bg_buffers.plane_b_pixels,
                ),
            ] {
                if !output.compare_slice(difference_location(component, "pixel"), a, b, None) {
                    return output;
                }
            }
            for (component, a, b) in [
                ("sprite_buffers", &self.sprite_buffers, &other.sprite_buffers),
                (
                    "interlaced_sprite_buffers",
                    &self.interlaced_sprite_buffers,
                    &other.interlaced_sprite_buffers,
                ),
            ] {
                if !output.compare_slice(
                    difference_location(component, "scanned_ids"),
                    &a.scanned_ids,
                    &b.scanned_ids,
                    None,
                ) || !output.compare_slice(
                    difference_location(component, "sprites"),
                    &a.sprites,
                    &b.sprites,
                    None,
                ) || !output.compare_slice(
                    difference_location(component, "last_tile_addresses"),
                    a.last_tile_addresses.as_slice(),
                    b.last_tile_addresses.as_slice(),
                    None,
                ) || !output.compare_slice(
                    difference_location(component, "pixels"),
                    a.pixels.as_slice(),
                    b.pixels.as_slice(),
                    None,
                ) {
                    return output;
                }
            }
            if !output.compare_slice(
                difference_location("vdp_event_times", "event"),
                &self.vdp_event_times,
                &other.vdp_event_times,
                None,
            ) {
                return output;
            }
            group!(vdp_event_idx);
            if !self.cram_dots.hybrid_differences(&other.cram_dots, &mut output) {
                return output;
            }
        }
        if !output.compare_slice(
            difference_location("vram", "byte"),
            self.vram.as_ref(),
            other.vram.as_ref(),
            Some(1),
        ) || !output.compare_slice(
            difference_location("cram", "word"),
            self.cram.as_ref(),
            other.cram.as_ref(),
            Some(2),
        ) || !output.compare_slice(
            difference_location("vsram", "word"),
            self.vsram.as_ref(),
            other.vsram.as_ref(),
            Some(2),
        ) {
            return output;
        }
        if !logical {
            array!(frame_buffer);
        }
        output
    }
}

// Internal state that is mostly not directly readable by software, but can be useful for debugging
#[derive(Debug, Clone)]
pub struct VdpInternalDebugState {
    pub scanline: u16,
    pub scanline_mclk: u64,
    pub pixel: u16,
    pub active_display_pixels: Range<u16>,
    pub internal_h: u16,
    pub external_h: u8,
    pub v: u8,
    pub vblank_flag: bool,
    pub v_interrupt_pending: bool,
    pub h_interrupt_pending: bool,
    pub h_interrupt_counter: u16,
    pub data_port_mode: DataPortMode,
    pub data_port_location: DataPortLocation,
    pub data_port_address: u32,
    pub write_flag: ControlWriteFlag,
    pub dma_active: bool,
}

#[derive(Debug, Clone)]
pub struct VdpDebugState {
    vram: Box<Vram>,
    cram: Box<Cram>,
    vsram: Box<Vsram>,
    color_tables: ColorTables,
    registers: Registers,
    latched_registers: Registers,
    debug_register: DebugRegister,
    cached_sprite_attributes: Box<[CachedSpriteData]>,
    internal_state: VdpInternalDebugState,
}

impl Vdp {
    #[must_use]
    pub fn to_debug_state(&self) -> VdpDebugState {
        let hv = self.hv_counter_internal(self.state.scanline_mclk_cycles);

        VdpDebugState {
            vram: self.vram.clone().into(),
            cram: self.cram.clone().into(),
            vsram: self.vsram.clone().into(),
            color_tables: self.color_tables.clone(),
            registers: self.registers.clone(),
            latched_registers: self.latched_registers.clone(),
            debug_register: self.debug_register,
            cached_sprite_attributes: self.cached_sprite_attributes.clone(),
            internal_state: VdpInternalDebugState {
                scanline: self.state.scanline,
                scanline_mclk: self.state.scanline_mclk_cycles,
                pixel: self.state.pixel,
                active_display_pixels: self
                    .registers
                    .horizontal_display_size
                    .active_display_h_range(),
                internal_h: hv.internal_h,
                external_h: hv.hv_counter.lsb(),
                v: hv.internal_v,
                vblank_flag: hv.vblank_flag,
                v_interrupt_pending: self.state.v_interrupt_pending,
                h_interrupt_pending: self.state.h_interrupt_pending,
                h_interrupt_counter: self.state.h_interrupt_counter,
                data_port_mode: self.control_port.mode,
                data_port_location: self.control_port.location,
                data_port_address: self.control_port.data_port_address,
                write_flag: self.control_port.write_flag,
                dma_active: self.control_port.dma_active,
            },
        }
    }

    pub fn debug_vram_view(&mut self) -> impl DebugMemoryView {
        DebugBytesView(self.vram.as_mut_slice())
    }

    pub fn debug_cram_view(&mut self) -> impl DebugMemoryView {
        DebugWordsView(self.cram.as_mut_slice(), Endian::Big)
    }

    pub fn debug_vsram_view(&mut self) -> impl DebugMemoryView {
        DebugWordsView(self.vsram.as_mut_slice(), Endian::Big)
    }
}

impl VdpDebugState {
    pub fn copy_cram(&self, out: &mut [CramEntry], modifier: ColorModifier) {
        for (out_entry, &cram_color) in out.iter_mut().zip(self.cram.as_ref()) {
            *out_entry = CramEntry {
                value: cram_color,
                color: parse_gen_color(cram_color, modifier, &self.color_tables),
            };
        }
    }

    pub fn copy_vram(&self, out: &mut [Color], palette: u8, row_len: usize) {
        for pattern in 0..vdp::VRAM_LEN / 32 {
            let base_idx = pattern / row_len * row_len * 64 + (pattern % row_len) * 8;

            for row in 0..8 {
                let colors = render::read_pattern_generator_row(
                    &self.vram,
                    PatternGeneratorRowArgs {
                        vertical_flip: false,
                        horizontal_flip: false,
                        pattern_generator: pattern as u16,
                        row: row as u16,
                        cell_height_shift: 3,
                    },
                );

                for (col, color_id) in colors.into_iter().enumerate() {
                    let out_idx = base_idx + row * row_len * 8 + col;
                    let color = colors::resolve(&self.cram, palette, color_id);
                    out[out_idx] = parse_gen_color(color, ColorModifier::None, &self.color_tables);
                    out[out_idx].a = 255;
                }
            }
        }
    }

    pub fn copy_h_scroll(&self, out: &mut [(u16, u16)]) {
        let h_scroll_table_addr = self.latched_registers.h_scroll_table_base_addr;

        for i in 0..256 {
            let h_scroll_addr = h_scroll_table_addr.wrapping_add(4 * i) as usize;
            let h_scroll_a =
                u16::from_be_bytes([self.vram[h_scroll_addr], self.vram[h_scroll_addr + 1]])
                    & 0x3FF;
            let h_scroll_b =
                u16::from_be_bytes([self.vram[h_scroll_addr + 2], self.vram[h_scroll_addr + 3]])
                    & 0x3FF;
            out[i as usize] = (h_scroll_a, h_scroll_b);
        }
    }

    pub fn dump_registers(&self, mut callback: impl FnMut(&str, &[(&str, &str)])) {
        callback(
            "Register #0",
            &[
                ("Horizontal interrupt enabled", bool_str(self.registers.h_interrupt_enabled)),
                ("HV counter latched", bool_str(self.registers.hv_counter_stopped)),
            ],
        );

        callback(
            "Register #1",
            &[
                ("Display enabled", bool_str(self.registers.display_enabled)),
                ("Vertical interrupt enabled", bool_str(self.registers.v_interrupt_enabled)),
                ("DMA enabled", bool_str(self.registers.dma_enabled)),
                ("Vertical resolution", &self.registers.vertical_display_size.to_string()),
                ("Mode", if self.registers.mode_4 { "4" } else { "5" }),
                ("VRAM size", &self.registers.vram_size.to_string()),
            ],
        );

        callback(
            "Register #2",
            &[(
                "Plane A nametable address",
                &format!("${:04X}", self.registers.scroll_a_base_nt_addr),
            )],
        );

        callback(
            "Register #3",
            &[(
                "Window nametable address",
                &format!("${:04X}", self.registers.window_base_nt_addr),
            )],
        );

        callback(
            "Register #4",
            &[(
                "Plane B nametable address",
                &format!("${:04X}", self.registers.scroll_b_base_nt_addr),
            )],
        );

        callback(
            "Register #5",
            &[(
                "Sprite attribute table address",
                &format!("${:04X}", self.registers.sprite_attribute_table_base_addr),
            )],
        );

        callback(
            "Register #7",
            &[
                ("Backdrop palette", &self.registers.background_palette.to_string()),
                ("Backdrop color index", &self.registers.background_color_id.to_string()),
            ],
        );

        callback(
            "Register #10",
            &[("Horizontal interrupt interval", &self.registers.h_interrupt_interval.to_string())],
        );

        callback(
            "Register #11",
            &[
                ("Vertical scroll mode", &self.registers.vertical_scroll_mode.to_string()),
                ("Horizontal scroll mode", &self.registers.horizontal_scroll_mode.to_string()),
            ],
        );

        callback(
            "Register #12",
            &[
                ("Horizontal resolution", &self.registers.horizontal_display_size.to_string()),
                ("Shadow/highlight enabled", bool_str(self.registers.shadow_highlight_flag)),
                ("Screen mode", &self.registers.interlacing_mode.to_string()),
            ],
        );

        callback(
            "Register #13",
            &[(
                "H scroll table address",
                &format!("${:04X}", self.registers.h_scroll_table_base_addr),
            )],
        );

        callback(
            "Register #15",
            &[(
                "Data port auto-increment",
                &format!("0x{:X}", self.registers.data_port_auto_increment),
            )],
        );

        callback(
            "Register #16",
            &[
                ("Vertical plane size", &self.registers.vertical_scroll_size.to_string()),
                ("Horizontal plane size", &self.registers.horizontal_scroll_size.to_string()),
            ],
        );

        callback(
            "Register #17",
            &[
                ("Window horizontal mode", &self.registers.window_horizontal_mode.to_string()),
                ("Window X", &self.registers.window_x_position.to_string()),
            ],
        );

        callback(
            "Register #18",
            &[
                ("Window vertical mode", &self.registers.window_vertical_mode.to_string()),
                ("Window Y", &self.registers.window_y_position.to_string()),
            ],
        );

        callback("Registers #19-20", &[("DMA length", &self.registers.dma_length.to_string())]);

        callback(
            "Registers #21-23",
            &[
                ("DMA source address", &format!("${:06X}", self.registers.dma_source_address)),
                ("DMA mode", &self.registers.dma_mode.to_string()),
            ],
        );

        callback(
            "Debug Register",
            &[
                ("Display disabled", bool_str(self.debug_register.display_disabled)),
                ("Forced layer", &self.debug_register.forced_plane.to_string()),
            ],
        );
    }

    pub fn copy_sprite_attributes(
        &self,
        out: &mut [SpriteAttributeEntry],
    ) -> CopySpriteAttributesResult {
        let h_display_size = self.registers.horizontal_display_size;
        let base_sprite_table_addr = self.registers.sprite_attribute_table_base_addr
            & h_display_size.sprite_attribute_table_mask();
        let len = h_display_size.sprite_table_len();

        let interlacing_mode = self.registers.interlacing_mode;
        let y_mask = sprites::y_position_mask(interlacing_mode);

        for i in 0..len {
            let sprite_table_addr = base_sprite_table_addr.wrapping_add(8 * i) as usize;
            let data = SpriteData::create(
                self.cached_sprite_attributes[i as usize],
                &self.vram[sprite_table_addr + 4..sprite_table_addr + 8],
            );

            out[i as usize] = SpriteAttributeEntry {
                tile_number: data.pattern_generator,
                x: data.h_position,
                y: data.v_position & y_mask,
                h_cells: data.h_size_cells,
                v_cells: data.v_size_cells,
                palette: data.palette,
                priority: data.priority,
                h_flip: data.horizontal_flip,
                v_flip: data.vertical_flip,
                link: data.link_data,
            };
        }

        CopySpriteAttributesResult {
            sprite_table_len: len,
            top_left_x: sprites::SPRITE_H_DISPLAY_START,
            top_left_y: interlacing_mode.sprite_display_top(),
        }
    }

    pub fn debug_vram_view(&mut self) -> impl DebugMemoryView {
        DebugBytesView(self.vram.as_mut_slice())
    }

    pub fn debug_cram_view(&mut self) -> impl DebugMemoryView {
        DebugWordsView(self.cram.as_mut_slice(), Endian::Big)
    }

    pub fn debug_vsram_view(&mut self) -> impl DebugMemoryView {
        DebugWordsView(self.vsram.as_mut_slice(), Endian::Big)
    }

    #[must_use]
    pub fn internal_state(&self) -> &VdpInternalDebugState {
        &self.internal_state
    }
}

fn bool_str(b: bool) -> &'static str {
    if b { "true" } else { "false" }
}

fn parse_gen_color(gen_color: u16, modifier: ColorModifier, tables: &ColorTables) -> Color {
    let r = ((gen_color >> 1) & 0x07) as u8;
    let g = ((gen_color >> 5) & 0x07) as u8;
    let b = ((gen_color >> 9) & 0x07) as u8;
    colors::gen_to_rgba(r, g, b, 255, modifier, tables)
}
