// VDP: Video Display Processor
// This module implements the VDP, which is responsible for rendering graphics in the CPT32 emulator.
// It includes VRAM for storing pixel data and registers for controlling display settings.
// VRAM: 0x10000000 - 0x103FFFFF
// Memory-mapped I/O at 0x80030000 - 0x8003FFFF

// Registers for VDP (from base address):
// 0x0000: /ENABLE:RW (Display enable) (0 = enable, 1 = disable)
// 0x0001: VDP_MODE:RW (Display mode) (0 = Graphics, 1 = PCG)
// 0x0002: STATUS:R- (Status register) (0 = OK, 1 = Error)
// 0x0003: BORDER_COLOR:RW (Overscan border color index, 0-63)
// 0xF000-FFFF: Mode-specific registers (Graphics or PCG mode)

use std::cell::RefCell;
use std::io;
use std::path::Path;
use std::rc::Rc;

use crate::bus::{Bus, Device};
use crate::devices::vdp::gp::{CLUT_ENTRY_SIZE, CLUT_START_ADDR, GP_HEIGHT, GP_WIDTH, Gp0};
use crate::devices::vdp::pcg::{PcgRenderer, PcgScreenMode};
use crate::devices::vdp::reg::DisplayMode;
use crate::devices::vdp::reg::VdpRegs;

pub const VDP_VRAM_BASE: u32 = 0x10000000;
pub const VDP_VRAM_SIZE: u32 = 0x00400000; // 4MB
pub const VDP_REG_BASE: u32 = 0x80030000;
pub const VDP_REG_SIZE: u32 = 0x00010000;

pub const VDP_ACTIVE_WIDTH: usize = GP_WIDTH;
pub const VDP_ACTIVE_HEIGHT: usize = GP_HEIGHT;
pub const VDP_BORDER_SIZE: usize = 8;
pub const VDP_FRAMEBUFFER_WIDTH: usize = VDP_ACTIVE_WIDTH;
pub const VDP_FRAMEBUFFER_HEIGHT: usize = VDP_ACTIVE_HEIGHT;
pub const VDP_VIRTUAL_CLOCK: u32 =
    (VDP_FRAMEBUFFER_WIDTH as u32) * (VDP_FRAMEBUFFER_HEIGHT as u32) * crate::sys::FRAME_RATE;
pub const VDP_CLOCK_DIVIDER: u32 = 4; // VDP runs at 1/4 of master clock
pub const PIXEL_CLOCK_DIVIDER: u32 = 8; // Pixel clock is 1/8 of master clock
pub const VDP_CLOCK: u32 = crate::sys::MASTER_CLOCK / VDP_CLOCK_DIVIDER; // 12MHz
pub const PIXEL_CLOCK: u32 = crate::sys::MASTER_CLOCK / PIXEL_CLOCK_DIVIDER; // 6MHz

pub struct VdpState {
    tick_count: u64,
    pcg_cursor_blink_tick: u64,
}

pub struct Vdp {
    vram: Rc<RefCell<Vec<u8>>>,
    gp0: Gp0,
    pcg: PcgRenderer,
    regs: VdpRegs,
    state: VdpState,
}

pub enum VdpFramebuffer<'a> {
    Graphics {
        vram: &'a RefCell<Vec<u8>>,
        renderer: &'a Gp0,
        border_color: u8,
    },
    Pcg {
        vram: &'a RefCell<Vec<u8>>,
        renderer: &'a PcgRenderer,
        screen_mode: PcgScreenMode,
        font_bank: u8,
        swap_fg_bg: bool,
        cursor_pos_x: u8,
        cursor_pos_y: u8,
        cursor_enable: bool,
        cursor_lines: u8,
        cursor_blink_period: u8,
        cursor_blink_tick: u64,
        border_color: u8,
    },
    Blank,
}

impl<'a> VdpFramebuffer<'a> {
    pub fn dimensions(&self) -> (usize, usize) {
        match self {
            VdpFramebuffer::Graphics { .. } => (VDP_FRAMEBUFFER_WIDTH, VDP_FRAMEBUFFER_HEIGHT),
            VdpFramebuffer::Pcg { screen_mode, .. } => (screen_mode.width(), screen_mode.height()),
            VdpFramebuffer::Blank => (VDP_FRAMEBUFFER_WIDTH, VDP_FRAMEBUFFER_HEIGHT),
        }
    }

    pub fn border_pixel(&self) -> (u8, u8, u8) {
        match self {
            VdpFramebuffer::Graphics {
                vram, border_color, ..
            } => Self::read_border_pixel(vram, *border_color),
            VdpFramebuffer::Pcg {
                vram, border_color, ..
            } => Self::read_border_pixel(vram, *border_color),
            VdpFramebuffer::Blank => (0, 0, 0),
        }
    }

    pub fn get_pixel(&self, x: usize, y: usize) -> (u8, u8, u8) {
        match self {
            VdpFramebuffer::Graphics { renderer, .. } => renderer.get_pixel(x, y),
            VdpFramebuffer::Pcg {
                renderer,
                screen_mode,
                font_bank,
                swap_fg_bg,
                cursor_pos_x,
                cursor_pos_y,
                cursor_enable,
                cursor_lines,
                cursor_blink_period,
                cursor_blink_tick,
                ..
            } => renderer.get_pixel(
                x,
                y,
                *screen_mode,
                *font_bank,
                *swap_fg_bg,
                *cursor_pos_x,
                *cursor_pos_y,
                *cursor_enable,
                *cursor_lines,
                *cursor_blink_period,
                *cursor_blink_tick,
            ),
            VdpFramebuffer::Blank => (0, 0, 0),
        }
    }

    fn read_border_pixel(vram: &RefCell<Vec<u8>>, border_color: u8) -> (u8, u8, u8) {
        let vram = vram.borrow();
        let clut_index = CLUT_START_ADDR + ((border_color as usize) & 0x3F) * CLUT_ENTRY_SIZE;
        (vram[clut_index], vram[clut_index + 1], vram[clut_index + 2])
    }
}

impl Vdp {
    pub fn new() -> Self {
        Self::with_font_path(None::<&Path>)
    }

    pub fn with_font_path<P: AsRef<Path>>(font_path: Option<P>) -> Self {
        let vram = Rc::new(RefCell::new(vec![0; VDP_VRAM_SIZE as usize]));
        let gp0 = Gp0::new(Rc::clone(&vram));
        let pcg = PcgRenderer::new(Rc::clone(&vram));
        let vdp = Self {
            vram,
            gp0,
            pcg,
            regs: VdpRegs::new(),
            state: VdpState {
                tick_count: 0,
                pcg_cursor_blink_tick: 0,
            },
        };
        vdp.gp0.init_clut();
        vdp.pcg.init_clut();
        if let Some(path) = font_path {
            vdp.load_pcg_font_from_file(path)
                .expect("Failed to load PCG font file");
        }
        vdp
    }

    pub fn framebuffer(&self) -> VdpFramebuffer<'_> {
        if !self.regs.display_enable {
            return VdpFramebuffer::Blank;
        }

        match self.regs.display_mode {
            DisplayMode::Graphics => VdpFramebuffer::Graphics {
                vram: self.vram.as_ref(),
                renderer: &self.gp0,
                border_color: self.regs.border_color,
            },
            DisplayMode::PCG => VdpFramebuffer::Pcg {
                vram: self.vram.as_ref(),
                renderer: &self.pcg,
                screen_mode: self.regs.pcg_screen_mode,
                font_bank: self.regs.pcg_font_bank,
                swap_fg_bg: self.regs.pcg_swap_fg_bg,
                cursor_pos_x: self.regs.pcg_cursor_pos_x,
                cursor_pos_y: self.regs.pcg_cursor_pos_y,
                cursor_enable: self.regs.pcg_cursor_enable,
                cursor_lines: self.regs.pcg_cursor_lines,
                cursor_blink_period: self.regs.pcg_cursor_blink_period,
                cursor_blink_tick: self.state.pcg_cursor_blink_tick,
                border_color: self.regs.border_color,
            },
        }
    }

    pub fn load_pcg_font_from_file<P: AsRef<Path>>(&self, path: P) -> io::Result<()> {
        self.pcg.load_font_file(path)
    }

    pub fn set_display_mode(&mut self, mode: DisplayMode) {
        self.regs.display_mode = mode;
    }

    pub fn tick(&mut self) {
        self.state.tick_count += 1;
        self.state.pcg_cursor_blink_tick += 1;
    }

    fn reset_state(&mut self) {
        self.regs = VdpRegs::new();
        self.state.tick_count = 0;
        self.state.pcg_cursor_blink_tick = 0;
    }

    fn read_common_register(&self, reg_addr: u32) -> u8 {
        match reg_addr {
            0x00 => self.regs.display_enable as u8,
            0x01 => self.regs.display_mode as u8,
            0x02 => self.regs.status,
            0x03 => self.regs.border_color,
            _ => 0,
        }
    }

    fn read_pcg_register(&self, reg_addr: u32) -> u8 {
        if self.regs.display_mode != DisplayMode::PCG {
            return 0;
        }

        match reg_addr {
            0xF000 => (self.regs.display_enable as u8) ^ 1,
            0xF001 => self.regs.pcg_font_bank,
            0xF002 => self.regs.pcg_swap_fg_bg as u8,
            0xF003 => self.regs.pcg_screen_mode as u8,
            0xF004 => self.regs.status,
            0xF005 => self.regs.pcg_cursor_pos_x,
            0xF006 => self.regs.pcg_cursor_pos_y,
            0xF007 => self.regs.pcg_cursor_enable as u8,
            0xF008 => self.regs.pcg_cursor_lines,
            0xF009 => self.regs.pcg_cursor_blink_period,
            0xFFFF => 0,
            _ => 0,
        }
    }

    fn write_common_register(&mut self, reg_addr: u32, value: u8) {
        match reg_addr {
            0x00 => self.regs.display_enable = (value & 1) != 0,
            0x01 => self.regs.set_display_mode(value),
            0x02 => self.regs.status = value & 1,
            0x03 => self.regs.border_color = value & 0x3F,
            _ => {}
        }
    }

    fn write_pcg_register(&mut self, reg_addr: u32, value: u8) {
        if reg_addr == 0xFFFF {
            if value & 1 != 0 {
                self.reset_state();
            }
            return;
        }

        if self.regs.display_mode != DisplayMode::PCG {
            return;
        }

        match reg_addr {
            0xF000 => self.regs.display_enable = (value & 1) == 0,
            0xF001 => self.regs.pcg_font_bank = value & 1,
            0xF002 => self.regs.pcg_swap_fg_bg = (value & 1) != 0,
            0xF003 => self.regs.set_pcg_screen_mode(value),
            0xF004 => self.regs.status = value & 1,
            0xF005 => self.regs.pcg_cursor_pos_x = value,
            0xF006 => self.regs.pcg_cursor_pos_y = value,
            0xF007 => self.regs.pcg_cursor_enable = (value & 1) != 0,
            0xF008 => self.regs.pcg_cursor_lines = value,
            0xF009 => self.regs.pcg_cursor_blink_period = value,
            0xFFFF => {
                if value & 1 != 0 {
                    self.reset_state();
                }
            }
            _ => {}
        }
    }
}

#[derive(Clone, Copy)]
enum VdpPort {
    Vram,
    Regs,
}

struct VdpDevice {
    vdp: Rc<RefCell<Vdp>>,
    port: VdpPort,
}

impl VdpDevice {
    fn new(vdp: Rc<RefCell<Vdp>>, port: VdpPort) -> Self {
        Self { vdp, port }
    }
}

impl Device for VdpDevice {
    fn read(&mut self, addr: u32) -> u8 {
        match self.port {
            VdpPort::Vram => self.vdp.borrow().vram.borrow()[addr as usize],
            VdpPort::Regs => {
                let vdp = self.vdp.borrow();
                if addr < 0x0100 {
                    vdp.read_common_register(addr)
                } else if addr >= 0xF000 {
                    vdp.read_pcg_register(addr)
                } else {
                    0
                }
            }
        }
    }

    fn write(&mut self, addr: u32, value: u8) {
        match self.port {
            VdpPort::Vram => self.vdp.borrow_mut().vram.borrow_mut()[addr as usize] = value,
            VdpPort::Regs => {
                let mut vdp = self.vdp.borrow_mut();
                if addr < 0x0100 {
                    vdp.write_common_register(addr, value);
                } else if addr >= 0xF000 {
                    vdp.write_pcg_register(addr, value);
                }
            }
        }
    }

    fn size(&self) -> u32 {
        match self.port {
            VdpPort::Vram => VDP_VRAM_SIZE,
            VdpPort::Regs => VDP_REG_SIZE,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn framebuffer_dimensions_match_active_area() {
        let vdp = Vdp::new();
        let fb = vdp.framebuffer();

        assert_eq!(
            fb.dimensions(),
            (VDP_FRAMEBUFFER_WIDTH, VDP_FRAMEBUFFER_HEIGHT)
        );
        assert_eq!(fb.get_pixel(0, 0), (0, 0, 0));
    }

    #[test]
    fn border_color_register_changes_overscan_color() {
        let mut vdp = Vdp::new();
        vdp.write_common_register(0x03, 0x0F);

        let fb = vdp.framebuffer();

        assert_eq!(fb.border_pixel(), (0x00, 0xFF, 0xFF));
        assert_eq!(fb.get_pixel(0, 0), (0, 0, 0));
    }
}

pub fn connect_vdp(bus: &mut Bus) -> Rc<RefCell<Vdp>> {
    connect_vdp_with_font(bus, Option::<&Path>::None)
}

pub fn connect_vdp_with_font<P: AsRef<Path>>(
    bus: &mut Bus,
    font_path: Option<P>,
) -> Rc<RefCell<Vdp>> {
    let vdp = Rc::new(RefCell::new(Vdp::with_font_path(font_path)));
    bus.add_device(
        VDP_VRAM_BASE,
        Box::new(VdpDevice::new(Rc::clone(&vdp), VdpPort::Vram)),
    );
    bus.add_device(
        VDP_REG_BASE,
        Box::new(VdpDevice::new(Rc::clone(&vdp), VdpPort::Regs)),
    );
    vdp
}
