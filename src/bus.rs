// Bus: Centralized I/O bus for connecting devices in the CPT32 emulator
// This module defines the Bus struct, which manages memory-mapped devices and
// routes read/write operations to the correct device based on address ranges.

// Internal Bus width: 32-bit address space (4GB), 32-bit data bus (8/16/32-bit access)
// External Bus width: 32-bit address space (4GB), 32-bit data bus (8/16/32-bit access)

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::devices::bmc::{Bmc, MEMORY_WINDOW_BASES, connect_bmc};
use crate::devices::dmac::dmac::{Dmac, DmacSpinState, connect_dmac};
use crate::devices::expansion::{
    EXPANSION_BASE, EXPANSION_SLOT_COUNT, EXPANSION_SLOT_SIZE, ExpansionCard,
    ExpansionCardDebugInfo, ExpansionUiEvent, ExpansionUiItem,
};
use crate::devices::irqc::irqc::IrqController;
use crate::devices::pec::idc::idc::{IDC_BASE, IDC_DATA_ADDRESS, IDC_SIZE, Idc};
use crate::devices::pec::pec::PeCState;
use crate::devices::pec::timing::PeCTimingState;
use crate::devices::vdp::vdp::Vdp;

struct DeviceMap {
    region: BusRegion,
    device: Box<dyn Device>,
    /// `Device::is_plain_memory`, sampled at registration.
    plain: bool,
    /// `Device::is_timing_insensitive`, sampled at registration.
    timing_insensitive: bool,
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
    fn debug_read(&mut self, _addr: u32) -> Option<u8> {
        None
    }
    fn write(&mut self, addr: u32, value: u8);
    /// Big-endian `width`-byte read (1, 2 or 4) at `addr`, equal to reading the
    /// bytes in ascending order. Memory devices override it to skip the
    /// per-byte dispatch; the Bus only uses it inside one device.
    fn read_be(&mut self, addr: u32, width: u8) -> u32 {
        (0..u32::from(width)).fold(0, |value, beat| {
            (value << 8) | u32::from(self.read(addr + beat))
        })
    }
    /// Big-endian `width`-byte write, equal to writing the bytes in ascending
    /// order (see `read_be`).
    fn write_be(&mut self, addr: u32, width: u8, value: u32) {
        for beat in 0..u32::from(width) {
            self.write(
                addr + beat,
                (value >> ((u32::from(width) - 1 - beat) * 8)) as u8,
            );
        }
    }
    fn size(&self) -> u32;
    /// Return the generation token for the cache-coherence unit containing `addr`.
    /// It must change whenever any byte in that unit changes. `None` disables
    /// instruction caching for the address.
    fn instruction_cache_generation(&self, _addr: u32) -> Option<u64> {
        None
    }
    /// Size of the aligned address unit (relative to the device start) that
    /// shares one generation token. The default 1 never shares a token.
    fn instruction_cache_unit(&self) -> u32 {
        1
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
    /// Debugger snapshot; only WASM expansion cards return `Some`.
    fn expansion_card_debug_info(&self) -> Option<ExpansionCardDebugInfo> {
        None
    }
    fn reset_expansion_card_debug_stats(&mut self) {}
    /// Run a `gui` expansion card's `cpt32_ui`; other devices return `None`.
    fn expansion_card_ui(&mut self) -> Option<Vec<ExpansionUiItem>> {
        None
    }
    fn expansion_card_ui_event(&mut self, _id: i32, _event: ExpansionUiEvent) {}
    /// Side-effect-free storage: accesses never fault, never touch other
    /// devices and cannot change any IRQ source. The Bus then skips IRQ
    /// re-sampling after the access and may arbitrate a word only once.
    /// Constant for the device's lifetime (the Bus samples it once).
    fn is_plain_memory(&self) -> bool {
        false
    }
    /// Constant for the device's lifetime (the Bus samples it once).
    /// Accesses are unobservable relative to system-clock device progress:
    /// the device is not an IRQ source, never faults, its values do not
    /// depend on the system clock or ticked devices (DMA, IDC, PeC timers,
    /// expansion cards), and its accesses do not change those devices, IRQ
    /// sources or the CPU. Plain memory qualifies; frame-driven video/audio
    /// and host-side devices do too. The Bus then skips relaxed-device
    /// flushes and IRQ re-sampling for the access and may arbitrate a word
    /// once; JIT batches may perform the access natively.
    fn is_timing_insensitive(&self) -> bool {
        self.is_plain_memory()
    }
    /// Aligned big-endian word at `offset` when reading its four bytes has no
    /// side effect, cannot fault and cannot change an IRQ source (status and
    /// configuration registers). `None` keeps the ordinary byte reads.
    fn peek_u32_be(&self, _offset: u32) -> Option<u32> {
        None
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

/// DMA state at a spin-loop head (`Bus::dma_mark`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum DmaMark {
    /// No channel is busy.
    Idle,
    Active {
        bmc: Bmc,
        dmac: DmacSpinState,
        /// Bus operations completed by all channels (`Dmac::total_steps`);
        /// only one channel is busy whenever the loop may be skipped.
        steps: u64,
    },
}

/// 4 KiB pages of the DMA-reachable memory windows (16 MiB Main RAM, 4 MiB
/// VRAM, 1 MiB PCMRAM) for recording CPU accesses during a transfer.
const HAZARD_PAGE_SHIFT: u32 = 12;
const HAZARD_PAGES: usize = (0x0100_0000 + 0x0040_0000 + 0x0010_0000) >> HAZARD_PAGE_SHIFT;
const HAZARD_WORDS: usize = HAZARD_PAGES / 64;

/// Window (0..3) and page index of a DMA-reachable memory address.
fn hazard_page(addr: u32) -> Option<(u8, usize)> {
    let page = |base: u32, first: usize| first + ((addr - base) >> HAZARD_PAGE_SHIFT) as usize;
    match addr {
        0x0000_0000..=0x00FF_FFFF => Some((0, page(0, 0))),
        0x1000_0000..=0x103F_FFFF => Some((1, page(0x1000_0000, 0x1000))),
        0x1800_0000..=0x180F_FFFF => Some((2, page(0x1800_0000, 0x1400))),
        _ => None,
    }
}

/// Maximum idle device cycles a relaxed CPU run may defer between syncs.
/// Bounds host-input IRQ observation latency (256 cycles = 5.3 us at 48 MHz).
pub(crate) const RELAXED_DEVICE_WINDOW: u64 = 256;

pub struct Bus {
    devices: Vec<DeviceMap>,
    access_source: BusAccessSource,
    bus_error: bool,
    bmc: Option<Rc<RefCell<Bmc>>>,
    dmac: Option<Rc<RefCell<Dmac>>>,
    l1_cache: Option<Rc<RefCell<crate::cpu_cache::L1Cache>>>,
    idc: Option<Rc<RefCell<Idc>>>,
    pec_input_pending: Option<Arc<AtomicBool>>,
    pec_timing: Option<Rc<RefCell<PeCTimingState>>>,
    irq_controller: Option<Rc<RefCell<IrqController>>>,
    system_cycles: u64,
    cycle_clock: Rc<Cell<u64>>,
    expansion_devices: [Option<usize>; 8],
    has_expansion_cards: bool,
    relaxed_cycles: u64,
    relaxed_limit: u64,
    /// Device index that last answered an instruction-generation query.
    generation_device: Cell<usize>,
    /// Device index that last served a bus access.
    access_device: Cell<usize>,
    /// The device `access_device` held before it last changed: a DMA beat
    /// and the CPU's polling read alternate between two devices.
    previous_access_device: Cell<usize>,
    /// Bus accesses with possible effects: every write except one that stores
    /// the bytes plain memory already holds, every bus error, and any read
    /// that is not plain memory or a `peek_u32_be` register. A spinning CPU
    /// with no such access between two loop heads cannot change devices or
    /// memory. Same-value stores matter: llvm-src32 code spills every value
    /// to the stack, so a C polling loop stores on every iteration.
    effectful_accesses: u64,
    /// Test hook: when false, every device is treated as timing-sensitive.
    honor_timing_insensitive: bool,
    /// Bytes mapped at each `MEMORY_WINDOW_BASES` window (0 = unmapped),
    /// shared with the BMC's MAIN_RAM_SIZE/VRAM_SIZE/PCMRAM_SIZE registers.
    memory_sizes: Rc<Cell<[u32; 3]>>,
    /// BUSY mask of DMAC channels without an IDC endpoint (shared with the
    /// DMAC; 0 without one).
    dma_busy: Rc<Cell<u8>>,
    /// DMAC START counter (shared with the DMAC).
    dma_epoch: Rc<Cell<u32>>,
    /// Memory pages the CPU accessed (data or instruction fetch/generation
    /// query) while a transfer started at `hazard_epoch` was busy. The spin
    /// fast-forward never runs DMA writes in bulk over such a page, because the
    /// loop could then observe or overwrite them in a different order.
    hazard_epoch: Cell<u32>,
    hazard_pages: Box<[Cell<u64>; HAZARD_WORDS]>,
    /// Device index + 1 for every 64 KiB page that one device covers
    /// entirely, 0 where a page is unmapped or shared by smaller devices
    /// (those fall back to the scan). Rebuilt by `add_device`.
    page_devices: Box<[u16]>,
}

const DEVICE_PAGE_SHIFT: u32 = 16;

impl Bus {
    pub fn new() -> Self {
        Self {
            devices: Vec::new(),
            access_source: BusAccessSource::Cpu,
            bus_error: false,
            bmc: None,
            dmac: None,
            l1_cache: None,
            idc: None,
            pec_input_pending: None,
            pec_timing: None,
            irq_controller: None,
            system_cycles: 0,
            cycle_clock: Rc::new(Cell::new(0)),
            expansion_devices: [None; 8],
            has_expansion_cards: false,
            relaxed_cycles: 0,
            relaxed_limit: 0,
            generation_device: Cell::new(0),
            access_device: Cell::new(0),
            previous_access_device: Cell::new(0),
            effectful_accesses: 0,
            honor_timing_insensitive: true,
            memory_sizes: Rc::new(Cell::new([0; 3])),
            dma_busy: Rc::new(Cell::new(0)),
            dma_epoch: Rc::new(Cell::new(0)),
            hazard_epoch: Cell::new(0),
            hazard_pages: Box::new(std::array::from_fn(|_| Cell::new(0))),
            page_devices: vec![0; 1 << (32 - DEVICE_PAGE_SHIFT)].into_boxed_slice(),
        }
    }

    pub(crate) fn memory_sizes(&self) -> Rc<Cell<[u32; 3]>> {
        Rc::clone(&self.memory_sizes)
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
        self.dma_busy = dmac.borrow().busy_cell();
        self.dma_epoch = dmac.borrow().epoch_cell();
        self.dmac = Some(dmac);
    }

    /// Record a CPU access for the spin fast-forward's DMA hazard check.
    #[inline]
    fn note_cpu_access(&self, addr: u32, width: u32) {
        if self.dma_busy.get() != 0 {
            self.note_dma_hazard(addr);
            if width > 1 {
                self.note_dma_hazard(addr.wrapping_add(width - 1));
            }
        }
    }

    fn note_dma_hazard(&self, addr: u32) {
        let epoch = self.dma_epoch.get();
        if self.hazard_epoch.get() != epoch {
            for word in self.hazard_pages.iter() {
                word.set(0);
            }
            self.hazard_epoch.set(epoch);
        }
        if let Some((_, page)) = hazard_page(addr) {
            let word = &self.hazard_pages[page / 64];
            word.set(word.get() | 1 << (page % 64));
        }
    }

    /// Whether the CPU accessed any page of `lo..=hi` during this transfer.
    /// A range that leaves one memory window counts as accessed.
    fn dma_hazard(&self, lo: u32, hi: u32) -> bool {
        let (Some((lo_window, lo_page)), Some((hi_window, hi_page))) =
            (hazard_page(lo), hazard_page(hi))
        else {
            return true;
        };
        if lo_window != hi_window {
            return true;
        }
        if self.hazard_epoch.get() != self.dma_epoch.get() {
            return false;
        }
        (lo_page..=hi_page).any(|page| self.hazard_pages[page / 64].get() & (1 << (page % 64)) != 0)
    }

    /// `addr..addr+len` lies in one DMA memory window and one mapped
    /// timing-insensitive (never faulting) device.
    fn dma_memory_span(&self, addr: u32, len: u32) -> bool {
        let Some(end) = addr.checked_add(len) else {
            return false;
        };
        let window = (end <= 0x0100_0000)
            || (addr >= 0x1000_0000 && end <= 0x1040_0000)
            || (addr >= 0x1800_0000 && end <= 0x1810_0000);
        window
            && self.find_device_index(addr).is_some_and(|index| {
                let mapped = &self.devices[index];
                mapped.timing_insensitive
                    && mapped.region.size - (addr - mapped.region.start) >= len
            })
    }

    /// DMA state compared at spin-loop heads. Transfers with an IDC endpoint
    /// alone count as `Idle`: they keep the quiet-window rule (`quiet_cycles`
    /// is 0 while they request the bus).
    #[inline]
    pub(crate) fn dma_mark(&self) -> DmaMark {
        if self.dma_busy.get() == 0 {
            return DmaMark::Idle;
        }
        self.active_dma_mark()
    }

    fn active_dma_mark(&self) -> DmaMark {
        let (Some(bmc), Some(dmac)) = (&self.bmc, &self.dmac) else {
            return DmaMark::Idle;
        };
        let (dmac, steps) = dmac.borrow().spin_state();
        DmaMark::Active {
            bmc: *bmc.borrow(),
            dmac,
            steps,
        }
    }

    /// Whether DMA allows a spin loop observed from `before` to `now` to repeat:
    /// no transfer, a stalled one, or one whose beats the fast-forward can run
    /// in bulk with arbitration and channel phase back where they were.
    #[inline]
    pub(crate) fn dma_repeats(&self, before: &DmaMark, now: &DmaMark) -> bool {
        if matches!((before, now), (DmaMark::Idle, DmaMark::Idle)) {
            return true;
        }
        self.active_dma_repeats(before, now)
    }

    fn active_dma_repeats(&self, before: &DmaMark, now: &DmaMark) -> bool {
        match (before, now) {
            (DmaMark::Idle, DmaMark::Idle) => true,
            (
                DmaMark::Active {
                    bmc: bmc0,
                    dmac: dmac0,
                    steps: steps0,
                },
                DmaMark::Active {
                    bmc: bmc1,
                    dmac: dmac1,
                    steps: steps1,
                },
            ) => {
                bmc0 == bmc1
                    && dmac0 == dmac1
                    && (steps0 == steps1 || self.dma_spin_channel().is_some())
            }
            _ => false,
        }
    }

    /// Whether a loop whose CPU state repeated but whose DMA state did not may
    /// keep its older head: a bulk transfer's beat phase realigns after a few
    /// iterations.
    pub(crate) fn dma_may_realign(&self, before: &DmaMark, now: &DmaMark) -> bool {
        matches!(
            (before, now),
            (DmaMark::Active { .. }, DmaMark::Active { .. })
        ) && self.dma_spin_channel().is_some()
    }

    fn dma_spin_channel(&self) -> Option<usize> {
        self.dmac.as_ref()?.borrow().spin_channel()
    }

    /// Whole further iterations (at most `max`) of a spin loop that repeated
    /// from `before` to `now` (`dma_repeats`) that keep every DMA beat inside
    /// memory the loop does not touch and stop before the transfer ends.
    #[inline]
    pub(crate) fn dma_spin_iterations(&self, before: &DmaMark, now: &DmaMark, max: u128) -> u128 {
        if matches!(now, DmaMark::Idle) {
            return max;
        }
        self.active_dma_spin_iterations(before, now, max)
    }

    fn active_dma_spin_iterations(&self, before: &DmaMark, now: &DmaMark, max: u128) -> u128 {
        let (DmaMark::Active { steps: steps0, .. }, DmaMark::Active { steps: steps1, .. }) =
            (before, now)
        else {
            return max;
        };
        let Some(channel) = self.dma_spin_channel() else {
            // `dma_repeats` allowed this only for a transfer that made no progress.
            return max;
        };
        let delta = steps1 - steps0;
        if delta == 0 {
            return max;
        }
        let want = (max.min(1 << 32) as u64).saturating_mul(delta);
        let reach = self
            .dmac
            .as_ref()
            .expect("spin channel implies a DMAC")
            .borrow()
            .spin_reach(channel, want, |addr, len| self.dma_memory_span(addr, len));
        let iterations = u128::from(reach.steps / delta).min(max);
        if iterations == 0 {
            return 0;
        }
        let Some((lo, hi)) = reach.dst else {
            return 0;
        };
        // Pipeline L1: a DMA write invalidates the line index the loop may be
        // fetching from, which changes hit/miss counts mid-iteration.
        if self.dma_hazard(lo, hi) || (self.l1_cache.is_some() && lo < 0x0100_0000) {
            return 0;
        }
        iterations
    }

    /// Advance devices through `cycles` skipped spin-loop cycles, running the
    /// DMA beats those `iterations` would have performed.
    pub(crate) fn advance_spin(
        &mut self,
        cycles: u64,
        before: &DmaMark,
        now: &DmaMark,
        iterations: u128,
    ) {
        self.advance_quiet_cycles(cycles);
        let (DmaMark::Active { steps: steps0, .. }, DmaMark::Active { steps: steps1, .. }) =
            (before, now)
        else {
            return;
        };
        let Some(channel) = self.dma_spin_channel() else {
            return;
        };
        let steps = u128::from(steps1 - steps0) * iterations;
        self.run_dma_steps(channel, steps.min(u128::from(u64::MAX)) as u64);
    }

    /// Run `steps` bus operations of `channel` back to back, as the per-cycle
    /// ticks would, without arbitration (the loop's BMC state repeats) or
    /// IRQ changes (`spin_reach` stops before the final operation).
    fn run_dma_steps(&mut self, channel: usize, steps: u64) {
        let dmac = Rc::clone(self.dmac.as_ref().expect("spin channel implies a DMAC"));
        // Memory beats never reach the DMAC register block, so the borrow can
        // span the loop.
        let mut dmac = dmac.borrow_mut();
        for _ in 0..steps {
            let op = dmac.next_op(channel).expect("channel stays busy");
            let value = self
                .execute_dma_op(op, channel)
                .expect("spin_reach covers only mapped memory");
            dmac.complete_op(channel, if op.write { op.value } else { value });
        }
    }

    pub(crate) fn attach_l1_cache(&mut self, cache: Rc<RefCell<crate::cpu_cache::L1Cache>>) {
        self.l1_cache = Some(cache);
    }

    pub(crate) fn attach_idc(&mut self, idc: Rc<RefCell<Idc>>) {
        if let Some(dmac) = &self.dmac {
            dmac.borrow_mut().attach_idc(Rc::clone(&idc));
        }
        self.idc = Some(idc);
    }
    pub(crate) fn attach_pec_input(&mut self, state: Arc<Mutex<PeCState>>) {
        let pending = state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pending_flag();
        self.pec_input_pending = Some(pending);
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
            // Every deadline is at least `now + 1`, so it cannot split a single cycle.
            if target - now > 1 {
                if let Some(until_irq) = self.cycles_until_pec_irq() {
                    next_cycle = next_cycle.min(now.saturating_add(until_irq.max(1)));
                }
                // A card due now (queued network frames) must not hide a later deadline.
                if let Some(deadline) = self.next_expansion_event_cycle_after(now) {
                    next_cycle = next_cycle.min(deadline);
                }
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

    fn next_expansion_event_cycle_after(&self, now: u64) -> Option<u64> {
        if !self.has_expansion_cards {
            return None;
        }
        self.expansion_devices
            .iter()
            .flatten()
            .filter_map(|&index| self.devices[index].device.next_event_cycle())
            .filter(|&cycle| cycle > now)
            .min()
    }

    fn run_due_expansion_events(&mut self) -> bool {
        let cycle = self.system_cycles;
        if !self.has_expansion_cards {
            return false;
        }
        let mut event_ran = false;
        // A card event can queue network frames for a card in an earlier slot,
        // which is then due at this same cycle. Bound the extra passes.
        for _ in 0..8 {
            let mut pass_ran = false;
            for slot in 0..usize::from(EXPANSION_SLOT_COUNT) {
                if let Some(index) = self.expansion_devices[slot] {
                    let device = &mut self.devices[index].device;
                    if device
                        .next_event_cycle()
                        .is_some_and(|deadline| deadline <= cycle)
                    {
                        device.run_scheduled_event(cycle);
                        pass_ran = true;
                    }
                }
            }
            event_ran |= pass_ran;
            if !pass_ran {
                break;
            }
        }
        event_ran
    }

    /// Debugger snapshots of installed expansion cards in slot order.
    pub fn expansion_card_debug_info(&self) -> Vec<ExpansionCardDebugInfo> {
        self.expansion_devices
            .iter()
            .flatten()
            .filter_map(|&index| self.devices[index].device.expansion_card_debug_info())
            .collect()
    }

    pub fn reset_expansion_card_debug_stats(&mut self) {
        for index in self.expansion_devices.iter().flatten().copied() {
            self.devices[index]
                .device
                .reset_expansion_card_debug_stats();
        }
    }

    /// Run the panel of the `gui` card in `slot` (`doc/spec_WASM_expansion_card.md`
    /// §3.4). The call may change card state, so IRQs are re-sampled afterwards.
    pub fn expansion_card_ui(&mut self, slot: u8) -> Option<Vec<ExpansionUiItem>> {
        let index = (*self.expansion_devices.get(usize::from(slot))?)?;
        let items = self.devices[index].device.expansion_card_ui();
        self.refresh_device_irqs();
        items
    }

    /// Queue user input for widget `id` of the card in `slot`.
    pub fn expansion_card_ui_event(&mut self, slot: u8, id: i32, event: ExpansionUiEvent) {
        if let Some(Some(index)) = self.expansion_devices.get(usize::from(slot)).copied() {
            self.devices[index]
                .device
                .expansion_card_ui_event(id, event);
        }
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
        if self.tick_idle_dma_devices()
            || (self.dma_busy.get() != 0 && self.tick_memory_dma_devices())
        {
            return;
        }
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

    /// Waiting form of the rest of `tick_devices`. When no DMA channel can
    /// request the bus and the IDC tick cannot reach a deadline or abort,
    /// there is no abort boundary, IDC failure, IRQ change or DMA beat, so only
    /// the IDC clock and BMC request latch move. Covers idle devices and a
    /// polled transfer waiting for its next sector.
    fn tick_idle_dma_devices(&mut self) -> bool {
        let (Some(bmc), Some(dmac)) = (&self.bmc, &self.dmac) else {
            return false;
        };
        if self
            .idc
            .as_ref()
            .is_some_and(|idc| idc.borrow().cycles_without_event() == 0)
        {
            return false;
        }
        // The IDC tick leaves its DMA readiness unchanged, so this is the
        // mask the ordinary path would service after it.
        if dmac.borrow().request_mask() != 0 {
            return false;
        }
        if let Some(idc) = &self.idc {
            idc.borrow_mut().advance_idle_cycles(1);
        }
        bmc.borrow_mut().grant(0);
        true
    }

    /// Rest of `tick_devices` while only memory transfers are busy and the
    /// IDC tick cannot reach a deadline or abort: there is no IDC abort
    /// boundary, IDC failure or IRQ change before the DMA beat, so only the
    /// IDC clock moves and the beat is serviced.
    fn tick_memory_dma_devices(&mut self) -> bool {
        let Some(dmac) = &self.dmac else {
            return false;
        };
        if !dmac.borrow().busy_without_idc_endpoint() {
            return false;
        }
        if let Some(idc) = &self.idc {
            let mut idc = idc.borrow_mut();
            if idc.cycles_without_event() == 0 {
                return false;
            }
            idc.advance_idle_cycles(1);
        }
        self.service_dma_request(false);
        true
    }

    /// Experimental idle-device batching. Active transfers retain the ordinary
    /// cycle-by-cycle path. Bus accesses flush the batch. Expansion-card
    /// deadlines end the window like timer deadlines (`cycles_until_next_event`);
    /// network frames that arrive during a batch are delivered when it flushes.
    pub(crate) fn tick_relaxed_devices(&mut self) {
        if self.relaxed_cycles == 0 {
            if self.has_pending_dma()
                || self
                    .dmac
                    .as_ref()
                    .is_some_and(|dmac| !dmac.borrow().can_batch_idle())
            {
                self.tick_devices();
                return;
            }
            self.relaxed_limit = self
                .cycles_until_next_event()
                .unwrap_or(RELAXED_DEVICE_WINDOW)
                .clamp(1, RELAXED_DEVICE_WINDOW);
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
        if self.has_pending_dma()
            || self
                .dmac
                .as_ref()
                .is_some_and(|dmac| !dmac.borrow().can_batch_idle())
        {
            return 0;
        }
        self.cycles_until_next_event()
            .unwrap_or(RELAXED_DEVICE_WINDOW)
            .clamp(1, RELAXED_DEVICE_WINDOW)
            - 1
    }

    #[cfg(feature = "jit")]
    pub(crate) fn defer_native_idle_cycles(&mut self, cycles: u64) {
        debug_assert!(cycles <= self.native_idle_budget());
        if self.relaxed_cycles == 0 {
            self.relaxed_limit = self
                .cycles_until_next_event()
                .unwrap_or(RELAXED_DEVICE_WINDOW)
                .clamp(1, RELAXED_DEVICE_WINDOW);
        }
        self.relaxed_cycles += cycles;
    }

    #[cfg(feature = "jit")]
    pub(crate) fn has_deferred_cycles(&self) -> bool {
        self.relaxed_cycles != 0
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

    pub(crate) fn effectful_accesses(&self) -> u64 {
        self.effectful_accesses
    }

    /// CPU cycles from now during which per-cycle device ticks are known to
    /// change nothing but clocks: no DMA request, IDC deadline, timer/RTC IRQ
    /// or expansion event. Pure reads observe constant values meanwhile.
    /// Host input and network frames for expansion cards arrive asynchronously
    /// and are not covered: they are taken at the end of the window.
    /// A transfer the spin fast-forward can run in bulk (`Dmac::spin_channel`)
    /// does not end the window; its beats are accounted by `advance_spin`.
    pub(crate) fn quiet_cycles(&self) -> u64 {
        if self.dmac.as_ref().is_some_and(|dmac| {
            let dmac = dmac.borrow();
            dmac.request_mask() != 0 && dmac.spin_channel().is_none()
        }) {
            return 0;
        }
        let mut quiet = self.idc.as_ref().map_or(u64::MAX, |idc| {
            idc.borrow()
                .cycles_without_event()
                .min(u128::from(u64::MAX)) as u64
        });
        if let Some(until) = self.cycles_until_next_event() {
            // The event is raised by the tick that reaches it.
            quiet = quiet.min(until.saturating_sub(1));
        }
        // Deferred relaxed cycles are already owed to the devices.
        quiet.saturating_sub(self.relaxed_cycles)
    }

    /// Advance devices other than an active DMA transfer through cycles that
    /// `quiet_cycles` proved eventless, equivalent to that many `tick_devices`
    /// calls with no bus access. Spin loops use `advance_spin`.
    fn advance_quiet_cycles(&mut self, cycles: u64) {
        debug_assert!(cycles <= self.quiet_cycles());
        self.flush_relaxed_devices();
        if let Some(idc) = &self.idc {
            idc.borrow_mut().advance_idle_cycles(cycles);
        }
        self.advance_system_cycles(cycles);
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
        let (op, irq_before, idc_channel) = {
            let dmac = dmac.borrow();
            (
                dmac.next_op(channel),
                dmac.irq_pending(),
                dmac.uses_idc_endpoint(channel),
            )
        };
        let result = match op {
            Some(op) => self.execute_dma_op(op, channel),
            None => None,
        };
        let completed = {
            let mut dmac = dmac.borrow_mut();
            match (op, result) {
                (Some(op), Some(value)) => {
                    dmac.complete_op(channel, if op.write { op.value } else { value });
                    true
                }
                (Some(_), None) => {
                    dmac.fail(channel, 2);
                    false
                }
                (None, _) => false,
            }
        };
        bmc.borrow_mut().release();
        // A completed Main RAM/VRAM/PCMRAM beat of a channel without an IDC
        // endpoint changes no IRQ source except the DMAC's own. (IDC data and
        // expansion-card beats may, and so may the memory beat that finishes
        // an IDC command.)
        let memory_beat = op.is_some_and(|op| {
            completed
                && op.address != IDC_DATA_ADDRESS
                && !Self::is_expansion_address(op.address)
                && !idc_channel
        });
        if !memory_beat || dmac.borrow().irq_pending() != irq_before {
            self.refresh_device_irqs();
        }
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

    fn is_expansion_address(address: u32) -> bool {
        let aperture_end = EXPANSION_BASE + u32::from(EXPANSION_SLOT_COUNT) * EXPANSION_SLOT_SIZE;
        (EXPANSION_BASE..aperture_end).contains(&address)
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
        let memory = (op.address < 0x0100_0000 && end <= 0x0100_0000)
            || (op.address >= 0x1000_0000 && end <= 0x1040_0000)
            || (op.address >= 0x1800_0000 && end <= 0x1810_0000);
        // BitBlt beats get no MMIO endpoint exceptions (spec_BMC_DMAC §2.4).
        let allowed = memory
            || (!op.memory_only
                && ((op.write && op.width == 4 && op.address == 0x8003_002C)
                    || expansion_slot.is_some()));
        if !allowed || op.address % u32::from(op.width) != 0 {
            return None;
        }
        if let Some(slot) = expansion_slot {
            let index = self.expansion_devices[slot]?;
            if self.devices[index].device.is_faulted() {
                return None;
            }
        }

        if memory {
            if let Some(index) = self.find_device_index(op.address) {
                let mapped = &mut self.devices[index];
                let offset = op.address - mapped.region.start;
                // Timing-insensitive devices never fault, so the beat is one call.
                if mapped.timing_insensitive && mapped.region.size - offset >= u32::from(op.width) {
                    if !op.write {
                        return Some(mapped.device.read_be(offset, op.width));
                    }
                    mapped.device.write_be(offset, op.width, op.value);
                    // RTL snoops each accepted DMAC write to Main RAM.
                    if op.address < 0x0100_0000 {
                        if let Some(cache) = &self.l1_cache {
                            cache
                                .borrow_mut()
                                .external_write(op.address, u32::from(op.width));
                        }
                    }
                    return Some(op.value);
                }
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
                // RTL snoops each accepted DMAC write to Main RAM. Invalidate
                // even if a later byte of this DMA beat fails.
                if op.address + u32::from(byte_index) < 0x0100_0000 {
                    if let Some(cache) = &self.l1_cache {
                        cache
                            .borrow_mut()
                            .external_write(op.address + u32::from(byte_index), 1);
                    }
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
        let pec_pending = self
            .pec_input_pending
            .as_ref()
            .is_some_and(|pending| pending.load(Ordering::Acquire));
        let pec_timing_pending = self
            .pec_timing
            .as_ref()
            .is_some_and(|state| state.borrow().irq_asserted());
        let mut levels = u16::from(dmac_pending) << 1
            | u16::from(idc_pending) << 2
            | u16::from(pec_pending) << 3
            | u16::from(pec_timing_pending) << 4;
        if self.has_expansion_cards {
            for slot in 0..usize::from(EXPANSION_SLOT_COUNT) {
                if self.expansion_devices[slot]
                    .is_some_and(|index| self.devices[index].device.irq_asserted())
                {
                    levels |= 1 << (slot + 5);
                }
            }
        }
        if let Some(irq) = &self.irq_controller {
            // Sources 1..=12: DMAC, IDC, PeC input, PeC timing, slots 0..=7.
            irq.borrow_mut().set_source_levels(0x1FFE, levels);
        }
    }

    fn arbitrate_cpu_access(&mut self) {
        if self.access_source != BusAccessSource::Cpu {
            return;
        }
        if let (Some(bmc), Some(dmac)) = (&self.bmc, &self.dmac) {
            if dmac.borrow().request_mask() == 0 {
                // Same BMC state as the CPU-only grant/release/claim sequence.
                let mut bmc = bmc.borrow_mut();
                bmc.grant(1);
                bmc.claim_cpu();
                return;
            }
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
        self.effectful_accesses += 1;
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

        if let Some(window) = MEMORY_WINDOW_BASES.iter().position(|&base| base == addr) {
            let mut sizes = self.memory_sizes.get();
            sizes[window] = size;
            self.memory_sizes.set(sizes);
        }

        self.devices.push(DeviceMap {
            region: BusRegion {
                start: addr,
                size,
                name,
            },
            plain: device.is_plain_memory(),
            timing_insensitive: device.is_timing_insensitive(),
            device,
        });
        let index = self.devices.len();
        if let Ok(entry) = u16::try_from(index) {
            let first = addr.div_ceil(1 << DEVICE_PAGE_SHIFT) as usize;
            let end = (u64::from(new_end) >> DEVICE_PAGE_SHIFT) as usize;
            for page in first..end {
                self.page_devices[page] = entry;
            }
        }
    }

    /// Iterate over mapped ranges in registration order without accessing devices.
    pub fn memory_map(&self) -> impl Iterator<Item = BusRegion> + '_ {
        self.devices.iter().map(|mapped| mapped.region)
    }

    /// Regions never overlap, so the remembered device gives the same answer
    /// as a full scan whenever it contains `addr`.
    #[inline]
    fn find_device_index(&self, addr: u32) -> Option<usize> {
        let contains = |mapped: &DeviceMap| {
            addr.checked_sub(mapped.region.start)
                .is_some_and(|offset| offset < mapped.region.size)
        };
        let remembered = self.access_device.get();
        if self.devices.get(remembered).is_some_and(contains) {
            return Some(remembered);
        }
        let previous = self.previous_access_device.get();
        let index = if self.devices.get(previous).is_some_and(contains) {
            previous
        } else {
            let paged = self.page_devices[(addr >> DEVICE_PAGE_SHIFT) as usize];
            if paged != 0 {
                usize::from(paged) - 1
            } else {
                self.devices.iter().position(contains)?
            }
        };
        self.previous_access_device.set(remembered);
        self.access_device.set(index);
        Some(index)
    }

    /// Whether `addr..addr+width` lies in one timing-insensitive device, so a
    /// CPU access there needs no device synchronization.
    #[cfg(feature = "jit")]
    pub(crate) fn is_timing_insensitive_range(&self, addr: u32, width: u32) -> bool {
        if !self.honor_timing_insensitive {
            return self.find_device_index(addr).is_some_and(|index| {
                let mapped = &self.devices[index];
                mapped.plain && mapped.region.size - (addr - mapped.region.start) >= width
            });
        }
        self.find_device_index(addr).is_some_and(|index| {
            let mapped = &self.devices[index];
            mapped.timing_insensitive && mapped.region.size - (addr - mapped.region.start) >= width
        })
    }

    #[inline]
    fn is_timing_insensitive_index(&self, index: Option<usize>) -> bool {
        self.honor_timing_insensitive
            && index.is_some_and(|index| self.devices[index].timing_insensitive)
    }

    /// Test hook: disable every optimization based on `is_timing_insensitive`.
    #[doc(hidden)]
    pub fn set_timing_insensitive_fast_paths(&mut self, enabled: bool) {
        self.honor_timing_insensitive = enabled;
    }

    /// Side-effect-free register word (e.g. a polled DMAC/IDC status) read
    /// with one arbitration: like plain memory, no DMA request can appear
    /// between the four byte beats and the read cannot change an IRQ source.
    fn read_pure_register_u32(&mut self, addr: u32) -> Option<u32> {
        if self.access_source != BusAccessSource::Cpu {
            return None;
        }
        let index = self.find_device_index(addr)?;
        let mapped = &self.devices[index];
        let offset = addr - mapped.region.start;
        if mapped.region.size - offset < 4 {
            return None;
        }
        self.flush_relaxed_devices();
        if self
            .dmac
            .as_ref()
            .is_some_and(|dmac| dmac.borrow().request_mask() != 0)
        {
            return None;
        }
        // Pure, so reading before the grant observes the same value.
        let value = self.devices[index].device.peek_u32_be(offset)?;
        self.arbitrate_cpu_access();
        self.release_cpu_access();
        Some(value)
    }

    /// Timing-insensitive device holding `addr..addr+width` when a CPU access
    /// can use the single-arbitration path: no DMA master can request the bus
    /// between beats, and such a device can neither raise a request nor
    /// change an IRQ source. No relaxed flush is needed (see the flag).
    #[inline]
    fn timing_insensitive_span(&mut self, addr: u32, width: u32) -> Option<(usize, u32)> {
        if self.access_source != BusAccessSource::Cpu {
            return None;
        }
        let index = self.find_device_index(addr)?;
        if !self.is_timing_insensitive_index(Some(index)) {
            return None;
        }
        let mapped = &self.devices[index];
        let offset = addr - mapped.region.start;
        if mapped.region.size - offset < width {
            return None;
        }
        if self
            .dmac
            .as_ref()
            .is_some_and(|dmac| dmac.borrow().request_mask() != 0)
        {
            return None;
        }
        Some((index, offset))
    }

    pub fn find_device(&mut self, addr: u32) -> Option<(&mut dyn Device, u32)> {
        let index = self.find_device_index(addr)?;
        let mapped = &mut self.devices[index];
        Some((&mut *mapped.device, addr - mapped.region.start))
    }

    pub fn find_device_mut(&mut self, addr: u32) -> Option<(&mut dyn Device, u32)> {
        self.find_device(addr)
    }

    /// Return the executable-memory generation at an address without a bus read.
    pub(crate) fn instruction_cache_generation(&self, addr: u32) -> Option<u64> {
        self.note_cpu_access(addr, 1);
        let (mapped, offset) = self.instruction_device(addr)?;
        mapped.device.instruction_cache_generation(offset)
    }

    /// Generation at `addr` plus the inclusive address range sharing it. A
    /// caller may reuse the token for that range until memory can change.
    pub(crate) fn instruction_cache_generation_span(&self, addr: u32) -> Option<(u64, u32, u32)> {
        self.note_cpu_access(addr, 1);
        let (mapped, offset) = self.instruction_device(addr)?;
        let generation = mapped.device.instruction_cache_generation(offset)?;
        let unit = mapped.device.instruction_cache_unit().max(1);
        let unit_start = offset - offset % unit;
        let unit_last = unit_start
            .saturating_add(unit - 1)
            .min(mapped.region.size - 1);
        Some((
            generation,
            mapped.region.start + unit_start,
            mapped.region.start + unit_last,
        ))
    }

    /// Mapped device for instruction metadata. Regions never overlap, so the
    /// remembered device is the same answer a full scan would give.
    fn instruction_device(&self, addr: u32) -> Option<(&DeviceMap, u32)> {
        let contains = |mapped: &DeviceMap| {
            addr.checked_sub(mapped.region.start)
                .filter(|&offset| offset < mapped.region.size)
        };
        if let Some(mapped) = self.devices.get(self.generation_device.get()) {
            if let Some(offset) = contains(mapped) {
                return Some((mapped, offset));
            }
        }
        let paged = self.page_devices[(addr >> DEVICE_PAGE_SHIFT) as usize];
        let index = if paged != 0 {
            usize::from(paged) - 1
        } else {
            self.devices
                .iter()
                .position(|mapped| contains(mapped).is_some())?
        };
        self.generation_device.set(index);
        let mapped = &self.devices[index];
        Some((mapped, addr - mapped.region.start))
    }

    #[cfg(feature = "jit")]
    pub(crate) fn peek_instruction_u8(&self, addr: u32) -> Option<u8> {
        let (mapped, offset) = self.instruction_device(addr)?;
        mapped.device.peek_instruction_u8(offset)
    }

    /// Enable instruction-generation tracking for cache-aware CPU modes.
    pub(crate) fn enable_instruction_cache_tracking(&mut self) {
        for mapped in &mut self.devices {
            mapped.device.enable_instruction_cache_tracking();
        }
    }

    pub fn read_u8(&mut self, addr: u32) -> u8 {
        self.note_cpu_access(addr, 1);
        let index = self.find_device_index(addr);
        let insensitive = self.is_timing_insensitive_index(index);
        if !insensitive {
            self.flush_relaxed_devices();
        }
        self.arbitrate_cpu_access();
        if let Some(index) = index {
            let mapped = &mut self.devices[index];
            let offset = addr - mapped.region.start;
            let device = &mut mapped.device;
            let value = device.read(offset);
            let pure = mapped.plain || device.peek_u32_be(offset & !3).is_some();
            if device.is_faulted() {
                self.latch_bus_error();
            }
            if !pure {
                self.effectful_accesses += 1;
            }
            if !insensitive {
                self.refresh_device_irqs();
            }
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
        self.note_cpu_access(addr, 1);
        let index = self.find_device_index(addr);
        let insensitive = self.is_timing_insensitive_index(index);
        if !insensitive {
            self.flush_relaxed_devices();
        }
        self.arbitrate_cpu_access();
        if let Some(index) = index {
            let mapped = &mut self.devices[index];
            let offset = addr - mapped.region.start;
            let device = &mut mapped.device;
            if !(mapped.plain && device.read(offset) == value) {
                self.effectful_accesses += 1;
            }
            device.write(offset, value);
            if device.is_faulted() {
                self.latch_bus_error();
            }
            if !insensitive {
                self.refresh_device_irqs();
            }
            self.release_cpu_access();
            return;
        }
        self.latch_bus_error();
        self.release_cpu_access();
    }

    /// Whether plain memory at `offset` already holds `value` (big-endian),
    /// so storing it changes nothing. Plain reads have no side effects.
    fn holds_u32_be(device: &mut dyn Device, offset: u32, value: u32) -> bool {
        device.read_be(offset, 4) == value
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
        self.note_cpu_access(addr, 4);
        if Self::invalid_idc_u32_access(addr) {
            self.flush_relaxed_devices();
            self.latch_bus_error();
            return 0;
        }
        if let Some((index, offset)) = self.timing_insensitive_span(addr, 4) {
            self.arbitrate_cpu_access();
            let mapped = &mut self.devices[index];
            let device = &mut mapped.device;
            let value = device.read_be(offset, 4);
            debug_assert!(
                !device.is_faulted(),
                "timing-insensitive devices never fault"
            );
            if !mapped.plain && device.peek_u32_be(offset).is_none() {
                self.effectful_accesses += 1;
            }
            self.release_cpu_access();
            return value;
        }
        if let Some(value) = self.read_pure_register_u32(addr) {
            return value;
        }
        let b0 = self.read_u8(addr) as u32;
        let b1 = self.read_u8(addr.wrapping_add(1)) as u32;
        let b2 = self.read_u8(addr.wrapping_add(2)) as u32;
        let b3 = self.read_u8(addr.wrapping_add(3)) as u32;
        (b0 << 24) | (b1 << 16) | (b2 << 8) | b3
    }

    pub fn write_u32_be(&mut self, addr: u32, value: u32) {
        self.note_cpu_access(addr, 4);
        if Self::invalid_idc_u32_access(addr) {
            self.flush_relaxed_devices();
            self.latch_bus_error();
            return;
        }
        if let Some((index, offset)) = self.timing_insensitive_span(addr, 4) {
            self.arbitrate_cpu_access();
            let mapped = &mut self.devices[index];
            let device = &mut mapped.device;
            if !(mapped.plain && Self::holds_u32_be(device.as_mut(), offset, value)) {
                self.effectful_accesses += 1;
            }
            device.write_be(offset, 4, value);
            debug_assert!(
                !device.is_faulted(),
                "timing-insensitive devices never fault"
            );
            self.release_cpu_access();
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
    connect_devices_with_memory(
        bus,
        crate::devices::ram::DEFAULT_RAM_SIZE,
        crate::devices::vdp::vdp::DEFAULT_VDP_VRAM_SIZE,
    )
}

/// `connect_devices_with_vdp` with configured Main RAM and VRAM sizes.
pub fn connect_devices_with_memory(
    bus: &mut Bus,
    ram_size: usize,
    vram_size: u32,
) -> Rc<RefCell<Vdp>> {
    connect_devices_with_sgu_version(bus, ram_size, vram_size, Default::default())
}

/// `connect_devices_with_memory` with a selected SGU generation.
pub fn connect_devices_with_sgu_version(
    bus: &mut Bus,
    ram_size: usize,
    vram_size: u32,
    sgu_version: crate::devices::sgu::s3w2::SguVersion,
) -> Rc<RefCell<Vdp>> {
    crate::devices::ram::connect_ram_with_size(bus, ram_size);
    connect_bmc_dmac(bus);
    let _sgu = crate::devices::sgu::sgu::connect_sgu_with_version(bus, sgu_version);
    crate::devices::vdp::vdp::connect_vdp_with_vram_size(bus, vram_size)
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
    fn relaxed_ticks_batch_until_sensitive_access_or_budget_flush() {
        let mut bus = Bus::new();
        crate::devices::ram::connect_ram(&mut bus);
        connect_bmc_dmac(&mut bus);
        let window = RELAXED_DEVICE_WINDOW;
        for _ in 0..window - 1 {
            bus.tick_relaxed_devices();
        }
        assert_eq!(bus.system_cycles(), 0);
        bus.tick_relaxed_devices();
        assert_eq!(bus.system_cycles(), window);
        // Timing-insensitive RAM accesses (byte and word) keep deferring.
        bus.tick_relaxed_devices();
        bus.write_u8(0, 7);
        bus.write_u32_be(4, 0x1234_5678);
        assert_eq!(bus.system_cycles(), window);
        bus.tick_relaxed_devices();
        assert_eq!(bus.read_u8(0), 7);
        assert_eq!(bus.read_u32_be(4), 0x1234_5678);
        assert_eq!(bus.system_cycles(), window);
        // A sensitive MMIO access (DMAC register) observes current device time.
        bus.read_u8(crate::devices::dmac::dmac::DMAC_BASE);
        assert_eq!(bus.system_cycles(), window + 2);
        bus.tick_relaxed_devices();
        bus.flush_relaxed_devices();
        assert_eq!(bus.system_cycles(), window + 3);
    }

    #[test]
    fn relaxed_ticks_stop_at_timer_deadline() {
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
    }

    /// Expansion card stand-in: one scheduled event that asserts its IRQ.
    struct DeadlineCard {
        deadline: Rc<std::cell::Cell<Option<u64>>>,
        ran_at: Rc<std::cell::Cell<Option<u64>>>,
    }

    impl Device for DeadlineCard {
        fn read(&mut self, _addr: u32) -> u8 {
            0
        }
        fn write(&mut self, _addr: u32, _value: u8) {}
        fn size(&self) -> u32 {
            EXPANSION_SLOT_SIZE
        }
        fn next_event_cycle(&self) -> Option<u64> {
            self.deadline.get()
        }
        fn run_scheduled_event(&mut self, cycle: u64) {
            self.deadline.set(None);
            self.ran_at.set(Some(cycle));
        }
        fn irq_asserted(&self) -> bool {
            self.ran_at.get().is_some()
        }
    }

    fn install_deadline_card(
        bus: &mut Bus,
        deadline: u64,
    ) -> (
        Rc<std::cell::Cell<Option<u64>>>,
        Rc<std::cell::Cell<Option<u64>>>,
    ) {
        let deadline = Rc::new(std::cell::Cell::new(Some(deadline)));
        let ran_at = Rc::new(std::cell::Cell::new(None));
        bus.add_device(
            EXPANSION_BASE,
            "test card",
            Box::new(DeadlineCard {
                deadline: Rc::clone(&deadline),
                ran_at: Rc::clone(&ran_at),
            }),
        );
        bus.expansion_devices[0] = Some(bus.devices.len() - 1);
        bus.has_expansion_cards = true;
        (deadline, ran_at)
    }

    #[test]
    fn installed_cards_keep_relaxed_batching_until_their_deadline() {
        let mut bus = Bus::new();
        let irq = crate::devices::irqc::irqc::connect_irqc(&mut bus);
        bus.attach_irq_controller(Rc::clone(&irq));
        let (deadline, ran_at) = install_deadline_card(&mut bus, 100);
        // Batched up to the card deadline, which ends the window on time.
        for _ in 0..99 {
            bus.tick_relaxed_devices();
        }
        assert_eq!(bus.system_cycles(), 0);
        bus.tick_relaxed_devices();
        assert_eq!(bus.system_cycles(), 100);
        assert_eq!(ran_at.get(), Some(100));
        // Slot 0 is IRQ source 5.
        assert_ne!(irq.borrow().pending() & (1 << 5), 0);
        // With nothing scheduled the window is the default batch.
        assert_eq!(deadline.get(), None);
        assert_eq!(bus.quiet_cycles(), u64::MAX);
        for _ in 0..RELAXED_DEVICE_WINDOW {
            bus.tick_relaxed_devices();
        }
        assert_eq!(bus.system_cycles(), 100 + RELAXED_DEVICE_WINDOW);
        // A card access observes current device time.
        bus.tick_relaxed_devices();
        bus.read_u8(EXPANSION_BASE);
        assert_eq!(bus.system_cycles(), 101 + RELAXED_DEVICE_WINDOW);
    }

    #[test]
    fn quiet_window_ends_before_a_card_deadline() {
        let mut bus = Bus::new();
        let (deadline, ran_at) = install_deadline_card(&mut bus, 50);
        assert_eq!(bus.quiet_cycles(), 49);
        bus.advance_quiet_cycles(49);
        assert_eq!(ran_at.get(), None);
        // An event due now (a queued network frame) leaves no quiet window.
        deadline.set(Some(bus.system_cycles()));
        assert_eq!(bus.quiet_cycles(), 0);
    }
}
