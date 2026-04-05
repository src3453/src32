pub struct VdpRegs {
    pub display_enable: bool,
}

impl VdpRegs {
    pub fn new() -> Self {
        Self {
            display_enable: true,
        }
    }
}
