use crate::devices::vdp::pcg::PcgScreenMode;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayMode {
    Graphics = 0,
    PCG = 1,
}

pub struct VdpRegs {
    pub display_enable: bool,
    pub display_mode: DisplayMode,
    pub border_color: u8,
    pub pcg_screen_mode: PcgScreenMode,
    pub pcg_font_bank: u8,
    pub pcg_swap_fg_bg: bool,
    pub pcg_cursor_pos_x: u8,
    pub pcg_cursor_pos_y: u8,
    pub pcg_cursor_enable: bool,
    pub pcg_cursor_lines: u8,
    pub pcg_cursor_blink_period: u8,
    pub status: u8,
}

impl VdpRegs {
    pub fn new() -> Self {
        Self {
            display_enable: true,
            display_mode: DisplayMode::Graphics,
            border_color: 0,
            pcg_screen_mode: PcgScreenMode::Columns40,
            pcg_font_bank: 0,
            pcg_swap_fg_bg: false,
            pcg_cursor_pos_x: 0,
            pcg_cursor_pos_y: 0,
            pcg_cursor_enable: false,
            pcg_cursor_lines: 0,
            pcg_cursor_blink_period: 0,
            status: 0,
        }
    }

    pub fn set_display_mode(&mut self, value: u8) {
        self.display_mode = match value & 1 {
            0 => DisplayMode::Graphics,
            _ => DisplayMode::PCG,
        };
    }

    pub fn set_pcg_screen_mode(&mut self, value: u8) {
        self.pcg_screen_mode = PcgScreenMode::from_u8(value);
    }
}
