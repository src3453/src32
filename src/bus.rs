// Bus: Centralized I/O bus for connecting devices in the CPT32 emulator
// This module defines the Bus struct, which manages memory-mapped devices and
// routes read/write operations to the correct device based on address ranges.

// Internal Bus width: 32-bit address space (4GB), 32-bit data bus (8/16/32-bit access)
// External Bus width: 32-bit address space (4GB), 32-bit data bus (8/16/32-bit access)

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use crate::devices::bmc::{Bmc, connect_bmc};
use crate::devices::dmac::dmac::{Dmac, connect_dmac};
use crate::devices::irqc::irqc::IrqController;
use crate::devices::pec::idc::idc::{IDC_BASE, IDC_DATA_ADDRESS, IDC_SIZE, Idc};
use crate::devices::pec::pec::PeCState;
use crate::devices::vdp::vdp::Vdp;

struct DeviceMap {
    region: BusRegion,
    device: Box<dyn Device>,
}

/// Snapshot of one registered system-bus address range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BusRegion {
    /// Inclusive first byte of the mapped range.
    pub start: u32,
    /// Number of contiguous mapped bytes from `start`.
    pub size: u32,
    /// Human-readable device or memory-region name.
    pub name: &'static str,
}

pub trait Device {
    fn read(&mut self, addr: u32) -> u8;
    fn write(&mut self, addr: u32, value: u8);
    fn size(&self) -> u32;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusAccessSource {
    Cpu,
    Debugger,
}

pub struct Bus {
    devices: Vec<DeviceMap>,
    access_source: BusAccessSource,
    bus_error: bool,
    bmc: Option<Rc<RefCell<Bmc>>>,
    dmac: Option<Rc<RefCell<Dmac>>>,
    idc: Option<Rc<RefCell<Idc>>>,
    pec_input: Option<Arc<Mutex<PeCState>>>,
    irq_controller: Option<Rc<RefCell<IrqController>>>,
}

impl Bus {
    pub fn new() -> Self {
        Self {
            devices: Vec::new(),
            access_source: BusAccessSource::Cpu,
            bus_error: false,
            bmc: None,
            dmac: None,
            idc: None,
            pec_input: None,
            irq_controller: None,
        }
    }

    pub(crate) fn attach_bmc(&mut self, bmc: Rc<RefCell<Bmc>>) {
        self.bmc = Some(bmc);
    }

    pub(crate) fn attach_dmac(&mut self, dmac: Rc<RefCell<Dmac>>) {
        self.dmac = Some(dmac);
    }

    pub(crate) fn attach_idc(&mut self, idc: Rc<RefCell<Idc>>) {
        if let Some(dmac) = &self.dmac {
            dmac.borrow_mut().attach_idc(Rc::clone(&idc));
        }
        self.idc = Some(idc);
    }
    pub(crate) fn attach_pec_input(&mut self, state: Arc<Mutex<PeCState>>) {
        self.pec_input = Some(state);
    }

    pub(crate) fn attach_irq_controller(&mut self, irq: Rc<RefCell<IrqController>>) {
        self.irq_controller = Some(irq);
    }

    /// Advance one arbitration opportunity while the CPU clock is running.
    pub fn tick_devices(&mut self) {
        if let Some(dmac) = &self.dmac {
            dmac.borrow_mut().process_idc_abort_boundaries();
        }
        let dma_failure = self.idc.as_ref().and_then(|idc| idc.borrow_mut().tick());
        if let (Some(channel), Some(dmac)) = (dma_failure, &self.dmac) {
            dmac.borrow_mut().fail_from_idc(channel, 2);
        }
        self.refresh_device_irqs();
        self.service_dma_request(false);
    }

    pub fn has_pending_dma(&self) -> bool {
        self.dmac
            .as_ref()
            .is_some_and(|dmac| dmac.borrow().has_active_transfer())
            || self.idc.as_ref().is_some_and(|idc| idc.borrow().is_busy())
    }

    fn request_mask(&self, include_cpu: bool) -> u8 {
        let mut requests = if include_cpu { 1 } else { 0 };
        if let Some(dmac) = &self.dmac {
            requests |= dmac.borrow().request_mask();
        }
        requests
    }

    fn service_dma_request(&mut self, include_cpu: bool) -> Option<u8> {
        let (Some(bmc), Some(dmac)) = (self.bmc.as_ref().cloned(), self.dmac.as_ref().cloned())
        else {
            return None;
        };
        let requests = self.request_mask(include_cpu);
        let selected = bmc.borrow_mut().grant(requests)?;
        if selected < 2 {
            bmc.borrow_mut().release();
            return Some(selected);
        }
        let channel = usize::from(selected - 2);
        let op = dmac.borrow().next_op(channel);
        let result = match op {
            Some(op) => self.execute_dma_op(op, channel),
            None => None,
        };
        {
            let mut dmac = dmac.borrow_mut();
            match (op, result) {
                (Some(op), Some(value)) => {
                    dmac.complete_op(channel, if op.write { op.value } else { value })
                }
                (Some(_), None) => dmac.fail(channel, 2),
                (None, _) => {}
            }
        }
        bmc.borrow_mut().release();
        self.refresh_device_irqs();
        Some(selected)
    }

    fn execute_dma_op(
        &mut self,
        op: crate::devices::dmac::dmac::DmaOp,
        channel: usize,
    ) -> Option<u32> {
        if op.address == IDC_DATA_ADDRESS {
            if op.width != 1 {
                return None;
            }
            let idc = self.idc.as_ref()?.clone();
            return if op.write {
                idc.borrow_mut()
                    .dma_write_data(channel, op.value as u8)
                    .then_some(op.value & 0xFF)
            } else {
                idc.borrow_mut().dma_read_data(channel).map(u32::from)
            };
        }

        let end = op.address.checked_add(u32::from(op.width))?;
        let allowed = (op.address < 0x0100_0000 && end <= 0x0100_0000)
            || (op.address >= 0x1000_0000 && end <= 0x1040_0000)
            || (op.address >= 0x1800_0000 && end <= 0x1810_0000)
            || (op.write && op.width == 4 && op.address == 0x8003_002C);
        if !allowed || op.address % u32::from(op.width) != 0 {
            return None;
        }

        if op.write {
            for byte_index in 0..op.width {
                let shift = (u32::from(op.width - byte_index - 1)) * 8;
                let byte = (op.value >> shift) as u8;
                let Some((device, offset)) =
                    self.find_device_mut(op.address + u32::from(byte_index))
                else {
                    return None;
                };
                device.write(offset, byte);
            }
            Some(op.value)
        } else {
            let mut value = 0u32;
            for byte_index in 0..op.width {
                let Some((device, offset)) = self.find_device(op.address + u32::from(byte_index))
                else {
                    return None;
                };
                value = (value << 8) | u32::from(device.read(offset));
            }
            Some(value)
        }
    }

    fn refresh_device_irqs(&mut self) {
        let dmac_pending = self
            .dmac
            .as_ref()
            .is_some_and(|dmac| dmac.borrow().irq_pending());
        let idc_pending = self
            .idc
            .as_ref()
            .is_some_and(|idc| idc.borrow().irq_asserted());
        let pec_pending = self.pec_input.as_ref().is_some_and(|state| {
            state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .has_pending_events()
        });
        if let Some(irq) = &self.irq_controller {
            let mut irq = irq.borrow_mut();
            irq.set_source(1, dmac_pending);
            irq.set_source(2, idc_pending);
            irq.set_source(3, pec_pending);
        }
    }

    fn arbitrate_cpu_access(&mut self) {
        if self.access_source != BusAccessSource::Cpu {
            return;
        }
        while self.service_dma_request(true) != Some(0) {
            if self.bmc.is_none() || self.dmac.is_none() {
                break;
            }
        }
        if let Some(bmc) = &self.bmc {
            // CPU owns the synchronous access until the byte operation returns.
            bmc.borrow_mut().claim_cpu();
        }
    }

    fn release_cpu_access(&mut self) {
        if let Some(bmc) = &self.bmc {
            bmc.borrow_mut().release();
        }
    }

    pub fn set_access_source(&mut self, source: BusAccessSource) {
        self.access_source = source;
    }
    pub fn access_source(&self) -> BusAccessSource {
        self.access_source
    }
    pub fn take_bus_error(&mut self) -> bool {
        let error = self.bus_error;
        self.bus_error = false;
        error
    }

    /// Register a named device; its declared size determines the mapped range.
    pub fn add_device(&mut self, addr: u32, name: &'static str, device: Box<dyn Device>) {
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
                .region
                .start
                .checked_add(mapped.region.size)
                .expect("Existing device range overflow");
            if !(new_end <= mapped.region.start || addr >= end) {
                panic!(
                    "Device overlap detected: new [0x{:08X}-0x{:08X}) overlaps with [0x{:08X}-0x{:08X})",
                    addr, new_end, mapped.region.start, end
                );
            }
        }

        self.devices.push(DeviceMap {
            region: BusRegion {
                start: addr,
                size,
                name,
            },
            device,
        });
    }
    /// Iterate over mapped ranges in registration order without accessing devices.
    pub fn memory_map(&self) -> impl Iterator<Item = BusRegion> + '_ {
        self.devices.iter().map(|mapped| mapped.region)
    }

    pub fn find_device(&mut self, addr: u32) -> Option<(&mut dyn Device, u32)> {
        for mapped in &mut self.devices {
            let end = mapped
                .region
                .start
                .checked_add(mapped.region.size)
                .expect("Existing device range overflow");
            if addr >= mapped.region.start && addr < end {
                return Some((&mut *mapped.device, addr - mapped.region.start));
            }
        }
        None
    }

    pub fn find_device_mut(&mut self, addr: u32) -> Option<(&mut dyn Device, u32)> {
        for mapped in &mut self.devices {
            let end = mapped
                .region
                .start
                .checked_add(mapped.region.size)
                .expect("Existing device range overflow");
            if addr >= mapped.region.start && addr < end {
                return Some((&mut *mapped.device, addr - mapped.region.start));
            }
        }
        None
    }

    pub fn read_u8(&mut self, addr: u32) -> u8 {
        self.arbitrate_cpu_access();
        if let Some((device, offset)) = self.find_device(addr) {
            let value = device.read(offset);
            self.release_cpu_access();
            return value;
        }
        self.bus_error = true;
        if self.access_source == BusAccessSource::Debugger {
            self.release_cpu_access();
            return 0;
        }
        panic!("Invalid I/O read: 0x{:08X}", addr);
    }

    /// Read a byte for a debugger view. `None` represents an open bus.
    pub fn read_debug_u8(&mut self, addr: u32) -> Option<u8> {
        if addr == IDC_DATA_ADDRESS {
            return self.idc.as_ref().map(|idc| idc.borrow().peek_data_byte());
        }
        if let Some((device, offset)) = self.find_device(addr) {
            return Some(device.read(offset));
        }
        self.bus_error = true;
        Some(0).filter(|_| self.access_source != BusAccessSource::Debugger)
    }

    pub fn write_u8(&mut self, addr: u32, value: u8) {
        self.arbitrate_cpu_access();
        if let Some((device, offset)) = self.find_device_mut(addr) {
            device.write(offset, value);
            self.refresh_device_irqs();
            self.release_cpu_access();
            return;
        }
        self.bus_error = true;
        if self.access_source == BusAccessSource::Debugger {
            self.release_cpu_access();
            return;
        }
        panic!("Invalid I/O write: 0x{:08X}", addr);
    }

    fn overlaps_idc_data(address: u32, width: u32) -> bool {
        address
            .checked_add(width)
            .is_some_and(|end| address <= IDC_DATA_ADDRESS && IDC_DATA_ADDRESS < end)
    }

    fn invalid_idc_u32_access(address: u32) -> bool {
        Self::overlaps_idc_data(address, 4)
            || (address >= IDC_BASE && address < IDC_BASE + IDC_SIZE && address & 3 != 0)
    }

    pub fn read_u32_be(&mut self, addr: u32) -> u32 {
        if Self::invalid_idc_u32_access(addr) {
            self.bus_error = true;
            return 0;
        }
        let b0 = self.read_u8(addr) as u32;
        let b1 = self.read_u8(addr.wrapping_add(1)) as u32;
        let b2 = self.read_u8(addr.wrapping_add(2)) as u32;
        let b3 = self.read_u8(addr.wrapping_add(3)) as u32;
        (b0 << 24) | (b1 << 16) | (b2 << 8) | b3
    }

    pub fn write_u32_be(&mut self, addr: u32, value: u32) {
        if Self::invalid_idc_u32_access(addr) {
            self.bus_error = true;
            return;
        }
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
    connect_bmc_dmac(bus);
    let _sgu = crate::devices::sgu::sgu::connect_sgu(bus);
    crate::devices::vdp::vdp::connect_vdp(bus)
}

pub fn connect_bmc_dmac(bus: &mut Bus) {
    let bmc = connect_bmc(bus);
    let _dmac = connect_dmac(bus, bmc);
}
