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
// 0x0004: BITMAP_COLOR_MODE:RW (0 = 256-color palette, 1 = RGB555, 2 = RGB888)
// 0xF000-FFFF: Mode-specific registers (Graphics or PCG mode)

use std::cell::RefCell;
use std::io;
use std::path::Path;
use std::rc::Rc;
use std::sync::OnceLock;

use crate::bus::{Bus, Device};
use crate::devices::sgc::sgc::{SGC_MMIO_BASE, SGC_MMIO_SIZE, Sgc};
use crate::devices::vdp::compositor::{Rgba, compose_gp, source_over};
use crate::devices::vdp::gp::{CLUT_ENTRY_SIZE, CLUT_START_ADDR, GP_HEIGHT, GP_WIDTH, Gp0};
use crate::devices::vdp::pcg::{PcgRenderer, PcgScreenMode};
use crate::devices::vdp::reg::VdpRegs;
use crate::devices::vdp::reg::{BitmapColorMode, DisplayMode};
use crate::devices::vpu::vpu::{VPU_MMIO_BASE, VPU_MMIO_SIZE, Vpu};

pub const VDP_VRAM_BASE: u32 = 0x10000000;
pub const VDP_VRAM_SIZE: u32 = 0x00400000; // 4MB
pub const VDP_REG_BASE: u32 = 0x80000000;
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
    vpu: Rc<RefCell<Vpu>>,
    sgc: Sgc,
}

/// Snapshot of the sources currently assigned to each graphics plane.
pub struct CompositorDebugInfo {
    pub display_mode: DisplayMode,
    pub enabled: bool,
    pub width: usize,
    pub height: usize,
    pub gp_components: [Vec<String>; 8],
}

pub enum VdpFramebuffer<'a> {
    Graphics {
        vram: &'a RefCell<Vec<u8>>,
        renderer: &'a Gp0,
        border_color: u8,
        bitmap_color_mode: BitmapColorMode,
        vpu: &'a RefCell<Vpu>,
        sgc_pixels: Vec<Rgba>,
        sgc_gp: u8,
        base_gp: u8,
        pcg: &'a PcgRenderer,
        pcg_overlay_enable: bool,
        pcg_screen_mode: PcgScreenMode,
        pcg_font_bank: u8,
        pcg_swap_fg_bg: bool,
        pcg_cursor_pos_x: u8,
        pcg_cursor_pos_y: u8,
        pcg_cursor_enable: bool,
        pcg_cursor_lines: u8,
        pcg_cursor_blink_period: u8,
        pcg_cursor_blink_tick: u64,
        pcg_output_gp: u8,
        composed_pixels: OnceLock<Vec<[u8; 3]>>,
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
        vpu: &'a RefCell<Vpu>,
        sgc_pixels: Vec<Rgba>,
        sgc_gp: u8,
        base_gp: u8,
        composed_pixels: OnceLock<Vec<[u8; 3]>>,
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
        let (width, height) = self.dimensions();
        if x >= width || y >= height {
            return (0, 0, 0);
        }
        let (composed_pixels, vpu) = match self {
            VdpFramebuffer::Graphics {
                composed_pixels,
                vpu,
                ..
            }
            | VdpFramebuffer::Pcg {
                composed_pixels,
                vpu,
                ..
            } => (composed_pixels, *vpu),
            VdpFramebuffer::Blank => return (0, 0, 0),
        };
        let pixels = composed_pixels.get_or_init(|| {
            let vpu = vpu.borrow();
            let mut pixels = Vec::with_capacity(width * height);
            for py in 0..height {
                for px in 0..width {
                    let (r, g, b) = self.sample_pixel(px, py, &vpu);
                    pixels.push([r, g, b]);
                }
            }
            pixels
        });
        let [r, g, b] = pixels[y * width + x];
        (r, g, b)
    }

    fn sample_pixel(&self, x: usize, y: usize, vpu: &Vpu) -> (u8, u8, u8) {
        let base = match self {
            VdpFramebuffer::Graphics {
                renderer,
                bitmap_color_mode,
                ..
            } => renderer.get_pixel(x, y, *bitmap_color_mode),
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
        };
        let (sgc_pixels, sgc_gp, base_gp) = match self {
            VdpFramebuffer::Graphics {
                sgc_pixels,
                sgc_gp,
                base_gp,
                ..
            }
            | VdpFramebuffer::Pcg {
                sgc_pixels,
                sgc_gp,
                base_gp,
                ..
            } => (sgc_pixels, *sgc_gp, *base_gp),
            VdpFramebuffer::Blank => return (0, 0, 0),
        };
        let mut layers = [[0, 0, 0, 0]; 8];
        layers[base_gp as usize] = [base.0, base.1, base.2, 255];
        if let VdpFramebuffer::Graphics {
            pcg,
            pcg_overlay_enable: true,
            pcg_screen_mode,
            pcg_font_bank,
            pcg_swap_fg_bg,
            pcg_cursor_pos_x,
            pcg_cursor_pos_y,
            pcg_cursor_enable,
            pcg_cursor_lines,
            pcg_cursor_blink_period,
            pcg_cursor_blink_tick,
            pcg_output_gp,
            ..
        } = self
        {
            if x < VDP_ACTIVE_WIDTH && y < VDP_ACTIVE_HEIGHT {
                let rgb = pcg.get_pixel(
                    x,
                    y,
                    *pcg_screen_mode,
                    *pcg_font_bank,
                    *pcg_swap_fg_bg,
                    *pcg_cursor_pos_x,
                    *pcg_cursor_pos_y,
                    *pcg_cursor_enable,
                    *pcg_cursor_lines,
                    *pcg_cursor_blink_period,
                    *pcg_cursor_blink_tick,
                );
                layers[*pcg_output_gp as usize] =
                    source_over([rgb.0, rgb.1, rgb.2, 255], layers[*pcg_output_gp as usize]);
            }
        }
        if x < VDP_ACTIVE_WIDTH && y < VDP_ACTIVE_HEIGHT {
            let sprite = sgc_pixels
                .get(y * VDP_ACTIVE_WIDTH + x)
                .copied()
                .unwrap_or([0, 0, 0, 0]);
            layers[sgc_gp as usize] = source_over(sprite, layers[sgc_gp as usize]);
        }
        let (r, g, b, a) = vpu.pixel(x, y);
        let vpu_pixel = [r, g, b, a];
        layers[vpu.output_gp as usize] = source_over(vpu_pixel, layers[vpu.output_gp as usize]);
        let composed = compose_gp(layers, [0, 0, 0, 255]);
        (composed[0], composed[1], composed[2])
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
        let vpu = Rc::new(RefCell::new(Vpu::new()));
        let sgc = Sgc::new(Rc::clone(&vram));
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
            vpu,
            sgc,
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

        let sgc_pixels = if self.sgc.enabled() {
            self.sgc.render_frame()
        } else {
            Vec::new()
        };
        match self.regs.display_mode {
            DisplayMode::Graphics => VdpFramebuffer::Graphics {
                vram: self.vram.as_ref(),
                renderer: &self.gp0,
                border_color: self.regs.border_color,
                bitmap_color_mode: self.regs.bitmap_color_mode,
                vpu: self.vpu.as_ref(),
                sgc_pixels,
                sgc_gp: self.sgc.output_gp(),
                base_gp: 0,
                pcg: &self.pcg,
                pcg_overlay_enable: self.regs.pcg_overlay_enable,
                pcg_screen_mode: self.regs.pcg_screen_mode,
                pcg_font_bank: self.regs.pcg_font_bank,
                pcg_swap_fg_bg: self.regs.pcg_swap_fg_bg,
                pcg_cursor_pos_x: self.regs.pcg_cursor_pos_x,
                pcg_cursor_pos_y: self.regs.pcg_cursor_pos_y,
                pcg_cursor_enable: self.regs.pcg_cursor_enable,
                pcg_cursor_lines: self.regs.pcg_cursor_lines,
                pcg_cursor_blink_period: self.regs.pcg_cursor_blink_period,
                pcg_cursor_blink_tick: self.state.pcg_cursor_blink_tick,
                pcg_output_gp: self.regs.pcg_output_gp,
                composed_pixels: OnceLock::new(),
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
                vpu: self.vpu.as_ref(),
                sgc_pixels,
                sgc_gp: self.sgc.output_gp(),
                base_gp: self.regs.pcg_output_gp,
                composed_pixels: OnceLock::new(),
            },
        }
    }

    /// Describes which renderers contribute to each GP in the current VDP setup.
    pub fn compositor_debug_info(&self) -> CompositorDebugInfo {
        let (width, height) = match self.regs.display_mode {
            DisplayMode::Graphics => (VDP_FRAMEBUFFER_WIDTH, VDP_FRAMEBUFFER_HEIGHT),
            DisplayMode::PCG => (
                self.regs.pcg_screen_mode.width(),
                self.regs.pcg_screen_mode.height(),
            ),
        };
        let mut gp_components: [Vec<String>; 8] = std::array::from_fn(|_| Vec::new());

        match self.regs.display_mode {
            DisplayMode::Graphics => gp_components[0].push(format!(
                "GP0 image: Graphics Plane (GP0), {}x{} {:?} image",
                GP_WIDTH, GP_HEIGHT, self.regs.bitmap_color_mode
            )),
            DisplayMode::PCG => gp_components[self.regs.pcg_output_gp as usize].push(format!(
                "Base image: PCG text, {}x{} ({} columns x {} rows)",
                width,
                height,
                self.regs.pcg_screen_mode.columns(),
                self.regs.pcg_screen_mode.rows()
            )),
        }

        if self.regs.display_mode == DisplayMode::Graphics && self.regs.pcg_overlay_enable {
            gp_components[self.regs.pcg_output_gp as usize].push(format!(
                "PCG overlay: {}x{} text plane ({} columns x {} rows)",
                self.regs.pcg_screen_mode.width(),
                self.regs.pcg_screen_mode.height(),
                self.regs.pcg_screen_mode.columns(),
                self.regs.pcg_screen_mode.rows()
            ));
        }
        if self.sgc.enabled() {
            gp_components[self.sgc.output_gp() as usize]
                .push("Screen Graphics Controller (SGC): 320x240 RGBA sprite plane".to_string());
        }
        let vpu = self.vpu.borrow();
        gp_components[vpu.output_gp as usize]
            .push("VPU: 320x240 RGBA pixel plane (alpha blended)".to_string());

        CompositorDebugInfo {
            display_mode: self.regs.display_mode,
            enabled: self.regs.display_enable,
            width,
            height,
            gp_components,
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
            0x04 => self.regs.bitmap_color_mode as u8,
            _ => 0,
        }
    }

    fn read_sgc_register(&self, reg_addr: u32) -> u8 {
        self.sgc.read_register(reg_addr)
    }
    fn write_sgc_register(&mut self, reg_addr: u32, value: u8) {
        self.sgc.write_register(reg_addr, value);
    }

    fn read_pcg_register(&self, reg_addr: u32) -> u8 {
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
            0xF00A => self.regs.pcg_output_gp,
            0xF00B => self.regs.pcg_overlay_enable as u8,
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
            0x04 => self.regs.set_bitmap_color_mode(value),
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
            0xF00A => {
                if value < 8 {
                    self.regs.pcg_output_gp = value;
                } else {
                    self.regs.status |= 1;
                }
            }
            0xF00B => self.regs.pcg_overlay_enable = value & 1 != 0,
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
    Sgc,
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
            VdpPort::Sgc => self.vdp.borrow().read_sgc_register(addr),
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
            VdpPort::Sgc => self.vdp.borrow_mut().write_sgc_register(addr, value),
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
            VdpPort::Sgc => SGC_MMIO_SIZE,
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

    fn set_unifont_cell(vdp: &Vdp, index: usize, code_unit: u16) {
        let mut vram = vdp.vram.borrow_mut();
        let text = crate::devices::vdp::pcg::PCG_TEXT_ADDR + index * 2;
        let [high, low] = code_unit.to_be_bytes();
        vram[text] = high;
        vram[text + 1] = low;
        vram[crate::devices::vdp::pcg::PCG_FG_ATTR_ADDR + index] = 1;
        vram[crate::devices::vdp::pcg::PCG_BG_ATTR_ADDR + index] = 2;
    }

    fn mode2_vdp() -> Vdp {
        let mut vdp = Vdp::new();
        vdp.write_common_register(0x01, DisplayMode::PCG as u8);
        vdp.write_pcg_register(0xF003, 2);
        vdp.write_pcg_register(0xF001, 1);
        set_unifont_cell(&vdp, 0, 0x3042);
        let mut vram = vdp.vram.borrow_mut();
        let clut = crate::devices::vdp::pcg::PCG_CLUT_START_ADDR;
        vram[clut + 3..clut + 6].copy_from_slice(&[255, 0, 0]);
        vram[clut + 6..clut + 9].copy_from_slice(&[0, 0, 200]);
        drop(vram);
        vdp
    }

    #[test]
    fn pcg_mode2_framebuffer_is_320_by_240() {
        let vdp = mode2_vdp();
        let fb = vdp.framebuffer();
        assert_eq!(fb.dimensions(), (320, 240));
        assert_eq!(fb.get_pixel(320, 239), (0, 0, 0));
        assert!(
            vdp.compositor_debug_info().gp_components[0]
                .iter()
                .any(|entry| entry.contains("20 columns x 15 rows"))
        );
    }

    #[test]
    fn pcg_mode2_utf16be_selects_unifont_glyph_and_colors() {
        let vdp = mode2_vdp();
        let fb = vdp.framebuffer();
        assert_eq!(fb.get_pixel(5, 1), (255, 0, 0));
        assert_eq!(fb.get_pixel(0, 1), (0, 0, 200));
        assert_eq!(fb.get_pixel(8, 7), (255, 0, 0));
        assert_eq!(fb.get_pixel(15, 7), (0, 0, 200));
    }

    #[test]
    fn pcg_mode2_cursor_maps_eight_mask_bits_to_sixteen_rows_at_last_cell() {
        let mut vdp = mode2_vdp();
        set_unifont_cell(&vdp, 299, 0x3042);
        vdp.write_pcg_register(0xF005, 19);
        vdp.write_pcg_register(0xF006, 14);
        vdp.write_pcg_register(0xF007, 1);
        vdp.write_pcg_register(0xF008, 0x7F);
        let fb = vdp.framebuffer();
        assert_eq!(fb.get_pixel(304, 224), (255, 255, 55));
        assert_eq!(fb.get_pixel(304, 225), (255, 255, 55));
        assert_eq!(fb.get_pixel(304, 226), (0, 0, 200));

        let mut vdp = mode2_vdp();
        set_unifont_cell(&vdp, 299, 0x3042);
        vdp.write_pcg_register(0xF005, 19);
        vdp.write_pcg_register(0xF006, 14);
        vdp.write_pcg_register(0xF007, 1);
        vdp.write_pcg_register(0xF008, 0xFE);
        let fb = vdp.framebuffer();
        assert_eq!(fb.get_pixel(319, 239), (255, 255, 55));
    }

    #[test]
    fn pcg_mode2_invalid_mode_retains_mode_and_sets_error_status() {
        let mut vdp = mode2_vdp();
        vdp.write_pcg_register(0xF003, 0x82);
        assert_eq!(vdp.regs.pcg_screen_mode, PcgScreenMode::Unifont16x16);
        assert_eq!(vdp.read_pcg_register(0xF004), 1);
        assert_eq!(vdp.framebuffer().dimensions(), (320, 240));
    }

    #[test]
    fn pcg_mode2_keeps_legacy_mode_dimensions_and_font_banks() {
        let mut vdp = Vdp::new();
        vdp.write_common_register(0x01, DisplayMode::PCG as u8);
        {
            let mut vram = vdp.vram.borrow_mut();
            let clut = crate::devices::vdp::pcg::PCG_CLUT_START_ADDR;
            vram[clut + 3..clut + 6].copy_from_slice(&[255, 0, 0]);
            vram[clut + 6..clut + 9].copy_from_slice(&[0, 0, 200]);
            vram[0] = 1;
            vram[1] = 2;
            vram[crate::devices::vdp::pcg::PCG_FG_ATTR_ADDR] = 1;
            vram[crate::devices::vdp::pcg::PCG_BG_ATTR_ADDR] = 2;
            vram[crate::devices::vdp::pcg::PCG_FG_ATTR_ADDR + 1] = 1;
            vram[crate::devices::vdp::pcg::PCG_BG_ATTR_ADDR + 1] = 2;
            vram[crate::devices::vdp::pcg::PCG_FONT_BANK0_ADDR + 8] = 0x80;
            vram[crate::devices::vdp::pcg::PCG_FONT_BANK1_ADDR + 8] = 0x40;
            vram[crate::devices::vdp::pcg::PCG_FONT_BANK1_ADDR + 16] = 0x80;
        }
        vdp.write_pcg_register(0xF003, 0);
        assert_eq!(vdp.framebuffer().dimensions(), (320, 240));
        assert_eq!(vdp.framebuffer().get_pixel(0, 0), (255, 0, 0));
        vdp.write_pcg_register(0xF001, 1);
        assert_eq!(vdp.framebuffer().get_pixel(1, 0), (255, 0, 0));
        vdp.write_pcg_register(0xF003, 1);
        assert_eq!(vdp.framebuffer().dimensions(), (640, 240));
        assert_eq!(vdp.framebuffer().get_pixel(8, 0), (255, 0, 0));
        assert_eq!(vdp.framebuffer().get_pixel(9, 0), (0, 0, 200));
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
    let vpu = Rc::clone(&vdp.borrow().vpu);
    bus.add_device(
        VDP_VRAM_BASE,
        Box::new(VdpDevice::new(Rc::clone(&vdp), VdpPort::Vram)),
    );
    bus.add_device(
        VDP_REG_BASE,
        Box::new(VdpDevice::new(Rc::clone(&vdp), VdpPort::Regs)),
    );
    bus.add_device(
        SGC_MMIO_BASE,
        Box::new(VdpDevice::new(Rc::clone(&vdp), VdpPort::Sgc)),
    );
    bus.add_device(VPU_MMIO_BASE, Box::new(VpuRegisterDevice::new(vpu)));
    vdp
}

struct VpuRegisterDevice(Rc<RefCell<Vpu>>);
impl VpuRegisterDevice {
    fn new(vpu: Rc<RefCell<Vpu>>) -> Self {
        Self(vpu)
    }
}
impl Device for VpuRegisterDevice {
    fn read(&mut self, addr: u32) -> u8 {
        let reg = addr & !3;
        let shift = (3 - (addr & 3)) * 8;
        let v = self.0.borrow();
        match reg {
            0 => (0x5650_5531u32 >> shift) as u8,
            4 => (v.status >> shift) as u8,
            0x0c => ((v.fifo.len() as u32) >> shift) as u8,
            0x10 => (4096u32 >> shift) as u8,
            0x14 => (v.output_gp as u32 >> shift) as u8,
            _ => 0,
        }
    }
    fn write(&mut self, addr: u32, value: u8) {
        let reg = addr & !3;
        let shift = (3 - (addr & 3)) * 8;
        let mut v = self.0.borrow_mut();
        match reg {
            4 => v.status &= !((value as u32) << shift),
            8 if shift == 0 && value & 2 != 0 => {
                v.fifo.clear();
                v.command.clear();
                v.expected = None;
                v.status = 0;
                v.write_latch = 0;
                v.write_mask = 0;
            }
            8 if shift == 0 && value & 4 != 0 => {
                v.fifo.clear();
                v.command.clear();
                v.expected = None;
                v.status = 0;
                v.write_latch = 0;
                v.write_mask = 0;
                v.plane.fill(0);
            }
            0x14 if shift == 0 => {
                if value < 8 {
                    v.output_gp = value;
                } else {
                    v.status |= 1 << 3;
                }
            }
            0x2c if shift == 0 => {
                if v.write_mask == 0x07 {
                    let word = v.write_latch | value as u32;
                    v.write_latch = 0;
                    v.write_mask = 0;
                    v.push(word);
                } else {
                    v.status |= 1 << 3;
                }
            }
            0x2c => {
                let byte = (3 - shift / 8) as u8;
                v.write_latch = (v.write_latch & !(0xff << shift)) | ((value as u32) << shift);
                v.write_mask |= 1 << byte;
            }
            _ => {}
        }
    }
    fn size(&self) -> u32 {
        VPU_MMIO_SIZE
    }
}
