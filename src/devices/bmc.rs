use std::cell::RefCell;
use std::rc::Rc;

use crate::bus::{Bus, Device};

pub const BMC_BASE: u32 = 0x8006_0000;
pub const BMC_SIZE: u32 = 0x1_0000;
const AGE_GRANTS: u16 = 256;

/// Shared arbitration state used by the MMIO register block and system bus.
pub struct Bmc {
    enabled: bool,
    priorities: [u8; 8],
    waited: [u16; 8],
    pending: u8,
    active: Option<u8>,
    next_master: u8,
    max_burst: u32,
}

impl Bmc {
    pub fn new() -> Self {
        Self {
            enabled: true,
            priorities: [0, 1, 2, 2, 2, 2, 2, 2],
            waited: [0; 8],
            pending: 0,
            active: None,
            next_master: 0,
            max_burst: 16,
        }
    }

    /// Select one requested master and account for arbitration wait time.
    pub fn grant(&mut self, requests: u8) -> Option<u8> {
        self.pending = requests;
        if requests == 0 {
            return None;
        }
        let eligible = if self.enabled { requests } else { requests & 1 };
        let eligible = if eligible == 0 {
            requests & 1
        } else {
            eligible
        };
        if eligible == 0 {
            return None;
        }

        // With one requester there is no priority/tie search, and no other
        // master accumulates wait time. Preserve all observable grant state.
        if requests.is_power_of_two() {
            let winner = requests.trailing_zeros() as u8;
            self.waited = [0; 8];
            self.active = Some(winner);
            self.next_master = (winner + 1) & 7;
            return Some(winner);
        }

        let mut best_priority = u8::MAX;
        let mut winner = None;
        for step in 0..8 {
            let id = (self.next_master + step) & 7;
            if eligible & (1 << id) == 0 {
                continue;
            }
            let effective = self.priorities[id as usize]
                .saturating_sub((self.waited[id as usize] / AGE_GRANTS).min(7) as u8);
            if effective < best_priority {
                best_priority = effective;
                winner = Some(id);
            }
        }
        let winner = winner?;
        for id in 0..8 {
            if requests & (1 << id) == 0 || id == winner {
                self.waited[id as usize] = 0;
            } else {
                self.waited[id as usize] = self.waited[id as usize].saturating_add(1);
            }
        }
        self.active = Some(winner);
        self.next_master = (winner + 1) & 7;
        Some(winner)
    }

    pub fn release(&mut self) {
        self.active = None;
    }

    pub(crate) fn claim_cpu(&mut self) {
        self.active = Some(0);
    }

    pub fn max_burst(&self) -> u32 {
        self.max_burst
    }

    pub fn set_priority(&mut self, master: usize, priority: u8) {
        if master < self.priorities.len() {
            self.priorities[master] = priority & 7;
        }
    }

    pub fn priority(&self, master: usize) -> u8 {
        self.priorities.get(master).copied().unwrap_or(7)
    }

    fn read_register(&self, addr: u32) -> u32 {
        match addr & !3 {
            0x000 => 0x424D_4301,
            0x004 => u32::from(self.enabled),
            0x008 => u32::from(self.active.is_some()) | (u32::from(self.active.is_some()) << 1),
            0x00C => self.active.map_or(u32::MAX, u32::from),
            0x010 => u32::from(self.pending),
            0x020..=0x03C => {
                let master = ((addr & !3) - 0x020) as usize / 4;
                u32::from(self.priorities[master])
            }
            0x040 => self.max_burst,
            _ => 0,
        }
    }

    fn write_byte(&mut self, addr: u32, value: u8) {
        let reg = addr & !3;
        let lane = addr & 3;
        let shift = (3 - lane) * 8;
        match reg {
            0x004 if lane == 3 => self.enabled = value & 1 != 0,
            0x020..=0x03C if lane == 3 => {
                let master = ((reg - 0x020) / 4) as usize;
                self.priorities[master] = value & 7;
            }
            0x040 => {
                let mask = !(0xFFu32 << shift);
                let next = (self.max_burst & mask) | (u32::from(value) << shift);
                self.max_burst = if next == 0 { 1 } else { next.clamp(1, 256) };
            }
            _ => {}
        }
    }
}

impl Default for Bmc {
    fn default() -> Self {
        Self::new()
    }
}

struct BmcRegisters(Rc<RefCell<Bmc>>);

impl Device for BmcRegisters {
    fn debug_read(&mut self, addr: u32) -> Option<u8> {
        let value = self.0.borrow().read_register(addr);
        Some((value >> ((3 - (addr & 3)) * 8)) as u8)
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
        BMC_SIZE
    }
}

pub fn connect_bmc(bus: &mut Bus) -> Rc<RefCell<Bmc>> {
    let bmc = Rc::new(RefCell::new(Bmc::new()));
    bus.add_device(
        BMC_BASE,
        "BMC MMIO",
        Box::new(BmcRegisters(Rc::clone(&bmc))),
    );
    bus.attach_bmc(Rc::clone(&bmc));
    bmc
}

#[cfg(test)]
mod tests {
    use super::*;

    // Deliberately keep the original general arbiter as a differential oracle.
    fn reference_grant(bmc: &mut Bmc, requests: u8) -> Option<u8> {
        bmc.pending = requests;
        let eligible = if bmc.enabled { requests } else { requests & 1 };
        let winner = (0..8u8)
            .map(|step| (bmc.next_master + step) & 7)
            .filter(|id| eligible & (1 << id) != 0)
            .min_by_key(|id| {
                bmc.priorities[*id as usize]
                    .saturating_sub((bmc.waited[*id as usize] / AGE_GRANTS).min(7) as u8)
            })?;
        for id in 0..8 {
            bmc.waited[id] = if requests & (1 << id) == 0 || id == winner as usize {
                0
            } else {
                bmc.waited[id].saturating_add(1)
            };
        }
        bmc.active = Some(winner);
        bmc.next_master = (winner + 1) & 7;
        Some(winner)
    }

    #[test]
    fn grant_matches_reference_for_all_masks_and_round_robin_positions() {
        for enabled in [false, true] {
            for next_master in 0..8 {
                for seed in 0..8usize {
                    for requests in 0..=u8::MAX {
                        let make = || Bmc {
                            enabled,
                            priorities: std::array::from_fn(|id| ((id + seed) & 7) as u8),
                            waited: std::array::from_fn(|id| {
                                [0, 255, 256, 511, 1791, 1792, u16::MAX - 1, u16::MAX]
                                    [(id + seed) & 7]
                            }),
                            pending: 0xff,
                            active: Some(6),
                            next_master,
                            max_burst: 16,
                        };
                        let mut actual = make();
                        let mut expected = make();
                        assert_eq!(
                            actual.grant(requests),
                            reference_grant(&mut expected, requests)
                        );
                        assert_eq!(actual.pending, expected.pending);
                        assert_eq!(actual.active, expected.active);
                        assert_eq!(actual.next_master, expected.next_master);
                        assert_eq!(actual.waited, expected.waited);
                    }
                }
            }
        }
    }

    #[test]
    fn chooses_highest_priority_then_round_robins_ties() {
        let mut bmc = Bmc::new();
        bmc.set_priority(0, 4);
        bmc.set_priority(1, 1);
        assert_eq!(bmc.grant(0b11), Some(1));
        bmc.release();

        bmc.set_priority(0, 2);
        bmc.set_priority(1, 2);
        assert_eq!(bmc.grant(0b11), Some(0));
        bmc.release();
        assert_eq!(bmc.grant(0b11), Some(1));
        bmc.release();
        assert_eq!(bmc.grant(0b11), Some(0));
    }

    #[test]
    fn promotes_a_waiting_low_priority_master() {
        let mut bmc = Bmc::new();
        bmc.set_priority(0, 0);
        bmc.set_priority(1, 7);

        let mut winner = None;
        for _ in 0..1800 {
            winner = bmc.grant(0b11);
            bmc.release();
            if winner == Some(1) {
                break;
            }
        }
        assert_eq!(winner, Some(1));
    }

    #[test]
    fn exposes_priority_and_identity_through_mmio() {
        let mut bus = Bus::new();
        connect_bmc(&mut bus);
        assert_eq!(bus.read_u32_be(BMC_BASE), 0x424D_4301);
        bus.write_u32_be(BMC_BASE + 0x020, 7);
        assert_eq!(bus.read_u32_be(BMC_BASE + 0x020), 7);
    }
}
