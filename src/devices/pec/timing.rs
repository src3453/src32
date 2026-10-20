use std::cell::{RefCell, RefMut};
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::bus::{Bus, Device};
use crate::sys::MASTER_CLOCK;

pub const PEC_TIMING_BASE: u32 = 0x8004_0180;
pub const PEC_TIMING_SIZE: u32 = 0x380;

const TIMER_ID: u32 = 0x180;
const TIMER_ENABLE: u32 = 0x184;
const TIMER_PENDING: u32 = 0x188;
const TIMER_ACK: u32 = 0x18C;
const RTC_CTRL: u32 = 0x200;
const RTC_STATUS: u32 = 0x204;
const RTC_SECONDS_LO: u32 = 0x208;
const RTC_SECONDS_HI: u32 = 0x20C;
const RTC_SUBSEC: u32 = 0x210;
const RTC_ALARM_LO: u32 = 0x214;
const RTC_ALARM_HI: u32 = 0x218;
const RTC_IRQ_CTRL: u32 = 0x21C;
const RTC_ACK: u32 = 0x220;
const TIMER_CHANNEL_BASE: u32 = 0x400;
const TIMER_CHANNEL_STRIDE: u32 = 0x40;
const TIMER_CHANNELS: usize = 4;
const TIMER_CTRL: u32 = 0x00;
const TIMER_STATUS: u32 = 0x04;
const TIMER_COUNT_LO: u32 = 0x08;
const TIMER_COUNT_HI: u32 = 0x0C;
const TIMER_COMPARE_LO: u32 = 0x10;
const TIMER_COMPARE_HI: u32 = 0x14;
const TIMER_RELOAD_LO: u32 = 0x18;
const TIMER_RELOAD_HI: u32 = 0x1C;
const TIMER_PRESCALER: u32 = 0x20;

const CTRL_MODE_MASK: u32 = 0x03;
const CTRL_ENABLE: u32 = 1 << 2;
const CTRL_IRQ_ENABLE: u32 = 1 << 3;
const CTRL_RESET: u32 = 1 << 4;
const RTC_ENABLE: u32 = 1;
const RTC_SET_ENABLE: u32 = 1 << 1;
const RTC_REALTIME: u32 = 1 << 2;
const RTC_ALARM_IRQ_ENABLE: u32 = 1;
const NANOS_PER_SECOND: i128 = 1_000_000_000;

#[derive(Clone, Copy, Default)]
struct HardwareTimer {
    control: u32,
    counter: u64,
    compare: u64,
    reload: u64,
    prescaler: u32,
    prescaler_phase: u64,
    pending: bool,
    compare_armed: bool,
}

impl HardwareTimer {
    fn enabled(&self) -> bool {
        self.control & CTRL_ENABLE != 0
    }

    fn irq_enabled(&self) -> bool {
        self.control & CTRL_IRQ_ENABLE != 0
    }

    fn mode(&self) -> u32 {
        self.control & CTRL_MODE_MASK
    }

    fn write_control(&mut self, value: u32) {
        self.control = value & (CTRL_MODE_MASK | CTRL_ENABLE | CTRL_IRQ_ENABLE);
        if value & CTRL_RESET != 0 {
            self.counter = 0;
            self.prescaler_phase = 0;
            self.pending = false;
            self.compare_armed = true;
        }
    }

    fn advance(&mut self, ticks: u64) {
        if ticks == 0 {
            return;
        }
        match self.mode() {
            0 => {
                if self.compare <= self.counter {
                    self.pending = true;
                    self.control &= !CTRL_ENABLE;
                } else if self.compare - self.counter <= ticks {
                    self.counter = self.compare;
                    self.pending = true;
                    self.control &= !CTRL_ENABLE;
                } else {
                    self.counter += ticks;
                }
            }
            1 => {
                self.counter = self.counter.saturating_add(ticks);
                if self.compare <= self.counter {
                    self.pending = true;
                    if self.reload == 0 {
                        self.control &= !CTRL_ENABLE;
                    } else {
                        let elapsed = u128::from(self.counter - self.compare);
                        let periods = elapsed / u128::from(self.reload) + 1;
                        let next = u128::from(self.compare) + periods * u128::from(self.reload);
                        if next > u128::from(u64::MAX) {
                            self.control &= !CTRL_ENABLE;
                        } else {
                            self.compare = next as u64;
                        }
                    }
                }
            }
            2 => {
                if self.compare_armed {
                    let distance = self.compare.wrapping_sub(self.counter);
                    let distance = if distance == 0 {
                        1_u128 << 64
                    } else {
                        u128::from(distance)
                    };
                    if u128::from(ticks) >= distance {
                        self.pending = true;
                        self.compare_armed = false;
                    }
                }
                self.counter = self.counter.wrapping_add(ticks);
            }
            _ => {}
        }
    }

    fn cycles_until_match(&self) -> Option<u64> {
        if !self.enabled() || !self.irq_enabled() || self.pending {
            return None;
        }
        let ticks = match self.mode() {
            0 | 1 => Some(if self.compare > self.counter {
                u128::from(self.compare - self.counter)
            } else {
                1
            }),
            2 if self.compare_armed => {
                let distance = self.compare.wrapping_sub(self.counter);
                Some(if distance == 0 {
                    1_u128 << 64
                } else {
                    u128::from(distance)
                })
            }
            _ => None,
        }?;
        let divisor = u128::from(self.prescaler) + 1;
        let first_tick = divisor - u128::from(self.prescaler_phase);
        let cycles = first_tick + (ticks - 1) * divisor;
        Some(cycles.min(u128::from(u64::MAX)) as u64)
    }
}

#[derive(Clone, Copy)]
struct ClockTime {
    seconds: u64,
    nanoseconds: u32,
}

/// Shared PeC timing state. The system advances this by master-clock cycles,
/// independent of how many instructions the CPU has retired.
pub(crate) struct PeCTimingState {
    timers: [HardwareTimer; TIMER_CHANNELS],
    timer_enable: u8,
    rtc_control: u32,
    rtc_seconds: u64,
    rtc_subsecond_cycles: u64,
    rtc_host_offset_ns: i128,
    rtc_irq_control: u32,
    rtc_alarm_seconds: u64,
    rtc_alarm_pending: bool,
    rtc_alarm_armed: bool,
    rtc_seconds_shadow: u64,
    rtc_seconds_write_mask: u8,
    rtc_host_alarm_poll_cycles: u64,
    rtc_alarm_shadow: u64,
    rtc_alarm_write_mask: u8,
}

impl Default for PeCTimingState {
    fn default() -> Self {
        Self {
            timers: [HardwareTimer::default(); TIMER_CHANNELS],
            timer_enable: 0,
            rtc_control: RTC_ENABLE,
            rtc_seconds: 0,
            rtc_subsecond_cycles: 0,
            rtc_host_offset_ns: 0,
            rtc_irq_control: 0,
            rtc_alarm_seconds: 0,
            rtc_alarm_pending: false,
            rtc_alarm_armed: false,
            rtc_seconds_shadow: 0,
            rtc_seconds_write_mask: 0,
            rtc_host_alarm_poll_cycles: 0,
            rtc_alarm_shadow: 0,
            rtc_alarm_write_mask: 0,
        }
    }
}

impl PeCTimingState {
    pub(crate) fn advance_system_cycles(&mut self, cycles: u64) {
        let cycle_count = u128::from(cycles);
        if self.timer_enable != 0 {
            for (channel, timer) in self.timers.iter_mut().enumerate() {
                if timer.enabled() && self.timer_enable & (1 << channel) != 0 {
                    // Per-cycle advances never overflow u64; avoid the u128
                    // runtime division on the 48 MHz device path.
                    let divisor = u64::from(timer.prescaler) + 1;
                    if let Some(accumulated) = timer.prescaler_phase.checked_add(cycles) {
                        if accumulated < divisor {
                            timer.prescaler_phase = accumulated;
                        } else {
                            timer.prescaler_phase = accumulated % divisor;
                            timer.advance(accumulated / divisor);
                        }
                        continue;
                    }
                    let divisor = u128::from(divisor);
                    let accumulated = u128::from(timer.prescaler_phase) + cycle_count;
                    let ticks = accumulated / divisor;
                    timer.prescaler_phase = (accumulated % divisor) as u64;
                    timer.advance(ticks.min(u128::from(u64::MAX)) as u64);
                }
            }
        }

        let rtc_fast = cycles != 0
            && self.rtc_enabled()
            && !self.rtc_realtime()
            && self
                .rtc_subsecond_cycles
                .checked_add(cycles)
                .is_some_and(|accumulated| accumulated < u64::from(MASTER_CLOCK));
        if rtc_fast {
            // Same result as the general path when no second boundary is crossed.
            self.rtc_subsecond_cycles += cycles;
        } else if cycles != 0 && self.rtc_enabled() && !self.rtc_realtime() {
            let frequency = u128::from(MASTER_CLOCK);
            let accumulated = u128::from(self.rtc_subsecond_cycles) + cycle_count;
            let elapsed_seconds = accumulated / frequency;
            let remaining_seconds = u128::from(u64::MAX - self.rtc_seconds);
            if elapsed_seconds > remaining_seconds {
                self.rtc_seconds = u64::MAX;
                self.rtc_subsecond_cycles = 0;
            } else {
                self.rtc_seconds += elapsed_seconds as u64;
                self.rtc_subsecond_cycles = (accumulated % frequency) as u64;
            }
        }

        if self.rtc_enabled()
            && self.rtc_realtime()
            && self.rtc_alarm_armed
            && !self.rtc_alarm_pending
            && cycles != 0
        {
            let poll_cycles = u64::from(MASTER_CLOCK) / 1_000;
            let accumulated = u128::from(self.rtc_host_alarm_poll_cycles) + cycle_count;
            if accumulated >= u128::from(poll_cycles) {
                self.rtc_host_alarm_poll_cycles = (accumulated % u128::from(poll_cycles)) as u64;
                self.check_rtc_alarm();
            } else {
                self.rtc_host_alarm_poll_cycles = accumulated as u64;
            }
        } else {
            self.rtc_host_alarm_poll_cycles = 0;
            self.check_rtc_alarm();
        }
    }

    pub(crate) fn irq_asserted(&self) -> bool {
        self.timers
            .iter()
            .any(|timer| timer.pending && timer.irq_enabled())
            || (self.rtc_alarm_pending && self.rtc_irq_control & RTC_ALARM_IRQ_ENABLE != 0)
    }

    pub(crate) fn cycles_until_irq(&self) -> Option<u64> {
        let timer_cycles = self
            .timers
            .iter()
            .enumerate()
            .filter(|(channel, _)| self.timer_enable & (1 << channel) != 0)
            .filter_map(|(_, timer)| timer.cycles_until_match())
            .min();
        let rtc_cycles = if self.rtc_enabled()
            && !self.rtc_realtime()
            && !self.rtc_alarm_pending
            && self.rtc_alarm_armed
            && self.rtc_irq_control & RTC_ALARM_IRQ_ENABLE != 0
        {
            let seconds_to_alarm = self.rtc_alarm_seconds.saturating_sub(self.rtc_seconds);
            let until_alarm = if seconds_to_alarm == 0 {
                1
            } else {
                u128::from(seconds_to_alarm) * u128::from(MASTER_CLOCK)
                    - u128::from(self.rtc_subsecond_cycles)
            };
            Some(until_alarm.min(u128::from(u64::MAX)) as u64)
        } else {
            None
        };
        match (timer_cycles, rtc_cycles) {
            (Some(timer), Some(rtc)) => Some(timer.min(rtc)),
            (Some(timer), None) => Some(timer),
            (None, Some(rtc)) => Some(rtc),
            (None, None) => None,
        }
    }

    fn rtc_enabled(&self) -> bool {
        self.rtc_control & RTC_ENABLE != 0
    }

    fn rtc_realtime(&self) -> bool {
        self.rtc_control & RTC_REALTIME != 0
    }

    fn host_time_ns() -> i128 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos().min(i128::MAX as u128) as i128)
            .unwrap_or(0)
    }

    fn internal_time_ns(&self) -> i128 {
        i128::from(self.rtc_seconds) * NANOS_PER_SECOND
            + (u128::from(self.rtc_subsecond_cycles) * 1_000_000_000 / u128::from(MASTER_CLOCK))
                as i128
    }

    fn current_time_ns(&self) -> i128 {
        if self.rtc_enabled() && self.rtc_realtime() {
            (Self::host_time_ns() + self.rtc_host_offset_ns).max(0)
        } else {
            self.internal_time_ns()
        }
    }

    fn set_internal_time_ns(&mut self, value: i128) {
        let value = value.max(0) as u128;
        let seconds = value / 1_000_000_000;
        if seconds > u128::from(u64::MAX) {
            self.rtc_seconds = u64::MAX;
            self.rtc_subsecond_cycles = 0;
        } else {
            self.rtc_seconds = seconds as u64;
            let nanoseconds = value % 1_000_000_000;
            self.rtc_subsecond_cycles =
                (nanoseconds * u128::from(MASTER_CLOCK) / 1_000_000_000) as u64;
        }
        self.rtc_seconds_shadow = self.rtc_seconds;
    }

    fn current_clock_time(&self) -> ClockTime {
        let now = self.current_time_ns() as u128;
        ClockTime {
            seconds: (now / 1_000_000_000).min(u128::from(u64::MAX)) as u64,
            nanoseconds: (now % 1_000_000_000) as u32,
        }
    }

    fn check_rtc_alarm(&mut self) {
        if self.rtc_enabled()
            && self.rtc_alarm_armed
            && !self.rtc_alarm_pending
            && self.current_clock_time().seconds >= self.rtc_alarm_seconds
        {
            self.rtc_alarm_pending = true;
            self.rtc_alarm_armed = false;
        }
    }

    fn write_rtc_control(&mut self, value: u32) {
        let old_control = self.rtc_control;
        let old_time = self.current_time_ns();
        let new_control = value & (RTC_ENABLE | RTC_SET_ENABLE | RTC_REALTIME);
        let old_enabled = old_control & RTC_ENABLE != 0;
        let new_enabled = new_control & RTC_ENABLE != 0;
        let old_realtime = old_control & RTC_REALTIME != 0;
        let new_realtime = new_control & RTC_REALTIME != 0;

        if old_enabled && !new_enabled {
            self.set_internal_time_ns(old_time);
        }
        if new_enabled && new_realtime && (!old_enabled || !old_realtime) {
            self.rtc_host_offset_ns = old_time - Self::host_time_ns();
        } else if new_enabled && !new_realtime && old_realtime {
            self.set_internal_time_ns(old_time);
        }
        self.rtc_control = new_control;
        self.check_rtc_alarm();
    }

    fn write_rtc_seconds(&mut self, byte: u32, value: u8) {
        if self.rtc_control & RTC_SET_ENABLE == 0 {
            return;
        }
        update_u64_byte(&mut self.rtc_seconds_shadow, byte, value);
        self.rtc_seconds_write_mask |= 1 << byte;
        if self.rtc_seconds_write_mask == u8::MAX {
            let seconds = self.rtc_seconds_shadow;
            let value_ns = i128::from(seconds) * NANOS_PER_SECOND;
            if self.rtc_enabled() && self.rtc_realtime() {
                self.rtc_host_offset_ns = value_ns - Self::host_time_ns();
                self.rtc_seconds = seconds;
                self.rtc_subsecond_cycles = 0;
            } else {
                self.rtc_seconds = seconds;
                self.rtc_subsecond_cycles = 0;
            }
            self.rtc_seconds_write_mask = 0;
            self.check_rtc_alarm();
        }
    }

    fn write_rtc_alarm(&mut self, byte: u32, value: u8) {
        update_u64_byte(&mut self.rtc_alarm_shadow, byte, value);
        self.rtc_alarm_write_mask |= 1 << byte;
        if self.rtc_alarm_write_mask == u8::MAX {
            self.rtc_alarm_seconds = self.rtc_alarm_shadow;
            self.rtc_alarm_pending = false;
            self.rtc_alarm_armed = true;
            self.rtc_alarm_write_mask = 0;
            self.check_rtc_alarm();
        }
    }

    fn timer_pending(&self) -> u32 {
        self.timers
            .iter()
            .enumerate()
            .fold(0, |mask, (channel, timer)| {
                mask | (u32::from(timer.pending) << channel)
            })
    }

    fn read_register(&mut self, offset: u32) -> u32 {
        self.check_rtc_alarm();
        match offset {
            TIMER_ID => 0x5154_0001,
            TIMER_ENABLE => u32::from(self.timer_enable),
            TIMER_PENDING => self.timer_pending(),
            TIMER_ACK => 0,
            RTC_CTRL => self.rtc_control,
            RTC_STATUS => {
                u32::from(self.rtc_alarm_pending) | (u32::from(self.rtc_alarm_armed) << 1)
            }
            RTC_SECONDS_LO => self.current_clock_time().seconds as u32,
            RTC_SECONDS_HI => (self.current_clock_time().seconds >> 32) as u32,
            RTC_SUBSEC => self.current_clock_time().nanoseconds,
            RTC_ALARM_LO => self.rtc_alarm_seconds as u32,
            RTC_ALARM_HI => (self.rtc_alarm_seconds >> 32) as u32,
            RTC_IRQ_CTRL => self.rtc_irq_control,
            RTC_ACK => 0,
            _ if (TIMER_CHANNEL_BASE
                ..TIMER_CHANNEL_BASE + TIMER_CHANNELS as u32 * TIMER_CHANNEL_STRIDE)
                .contains(&offset) =>
            {
                let channel = ((offset - TIMER_CHANNEL_BASE) / TIMER_CHANNEL_STRIDE) as usize;
                let register = (offset - TIMER_CHANNEL_BASE) % TIMER_CHANNEL_STRIDE;
                let timer = &self.timers[channel];
                match register {
                    TIMER_CTRL => timer.control,
                    TIMER_STATUS => u32::from(timer.enabled()) | (u32::from(timer.pending) << 1),
                    TIMER_COUNT_LO => timer.counter as u32,
                    TIMER_COUNT_HI => (timer.counter >> 32) as u32,
                    TIMER_COMPARE_LO => timer.compare as u32,
                    TIMER_COMPARE_HI => (timer.compare >> 32) as u32,
                    TIMER_RELOAD_LO => timer.reload as u32,
                    TIMER_RELOAD_HI => (timer.reload >> 32) as u32,
                    TIMER_PRESCALER => timer.prescaler,
                    _ => 0,
                }
            }
            _ => 0,
        }
    }

    fn write_byte(&mut self, offset: u32, value: u8) {
        let register = offset & !3;
        let byte = offset & 3;
        let shift = (3 - byte) * 8;
        match register {
            TIMER_ENABLE => {
                let mask = 0xFF << shift;
                self.timer_enable = ((u32::from(self.timer_enable) & !mask)
                    | (u32::from(value) << shift)) as u8
                    & ((1 << TIMER_CHANNELS) - 1);
            }
            TIMER_ACK => {
                let clear = u32::from(value) << shift;
                for (channel, timer) in self.timers.iter_mut().enumerate() {
                    if clear & (1 << channel) != 0 {
                        timer.pending = false;
                    }
                }
            }
            RTC_CTRL => {
                let next = merge_u32_byte(self.rtc_control, byte, value);
                self.write_rtc_control(next);
            }
            RTC_IRQ_CTRL => {
                self.rtc_irq_control =
                    merge_u32_byte(self.rtc_irq_control, byte, value) & RTC_ALARM_IRQ_ENABLE
            }
            RTC_ACK if byte == 3 && value & 1 != 0 => self.rtc_alarm_pending = false,
            RTC_SECONDS_LO | RTC_SECONDS_HI => {
                let byte_index = if register == RTC_SECONDS_HI {
                    byte
                } else {
                    byte + 4
                };
                self.write_rtc_seconds(byte_index, value);
            }
            RTC_ALARM_LO | RTC_ALARM_HI => {
                let byte_index = if register == RTC_ALARM_HI {
                    byte
                } else {
                    byte + 4
                };
                self.write_rtc_alarm(byte_index, value);
            }
            _ if (TIMER_CHANNEL_BASE
                ..TIMER_CHANNEL_BASE + TIMER_CHANNELS as u32 * TIMER_CHANNEL_STRIDE)
                .contains(&register) =>
            {
                let channel = ((register - TIMER_CHANNEL_BASE) / TIMER_CHANNEL_STRIDE) as usize;
                let channel_register = (register - TIMER_CHANNEL_BASE) % TIMER_CHANNEL_STRIDE;
                let timer = &mut self.timers[channel];
                match channel_register {
                    TIMER_CTRL => {
                        let next = merge_u32_byte(timer.control, byte, value);
                        timer.write_control(next);
                    }
                    TIMER_COUNT_LO => update_u64_byte(&mut timer.counter, byte + 4, value),
                    TIMER_COUNT_HI => update_u64_byte(&mut timer.counter, byte, value),
                    TIMER_COMPARE_LO => {
                        update_u64_byte(&mut timer.compare, byte + 4, value);
                        timer.compare_armed = true;
                    }
                    TIMER_COMPARE_HI => {
                        update_u64_byte(&mut timer.compare, byte, value);
                        timer.compare_armed = true;
                    }
                    TIMER_RELOAD_LO => update_u64_byte(&mut timer.reload, byte + 4, value),
                    TIMER_RELOAD_HI => update_u64_byte(&mut timer.reload, byte, value),
                    TIMER_PRESCALER => {
                        timer.prescaler = merge_u32_byte(timer.prescaler, byte, value);
                        timer.prescaler_phase = 0;
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

fn merge_u32_byte(value: u32, byte: u32, next: u8) -> u32 {
    let shift = (3 - byte) * 8;
    (value & !(0xFF << shift)) | (u32::from(next) << shift)
}

fn update_u64_byte(value: &mut u64, byte: u32, next: u8) {
    let shift = if byte < 4 {
        32 + (3 - byte) * 8
    } else {
        (7 - byte) * 8
    };
    *value = (*value & !(0xFF_u64 << shift)) | (u64::from(next) << shift);
}

struct PeCTimingDevice {
    state: Rc<RefCell<PeCTimingState>>,
    count_hi_latch: [Option<u32>; TIMER_CHANNELS],
    rtc_seconds_lo_latch: Option<u32>,
    rtc_seconds_hi_latch: Option<u32>,
}

impl Device for PeCTimingDevice {
    fn read(&mut self, addr: u32) -> u8 {
        let offset = addr + 0x180;
        if (TIMER_CHANNEL_BASE..TIMER_CHANNEL_BASE + TIMER_CHANNELS as u32 * TIMER_CHANNEL_STRIDE)
            .contains(&offset)
        {
            let channel = ((offset - TIMER_CHANNEL_BASE) / TIMER_CHANNEL_STRIDE) as usize;
            let channel_base = TIMER_CHANNEL_BASE + channel as u32 * TIMER_CHANNEL_STRIDE;
            if offset == channel_base + TIMER_COUNT_LO {
                let state = borrow_timing_state(&self.state);
                self.count_hi_latch[channel] = Some((state.timers[channel].counter >> 32) as u32);
            }
            let count_hi = channel_base + TIMER_COUNT_HI;
            if offset >= count_hi && offset < count_hi + 4 {
                if let Some(value) = self.count_hi_latch[channel] {
                    let byte = offset - count_hi;
                    let result = (value >> ((3 - byte) * 8)) as u8;
                    if byte == 3 {
                        self.count_hi_latch[channel] = None;
                    }
                    return result;
                }
            }
        }
        if offset == RTC_SECONDS_LO {
            let state = borrow_timing_state(&self.state);
            let seconds = state.current_clock_time().seconds;
            self.rtc_seconds_lo_latch = Some(seconds as u32);
            self.rtc_seconds_hi_latch = Some((seconds >> 32) as u32);
        }
        if offset >= RTC_SECONDS_LO && offset < RTC_SECONDS_LO + 4 {
            if let Some(value) = self.rtc_seconds_lo_latch {
                let result = (value >> ((3 - (offset & 3)) * 8)) as u8;
                if offset == RTC_SECONDS_LO + 3 {
                    self.rtc_seconds_lo_latch = None;
                }
                return result;
            }
        }
        if offset >= RTC_SECONDS_HI && offset < RTC_SECONDS_HI + 4 {
            if let Some(value) = self.rtc_seconds_hi_latch {
                let result = (value >> ((3 - (offset & 3)) * 8)) as u8;
                if offset == RTC_SECONDS_HI + 3 {
                    self.rtc_seconds_hi_latch = None;
                }
                return result;
            }
        }

        let register = offset & !3;
        let shift = (3 - (offset & 3)) * 8;
        let mut state = borrow_timing_state(&self.state);
        (state.read_register(register) >> shift) as u8
    }

    fn write(&mut self, addr: u32, value: u8) {
        borrow_timing_state(&self.state).write_byte(addr + 0x180, value);
    }

    fn size(&self) -> u32 {
        PEC_TIMING_SIZE
    }
}

fn borrow_timing_state(state: &RefCell<PeCTimingState>) -> RefMut<'_, PeCTimingState> {
    state.borrow_mut()
}

/// Register the PeC 48 MHz quad timer and RTC MMIO block.
pub fn connect_pec_timer_rtc(bus: &mut Bus) {
    let state = Rc::new(RefCell::new(PeCTimingState::default()));
    bus.attach_pec_timing(Rc::clone(&state));
    bus.add_device(
        PEC_TIMING_BASE,
        "PeC Timer/RTC MMIO",
        Box::new(PeCTimingDevice {
            state,
            count_hi_latch: [None; TIMER_CHANNELS],
            rtc_seconds_lo_latch: None,
            rtc_seconds_hi_latch: None,
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::irqc::irqc::connect_irqc;
    use std::rc::Rc;

    #[test]
    fn single_cycle_advances_match_one_batched_advance() {
        let make = || {
            let mut state = PeCTimingState::default();
            state.rtc_subsecond_cycles = u64::from(MASTER_CLOCK) - 5;
            state.timer_enable = 0b11;
            state.timers[0] = HardwareTimer {
                control: 1 | CTRL_ENABLE,
                compare: u64::MAX,
                prescaler: 2,
                prescaler_phase: 1,
                ..HardwareTimer::default()
            };
            state.timers[1] = HardwareTimer {
                control: CTRL_ENABLE,
                compare: 4,
                prescaler: 0,
                ..HardwareTimer::default()
            };
            state
        };
        let mut stepped = make();
        for _ in 0..17 {
            stepped.advance_system_cycles(1);
        }
        let mut batched = make();
        batched.advance_system_cycles(17);
        assert_eq!(stepped.rtc_seconds, 1);
        assert_eq!(stepped.rtc_subsecond_cycles, 12);
        assert_eq!(
            (stepped.rtc_seconds, stepped.rtc_subsecond_cycles),
            (batched.rtc_seconds, batched.rtc_subsecond_cycles)
        );
        for (stepped, batched) in stepped.timers.iter().zip(&batched.timers) {
            assert_eq!(stepped.counter, batched.counter);
            assert_eq!(stepped.prescaler_phase, batched.prescaler_phase);
            assert_eq!(stepped.pending, batched.pending);
            assert_eq!(stepped.control, batched.control);
        }
        assert_eq!(stepped.timers[0].counter, 6);
        assert!(stepped.timers[1].pending);

        // Overflowing sums still take the exact u128 path.
        let mut huge = make();
        huge.advance_system_cycles(u64::MAX);
        assert_eq!(
            huge.timers[0].counter,
            ((u128::from(u64::MAX) + 1) / 3) as u64
        );
        assert_eq!(huge.timers[0].prescaler_phase, 1);
    }

    fn timing_device() -> PeCTimingDevice {
        PeCTimingDevice {
            state: Rc::new(RefCell::new(PeCTimingState::default())),
            count_hi_latch: [None; TIMER_CHANNELS],
            rtc_seconds_lo_latch: None,
            rtc_seconds_hi_latch: None,
        }
    }

    fn read_be(device: &mut PeCTimingDevice, register: u32) -> u32 {
        (0..4).fold(0, |value, byte| {
            (value << 8) | u32::from(device.read(register - 0x180 + byte))
        })
    }

    fn write_be(device: &mut PeCTimingDevice, register: u32, value: u32) {
        for byte in 0..4 {
            device.write(register - 0x180 + byte, (value >> ((3 - byte) * 8)) as u8);
        }
    }

    #[test]
    fn rtc_seconds_low_read_latches_high_word_across_rollover() {
        let mut device = timing_device();
        {
            let mut state = borrow_timing_state(&device.state);
            state.rtc_seconds = u64::from(u32::MAX);
            state.rtc_subsecond_cycles = u64::from(MASTER_CLOCK) - 1;
        }
        let low = read_be(&mut device, RTC_SECONDS_LO);
        borrow_timing_state(&device.state).advance_system_cycles(1);
        let high = read_be(&mut device, RTC_SECONDS_HI);

        assert_eq!(low, u32::MAX);
        assert_eq!(high, 0);
        assert_eq!(
            borrow_timing_state(&device.state).rtc_seconds,
            0x1_0000_0000
        );
    }

    #[test]
    fn rtc_can_switch_to_host_time_and_set_its_epoch_offset() {
        let mut device = timing_device();
        write_be(&mut device, RTC_CTRL, RTC_ENABLE | RTC_SET_ENABLE);
        write_be(&mut device, RTC_SECONDS_HI, u32::MAX);
        write_be(&mut device, RTC_SECONDS_LO, u32::MAX);
        write_be(&mut device, RTC_CTRL, RTC_ENABLE | RTC_REALTIME);

        assert_eq!(read_be(&mut device, RTC_SECONDS_HI), u32::MAX);
        assert_eq!(read_be(&mut device, RTC_SECONDS_LO), u32::MAX);
    }

    fn bus_addr(register: u32) -> u32 {
        PEC_TIMING_BASE + register - 0x180
    }

    #[test]
    fn timer_uses_master_cycles_prescaler_and_sticky_irq_status() {
        let mut device = timing_device();
        write_be(&mut device, TIMER_ENABLE, 1);
        write_be(&mut device, TIMER_CHANNEL_BASE + TIMER_COMPARE_LO, 3);
        write_be(&mut device, TIMER_CHANNEL_BASE + TIMER_PRESCALER, 1);
        write_be(
            &mut device,
            TIMER_CHANNEL_BASE + TIMER_CTRL,
            CTRL_ENABLE | CTRL_IRQ_ENABLE,
        );
        {
            let mut state = borrow_timing_state(&device.state);
            state.advance_system_cycles(5);
        }
        assert_eq!(read_be(&mut device, TIMER_CHANNEL_BASE + TIMER_COUNT_LO), 2);
        assert_eq!(read_be(&mut device, TIMER_PENDING), 0);
        borrow_timing_state(&device.state).advance_system_cycles(2);
        assert_eq!(read_be(&mut device, TIMER_CHANNEL_BASE + TIMER_COUNT_LO), 3);
        assert_eq!(read_be(&mut device, TIMER_PENDING), 1);
        assert_eq!(read_be(&mut device, TIMER_CHANNEL_BASE + TIMER_STATUS), 2);
        write_be(&mut device, TIMER_ACK, 1);
        assert_eq!(read_be(&mut device, TIMER_PENDING), 0);
    }

    #[test]
    fn periodic_timer_keeps_deadlines_anchored_to_the_period() {
        let mut state = PeCTimingState::default();
        state.timer_enable = 1;
        state.timers[0].control = CTRL_ENABLE | (1 << CTRL_MODE_MASK.trailing_zeros());
        state.timers[0].compare = 5;
        state.timers[0].reload = 4;
        state.advance_system_cycles(14);
        assert_eq!(state.timers[0].counter, 14);
        assert_eq!(state.timers[0].compare, 17);
        assert!(state.timers[0].pending);
        assert!(state.timers[0].enabled());
    }

    #[test]
    fn timer_low_read_latches_high_word_across_rollover() {
        let mut device = timing_device();
        write_be(&mut device, TIMER_ENABLE, 1);
        write_be(&mut device, TIMER_CHANNEL_BASE + TIMER_COUNT_HI, 1);
        write_be(&mut device, TIMER_CHANNEL_BASE + TIMER_COUNT_LO, u32::MAX);
        write_be(
            &mut device,
            TIMER_CHANNEL_BASE + TIMER_CTRL,
            2 | CTRL_ENABLE,
        );
        let low = read_be(&mut device, TIMER_CHANNEL_BASE + TIMER_COUNT_LO);
        assert_eq!(device.count_hi_latch[0], Some(1));
        borrow_timing_state(&device.state).advance_system_cycles(1);
        assert_eq!(device.count_hi_latch[0], Some(1));
        let high = read_be(&mut device, TIMER_CHANNEL_BASE + TIMER_COUNT_HI);
        assert_eq!(device.count_hi_latch[0], None);
        assert_eq!(low, u32::MAX);
        assert_eq!(high, 1);
        assert_eq!(
            borrow_timing_state(&device.state).timers[0].counter,
            0x2_0000_0000
        );
    }

    #[test]
    fn rtc_tracks_virtual_master_time_and_fires_one_shot_alarm_on_irq4() {
        let mut bus = Bus::new();
        connect_pec_timer_rtc(&mut bus);
        let irq = connect_irqc(&mut bus);
        bus.attach_irq_controller(Rc::clone(&irq));
        irq.borrow_mut().set_enable_mask(1 << 4);

        bus.write_u32_be(bus_addr(RTC_CTRL), RTC_ENABLE | RTC_SET_ENABLE);
        bus.write_u32_be(bus_addr(RTC_SECONDS_HI), 0);
        bus.write_u32_be(bus_addr(RTC_SECONDS_LO), 40);
        bus.write_u32_be(bus_addr(RTC_ALARM_HI), 0);
        bus.write_u32_be(bus_addr(RTC_ALARM_LO), 41);
        bus.write_u32_be(bus_addr(RTC_IRQ_CTRL), RTC_ALARM_IRQ_ENABLE);
        bus.write_u32_be(bus_addr(RTC_CTRL), RTC_ENABLE);

        bus.advance_system_cycles(u64::from(MASTER_CLOCK) - 1);
        assert_eq!(bus.read_u32_be(bus_addr(RTC_SECONDS_LO)), 40);
        assert_eq!(bus.read_u32_be(bus_addr(RTC_STATUS)) & 1, 0);
        assert_eq!(bus.read_u32_be(bus_addr(RTC_SUBSEC)), 999_999_979);
        bus.advance_system_cycles(1);
        assert_eq!(bus.read_u32_be(bus_addr(RTC_SECONDS_LO)), 41);
        assert_eq!(bus.read_u32_be(bus_addr(RTC_STATUS)) & 1, 1);
        assert_ne!(irq.borrow().pending() & (1 << 4), 0);

        bus.write_u32_be(bus_addr(RTC_ACK), 1);
        assert_eq!(bus.read_u32_be(bus_addr(RTC_STATUS)) & 1, 0);
        bus.advance_system_cycles(1);
        assert_eq!(bus.read_u32_be(bus_addr(RTC_STATUS)) & 1, 0);
    }

    #[test]
    fn halted_cycle_skip_stops_at_timer_deadline() {
        let mut bus = Bus::new();
        connect_pec_timer_rtc(&mut bus);
        bus.write_u32_be(bus_addr(TIMER_ENABLE), 1);
        bus.write_u32_be(bus_addr(TIMER_CHANNEL_BASE + TIMER_COMPARE_LO), 7);
        bus.write_u32_be(
            bus_addr(TIMER_CHANNEL_BASE + TIMER_CTRL),
            CTRL_ENABLE | CTRL_IRQ_ENABLE,
        );
        assert_eq!(bus.cycles_until_pec_irq(), Some(7));
        bus.advance_system_cycles(7);
        assert_eq!(bus.read_u32_be(bus_addr(TIMER_PENDING)), 1);
        assert_eq!(bus.cycles_until_pec_irq(), None);
    }
}
