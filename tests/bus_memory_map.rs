use cpt32::bus::{Bus, BusRegion};
use cpt32::devices::ram::connect_ram;
use cpt32::devices::vdp::vdp::connect_vdp;

#[test]
fn memory_map_reports_registered_ram_and_mmio_regions() {
    let mut bus = Bus::new();
    connect_ram(&mut bus);
    connect_vdp(&mut bus);

    assert_eq!(
        bus.memory_map().collect::<Vec<_>>(),
        vec![
            BusRegion {
                start: 0x0000_0000,
                size: 0x0100_0000,
                name: "Main RAM",
            },
            BusRegion {
                start: 0x1000_0000,
                size: 0x0040_0000,
                name: "VRAM",
            },
            BusRegion {
                start: 0x8000_0000,
                size: 0x0001_0000,
                name: "VDP MMIO",
            },
            BusRegion {
                start: 0x8001_0000,
                size: 0x0001_0000,
                name: "SGC MMIO",
            },
            BusRegion {
                start: 0x8003_0000,
                size: 0x0001_0000,
                name: "VPU MMIO",
            },
        ]
    );
}
