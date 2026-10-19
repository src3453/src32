// Bus: Centralized I/O bus for connecting devices in the CPT32 emulator
// This module defines the Bus struct, which manages memory-mapped devices and
// routes read/write operations to the correct device based on address ranges.

// Internal Bus width: 32-bit address space (4GB), 32-bit data bus (8/16/32-bit access)
// External Bus width: 32-bit address space (4GB), 32-bit data bus (8/16/32-bit access)

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use crate::devices::bmc::{Bmc, connect_bmc};
use crate::devices::dmac::dmac::{Dmac, connect_dmac};
use crate::devices::expansion::{
    EXPANSION_BASE, EXPANSION_SLOT_COUNT, EXPANSION_SLOT_SIZE, ExpansionCard,
};
use crate::devices::irqc::irqc::IrqController;
use crate::devices::pec::idc::idc::{IDC_BASE, IDC_DATA_ADDRESS, IDC_SIZE, Idc};
use crate::devices::pec::pec::PeCState;
use crate::devices::pec::timing::PeCTimingState;
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
    /// Read a byte for a debugger without triggering device read side effects.
    /// Devices that cannot provide a stable snapshot should return `None`.
    fn debug_read(&self, _addr: u32) -> Option<u8> {
        None
    }
    fn write(&mut self, addr: u32, value: u8);
    fn size(&self) -> u32;
    /// Return the generation token for the cache-coherence unit containing `addr`.
    /// It must change whenever any byte in that unit changes. `None` disables
    /// instruction caching for the address.
    fn instruction_cache_generation(&self, _addr: u32) -> Option<u64> {
        None
    }
    /// Side-effect-free instruction preview. Opt in only for stable memory.
    fn peek_instruction_u8(&self, _addr: u32) -> Option<u8> {
        None
    }
    /// Enable or initialize generation tracking before a cached CPU is created.
    fn enable_instruction_cache_tracking(&mut self) {}
    fn next_event_cycle(&self) -> Option<u64> {
        None
    }
    fn run_scheduled_event(&mut self, _cycle: u64) {}
    fn is_faulted(&self) -> bool {
        false
    }
    fn irq_asserted(&self) -> bool {
        false
    }
    fn reset(&mut self) {}
    /// Add stereo samples at the host output rate. Called serially with MMIO.
    fn render_audio(&mut self, _sample_rate: u32, _left: &mut [i16], _right: &mut [i16]) {}
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
    pec_timing: Option<Rc<RefCell<PeCTimingState>>>,
    irq_controller: Option<Rc<RefCell<IrqController>>>,
    system_cycles: u64,
    cycle_clock: Rc<Cell<u64>>,
    expansion_devices: [Option<usize>; 8],
    has_expansion_cards: bool,
    relaxed_cycles: u64,
    relaxed_limit: u64,
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
            pec_timing: None,
            irq_controller: None,
            system_cycles: 0,
            cycle_clock: Rc::new(Cell::new(0)),
            expansion_devices: [None; 8],
            has_expansion_cards: false,
            relaxed_cycles: 0,
            relaxed_limit: 0,
        }
    }

    pub(crate) fn cycle_clock(&self) -> Rc<Cell<u64>> {
        Rc::clone(&self.cycle_clock)
    }

    pub fn system_cycles(&self) -> u64 {
        self.system_cycles
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
    pub(crate) fn attach_pec_timing(&mut self, state: Rc<RefCell<PeCTimingState>>) {
        self.pec_timing = Some(state);
    }

    fn advance_pec_timing_cycles(&mut self, cycles: u64) {
        if let Some(state) = &self.pec_timing {
            state.borrow_mut().advance_system_cycles(cycles);
        }
    }

    pub fn advance_system_cycles(&mut self, cycles: u64) {
        let target = self.system_cycles.saturating_add(cycles);
        let event_at_start = self.run_due_expansion_events();
        if cycles == 0 || target == self.system_cycles || event_at_start {
            self.refresh_device_irqs();
        }
        while self.system_cycles < target {
            let now = self.system_cycles;
            let mut next_cycle = target;
            if let Some(until_irq) = self.cycles_until_pec_irq() {
                next_cycle = next_cycle.min(now.saturating_add(until_irq.max(1)));
            }
            if let Some(deadline) = self
                .next_expansion_event_cycle()
                .filter(|&cycle| cycle > now)
            {
                next_cycle = next_cycle.min(deadline);
            }
            let elapsed = next_cycle - now;
            self.advance_pec_timing_cycles(elapsed);
            self.system_cycles = next_cycle;
            self.cycle_clock.set(next_cycle);
            self.run_due_expansion_events();
            self.refresh_device_irqs();
        }
    }

    pub(crate) fn cycles_until_pec_irq(&self) -> Option<u64> {
        self.pec_timing
            .as_ref()
            .and_then(|state| state.borrow().cycles_until_irq())
    }

    pub fn cycles_until_next_event(&self) -> Option<u64> {
        let now = self.system_cycles;
        let card_cycles = self
            .next_expansion_event_cycle()
            .map(|deadline| deadline.saturating_sub(now));
        match (self.cycles_until_pec_irq(), card_cycles) {
            (Some(pec), Some(card)) => Some(pec.min(card)),
            (Some(pec), None) => Some(pec),
            (None, Some(card)) => Some(card),
            (None, None) => None,
        }
    }

    fn next_expansion_event_cycle(&self) -> Option<u64> {
        if !self.has_expansion_cards {
            return None;
        }
        self.expansion_devices
            .iter()
            .flatten()
            .filter_map(|&index| self.devices[index].device.next_event_cycle())
            .min()
    }

    fn run_due_expansion_events(&mut self) -> bool {
        let cycle = self.system_cycles;
        if !self.has_expansion_cards {
            return false;
        }
        let mut event_ran = false;
        for slot in 0..usize::from(EXPANSION_SLOT_COUNT) {
            if let Some(index) = self.expansion_devices[slot] {
                let device = &mut self.devices[index].device;
                if device
                    .next_event_cycle()
                    .is_some_and(|deadline| deadline <= cycle)
                {
                    device.run_scheduled_event(cycle);
                    event_ran = true;
                }
            }
        }
        event_ran
    }

    pub fn reset_expansion_cards(&mut self) {
        for index in self.expansion_devices.iter().flatten().copied() {
            self.devices[index].device.reset();
        }
        self.refresh_device_irqs();
    }

    pub fn mix_expansion_audio(&mut self, sample_rate: u32, left: &mut [i16], right: &mut [i16]) {
        if sample_rate == 0 || sample_rate > i32::MAX as u32 {
            return;
        }
        for index in self.expansion_devices.iter().flatten().copied() {
            self.devices[index]
                .device
                .render_audio(sample_rate, left, right);
        }
        self.refresh_device_irqs();
    }

    pub(crate) fn install_expansion_cards(
        &mut self,
        mut cards: Vec<(u8, ExpansionCard)>,
    ) -> Result<(), String> {
        let mut slots = [false; EXPANSION_SLOT_COUNT as usize];
        for (slot, card) in &cards {
            if *slot >= EXPANSION_SLOT_COUNT || card.slot() != *slot {
                return Err(format!(
                    "expansion card slot mismatch: requested {slot}, card slot {}",
                    card.slot()
                ));
            }
            let slot_index = usize::from(*slot);
            if slots[slot_index] || self.expansion_devices[slot_index].is_some() {
                return Err(format!("expansion slot {slot} is already occupied"));
            }
            slots[slot_index] = true;
            let address = EXPANSION_BASE + u32::from(*slot) * EXPANSION_SLOT_SIZE;
            let end = address + EXPANSION_SLOT_SIZE;
            for mapped in &self.devices {
                let mapped_end = mapped.region.start + mapped.region.size;
                if !(end <= mapped.region.start || address >= mapped_end) {
                    return Err(format!(
                        "expansion slot {slot} overlaps mapped device {}",
                        mapped.region.name
                    ));
                }
            }
        }

        cards.sort_unstable_by_key(|(slot, _)| *slot);
        for (slot, card) in cards {
            let address = EXPANSION_BASE + u32::from(slot) * EXPANSION_SLOT_SIZE;
            self.add_device(address, "WASM expansion card", Box::new(card));
            self.expansion_devices[usize::from(slot)] = Some(self.devices.len() - 1);
            self.has_expansion_cards = true;
        }
        self.refresh_device_irqs();
        Ok(())
    }

    pub fn attach_irq_controller(&mut self, irq: Rc<RefCell<IrqController>>) {
        self.irq_controller = Some(irq);
        self.refresh_device_irqs();
    }

    /// Advance one 48 MHz system cycle and one bus arbitration opportunity.
    pub fn tick_devices(&mut self) {
        self.advance_system_cycles(1);
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

    /// Experimental idle-device batching. Active transfers and expansion cards
    /// retain the ordinary cycle-by-cycle path. Bus accesses flush the batch.
    pub(crate) fn tick_relaxed_devices(&mut self) {
        if self.relaxed_cycles == 0 {
            if self.has_expansion_cards
                || self.has_pending_dma()
                || self
                    .dmac
                    .as_ref()
                    .is_some_and(|dmac| !dmac.borrow().can_batch_idle())
            {
                self.tick_devices();
                return;
            }
            self.relaxed_limit = self.cycles_until_next_event().unwrap_or(32).clamp(1, 32);
        }
        self.relaxed_cycles += 1;
        if self.relaxed_cycles >= self.relaxed_limit {
            self.flush_relaxed_devices();
        }
    }

    /// Cycles that cannot cause device progress, IRQ changes, or DMA writes.
    /// Leave the deadline cycle to the ordinary per-instruction path. A batch
    /// must contain no bus accesses, and may only be reserved by exclusive CPU run.
    #[cfg(feature = "jit")]
    pub(crate) fn native_idle_budget(&self) -> u64 {
        if self.relaxed_cycles != 0 {
            return self.relaxed_limit.saturating_sub(self.relaxed_cycles + 1);
        }
        if self.has_expansion_cards
            || self.has_pending_dma()
            || self
                .dmac
                .as_ref()
                .is_some_and(|dmac| !dmac.borrow().can_batch_idle())
        {
            return 0;
        }
        self.cycles_until_next_event().unwrap_or(32).clamp(1, 32) - 1
    }

    #[cfg(feature = "jit")]
    pub(crate) fn defer_native_idle_cycles(&mut self, cycles: u64) {
        debug_assert!(cycles <= self.native_idle_budget());
        if self.relaxed_cycles == 0 {
            self.relaxed_limit = self.cycles_until_next_event().unwrap_or(32).clamp(1, 32);
        }
        self.relaxed_cycles += cycles;
    }

    pub(crate) fn flush_relaxed_devices(&mut self) {
        let cycles = std::mem::take(&mut self.relaxed_cycles);
        if cycles != 0 {
            // No IDC command/DMA can start without an intervening bus access.
            if let Some(idc) = &self.idc {
                idc.borrow_mut().advance_idle_cycles(cycles);
            }
            self.advance_system_cycles(cycles);
        }
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

    fn expansion_slot_for_beat(address: u32, end: u32) -> Option<usize> {
        let aperture_end = EXPANSION_BASE + u32::from(EXPANSION_SLOT_COUNT) * EXPANSION_SLOT_SIZE;
        if !(EXPANSION_BASE..aperture_end).contains(&address) {
            return None;
        }
        let slot = ((address - EXPANSION_BASE) / EXPANSION_SLOT_SIZE) as usize;
        let slot_end = EXPANSION_BASE + (slot as u32 + 1) * EXPANSION_SLOT_SIZE;
        (end <= slot_end).then_some(slot)
    }

    fn is_unoccupied_expansion_address(&self, address: u32) -> bool {
        let aperture_end = EXPANSION_BASE + u32::from(EXPANSION_SLOT_COUNT) * EXPANSION_SLOT_SIZE;
        if !(EXPANSION_BASE..aperture_end).contains(&address) {
            return false;
        }
        let slot = ((address - EXPANSION_BASE) / EXPANSION_SLOT_SIZE) as usize;
        self.expansion_devices[slot].is_none()
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
        let expansion_slot = Self::expansion_slot_for_beat(op.address, end);
        let allowed = (op.address < 0x0100_0000 && end <= 0x0100_0000)
            || (op.address >= 0x1000_0000 && end <= 0x1040_0000)
            || (op.address >= 0x1800_0000 && end <= 0x1810_0000)
            || (op.write && op.width == 4 && op.address == 0x8003_002C)
            || expansion_slot.is_some();
        if !allowed || op.address % u32::from(op.width) != 0 {
            return None;
        }
        if let Some(slot) = expansion_slot {
            let index = self.expansion_devices[slot]?;
            if self.devices[index].device.is_faulted() {
                return None;
            }
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
                if device.is_faulted() {
                    return None;
                }
                device.write(offset, byte);
                if device.is_faulted() {
                    return None;
                }
            }
            Some(op.value)
        } else {
            let mut value = 0u32;
            for byte_index in 0..op.width {
                let Some((device, offset)) = self.find_device(op.address + u32::from(byte_index))
                else {
                    return None;
                };
                let byte = device.read(offset);
                if device.is_faulted() {
                    return None;
                }
                value = (value << 8) | u32::from(byte);
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
        let pec_timing_pending = self
            .pec_timing
            .as_ref()
            .is_some_and(|state| state.borrow().irq_asserted());
        let expansion_pending = if self.has_expansion_cards {
            std::array::from_fn(|slot| {
                self.expansion_devices[slot]
                    .is_some_and(|index| self.devices[index].device.irq_asserted())
            })
        } else {
            [false; 8]
        };
        if let Some(irq) = &self.irq_controller {
            let mut irq = irq.borrow_mut();
            irq.set_source(1, dmac_pending);
            irq.set_source(2, idc_pending);
            irq.set_source(3, pec_pending);
            irq.set_source(4, pec_timing_pending);
            for (slot, asserted) in expansion_pending.into_iter().enumerate() {
                irq.set_source((slot + 5) as u8, asserted);
            }
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

    fn latch_bus_error(&mut self) {
        if self.access_source == BusAccessSource::Cpu {
            self.bus_error = true;
        }
    }
    pub fn take_bus_error(&mut self) -> bool {
        let error = self.bus_error;
        self.bus_error = false;
        error
    }

    /// Register a named device; its declared size determines the mapped range.
    pub fn add_device(&mut self, addr: u32, name: &'static str, device: Box<dyn Device>) {
        let size = device.size();
        tracing::info!(
            start = format!("0x{:08X}", addr),
            end = format!("0x{:08X}", addr.wrapping_add(size).wrapping_sub(1)),
            size = format!("0x{:08X}", size),
            name,
            "Mapped bus device"
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

    /// Return the executable-memory generation at an address without a bus read.
    pub(crate) fn instruction_cache_generation(&self, addr: u32) -> Option<u64> {
        self.devices.iter().find_map(|mapped| {
            let offset = addr.checked_sub(mapped.region.start)?;
            (offset < mapped.region.size)
                .then(|| mapped.device.instruction_cache_generation(offset))
                .flatten()
        })
    }

    #[cfg(feature = "jit")]
    pub(crate) fn peek_instruction_u8(&self, addr: u32) -> Option<u8> {
        self.devices.iter().find_map(|mapped| {
            let offset = addr.checked_sub(mapped.region.start)?;
            (offset < mapped.region.size)
                .then(|| mapped.device.peek_instruction_u8(offset))
                .flatten()
        })
    }

    /// Enable instruction-generation tracking for cache-aware CPU modes.
    pub(crate) fn enable_instruction_cache_tracking(&mut self) {
        for mapped in &mut self.devices {
            mapped.device.enable_instruction_cache_tracking();
        }
    }

    pub fn read_u8(&mut self, addr: u32) -> u8 {
        self.flush_relaxed_devices();
        self.arbitrate_cpu_access();
        if let Some((device, offset)) = self.find_device(addr) {
            let value = device.read(offset);
            if device.is_faulted() {
                self.latch_bus_error();
            }
            self.refresh_device_irqs();
            self.release_cpu_access();
            return value;
        }
        // An empty expansion aperture reads as open-bus zero so guest drivers
        // can probe optional cards without taking a bus-error exception.
        if !self.is_unoccupied_expansion_address(addr) {
            self.latch_bus_error();
        }
        self.release_cpu_access();
        0
    }

    /// Read a byte for a passive debugger view. `None` means no safe snapshot
    /// is available. This path never performs a normal device read.
    pub fn read_debug_u8(&mut self, addr: u32) -> Option<u8> {
        if addr == IDC_DATA_ADDRESS {
            return self.idc.as_ref().map(|idc| idc.borrow().peek_data_byte());
        }
        if let Some((device, offset)) = self.find_device(addr) {
            return device.debug_read(offset);
        }
        None
    }

    pub fn write_u8(&mut self, addr: u32, value: u8) {
        self.flush_relaxed_devices();
        self.arbitrate_cpu_access();
        if let Some((device, offset)) = self.find_device_mut(addr) {
            device.write(offset, value);
            if device.is_faulted() {
                self.latch_bus_error();
            }
            self.refresh_device_irqs();
            self.release_cpu_access();
            return;
        }
        self.latch_bus_error();
        self.release_cpu_access();
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
            self.flush_relaxed_devices();
            self.latch_bus_error();
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
            self.flush_relaxed_devices();
            self.latch_bus_error();
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

#[cfg(test)]
mod expansion_dma_boundary_tests {
    use super::*;

    #[test]
    fn dma_beat_must_remain_inside_one_slot_window() {
        let slot0_end = EXPANSION_BASE + EXPANSION_SLOT_SIZE;
        assert_eq!(
            Bus::expansion_slot_for_beat(slot0_end - 2, slot0_end),
            Some(0)
        );
        assert_eq!(
            Bus::expansion_slot_for_beat(slot0_end - 1, slot0_end),
            Some(0)
        );
        assert_eq!(
            Bus::expansion_slot_for_beat(slot0_end - 1, slot0_end + 1),
            None
        );
        assert_eq!(Bus::expansion_slot_for_beat(0xB000_0000, 0xB000_0001), None);
    }
}

#[cfg(test)]
mod relaxed_device_tests {
    use super::*;
    use crate::devices::pec::timing::connect_pec_timer_rtc;

    #[test]
    fn relaxed_ticks_batch_but_bus_access_and_budget_flush_time() {
        let mut bus = Bus::new();
        crate::devices::ram::connect_ram(&mut bus);
        for _ in 0..31 {
            bus.tick_relaxed_devices();
        }
        assert_eq!(bus.system_cycles(), 0);
        bus.tick_relaxed_devices();
        assert_eq!(bus.system_cycles(), 32);
        bus.tick_relaxed_devices();
        bus.write_u8(0, 7);
        assert_eq!(bus.system_cycles(), 33);
        bus.tick_relaxed_devices();
        assert_eq!(bus.read_u8(0), 7);
        assert_eq!(bus.system_cycles(), 34);
        bus.tick_relaxed_devices();
        bus.flush_relaxed_devices();
        assert_eq!(bus.system_cycles(), 35);
    }

    #[test]
    fn relaxed_ticks_stop_at_timer_deadline_and_disable_for_cards() {
        let mut bus = Bus::new();
        connect_pec_timer_rtc(&mut bus);
        let irq = crate::devices::irqc::irqc::connect_irqc(&mut bus);
        bus.attach_irq_controller(Rc::clone(&irq));
        bus.write_u32_be(0x8004_0184, 1);
        bus.write_u32_be(0x8004_0410, 7);
        bus.write_u32_be(0x8004_0400, 12);
        for _ in 0..6 {
            bus.tick_relaxed_devices();
        }
        assert_eq!(irq.borrow().pending(), 0);
        bus.tick_relaxed_devices();
        assert_eq!(bus.system_cycles(), 7);
        assert_ne!(irq.borrow().pending() & (1 << 4), 0);
        bus.has_expansion_cards = true;
        bus.tick_relaxed_devices();
        assert_eq!(bus.system_cycles(), 8);
    }
}
