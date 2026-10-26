// SGU: Sound Generator Unit
// This module provides the bus device wrapper for the 3WS8PN (S3W2) sound generator.
// SGU MMIO: 0x80020000 - 0x800208FF and 0x80021000 - 0x800218FF
// PCMRAM:   0x18000000 - 0x180FFFFF (1MB)

use std::cell::RefCell;
use std::rc::Rc;

use crate::bus::{Bus, Device};
use std::path::Path;

use crate::devices::sgu::s3w2::{PCM_RAM_SIZE, S3w2Sound, SguVersion};

pub const SGU_REG_BASE: u32 = 0x8002_0000;
pub const SGU2_REG_BASE: u32 = 0x8002_1000;
pub const SGU_REG_SIZE: u32 = 0x0000_0900; // 0x900 bytes (0x800 wavetable SRAM + 0x100 registers)

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

/// Reads `sgu.version` (`"1"`, `"2"` or `"2.1"`; default 2.1) from config.yaml.
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
        _ => return Err("`sgu.version` must be 1, 2 or 2.1".into()),
    };
    SguVersion::parse(&text)
        .ok_or_else(|| format!("`sgu.version` must be 1, 2 or 2.1 (got `{text}`)"))
}

pub fn connect_sgu(bus: &mut Bus) -> [Rc<RefCell<S3w2Sound>>; 2] {
    connect_sgu_with_version(bus, SguVersion::default())
}

pub fn connect_sgu_with_version(
    bus: &mut Bus,
    version: SguVersion,
) -> [Rc<RefCell<S3w2Sound>>; 2] {
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
    sgus
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::sgu::s3w2::PHASE_FRAC_BITS;

    #[test]
    fn config_selects_sgu_version() {
        assert_eq!(sgu_version_from_yaml(""), Ok(SguVersion::V2_1));
        assert_eq!(sgu_version_from_yaml("sgu:\n  version: 1\n"), Ok(SguVersion::V1));
        assert_eq!(sgu_version_from_yaml("sgu:\n  version: \"2\"\n"), Ok(SguVersion::V2));
        assert_eq!(sgu_version_from_yaml("sgu:\n  version: 2.1\n"), Ok(SguVersion::V2_1));
        assert!(sgu_version_from_yaml("sgu:\n  version: 3\n").is_err());
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
