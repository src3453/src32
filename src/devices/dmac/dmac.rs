use std::cell::RefCell;
use std::rc::Rc;

use crate::bus::{Bus, Device};
use crate::devices::bmc::Bmc;

use crate::devices::pec::idc::idc::{DmaDirection, IDC_BASE, IDC_DATA_ADDRESS, IDC_SIZE, Idc};
pub const DMAC_BASE: u32 = 0x8005_0000;
pub const DMAC_SIZE: u32 = 0x1_0000;
const CHANNEL_COUNT: usize = 6;
const CHANNEL_BASE: u32 = 0x100;
const CHANNEL_STRIDE: u32 = 0x20;

fn is_idc_address(address: u32) -> bool {
    address >= IDC_BASE && address < IDC_BASE + IDC_SIZE
}

const STATUS_BUSY: u32 = 1;
const STATUS_DONE: u32 = 1 << 1;
const STATUS_ERROR: u32 = 1 << 2;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Phase {
    Read,
    Write,
}

#[derive(Clone, Copy)]
pub(crate) struct DmaOp {
    pub address: u32,
    pub width: u8,
    pub write: bool,
    pub value: u32,
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
        match channel.phase {
            Phase::Read => Some(DmaOp {
                address: channel.src_current,
                width,
                write: false,
                value: 0,
            }),
            Phase::Write => Some(DmaOp {
                address: channel.dst_current,
                width,
                write: true,
                value: channel.buffer,
            }),
        }
    }

    pub(crate) fn complete_op(&mut self, index: usize, value: u32) {
        let (idc_endpoint, done) = {
            let channel = &mut self.channels[index];
            if channel.phase == Phase::Read {
                channel.buffer = value;
                channel.phase = Phase::Write;
                return;
            }
            let width = channel.width().unwrap_or(1) as u32;
            if channel.control & (1 << 2) == 0 {
                channel.src_current = channel.src_current.wrapping_add(width);
            }
            if channel.control & (1 << 4) == 0 {
                channel.dst_current = channel.dst_current.wrapping_add(width);
            }
            channel.remain = channel.remain.saturating_sub(width);
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
        channel.remain = channel.count;
        channel.phase = Phase::Read;
        channel.status = STATUS_BUSY;
    }

    fn abort(&mut self, index: usize) {
        if self.channels[index].status & STATUS_BUSY != 0 {
            self.fail(index, 3);
        }
    }

    fn read_register(&self, addr: u32) -> u32 {
        match addr & !3 {
            0x000 => 0x444D_4101,
            0x004 => u32::from(self.enabled),
            0x008 => u32::from(self.channel_enable),
            0x00C => u32::from(self.irq_enable),
            0x010 => u32::from(self.irq_status),
            0x014 => 0,
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
                    channel.control = (channel.control & !(0xFF << shift)) | byte_value;
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
