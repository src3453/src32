use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::bus::{Bus, Device};
use crate::devices::bmc::Bmc;

use crate::devices::pec::idc::idc::{DmaDirection, IDC_BASE, IDC_DATA_ADDRESS, IDC_SIZE, Idc};
pub const DMAC_BASE: u32 = 0x8005_0000;
pub const DMAC_SIZE: u32 = 0x1_0000;
const CHANNEL_COUNT: usize = 6;
const CHANNEL_BASE: u32 = 0x100;
const CHANNEL_STRIDE: u32 = 0x20;
/// Per-channel BitBlt registers (ROWS, strides, KEY, ROWS_REMAIN), spec §2.4.
const BLIT_BASE: u32 = 0x200;
/// CAPS bit 0: BitBlt (2D, ROP, colour key) is implemented.
const CAPS_BLIT: u32 = 1;

const CONTROL_MODE_2D: u32 = 1 << 6;
const CONTROL_KEY_EN: u32 = 1 << 7;
const CONTROL_ROP_SHIFT: u32 = 10;
const CONTROL_ROP_MASK: u32 = 3 << CONTROL_ROP_SHIFT;
const CONTROL_WRITABLE: u32 = 0x0FFF;

fn is_idc_address(address: u32) -> bool {
    address >= IDC_BASE && address < IDC_BASE + IDC_SIZE
}

const STATUS_BUSY: u32 = 1;
const STATUS_DONE: u32 = 1 << 1;
const STATUS_ERROR: u32 = 1 << 2;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Phase {
    Read,
    /// ROP != COPY: read the destination beat before writing it.
    ReadDst,
    Write,
}

#[derive(Clone, Copy)]
pub(crate) struct DmaOp {
    pub address: u32,
    pub width: u8,
    pub write: bool,
    pub value: u32,
    /// BitBlt beats may only touch Main RAM, VRAM and PCMRAM (spec §2.4).
    pub memory_only: bool,
}

pub(crate) struct Channel {
    src: u32,
    dst: u32,
    count: u32,
    control: u32,
    status: u32,
    remain: u32,
    src_current: u32,
    dst_current: u32,
    phase: Phase,
    buffer: u32,
    pending_command: u8,
    rows: u32,
    src_stride: u32,
    dst_stride: u32,
    key: u32,
    rows_remain: u32,
    src_row: u32,
    dst_row: u32,
}

impl Channel {
    fn new() -> Self {
        Self {
            src: 0,
            dst: 0,
            count: 0,
            control: 0,
            status: 0,
            remain: 0,
            src_current: 0,
            dst_current: 0,
            phase: Phase::Read,
            buffer: 0,
            pending_command: 0,
            rows: 0,
            src_stride: 0,
            dst_stride: 0,
            key: 0,
            rows_remain: 0,
            src_row: 0,
            dst_row: 0,
        }
    }

    fn is_blit(&self) -> bool {
        self.control & (CONTROL_MODE_2D | CONTROL_KEY_EN | CONTROL_ROP_MASK) != 0
    }

    fn rop(&self) -> u32 {
        (self.control & CONTROL_ROP_MASK) >> CONTROL_ROP_SHIFT
    }

    fn width_mask(width: u8) -> u32 {
        match width {
            1 => 0xFF,
            2 => 0xFFFF,
            _ => 0xFFFF_FFFF,
        }
    }

    fn width(&self) -> Option<u8> {
        match self.control & 3 {
            0 => Some(1),
            1 => Some(2),
            2 => Some(4),
            _ => None,
        }
    }
}

pub struct Dmac {
    enabled: bool,
    channel_enable: u8,
    irq_enable: u8,
    irq_status: u8,
    pub(crate) channels: [Channel; CHANNEL_COUNT],
    bmc: Rc<RefCell<Bmc>>,
    idc: Option<Rc<RefCell<Idc>>>,
    /// Completed bus operations (phases) per channel, including key skips.
    /// The spin fast-forward measures DMA progress per loop iteration with it.
    steps: [u64; CHANNEL_COUNT],
    /// BUSY bit of every channel without an IDC endpoint (memory transfers),
    /// shared with the Bus so it can tell cheaply whether a CPU access may
    /// race one. IDC transfers keep the ordinary per-cycle handling.
    busy: Rc<Cell<u8>>,
    /// Some channel with an IDC endpoint is busy.
    idc_busy: bool,
    /// Bumped by every accepted START (shared with the Bus).
    epoch: Rc<Cell<u32>>,
}

/// DMAC state at a spin-loop head that must repeat for the loop to repeat.
/// Channel configuration cannot change while BUSY, so it is not included.
/// Neither is the beat phase: a busy memory channel requests the bus on every
/// cycle whatever its phase, so how many operations it is granted per loop
/// iteration depends only on the BMC state, not on the data (a colour-keyed
/// pixel only changes how many of those operations one beat takes).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct DmacSpinState {
    enabled: bool,
    busy: u8,
    irq_status: u8,
}

/// How far a channel can run in bulk: `steps` bus operations that stay inside
/// mapped memory and neither finish nor fail the transfer. `dst` is the
/// inclusive address range those steps may write (`None` when nothing).
pub(crate) struct DmaReach {
    pub steps: u64,
    pub dst: Option<(u32, u32)>,
}

impl Dmac {
    pub fn new(bmc: Rc<RefCell<Bmc>>) -> Self {
        Self {
            enabled: true,
            channel_enable: 0,
            irq_enable: 0,
            irq_status: 0,
            channels: std::array::from_fn(|_| Channel::new()),
            bmc,
            idc: None,
            steps: [0; CHANNEL_COUNT],
            busy: Rc::new(Cell::new(0)),
            idc_busy: false,
            epoch: Rc::new(Cell::new(0)),
        }
    }

    pub(crate) fn busy_cell(&self) -> Rc<Cell<u8>> {
        Rc::clone(&self.busy)
    }

    pub(crate) fn epoch_cell(&self) -> Rc<Cell<u32>> {
        Rc::clone(&self.epoch)
    }

    fn update_busy(&mut self) {
        let mut memory = 0u8;
        let mut idc = false;
        for (index, channel) in self.channels.iter().enumerate() {
            if channel.status & STATUS_BUSY == 0 {
                continue;
            }
            if is_idc_address(channel.src) || is_idc_address(channel.dst) {
                idc = true;
            } else {
                memory |= 1 << index;
            }
        }
        self.busy.set(memory);
        self.idc_busy = idc;
    }

    pub(crate) fn uses_idc_endpoint(&self, index: usize) -> bool {
        let channel = &self.channels[index];
        is_idc_address(channel.src) || is_idc_address(channel.dst)
    }

    /// Some channel is busy and none of the busy ones has an IDC endpoint.
    pub(crate) fn busy_without_idc_endpoint(&self) -> bool {
        self.busy.get() != 0 && !self.idc_busy
    }

    pub(crate) fn spin_state(&self) -> (DmacSpinState, u64) {
        (
            DmacSpinState {
                enabled: self.enabled,
                busy: self.busy.get(),
                irq_status: self.irq_status,
            },
            self.steps.iter().sum(),
        )
    }

    /// The only busy channel when the spin fast-forward may run its beats in
    /// bulk: a transfer without an IDC endpoint, whose bus requests depend on
    /// sector timing and abort boundaries.
    pub(crate) fn spin_channel(&self) -> Option<usize> {
        let busy = self.busy.get();
        if !self.enabled || self.idc_busy || busy.count_ones() != 1 {
            return None;
        }
        let index = busy.trailing_zeros() as usize;
        self.channels[index].width().is_some().then_some(index)
    }

    /// Bus operations channel `index` can perform from now without leaving
    /// memory (`in_memory(address, len)`) or reaching its final operation,
    /// which changes STATUS and may raise the IRQ. Stops counting once at least
    /// `max_steps` are known.
    pub(crate) fn spin_reach(
        &self,
        index: usize,
        max_steps: u64,
        in_memory: impl Fn(u32, u32) -> bool,
    ) -> DmaReach {
        let channel = &self.channels[index];
        let Some(width) = channel.width() else {
            return DmaReach {
                steps: 0,
                dst: None,
            };
        };
        let width = u32::from(width);
        // Fewest operations one beat can take; a keyed (transparent) source
        // pixel takes only its read. Counting the fewest keeps `steps` a bound.
        let (per_beat, done_in_beat): (u64, u64) = if channel.control & CONTROL_KEY_EN != 0 {
            (1, 0)
        } else {
            let per_beat = if channel.rop() != 0 { 3 } else { 2 };
            let done = match channel.phase {
                Phase::Read => 0,
                Phase::ReadDst => 1,
                Phase::Write => per_beat - 1,
            };
            (per_beat, done)
        };
        let src_fixed = channel.control & (1 << 2) != 0;
        let dst_fixed = channel.control & (1 << 4) != 0;
        let mut beats = 0u64;
        let mut dst: Option<(u32, u32)> = None;
        let mut complete = true;
        for row in 0..channel.rows_remain {
            let (src, dst_start, bytes) = if row == 0 {
                (channel.src_current, channel.dst_current, channel.remain)
            } else {
                (
                    channel
                        .src_row
                        .wrapping_add(channel.src_stride.wrapping_mul(row)),
                    channel
                        .dst_row
                        .wrapping_add(channel.dst_stride.wrapping_mul(row)),
                    channel.count,
                )
            };
            let src_len = if src_fixed { width } else { bytes };
            let dst_len = if dst_fixed { width } else { bytes };
            if bytes < width || !in_memory(src, src_len) || !in_memory(dst_start, dst_len) {
                complete = false;
                break;
            }
            beats += u64::from(bytes / width);
            let last = dst_start + (dst_len - 1);
            dst = Some(dst.map_or((dst_start, last), |(lo, hi)| {
                (lo.min(dst_start), hi.max(last))
            }));
            if beats * per_beat >= max_steps.saturating_add(per_beat + done_in_beat) {
                complete = false;
                break;
            }
        }
        let total = (beats * per_beat).saturating_sub(done_in_beat);
        let steps = if complete {
            total.saturating_sub(1)
        } else {
            total
        };
        DmaReach {
            steps,
            dst: if steps == 0 { None } else { dst },
        }
    }

    pub(crate) fn attach_idc(&mut self, idc: Rc<RefCell<Idc>>) {
        self.idc = Some(idc);
    }

    pub(crate) fn request_mask(&self) -> u8 {
        if !self.enabled {
            return 0;
        }
        let mut requests = 0;
        for (index, channel) in self.channels.iter().enumerate() {
            if channel.status & STATUS_BUSY == 0 {
                continue;
            }
            let uses_idc = channel.src == IDC_DATA_ADDRESS || channel.dst == IDC_DATA_ADDRESS;
            let can_request = if !uses_idc {
                true
            } else if channel.src == IDC_DATA_ADDRESS && channel.phase == Phase::Write {
                true
            } else {
                self.idc
                    .as_ref()
                    .is_some_and(|idc| idc.borrow().dma_request_ready(index))
            };
            if can_request {
                requests |= 1 << (index + 2);
            }
        }
        requests
    }

    pub(crate) fn next_op(&self, index: usize) -> Option<DmaOp> {
        let channel = self.channels.get(index)?;
        if channel.status & STATUS_BUSY == 0 {
            return None;
        }
        let width = channel.width()?;
        let (address, write, value) = match channel.phase {
            Phase::Read => (channel.src_current, false, 0),
            Phase::ReadDst => (channel.dst_current, false, 0),
            Phase::Write => (channel.dst_current, true, channel.buffer),
        };
        Some(DmaOp {
            address,
            width,
            write,
            value,
            memory_only: channel.is_blit(),
        })
    }

    pub(crate) fn complete_op(&mut self, index: usize, value: u32) {
        self.steps[index] += 1;
        let channel = &mut self.channels[index];
        match channel.phase {
            Phase::Read => {
                channel.buffer = value;
                let mask = Channel::width_mask(channel.width().unwrap_or(1));
                if channel.control & CONTROL_KEY_EN != 0 && value & mask == channel.key & mask {
                    // Transparent beat: no destination read or write (spec §2.4).
                    self.advance_beat(index);
                } else if channel.rop() != 0 {
                    channel.phase = Phase::ReadDst;
                } else {
                    channel.phase = Phase::Write;
                }
            }
            Phase::ReadDst => {
                channel.buffer = match channel.rop() {
                    1 => channel.buffer & value,
                    2 => channel.buffer | value,
                    _ => channel.buffer ^ value,
                };
                channel.phase = Phase::Write;
            }
            Phase::Write => self.advance_beat(index),
        }
    }

    /// Step past one completed (or key-skipped) beat: advance the addresses,
    /// move to the next row in 2D mode, and finish the channel after the last.
    fn advance_beat(&mut self, index: usize) {
        let (idc_endpoint, done) = {
            let channel = &mut self.channels[index];
            let width = channel.width().unwrap_or(1) as u32;
            if channel.control & (1 << 2) == 0 {
                channel.src_current = channel.src_current.wrapping_add(width);
            }
            if channel.control & (1 << 4) == 0 {
                channel.dst_current = channel.dst_current.wrapping_add(width);
            }
            channel.remain = channel.remain.saturating_sub(width);
            if channel.remain == 0 {
                channel.rows_remain = channel.rows_remain.saturating_sub(1);
                if channel.rows_remain != 0 {
                    channel.src_row = channel.src_row.wrapping_add(channel.src_stride);
                    channel.dst_row = channel.dst_row.wrapping_add(channel.dst_stride);
                    channel.src_current = channel.src_row;
                    channel.dst_current = channel.dst_row;
                    channel.remain = channel.count;
                }
            }
            (
                channel.src == IDC_DATA_ADDRESS || channel.dst == IDC_DATA_ADDRESS,
                channel.remain == 0,
            )
        };
        let abort_at_boundary = idc_endpoint
            && self
                .idc
                .as_ref()
                .is_some_and(|idc| idc.borrow_mut().dma_byte_completed(index));
        if abort_at_boundary {
            self.fail_without_idc(index, 3);
            return;
        }
        if done {
            self.channels[index].status &= !STATUS_BUSY;
            self.channels[index].status |= STATUS_DONE;
            self.update_busy();
            self.raise_irq(index);
        } else {
            self.channels[index].phase = Phase::Read;
        }
    }

    pub(crate) fn fail(&mut self, index: usize, code: u32) {
        let notify_idc = self
            .idc
            .as_ref()
            .is_some_and(|idc| idc.borrow().dma_channel() == Some(index));
        self.fail_without_idc(index, code);
        if notify_idc {
            if let Some(idc) = &self.idc {
                idc.borrow_mut().dma_failed(index);
            }
        }
    }

    fn fail_without_idc(&mut self, index: usize, code: u32) {
        let channel = &mut self.channels[index];
        channel.status &= !(STATUS_BUSY | STATUS_DONE | STATUS_ERROR | 0xF0);
        channel.status |= STATUS_ERROR | ((code & 0xF) << 4);
        self.update_busy();
        self.raise_irq(index);
    }

    pub(crate) fn fail_from_idc(&mut self, index: usize, code: u32) {
        if index < CHANNEL_COUNT && self.channels[index].status & STATUS_BUSY != 0 {
            self.fail_without_idc(index, code);
        }
    }

    fn reject_start(&mut self, index: usize, code: u32, notify_idc: bool) {
        self.fail_without_idc(index, code);
        if notify_idc {
            if let Some(idc) = &self.idc {
                if idc.borrow().dma_requirements().is_some() {
                    idc.borrow_mut().dma_start_failed();
                }
            }
        }
    }

    fn raise_irq(&mut self, index: usize) {
        if self.irq_enable & (1 << index) != 0 {
            self.irq_status |= 1 << index;
        }
    }

    pub(crate) fn irq_pending(&self) -> bool {
        self.irq_status != 0
    }
    pub(crate) fn has_active_transfer(&self) -> bool {
        self.enabled
            && self
                .channels
                .iter()
                .any(|channel| channel.status & STATUS_BUSY != 0)
    }

    pub(crate) fn can_batch_idle(&self) -> bool {
        // Even a globally disabled busy channel may need abort processing.
        self.channels
            .iter()
            .all(|channel| channel.status & STATUS_BUSY == 0)
    }
    pub(crate) fn process_idc_abort_boundaries(&mut self) {
        for index in 0..CHANNEL_COUNT {
            let channel = &self.channels[index];
            if channel.status & STATUS_BUSY == 0
                || channel.phase != Phase::Read
                || !(is_idc_address(channel.src) || is_idc_address(channel.dst))
            {
                continue;
            }
            let abort = self
                .idc
                .as_ref()
                .is_some_and(|idc| idc.borrow_mut().abort_dma_at_boundary(index));
            if abort {
                self.fail_without_idc(index, 3);
            }
        }
    }

    fn start(&mut self, index: usize) {
        let channel = &self.channels[index];
        let endpoint_configured = is_idc_address(channel.src) || is_idc_address(channel.dst);
        if !self.enabled || self.channel_enable & (1 << index) == 0 {
            self.reject_start(index, 1, endpoint_configured);
            return;
        }
        if channel.status & STATUS_BUSY != 0 {
            return;
        }
        let (src, dst, count, control) = (channel.src, channel.dst, channel.count, channel.control);
        let Some(width) = channel.width() else {
            self.reject_start(index, 1, endpoint_configured);
            return;
        };
        let src_mode = (control >> 2) & 3;
        let dst_mode = (control >> 4) & 3;
        if count == 0
            || count % u32::from(width) != 0
            || src % u32::from(width) != 0
            || dst % u32::from(width) != 0
            || src_mode > 1
            || dst_mode > 1
        {
            self.reject_start(index, 1, endpoint_configured);
            return;
        }
        if channel.is_blit() {
            let width = u32::from(width);
            let bad_2d = control & CONTROL_MODE_2D != 0
                && (channel.rows == 0
                    || channel.src_stride % width != 0
                    || channel.dst_stride % width != 0);
            if endpoint_configured || bad_2d {
                self.reject_start(index, 1, endpoint_configured);
                return;
            }
        }

        if endpoint_configured {
            let active = self
                .idc
                .as_ref()
                .and_then(|idc| idc.borrow().dma_requirements());
            let another_channel_uses_endpoint =
                self.channels.iter().enumerate().any(|(other, channel)| {
                    other != index
                        && (channel.src == IDC_DATA_ADDRESS || channel.dst == IDC_DATA_ADDRESS)
                });
            let valid = active.is_some_and(|(direction, expected_count)| {
                if another_channel_uses_endpoint || count != expected_count || width != 1 {
                    return false;
                }
                match direction {
                    DmaDirection::DeviceToMemory => {
                        src == IDC_DATA_ADDRESS
                            && src_mode == 1
                            && dst != IDC_DATA_ADDRESS
                            && dst_mode == 0
                    }
                    DmaDirection::MemoryToDevice => {
                        dst == IDC_DATA_ADDRESS
                            && dst_mode == 1
                            && src != IDC_DATA_ADDRESS
                            && src_mode == 0
                    }
                }
            });
            if !valid {
                self.reject_start(index, 1, active.is_some());
                return;
            }
            let attached = self
                .idc
                .as_ref()
                .is_some_and(|idc| idc.borrow_mut().attach_dma(index));
            if !attached {
                self.reject_start(index, 1, true);
                return;
            }
        }

        let channel = &mut self.channels[index];
        channel.src_current = channel.src;
        channel.dst_current = channel.dst;
        channel.src_row = channel.src;
        channel.dst_row = channel.dst;
        channel.remain = channel.count;
        channel.rows_remain = if channel.control & CONTROL_MODE_2D != 0 {
            channel.rows
        } else {
            1
        };
        channel.phase = Phase::Read;
        channel.status = STATUS_BUSY;
        self.update_busy();
        self.epoch.set(self.epoch.get().wrapping_add(1));
    }

    fn abort(&mut self, index: usize) {
        if self.channels[index].status & STATUS_BUSY != 0 {
            self.fail(index, 3);
        }
    }

    /// REMAIN or ROWS_REMAIN of a busy memory (non-IDC) channel.
    fn is_progress_register(&self, addr: u32) -> bool {
        let (base, field) = if addr >= BLIT_BASE {
            (BLIT_BASE, 0x10)
        } else if addr >= CHANNEL_BASE {
            (CHANNEL_BASE, 0x14)
        } else {
            return false;
        };
        let relative = (addr & !3) - base;
        let index = relative / CHANNEL_STRIDE;
        relative % CHANNEL_STRIDE == field
            && index < CHANNEL_COUNT as u32
            && self.busy.get() & (1 << index) != 0
    }

    fn read_register(&self, addr: u32) -> u32 {
        match addr & !3 {
            0x000 => 0x444D_4101,
            0x004 => u32::from(self.enabled),
            0x008 => u32::from(self.channel_enable),
            0x00C => u32::from(self.irq_enable),
            0x010 => u32::from(self.irq_status),
            0x014 => 0,
            0x018 => CAPS_BLIT,
            offset if offset >= BLIT_BASE => {
                let relative = offset - BLIT_BASE;
                let index = (relative / CHANNEL_STRIDE) as usize;
                if index >= CHANNEL_COUNT {
                    return 0;
                }
                let channel = &self.channels[index];
                match relative % CHANNEL_STRIDE {
                    0x00 => channel.rows,
                    0x04 => channel.src_stride,
                    0x08 => channel.dst_stride,
                    0x0C => channel.key,
                    0x10 => channel.rows_remain,
                    _ => 0,
                }
            }
            offset if offset >= CHANNEL_BASE => {
                let relative = offset - CHANNEL_BASE;
                let index = (relative / CHANNEL_STRIDE) as usize;
                if index >= CHANNEL_COUNT {
                    return 0;
                }
                match relative % CHANNEL_STRIDE {
                    0x00 => self.channels[index].src,
                    0x04 => self.channels[index].dst,
                    0x08 => self.channels[index].count,
                    0x0C => self.channels[index].control & !((1 << 8) | (1 << 9)),
                    0x10 => self.channels[index].status,
                    0x14 => self.channels[index].remain,
                    0x18 => u32::from(self.bmc.borrow().priority(index + 2)),
                    _ => 0,
                }
            }
            _ => 0,
        }
    }

    fn write_byte(&mut self, addr: u32, value: u8) {
        let reg = addr & !3;
        let lane = addr & 3;
        let shift = (3 - lane) * 8;
        let byte_value = u32::from(value) << shift;
        match reg {
            0x004 if lane == 3 => {
                let mask = !(0xFFu32 << shift);
                self.enabled = ((u32::from(self.enabled) & mask) | byte_value) & 1 != 0;
                if !self.enabled {
                    for index in 0..CHANNEL_COUNT {
                        self.abort(index);
                    }
                }
            }
            0x008 if lane == 3 => {
                let mask = !(0xFFu32 << shift);
                self.channel_enable =
                    ((u32::from(self.channel_enable) & mask) | byte_value) as u8 & 0x3F;
            }
            0x00C if lane == 3 => {
                let mask = !(0xFFu32 << shift);
                self.irq_enable = ((u32::from(self.irq_enable) & mask) | byte_value) as u8 & 0x3F;
            }
            0x010 if lane == 3 => self.irq_status &= !value,
            0x014 if lane == 3 => {
                for index in 0..CHANNEL_COUNT {
                    if value & (1 << index) != 0 {
                        self.abort(index);
                    }
                }
            }
            offset if offset >= BLIT_BASE => {
                let relative = offset - BLIT_BASE;
                let index = (relative / CHANNEL_STRIDE) as usize;
                if index >= CHANNEL_COUNT || self.channels[index].status & STATUS_BUSY != 0 {
                    return;
                }
                let channel = &mut self.channels[index];
                let field = relative % CHANNEL_STRIDE;
                let target = match field {
                    0x00 => &mut channel.rows,
                    0x04 => &mut channel.src_stride,
                    0x08 => &mut channel.dst_stride,
                    0x0C => &mut channel.key,
                    _ => return,
                };
                *target = (*target & !(0xFF << shift)) | byte_value;
                if field == 0x00 {
                    channel.rows_remain = channel.rows;
                }
            }
            offset if offset >= CHANNEL_BASE => {
                let relative = offset - CHANNEL_BASE;
                let index = (relative / CHANNEL_STRIDE) as usize;
                if index >= CHANNEL_COUNT {
                    return;
                }
                let field = relative % CHANNEL_STRIDE;
                if field == 0x10 {
                    let clear = (u32::from(value) << shift) & (STATUS_DONE | STATUS_ERROR);
                    self.channels[index].status &= !clear;
                    if clear & STATUS_ERROR != 0 {
                        self.channels[index].status &= !0xF0;
                    }
                    return;
                }
                if field == 0x18 && lane == 3 {
                    self.bmc.borrow_mut().set_priority(index + 2, value & 7);
                    return;
                }
                if self.channels[index].status & STATUS_BUSY != 0 {
                    return;
                }
                if field == 0x0C {
                    let channel = &mut self.channels[index];
                    channel.control =
                        ((channel.control & !(0xFF << shift)) | byte_value) & CONTROL_WRITABLE;
                    if lane == 2 {
                        channel.pending_command = value & 3;
                    }
                    if lane == 3 && channel.pending_command != 0 {
                        let command = std::mem::take(&mut channel.pending_command);
                        if command & 1 != 0 {
                            self.start(index);
                        } else if command & 2 != 0 {
                            self.abort(index);
                        }
                    }
                    return;
                }
                let target = match field {
                    0x00 => &mut self.channels[index].src,
                    0x04 => &mut self.channels[index].dst,
                    0x08 => &mut self.channels[index].count,
                    _ => return,
                };
                *target = (*target & !(0xFF << shift)) | byte_value;
                if field == 0x08 {
                    self.channels[index].remain = self.channels[index].count;
                }
            }
            _ => {}
        }
    }
}

struct DmacRegisters(Rc<RefCell<Dmac>>);

impl Device for DmacRegisters {
    fn debug_read(&mut self, addr: u32) -> Option<u8> {
        let value = self.0.borrow().read_register(addr);
        Some((value >> ((3 - (addr & 3)) * 8)) as u8)
    }

    fn peek_u32_be(&self, addr: u32) -> Option<u32> {
        let dmac = self.0.borrow();
        // REMAIN and ROWS_REMAIN of a busy channel change with every beat, so
        // a loop reading them is not polling a constant (spin fast-forward).
        (addr & 3 == 0 && !dmac.is_progress_register(addr)).then(|| dmac.read_register(addr))
    }

    fn read(&mut self, addr: u32) -> u8 {
        let value = self.0.borrow().read_register(addr);
        let shift = (3 - (addr & 3)) * 8;
        (value >> shift) as u8
    }

    fn write(&mut self, addr: u32, value: u8) {
        self.0.borrow_mut().write_byte(addr, value);
    }

    fn size(&self) -> u32 {
        DMAC_SIZE
    }
}

pub fn connect_dmac(bus: &mut Bus, bmc: Rc<RefCell<Bmc>>) -> Rc<RefCell<Dmac>> {
    let dmac = Rc::new(RefCell::new(Dmac::new(bmc)));
    bus.add_device(
        DMAC_BASE,
        "DMAC MMIO",
        Box::new(DmacRegisters(Rc::clone(&dmac))),
    );
    bus.attach_dmac(Rc::clone(&dmac));
    dmac
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::bmc::connect_bmc;
    use crate::devices::irqc::irqc::connect_irqc;
    use crate::devices::ram::connect_ram;

    const CH0: u32 = DMAC_BASE + CHANNEL_BASE;

    fn setup_bus(with_irq: bool) -> Bus {
        let mut bus = Bus::new();
        connect_ram(&mut bus);
        let bmc = connect_bmc(&mut bus);
        connect_dmac(&mut bus, bmc);
        if with_irq {
            let irq = connect_irqc(&mut bus);
            bus.attach_irq_controller(irq);
        }
        bus
    }

    fn configure_channel0(bus: &mut Bus, src: u32, dst: u32, count: u32, control: u32) {
        bus.write_u32_be(CH0, src);
        bus.write_u32_be(CH0 + 4, dst);
        bus.write_u32_be(CH0 + 8, count);
        bus.write_u32_be(DMAC_BASE + 8, 1);
        bus.write_u32_be(CH0 + 0x0C, control | (1 << 8));
    }

    fn run_dma(bus: &mut Bus, ticks: usize) {
        for _ in 0..ticks {
            bus.tick_devices();
        }
    }

    #[test]
    fn copies_a_word_and_sets_completion_irq() {
        let mut bus = setup_bus(true);
        bus.write_u32_be(0x100, 0x1234_5678);
        bus.write_u32_be(DMAC_BASE + 0x0C, 1);
        configure_channel0(&mut bus, 0x100, 0x200, 4, 2);

        run_dma(&mut bus, 8);

        assert_eq!(bus.read_u32_be(0x200), 0x1234_5678);
        assert_eq!(bus.read_u32_be(CH0 + 0x10), STATUS_DONE);
        assert_eq!(bus.read_u32_be(CH0 + 0x14), 0);
        assert_eq!(bus.read_u32_be(DMAC_BASE + 0x10), 1);
        assert_ne!(
            bus.read_u8(crate::devices::irqc::irqc::IRQC_BASE + 1) & (1 << 1),
            0
        );
    }

    #[test]
    fn dma_write_invalidates_l1_index_for_both_banks() {
        let mut bus = setup_bus(false);
        let cache = crate::cpu_cache::connect_cache(&mut bus);
        // 0x2200 conflicts with destination 0x200 in the direct-mapped cache.
        cache.borrow_mut().fetch(0x2200);
        cache.borrow_mut().load(0x2200);
        bus.write_u32_be(0x100, 0x1234_5678);
        configure_channel0(&mut bus, 0x100, 0x200, 4, 2);
        run_dma(&mut bus, 8);
        assert_eq!(bus.read_u32_be(0x200), 0x1234_5678);
        cache.borrow_mut().fetch(0x2200);
        cache.borrow_mut().load(0x2200);
        assert_eq!(cache.borrow().read_reg(0x14), 2);
        assert_eq!(cache.borrow().read_reg(0x1c), 2);
    }

    #[test]
    fn runs_all_six_channels_concurrently() {
        let mut bus = setup_bus(false);
        bus.write_u32_be(DMAC_BASE + 8, 0x3F);
        for channel in 0..CHANNEL_COUNT {
            let base = DMAC_BASE + CHANNEL_BASE + channel as u32 * CHANNEL_STRIDE;
            bus.write_u8(0x100 + channel as u32, 0xA0 + channel as u8);
            bus.write_u32_be(base, 0x100 + channel as u32);
            bus.write_u32_be(base + 4, 0x200 + channel as u32);
            bus.write_u32_be(base + 8, 1);
            bus.write_u32_be(base + 0x0C, 1 << 8);
        }

        run_dma(&mut bus, 32);

        for channel in 0..CHANNEL_COUNT {
            let base = DMAC_BASE + CHANNEL_BASE + channel as u32 * CHANNEL_STRIDE;
            assert_eq!(bus.read_u8(0x200 + channel as u32), 0xA0 + channel as u8);
            assert_eq!(bus.read_u32_be(base + 0x10), STATUS_DONE);
        }
    }

    #[test]
    fn rejects_mmio_as_a_dma_destination() {
        let mut bus = setup_bus(false);
        bus.write_u8(0x100, 0xA5);
        configure_channel0(&mut bus, 0x100, 0x8000_0000, 1, 0);

        run_dma(&mut bus, 4);

        let status = bus.read_u32_be(CH0 + 0x10);
        assert_ne!(status & STATUS_ERROR, 0);
        assert_eq!((status >> 4) & 0xF, 2);
    }

    // The RTL testbench (src32-sv/sim/tb_dmac.sv) asserts these same values; keeping
    // them here makes the emulator the checked golden reference for that bench.
    fn start_channel(bus: &mut Bus, ch: u32, src: u32, dst: u32, count: u32, control: u32) {
        let base = DMAC_BASE + CHANNEL_BASE + ch * CHANNEL_STRIDE;
        bus.write_u32_be(base, src);
        bus.write_u32_be(base + 4, dst);
        bus.write_u32_be(base + 8, count);
        bus.write_u32_be(base + 0x0C, control | (1 << 8));
    }

    fn channel_status(bus: &mut Bus, ch: u32) -> u32 {
        bus.read_u32_be(DMAC_BASE + CHANNEL_BASE + ch * CHANNEL_STRIDE + 0x10)
    }

    #[test]
    fn rtl_reference_start_validation_codes() {
        let mut bus = setup_bus(false);
        bus.write_u32_be(DMAC_BASE + 8, 0x3F);
        // (src, dst, count, control): every case is a configuration error, code 1.
        let bad = [
            (0x100, 0x300, 0, 0),
            (0x100, 0x300, 6, 2),
            (0x101, 0x300, 4, 1),
            (0x100, 0x302, 4, 2),
            (0x100, 0x300, 4, 3),
            (0x100, 0x300, 4, 2 << 2),
            (0x100, 0x300, 4, 2 << 4),
            (0x100, 0x8004_1024, 4, 1 << 4),
        ];
        for (src, dst, count, control) in bad {
            start_channel(&mut bus, 2, src, dst, count, control);
            assert_eq!(
                channel_status(&mut bus, 2),
                0x14,
                "{src:#x} {dst:#x} {count} {control:#x}"
            );
            bus.write_u32_be(DMAC_BASE + CHANNEL_BASE + 2 * CHANNEL_STRIDE + 0x10, 4);
        }
        bus.write_u32_be(DMAC_BASE + 8, 0x3B); // channel 2 disabled
        start_channel(&mut bus, 2, 0x100, 0x300, 4, 2);
        assert_eq!(channel_status(&mut bus, 2), 0x14);
        bus.write_u32_be(DMAC_BASE + CHANNEL_BASE + 2 * CHANNEL_STRIDE + 0x10, 4);
        bus.write_u32_be(DMAC_BASE + 8, 0x3F);
        bus.write_u32_be(DMAC_BASE + 4, 0); // DMAC disabled
        start_channel(&mut bus, 2, 0x100, 0x300, 4, 2);
        assert_eq!(channel_status(&mut bus, 2), 0x14);
        bus.write_u32_be(DMAC_BASE + 4, 1);
        bus.write_u32_be(DMAC_BASE + CHANNEL_BASE + 2 * CHANNEL_STRIDE + 0x10, 4);
        assert_eq!(channel_status(&mut bus, 2), 0);
    }

    #[test]
    fn rtl_reference_address_windows_and_overlap() {
        let mut bus = setup_bus(false);
        bus.write_u32_be(DMAC_BASE + 8, 0x3F);
        // Just past the 16 MiB Main RAM window, and a non-FIFO MMIO word: code 2.
        for (src, dst) in [(0x0100_0000, 0x200), (0x100, 0x8003_0028)] {
            start_channel(&mut bus, 1, src, dst, 4, 2);
            run_dma(&mut bus, 16);
            assert_eq!(channel_status(&mut bus, 1), 0x24);
            bus.write_u32_be(DMAC_BASE + CHANNEL_BASE + CHANNEL_STRIDE + 0x10, 4);
        }
        // Overlapping copy behaves like a forward byte loop: 1,2,1,2,1,2.
        for (i, b) in [1u8, 2, 3, 4, 5, 6, 7, 8].into_iter().enumerate() {
            bus.write_u8(0x700 + i as u32, b);
        }
        start_channel(&mut bus, 4, 0x700, 0x702, 6, 0);
        run_dma(&mut bus, 64);
        let copied: Vec<u8> = (0x700..0x708).map(|a| bus.read_u8(a)).collect();
        assert_eq!(copied, [1, 2, 1, 2, 1, 2, 1, 2]);
        assert_eq!(channel_status(&mut bus, 4), STATUS_DONE);
    }

    #[test]
    fn rtl_reference_busy_abort_and_priority() {
        let mut bus = setup_bus(false);
        bus.write_u32_be(DMAC_BASE + 8, 0x3F);
        start_channel(&mut bus, 0, 0x800, 0x900, 64, 2);
        // BUSY: configuration and START are ignored.
        bus.write_u32_be(CH0, 0x1234);
        bus.write_u32_be(CH0 + 8, 4);
        bus.write_u32_be(CH0 + 0x0C, 1 << 8);
        assert_eq!(bus.read_u32_be(CH0), 0x800);
        assert_eq!(bus.read_u32_be(CH0 + 8), 64);
        assert_eq!(bus.read_u32_be(CH0 + 0x0C), 2);
        // Global disable aborts running channels with code 3.
        run_dma(&mut bus, 10);
        bus.write_u32_be(DMAC_BASE + 4, 0);
        assert_eq!(channel_status(&mut bus, 0), 0x34);
        bus.write_u32_be(DMAC_BASE + 4, 1);
        // CHn_PRIORITY aliases BMC MASTER_PRIORITY[n+2]; both keep the low 3 bits.
        bus.write_u32_be(CH0 + CHANNEL_STRIDE + 0x18, 0xFFFF_FFF8);
        assert_eq!(bus.read_u32_be(CH0 + CHANNEL_STRIDE + 0x18), 0);
        assert_eq!(
            bus.read_u32_be(crate::devices::bmc::BMC_BASE + 0x020 + 4 * 3),
            0
        );
        // A split CONTROL write latches START on byte 2 and fires on byte 3.
        bus.write_u32_be(DMAC_BASE + CHANNEL_BASE + 2 * CHANNEL_STRIDE + 8, 4);
        bus.write_u32_be(DMAC_BASE + CHANNEL_BASE + 2 * CHANNEL_STRIDE, 0x100);
        bus.write_u32_be(DMAC_BASE + CHANNEL_BASE + 2 * CHANNEL_STRIDE + 4, 0x300);
        let ctl = DMAC_BASE + CHANNEL_BASE + 2 * CHANNEL_STRIDE + 0x0C;
        bus.write_u8(ctl + 2, 1);
        assert_eq!(channel_status(&mut bus, 2), 0);
        bus.write_u8(ctl + 3, 2);
        assert_eq!(channel_status(&mut bus, 2), STATUS_BUSY);
    }

    const BLIT0: u32 = DMAC_BASE + BLIT_BASE;
    const MODE_2D: u32 = CONTROL_MODE_2D;
    const KEY_EN: u32 = CONTROL_KEY_EN;

    fn set_blit0(bus: &mut Bus, rows: u32, src_stride: i32, dst_stride: i32, key: u32) {
        bus.write_u32_be(BLIT0, rows);
        bus.write_u32_be(BLIT0 + 4, src_stride as u32);
        bus.write_u32_be(BLIT0 + 8, dst_stride as u32);
        bus.write_u32_be(BLIT0 + 0x0C, key);
    }

    fn run_until_idle(bus: &mut Bus) -> u32 {
        for _ in 0..10_000 {
            bus.tick_devices();
            let status = bus.read_u32_be(CH0 + 0x10);
            if status & STATUS_BUSY == 0 {
                return status;
            }
        }
        panic!("DMA channel 0 did not finish");
    }

    #[test]
    fn caps_reports_blit_and_id_is_unchanged() {
        let mut bus = setup_bus(false);
        // Foundation's kmem_copy and the BIOS compare the ID word exactly.
        assert_eq!(bus.read_u32_be(DMAC_BASE), 0x444D_4101);
        assert_eq!(bus.read_u32_be(DMAC_BASE + 0x18), CAPS_BLIT);
        bus.write_u32_be(DMAC_BASE + 0x18, 0);
        assert_eq!(bus.read_u32_be(DMAC_BASE + 0x18), CAPS_BLIT);
        // Reserved CONTROL bits read 0; the BitBlt bits are kept.
        bus.write_u32_be(CH0 + 0x0C, 0xFFFF_F0C0);
        assert_eq!(bus.read_u32_be(CH0 + 0x0C), 0xC0);
        bus.write_u32_be(BLIT0, 7);
        assert_eq!(bus.read_u32_be(BLIT0 + 0x10), 7);
    }

    #[test]
    fn blit_copies_a_rectangle_between_strides() {
        let mut bus = setup_bus(false);
        // 3 rows of 4 bytes at stride 8 -> stride 16, leaving gaps untouched.
        for row in 0..3u32 {
            for col in 0..8u32 {
                bus.write_u8(0x1000 + row * 8 + col, (row * 0x10 + col) as u8);
            }
        }
        for a in 0x2000..0x2040 {
            bus.write_u8(a, 0xEE);
        }
        set_blit0(&mut bus, 3, 8, 16, 0);
        configure_channel0(&mut bus, 0x1001, 0x2002, 4, MODE_2D);
        assert_eq!(run_until_idle(&mut bus), STATUS_DONE);
        for row in 0..4u32 {
            for col in 0..16u32 {
                let expected = if row < 3 && (2..6).contains(&col) {
                    (row * 0x10 + col - 1) as u8
                } else {
                    0xEE
                };
                assert_eq!(
                    bus.read_u8(0x2000 + row * 16 + col),
                    expected,
                    "{row},{col}"
                );
            }
        }
        assert_eq!(bus.read_u32_be(CH0 + 0x14), 0);
        assert_eq!(bus.read_u32_be(BLIT0 + 0x10), 0);
    }

    #[test]
    fn blit_progress_registers_track_the_current_row() {
        let mut bus = setup_bus(false);
        set_blit0(&mut bus, 2, 4, 4, 0);
        configure_channel0(&mut bus, 0x1000, 0x2000, 2, MODE_2D);
        assert_eq!(bus.read_u32_be(BLIT0 + 0x10), 2);
        // Two bus beats (read + write) per byte; after the first row REMAIN reloads.
        run_dma(&mut bus, 4);
        assert_eq!(bus.read_u32_be(CH0 + 0x14), 2);
        assert_eq!(bus.read_u32_be(BLIT0 + 0x10), 1);
        // BUSY: BitBlt registers ignore writes like the other configuration registers.
        bus.write_u32_be(BLIT0, 9);
        bus.write_u32_be(BLIT0 + 4, 0x40);
        assert_eq!(bus.read_u32_be(BLIT0), 2);
        assert_eq!(bus.read_u32_be(BLIT0 + 4), 4);
        assert_eq!(run_until_idle(&mut bus), STATUS_DONE);
    }

    #[test]
    fn blit_fills_with_a_fixed_source_and_zero_stride() {
        let mut bus = setup_bus(false);
        bus.write_u32_be(0x1000, 0xCAFE_F00D);
        set_blit0(&mut bus, 4, 0, 32, 0);
        configure_channel0(&mut bus, 0x1000, 0x2000, 8, 2 | (1 << 2) | MODE_2D);
        assert_eq!(run_until_idle(&mut bus), STATUS_DONE);
        for row in 0..4 {
            assert_eq!(bus.read_u32_be(0x2000 + row * 32), 0xCAFE_F00D);
            assert_eq!(bus.read_u32_be(0x2004 + row * 32), 0xCAFE_F00D);
            assert_eq!(bus.read_u32_be(0x2008 + row * 32), 0);
        }
    }

    #[test]
    fn blit_negative_stride_scrolls_overlapping_rows_down() {
        let mut bus = setup_bus(false);
        // Rows 0..3 of 4 bytes; move rows 0..2 down by one row in place.
        for row in 0..4u32 {
            bus.write_u32_be(0x3000 + row * 4, 0x1111_1111 * (row + 1));
        }
        set_blit0(&mut bus, 3, -4, -4, 0);
        configure_channel0(&mut bus, 0x3008, 0x300C, 4, 2 | MODE_2D);
        assert_eq!(run_until_idle(&mut bus), STATUS_DONE);
        let rows: Vec<u32> = (0..4).map(|r| bus.read_u32_be(0x3000 + r * 4)).collect();
        assert_eq!(rows, [0x1111_1111, 0x1111_1111, 0x2222_2222, 0x3333_3333]);
    }

    #[test]
    fn blit_colour_key_skips_matching_source_pixels() {
        let mut bus = setup_bus(false);
        for (i, b) in [0u8, 5, 0, 7, 9, 0].into_iter().enumerate() {
            bus.write_u8(0x1000 + i as u32, b);
        }
        for a in 0x2000..0x2010 {
            bus.write_u8(a, 0xAA);
        }
        // 2 rows x 3 pixels, key 0 (upper key bits beyond WIDTH are ignored).
        set_blit0(&mut bus, 2, 3, 8, 0xFFFF_FF00);
        configure_channel0(&mut bus, 0x1000, 0x2000, 3, MODE_2D | KEY_EN);
        assert_eq!(run_until_idle(&mut bus), STATUS_DONE);
        let row0: Vec<u8> = (0x2000..0x2004).map(|a| bus.read_u8(a)).collect();
        let row1: Vec<u8> = (0x2008..0x200C).map(|a| bus.read_u8(a)).collect();
        assert_eq!(row0, [0xAA, 5, 0xAA, 0xAA]);
        assert_eq!(row1, [7, 9, 0xAA, 0xAA]);
    }

    #[test]
    fn blit_colour_key_works_in_1d_mode() {
        let mut bus = setup_bus(false);
        bus.write_u32_be(0x1000, 0x7C00_0000); // RGB555 red, then key 0x0000
        bus.write_u32_be(0x2000, 0x1234_5678);
        set_blit0(&mut bus, 0, 0, 0, 0);
        configure_channel0(&mut bus, 0x1000, 0x2000, 4, 1 | KEY_EN);
        assert_eq!(run_until_idle(&mut bus), STATUS_DONE);
        assert_eq!(bus.read_u32_be(0x2000), 0x7C00_5678);
        assert_eq!(bus.read_u32_be(BLIT0 + 0x10), 0);
    }

    #[test]
    fn blit_raster_operations_combine_with_the_destination() {
        for (rop, expected) in [(0u32, 0xF0F0), (1, 0xC0C0), (2, 0xFCFC), (3, 0x3C3C)] {
            let mut bus = setup_bus(false);
            bus.write_u32_be(0x1000, 0xF0F0_1111);
            bus.write_u32_be(0x2000, 0xCCCC_2222);
            configure_channel0(&mut bus, 0x1000, 0x2000, 2, 1 | (rop << CONTROL_ROP_SHIFT));
            assert_eq!(run_until_idle(&mut bus), STATUS_DONE, "rop {rop}");
            assert_eq!(
                bus.read_u32_be(0x2000),
                (expected << 16) | 0x2222,
                "rop {rop}"
            );
        }
    }

    #[test]
    fn blit_abort_between_destination_read_and_write_keeps_destination() {
        let mut bus = setup_bus(false);
        bus.write_u8(0x1000, 0x0F);
        bus.write_u8(0x2000, 0xF0);
        configure_channel0(&mut bus, 0x1000, 0x2000, 1, 3 << CONTROL_ROP_SHIFT);
        bus.tick_devices(); // source read
        bus.tick_devices(); // destination read
        bus.write_u32_be(DMAC_BASE + 0x14, 1);
        assert_eq!(bus.read_u32_be(CH0 + 0x10), 0x34);
        assert_eq!(bus.read_u8(0x2000), 0xF0);
    }

    #[test]
    fn blit_rejects_bad_2d_settings_and_mmio_endpoints() {
        let mut bus = setup_bus(false);
        // (rows, src_stride, dst_stride, src, dst, control): configuration error, code 1.
        let bad = [
            (0, 4, 4, 0x1000, 0x2000, 2 | MODE_2D),
            (2, 2, 4, 0x1000, 0x2000, 2 | MODE_2D),
            (2, 4, 6, 0x1000, 0x2000, 2 | MODE_2D),
            (1, 0, 0, 0x8004_1024, 0x2000, (1 << 2) | KEY_EN),
            (1, 0, 0, 0x1000, 0x8004_1024, (1 << 4) | MODE_2D),
        ];
        for (rows, ss, ds, src, dst, control) in bad {
            set_blit0(&mut bus, rows, ss, ds, 0);
            configure_channel0(&mut bus, src, dst, 4, control);
            assert_eq!(
                bus.read_u32_be(CH0 + 0x10),
                0x14,
                "{rows} {ss} {ds} {src:#x}"
            );
            bus.write_u32_be(CH0 + 0x10, 4);
        }
        // In 1D mode ROWS and strides are ignored, so the bad values above do not matter.
        set_blit0(&mut bus, 0, 3, 3, 0x55);
        configure_channel0(&mut bus, 0x1000, 0x2000, 4, 2 | KEY_EN);
        assert_eq!(run_until_idle(&mut bus), STATUS_DONE);
    }

    #[test]
    fn blit_beats_get_no_mmio_endpoint_exceptions() {
        let mut bus = setup_bus(false);
        // The VPU FIFO is a valid plain 32-bit fixed destination, but not for BitBlt.
        set_blit0(&mut bus, 1, 0, 0, 0);
        configure_channel0(&mut bus, 0x1000, 0x8003_002C, 4, 2 | (1 << 4) | KEY_EN);
        bus.write_u32_be(0x1000, 1);
        assert_eq!(run_until_idle(&mut bus), 0x24);
        bus.write_u32_be(CH0 + 0x10, 4);
        // A 2D walk that leaves Main RAM stops with code 2 and keeps its progress.
        set_blit0(&mut bus, 3, 0, 0x0080_0000, 0);
        configure_channel0(&mut bus, 0x1000, 0x00FF_F000, 4, 2 | MODE_2D);
        assert_eq!(run_until_idle(&mut bus), 0x24);
        assert_eq!(bus.read_u32_be(BLIT0 + 0x10), 2);
        assert_eq!(bus.read_u32_be(CH0 + 0x14), 4);
    }

    #[test]
    fn abort_stops_at_the_next_beat_boundary() {
        let mut bus = setup_bus(false);
        bus.write_u8(0x100, 0x5A);
        configure_channel0(&mut bus, 0x100, 0x200, 1, 0);
        bus.tick_devices(); // Complete the source read, leaving the write pending.
        bus.write_u32_be(DMAC_BASE + 0x14, 1);

        let status = bus.read_u32_be(CH0 + 0x10);
        assert_eq!(status & STATUS_BUSY, 0);
        assert_eq!(bus.read_u8(0x200), 0);
        assert_ne!(bus.read_u32_be(CH0 + 0x10) & STATUS_ERROR, 0);
        assert_eq!((bus.read_u32_be(CH0 + 0x10) >> 4) & 0xF, 3);
    }
}
