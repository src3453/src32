
// VDP: Video Display Processor
// VRAM: 0x10000000 - 0x103FFFFF
// Memory-mapped I/O at 0x80030000 - 0x8003FFFF

use crate::bus::Bus;
use crate::devices::vdp::gp::Gp0;
use crate::devices::vdp::reg::VdpRegs;

pub fn connect_vdp(bus: &mut Bus) {        
    bus.add_device(0x10000000, Box::new(Gp0::new(0x20000)));
    bus.add_device(0x80030000, Box::new(VdpRegs::new()));
}