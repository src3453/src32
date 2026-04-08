pub enum DisplayMode {
    Graphics=0,
    PCG=1,
}

pub struct VdpRegs {
    pub display_enable: bool,
    pub display_mode: DisplayMode,
}

impl VdpRegs {
    pub fn new() -> Self {
        Self {
            display_enable: true,
            display_mode: DisplayMode::Graphics,
        }
    }
}
