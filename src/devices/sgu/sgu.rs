// SGU: Sound Generator Unit
// This module provides the bus device wrapper for the 3WS8PN (S3W2) sound generator.
// SGU MMIO: 0x80020000 - 0x800208FF and 0x80021000 - 0x800218FF
// 34SI22:   0x80022000 - 0x800220FF (SGU v3; doc/spec_34SI22.md section 10)
// PCMRAM:   0x18000000 - 0x180FFFFF (1MB)

use std::cell::RefCell;
use std::rc::Rc;

use crate::bus::{Bus, Device};
use std::path::Path;

use crate::devices::sgu::s3w2::{PCM_RAM_SIZE, S3w2Sound, SguVersion, saturate_output};
use crate::devices::sgu::si22::{SI22_REG_COUNT, Si22, Si22Memory};

pub const SGU_REG_BASE: u32 = 0x8002_0000;
pub const SGU2_REG_BASE: u32 = 0x8002_1000;
pub const SGU_REG_SIZE: u32 = 0x0000_0900; // 0x900 bytes (0x800 wavetable SRAM + 0x100 registers)

pub const SI22_REG_BASE: u32 = 0x8002_2000;
pub const SI22_REG_SIZE: u32 = 0x100;

pub const PCM_RAM_BASE: u32 = 0x1800_0000;
pub const PCM_RAM_TOTAL_SIZE: u32 = PCM_RAM_SIZE as u32; // 1MB

pub enum SguPort {
    Regs(usize),
    PcmRam,
}

pub struct SguDevice {
    sgus: [Rc<RefCell<S3w2Sound>>; 2],
    port: SguPort,
}

impl SguDevice {
    pub fn new(sgus: [Rc<RefCell<S3w2Sound>>; 2], port: SguPort) -> Self {
        Self { sgus, port }
    }
}

impl Device for SguDevice {
    /// SGU registers and PCM RAM are rendered per host frame; no IRQ source.
    fn is_timing_insensitive(&self) -> bool {
        true
    }

    fn read(&mut self, addr: u32) -> u8 {
        match self.port {
            SguPort::Regs(chip) => self.sgus[chip].borrow_mut().read_register(addr),
            SguPort::PcmRam => self.sgus[0].borrow().read_pcm_ram(addr),
        }
    }

    fn debug_read(&mut self, addr: u32) -> Option<u8> {
        Some(match self.port {
            SguPort::Regs(chip) => self.sgus[chip].borrow().peek_register(addr),
            SguPort::PcmRam => self.sgus[0].borrow().read_pcm_ram(addr),
        })
    }

    fn write(&mut self, addr: u32, value: u8) {
        match self.port {
            SguPort::Regs(chip) => self.sgus[chip].borrow_mut().write_register(addr, value),
            SguPort::PcmRam => {
                // Both cores reference the same RAM; notify each core so cached sample data is invalidated.
                for sgu in &self.sgus {
                    sgu.borrow_mut().write_pcm_ram(addr, value);
                }
            }
        }
    }

    fn size(&self) -> u32 {
        match self.port {
            SguPort::Regs(_) => SGU_REG_SIZE,
            SguPort::PcmRam => PCM_RAM_TOTAL_SIZE,
        }
    }
}

/// 48 kHz stereo ADC input of the 34SI22 (the emulator connects the host's audio input).
pub trait AdcSource {
    fn next_frame(&mut self) -> (i16, i16);
}

/// The 34SI22 on CPT32: its 24-bit memory bus reaches the shared PCM RAM (1 MiB, wrapping).
struct PcmRamBus<'a>(&'a mut [u8]);

impl Si22Memory for PcmRamBus<'_> {
    fn read(&mut self, address: u32) -> u8 {
        self.0[address as usize % self.0.len()]
    }

    fn write(&mut self, address: u32, value: u8) {
        let len = self.0.len();
        self.0[address as usize % len] = value;
    }
}

pub struct Si22Unit {
    pub chip: Si22,
    pcm_ram: Rc<RefCell<Vec<u8>>>,
    adc: Option<Box<dyn AdcSource>>,
    /// The guest read REC_A_ADC / REC_B_ADC (the host opens its input then too).
    adc_read: std::cell::Cell<bool>,
}

impl Si22Unit {
    pub fn new(pcm_ram: Rc<RefCell<Vec<u8>>>) -> Self {
        Self {
            chip: Si22::new(),
            pcm_ram,
            adc: None,
            adc_read: std::cell::Cell::new(false),
        }
    }

    /// Connect the ADC; without one the chip records silence.
    pub fn set_adc(&mut self, adc: Box<dyn AdcSource>) {
        self.adc = Some(adc);
    }

    pub fn has_adc(&self) -> bool {
        self.adc.is_some()
    }

    /// True once the guest uses the ADC: a recording is armed or REC_x_ADC was read.
    pub fn wants_adc(&self) -> bool {
        self.chip.input_active() || self.adc_read.get()
    }

    /// Guest register read: like `read_register`, and notes that the ADC is in use.
    pub fn read_register_from_guest(&self, offset: u32) -> u8 {
        if (0x25..=0x28).contains(&offset) {
            self.adc_read.set(true);
        }
        self.read_register(offset)
    }

    pub fn read_register(&self, offset: u32) -> u8 {
        if offset < SI22_REG_COUNT {
            self.chip.read_register(offset)
        } else {
            0
        }
    }

    pub fn write_register(&mut self, offset: u32, value: u8) {
        if offset < SI22_REG_COUNT {
            let mut ram = self.pcm_ram.borrow_mut();
            self.chip
                .write_register(offset, value, &mut PcmRamBus(&mut ram));
        }
    }

    fn tick(&mut self) -> (i16, i16) {
        let adc = self.adc.as_mut().map_or((0, 0), |adc| adc.next_frame());
        let mut ram = self.pcm_ram.borrow_mut();
        self.chip.tick(adc, &mut PcmRamBus(&mut ram))
    }

    /// Add the DAC output to the SGU master mix (32 bits, saturated once by the caller).
    pub fn clock_mixed_into_i32(&mut self, left_out: &mut [i32], right_out: &mut [i32]) {
        for (l, r) in left_out.iter_mut().zip(right_out.iter_mut()) {
            let (dl, dr) = self.tick();
            *l += i32::from(dl);
            *r += i32::from(dr);
        }
    }

    /// Add the DAC output to 16-bit buffers with saturation (host audio path).
    pub fn clock_mixed_into(&mut self, left_out: &mut [i16], right_out: &mut [i16]) {
        for (l, r) in left_out.iter_mut().zip(right_out.iter_mut()) {
            let (dl, dr) = self.tick();
            *l = saturate_output(i32::from(*l) + i32::from(dl));
            *r = saturate_output(i32::from(*r) + i32::from(dr));
        }
    }
}

pub struct Si22Device(pub Rc<RefCell<Si22Unit>>);

impl Device for Si22Device {
    /// Rendered per host frame like the SGU; no IRQ source.
    fn is_timing_insensitive(&self) -> bool {
        true
    }

    fn read(&mut self, addr: u32) -> u8 {
        self.0.borrow().read_register_from_guest(addr)
    }

    fn debug_read(&mut self, addr: u32) -> Option<u8> {
        Some(self.0.borrow().read_register(addr))
    }

    fn write(&mut self, addr: u32, value: u8) {
        self.0.borrow_mut().write_register(addr, value);
    }

    fn size(&self) -> u32 {
        SI22_REG_SIZE
    }
}

/// Reads `sgu.version` (`"1"`, `"2"`, `"2.1"` or `"3"`; default 3) from config.yaml.
/// A missing file selects the default.
pub fn sgu_version_from_config(path: &Path) -> Result<SguVersion, String> {
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(format!("{}: {error}", path.display())),
    };
    sgu_version_from_yaml(&contents)
}

pub fn sgu_version_from_yaml(contents: &str) -> Result<SguVersion, String> {
    let root: yaml_serde::Value = if contents.trim().is_empty() {
        yaml_serde::Value::Null
    } else {
        yaml_serde::from_str(contents).map_err(|error| format!("invalid YAML: {error}"))?
    };
    let Some(sgu) = root.get("sgu") else {
        return Ok(SguVersion::default());
    };
    let table = sgu.as_mapping().ok_or("`sgu` must be a mapping")?;
    let Some(value) = table.get("version") else {
        return Ok(SguVersion::default());
    };
    let text = match value {
        yaml_serde::Value::String(text) => text.clone(),
        yaml_serde::Value::Number(number) => number.to_string(),
        _ => return Err("`sgu.version` must be 1, 2, 2.1 or 3".into()),
    };
    SguVersion::parse(&text)
        .ok_or_else(|| format!("`sgu.version` must be 1, 2, 2.1 or 3 (got `{text}`)"))
}

pub fn connect_sgu(bus: &mut Bus) -> [Rc<RefCell<S3w2Sound>>; 2] {
    connect_sgu_with_version(bus, SguVersion::default())
}

pub fn connect_sgu_with_version(
    bus: &mut Bus,
    version: SguVersion,
) -> [Rc<RefCell<S3w2Sound>>; 2] {
    connect_sound_with_version(bus, version).sgus
}

/// The sound devices of one machine: two SGU cores and, from v3, the 34SI22.
pub struct SoundUnits {
    pub sgus: [Rc<RefCell<S3w2Sound>>; 2],
    pub si22: Option<Rc<RefCell<Si22Unit>>>,
}

pub fn connect_sound_with_version(bus: &mut Bus, version: SguVersion) -> SoundUnits {
    let pcm_ram = Rc::new(RefCell::new(vec![0; PCM_RAM_SIZE]));
    let sgus = [
        Rc::new(RefCell::new(S3w2Sound::with_version(Rc::clone(&pcm_ram), version))),
        Rc::new(RefCell::new(S3w2Sound::with_version(pcm_ram, version))),
    ];
    bus.add_device(
        SGU_REG_BASE,
        "SGU 0 MMIO",
        Box::new(SguDevice::new(sgus.clone(), SguPort::Regs(0))),
    );
    // v1 has a single SGU: SGU 1's MMIO window is left unmapped.
    if version.core_count() > 1 {
        bus.add_device(
            SGU2_REG_BASE,
            "SGU 1 MMIO",
            Box::new(SguDevice::new(sgus.clone(), SguPort::Regs(1))),
        );
    }
    bus.add_device(
        PCM_RAM_BASE,
        "PCMRAM",
        Box::new(SguDevice::new(sgus.clone(), SguPort::PcmRam)),
    );
    let si22 = version.has_34si22().then(|| {
        let pcm_ram = Rc::clone(&sgus[0].borrow().pcm_ram);
        let unit = Rc::new(RefCell::new(Si22Unit::new(pcm_ram)));
        bus.add_device(
            SI22_REG_BASE,
            "34SI22 MMIO",
            Box::new(Si22Device(Rc::clone(&unit))),
        );
        unit
    });
    SoundUnits { sgus, si22 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::sgu::s3w2::PHASE_FRAC_BITS;

    #[test]
    fn config_selects_sgu_version() {
        assert_eq!(sgu_version_from_yaml(""), Ok(SguVersion::V3));
        assert_eq!(sgu_version_from_yaml("sgu:\n  version: 3\n"), Ok(SguVersion::V3));
        assert_eq!(sgu_version_from_yaml("sgu:\n  version: 1\n"), Ok(SguVersion::V1));
        assert_eq!(sgu_version_from_yaml("sgu:\n  version: \"2\"\n"), Ok(SguVersion::V2));
        assert_eq!(sgu_version_from_yaml("sgu:\n  version: 2.1\n"), Ok(SguVersion::V2_1));
        assert!(sgu_version_from_yaml("sgu:\n  version: 4\n").is_err());
        assert!(sgu_version_from_yaml("sgu: 2\n").is_err());
        let mut bus = Bus::new();
        let sgus = connect_sgu_with_version(&mut bus, SguVersion::V1);
        assert_eq!(sgus[1].borrow().version, SguVersion::V1);
        // v1: only SGU 0 is mapped; the PCM RAM stays available.
        bus.write_u8(SGU_REG_BASE + 0x803, 7);
        assert_eq!(bus.read_u8(SGU_REG_BASE + 0x803), 7);
        bus.write_u8(SGU2_REG_BASE + 0x803, 7);
        assert_eq!(bus.read_u8(SGU2_REG_BASE + 0x803), 0);
        assert_eq!(sgus[1].borrow().channels[0].volume, 0);
        bus.write_u8(PCM_RAM_BASE + 5, 9);
        assert_eq!(bus.read_u8(PCM_RAM_BASE + 5), 9);
        let mut bus = Bus::new();
        connect_sgu_with_version(&mut bus, SguVersion::V2);
        bus.write_u8(SGU2_REG_BASE + 0x803, 7);
        assert_eq!(bus.read_u8(SGU2_REG_BASE + 0x803), 7);
    }

    #[test]
    fn si22_is_mapped_only_in_v3_and_records_into_pcm_ram() {
        let mut bus = Bus::new();
        assert!(connect_sound_with_version(&mut bus, SguVersion::V2_1).si22.is_none());
        bus.write_u8(SI22_REG_BASE + 0x23, 5);
        assert_eq!(bus.read_u8(SI22_REG_BASE + 0x23), 0);

        struct Ramp(i16);
        impl AdcSource for Ramp {
            fn next_frame(&mut self) -> (i16, i16) {
                self.0 = self.0.wrapping_add(0x100);
                (self.0, 0)
            }
        }
        let mut bus = Bus::new();
        let units = connect_sound_with_version(&mut bus, SguVersion::V3);
        let si22 = units.si22.unwrap();
        si22.borrow_mut().set_adc(Box::new(Ramp(0)));
        // REC A: PCM8 into 0x100..0x104 given as 0x100100..0x100104 (wraps at 1 MiB).
        for (offset, value) in [(0x10, 0x10), (0x11, 0x01), (0x12, 0x00), (0x13, 0x10), (0x14, 0x01), (0x15, 0x04)] {
            bus.write_u8(SI22_REG_BASE + offset, value);
        }
        assert_eq!(bus.read_u8(SI22_REG_BASE + 0x10), 0x10);
        bus.write_u8(SI22_REG_BASE + 0x1C, 1);
        let (mut l, mut r) = ([0i32; 8], [0i32; 8]);
        si22.borrow_mut().clock_mixed_into_i32(&mut l, &mut r);
        assert_eq!(bus.read_u8(SI22_REG_BASE + 0x1C), 3);
        let recorded: Vec<u8> = (0..5).map(|i| bus.read_u8(PCM_RAM_BASE + 0x100 + i)).collect();
        assert_eq!(recorded, [0x81, 0x82, 0x83, 0x84, 0]);

        // PLAY A plays it back to both sides at 0 dB.
        for (offset, value) in [(0x01, 0x01), (0x04, 0x01), (0x05, 0x04), (0x0D, 0x0F), (0x0C, 0x01)] {
            bus.write_u8(SI22_REG_BASE + offset, value);
        }
        let (mut l, mut r) = ([0i32; 5], [0i32; 5]);
        si22.borrow_mut().clock_mixed_into_i32(&mut l, &mut r);
        assert_eq!(l, [0x100, 0x200, 0x300, 0x400, 0]);
        assert_eq!(r, l);

        // REC_A_ADC follows the ADC while nothing records; a debugger peek does not
        // count as using the ADC, a guest read does.
        assert_eq!(bus.read_debug_u8(SI22_REG_BASE + 0x25), Some(0x0D)); // 13th tick
        assert!(!si22.borrow().wants_adc());
        assert_eq!(bus.read_u8(SI22_REG_BASE + 0x26), 0x00);
        assert!(si22.borrow().wants_adc());
    }

    #[test]
    fn debugger_register_read_does_not_reset_channel_phase() {
        let mut bus = Bus::new();
        let sgus = connect_sgu(&mut bus);
        sgus[0].borrow_mut().channels[0].phase = 192 << PHASE_FRAC_BITS;

        assert_eq!(bus.read_debug_u8(SGU_REG_BASE + 0x80A), Some(0));
        assert_eq!(sgus[0].borrow().channels[0].phase, 192 << PHASE_FRAC_BITS);

        sgus[0].borrow_mut().channels[0].phase = 128 << PHASE_FRAC_BITS;
        assert_eq!(bus.read_u8(SGU_REG_BASE + 0x80A), 0);
        assert_eq!(sgus[0].borrow().channels[0].phase, 0);
    }
}
