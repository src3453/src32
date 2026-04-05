use std::cell::RefCell;
use std::rc::Rc;

use crate::devices::vdp::vdp::Vdp;

struct DeviceMap {
    addr: u32,
    size: u32,
    device: Box<dyn Device>,
}

pub trait Device {
    fn read(&self, addr: u32) -> u8;
    fn write(&mut self, addr: u32, value: u8);
    fn size(&self) -> u32;
}

pub struct Bus {
    devices: Vec<DeviceMap>,
}

impl Bus {
    pub fn new() -> Self {
        Self {
            devices: Vec::new(),
        }
    }



    pub fn add_device(&mut self, addr: u32, device: Box<dyn Device>) {
        let size = device.size();
        println!(
            "Bus: Adding device at 0x{:08X}-0x{:08X} (size: 0x{:X})",
            addr,
            addr.wrapping_add(size),
            size
        );
        if size == 0 {
            return;
        }

        let new_end = addr.checked_add(size).expect("Device range overflow");
        for mapped in &self.devices {
            let end = mapped
                .addr
                .checked_add(mapped.size)
                .expect("Existing device range overflow");
            if !(new_end <= mapped.addr || addr >= end) {
                panic!(
                    "Device overlap detected: new [0x{:08X}-0x{:08X}) overlaps with [0x{:08X}-0x{:08X})",
                    addr, new_end, mapped.addr, end
                );
            }
        }

        self.devices.push(DeviceMap { addr, size, device });
    }

    pub fn find_device(&self, addr: u32) -> Option<(&dyn Device, u32)> {
        for mapped in &self.devices {
            let end = mapped
                .addr
                .checked_add(mapped.size)
                .expect("Existing device range overflow");
            if addr >= mapped.addr && addr < end {
                return Some((&*mapped.device, addr - mapped.addr));
            }
        }
        None
    }

    pub fn find_device_mut(&mut self, addr: u32) -> Option<(&mut dyn Device, u32)> {
        for mapped in &mut self.devices {
            let end = mapped
                .addr
                .checked_add(mapped.size)
                .expect("Existing device range overflow");
            if addr >= mapped.addr && addr < end {
                return Some((&mut *mapped.device, addr - mapped.addr));
            }
        }
        None
    }

    pub fn read_u8(&self, addr: u32) -> u8 {
        if let Some((device, offset)) = self.find_device(addr) {
            return device.read(offset);
        }
        panic!("Invalid I/O read: 0x{:08X}", addr);
    }

    pub fn write_u8(&mut self, addr: u32, value: u8) {
        if let Some((device, offset)) = self.find_device_mut(addr) {
            device.write(offset, value);
            return;
        }
        panic!("Invalid I/O write: 0x{:08X}", addr);
    }

    pub fn read_u32_be(&self, addr: u32) -> u32 {
        let b0 = self.read_u8(addr) as u32;
        let b1 = self.read_u8(addr.wrapping_add(1)) as u32;
        let b2 = self.read_u8(addr.wrapping_add(2)) as u32;
        let b3 = self.read_u8(addr.wrapping_add(3)) as u32;
        (b0 << 24) | (b1 << 16) | (b2 << 8) | b3
    }

    pub fn write_u32_be(&mut self, addr: u32, value: u32) {
        self.write_u8(addr, ((value >> 24) & 0xFF) as u8);
        self.write_u8(addr.wrapping_add(1), ((value >> 16) & 0xFF) as u8);
        self.write_u8(addr.wrapping_add(2), ((value >> 8) & 0xFF) as u8);
        self.write_u8(addr.wrapping_add(3), (value & 0xFF) as u8);
    }
}

pub fn connect_devices(bus: &mut Bus) {
    let _ = connect_devices_with_vdp(bus);
}

pub fn connect_devices_with_vdp(bus: &mut Bus) -> Rc<RefCell<Vdp>> {
    crate::devices::ram::connect_ram(bus);
    crate::devices::vdp::vdp::connect_vdp(bus)
}