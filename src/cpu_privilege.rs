//! Extension P: privilege levels and system calls (doc/spec_CPU_priv.md).
//!
//! Every CPU kind keeps its trap state in one `SystemState` so trap entry,
//! IRET, system registers and user-mode protection behave identically.

use crate::cpu::InstructionMode;
use crate::devices::cpu::{BUS_ERROR_VECTOR, ILLEGAL_INSTRUCTION_VECTOR, SYSCALL_VECTOR};

/// CPUID feature bit for Extension P.
pub const EXT_P: u32 = 0x100;

pub const STATUS_IE: u32 = 1 << 0;
pub const STATUS_PIE: u32 = 1 << 1;
pub const STATUS_U: u32 = 1 << 2;
pub const STATUS_PU: u32 = 1 << 3;
pub const STATUS_PSM: u32 = 1 << 4;

pub const SR_STATUS: u16 = 0x00;
pub const SR_EPC: u16 = 0x01;
pub const SR_CAUSE: u16 = 0x02;
pub const SR_USP: u16 = 0x08;
pub const SR_UFP: u16 = 0x09;
pub const SR_UGP: u16 = 0x0A;
/// Region `n` uses SR `0x10 + 4n` (START), `+1` (END) and `+2` (FLAGS).
pub const SR_REGION_BASE: u16 = 0x10;
pub const REGION_COUNT: usize = 8;

/// Set in CAUSE for every exception; IRQs leave it clear (CAUSE = 0..15).
pub const CAUSE_EXCEPTION: u32 = 0x8000_0000;

/// User code can never touch vectors, IRQC or CPU state at or above this.
pub const CPU_RESERVED_BASE: u32 = 0xFFFF_0000;

/// First banked register; R28 (SP), R29 (FP) and R30 (GP) are banked.
const BANK_FIRST: usize = 28;
const BANK_SIZE: usize = 3;

/// Kind of user-mode memory access; the value is the region FLAGS bit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Read = 1,
    Write = 2,
    Fetch = 4,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exception {
    IllegalInstruction,
    BusError,
    /// `imm16`: bits 7..0 call number, bits 15..8 context byte.
    Syscall(u16),
    PrivilegedInstruction,
    ProtectionFault(Access),
}

impl Exception {
    pub fn cause(self) -> u32 {
        let (code, detail) = match self {
            Exception::IllegalInstruction => (1, 0),
            Exception::BusError => (2, 0),
            Exception::Syscall(imm) => (3, u32::from(imm)),
            Exception::PrivilegedInstruction => (4, 0),
            Exception::ProtectionFault(access) => (5, access as u32),
        };
        CAUSE_EXCEPTION | (code << 16) | detail
    }

    /// Address of the vector word holding the handler address.
    pub fn vector(self) -> u32 {
        match self {
            Exception::IllegalInstruction | Exception::PrivilegedInstruction => {
                ILLEGAL_INSTRUCTION_VECTOR
            }
            Exception::BusError | Exception::ProtectionFault(_) => BUS_ERROR_VECTOR,
            Exception::Syscall(_) => SYSCALL_VECTOR,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Region {
    pub start: u32,
    /// Exclusive.
    pub end: u32,
    /// Bit 0 R, bit 1 W, bit 2 X.
    pub flags: u32,
}

impl Region {
    fn grants(&self, addr: u32, access: Access) -> bool {
        self.flags & access as u32 != 0 && self.start <= addr && addr < self.end
    }
}

/// Trap and privilege state (STATUS, EPC, CAUSE, user bank, regions).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SystemState {
    pub ie: bool,
    pub pie: bool,
    /// Current mode is User.
    pub user: bool,
    pub pu: bool,
    /// Instruction mode to restore on IRET (STATUS.PSM).
    pub psm: InstructionMode,
    pub epc: u32,
    pub cause: u32,
    /// R28..R30 of the mode that is not current.
    other_bank: [u32; BANK_SIZE],
    regions: [Region; REGION_COUNT],
}

impl Default for SystemState {
    fn default() -> Self {
        Self {
            ie: true,
            pie: true,
            user: false,
            pu: false,
            psm: InstructionMode::Normal,
            epc: 0,
            cause: 0,
            other_bank: [0; BANK_SIZE],
            regions: [Region::default(); REGION_COUNT],
        }
    }
}

impl SystemState {
    pub fn status(&self) -> u32 {
        let mut status = 0;
        if self.ie {
            status |= STATUS_IE;
        }
        if self.pie {
            status |= STATUS_PIE;
        }
        if self.user {
            status |= STATUS_U;
        }
        if self.pu {
            status |= STATUS_PU;
        }
        if self.psm == InstructionMode::Short {
            status |= STATUS_PSM;
        }
        status
    }

    /// STATUS write from `MTSR` or the host debugger. `U` is read-only.
    pub fn set_status(&mut self, value: u32) {
        self.ie = value & STATUS_IE != 0;
        self.pie = value & STATUS_PIE != 0;
        self.pu = value & STATUS_PU != 0;
        self.psm = if value & STATUS_PSM != 0 {
            InstructionMode::Short
        } else {
            InstructionMode::Normal
        };
    }

    fn swap_bank(&mut self, reg: &mut [u32; 32]) {
        for (index, other) in self.other_bank.iter_mut().enumerate() {
            std::mem::swap(&mut reg[BANK_FIRST + index], other);
        }
    }

    /// Common entry for IRQs and exceptions. The caller sets PC and the
    /// Normal instruction mode.
    pub fn enter_trap(&mut self, reg: &mut [u32; 32], epc: u32, cause: u32, mode: InstructionMode) {
        self.epc = epc;
        self.cause = cause;
        self.pie = self.ie;
        self.ie = false;
        self.pu = self.user;
        self.psm = mode;
        if self.user {
            self.swap_bank(reg);
            self.user = false;
        }
    }

    /// IRET: returns the PC and instruction mode to resume.
    pub fn leave_trap(&mut self, reg: &mut [u32; 32]) -> (u32, InstructionMode) {
        self.ie = self.pie;
        if self.pu && !self.user {
            self.swap_bank(reg);
            self.user = true;
        }
        (self.epc, self.psm)
    }

    /// Index of the user copy of R28..R30 (`USP`, `UFP`, `UGP`): in U mode the
    /// live registers, in S mode the saved bank.
    fn user_bank_index(sr: u16) -> usize {
        usize::from(sr - SR_USP)
    }

    /// `MFSR`; `None` for an unknown SR number.
    pub fn read_sr(&self, sr: u16, reg: &[u32; 32]) -> Option<u32> {
        match sr {
            SR_STATUS => Some(self.status()),
            SR_EPC => Some(self.epc),
            SR_CAUSE => Some(self.cause),
            SR_USP | SR_UFP | SR_UGP => {
                let index = Self::user_bank_index(sr);
                Some(if self.user {
                    reg[BANK_FIRST + index]
                } else {
                    self.other_bank[index]
                })
            }
            _ => {
                let (region, field) = Self::region_field(sr)?;
                let region = self.regions[region];
                Some([region.start, region.end, region.flags][field])
            }
        }
    }

    /// `MTSR`; false for an unknown SR number.
    pub fn write_sr(&mut self, sr: u16, value: u32, reg: &mut [u32; 32]) -> bool {
        match sr {
            SR_STATUS => self.set_status(value),
            SR_EPC => self.epc = value,
            SR_CAUSE => self.cause = value,
            SR_USP | SR_UFP | SR_UGP => {
                let index = Self::user_bank_index(sr);
                if self.user {
                    reg[BANK_FIRST + index] = value;
                } else {
                    self.other_bank[index] = value;
                }
            }
            _ => {
                let Some((region, field)) = Self::region_field(sr) else {
                    return false;
                };
                let region = &mut self.regions[region];
                match field {
                    0 => region.start = value,
                    1 => region.end = value,
                    _ => region.flags = value & 7,
                }
            }
        }
        true
    }

    fn region_field(sr: u16) -> Option<(usize, usize)> {
        let offset = usize::from(sr.checked_sub(SR_REGION_BASE)?);
        let (region, field) = (offset / 4, offset % 4);
        (region < REGION_COUNT && field < 3).then_some((region, field))
    }

    /// Whether the current mode may perform this access. S mode is never
    /// checked; in U mode every byte needs a region granting `access`.
    pub fn allows(&self, addr: u32, len: u32, access: Access) -> bool {
        !self.user
            || (0..len).all(|index| {
                let byte = addr.wrapping_add(index);
                byte < CPU_RESERVED_BASE
                    && self
                        .regions
                        .iter()
                        .any(|region| region.grants(byte, access))
            })
    }
}

/// Assembler/disassembler name of a system register.
pub fn sr_name(sr: u16) -> Option<String> {
    let fixed = match sr {
        SR_STATUS => Some("STATUS"),
        SR_EPC => Some("EPC"),
        SR_CAUSE => Some("CAUSE"),
        SR_USP => Some("USP"),
        SR_UFP => Some("UFP"),
        SR_UGP => Some("UGP"),
        _ => None,
    };
    if let Some(name) = fixed {
        return Some(name.to_string());
    }
    let (region, field) = SystemState::region_field(sr)?;
    Some(format!("RGN{region}_{}", ["START", "END", "FLAGS"][field]))
}

/// Disassembly shared by the CPU kinds.
pub fn format_syscall(imm: u16) -> String {
    match imm >> 8 {
        0 => format!("SYSCALL {}", imm & 0xFF),
        context => format!("SYSCALL {}, {}", imm & 0xFF, context),
    }
}

pub fn format_sr(sr: u16) -> String {
    sr_name(sr).unwrap_or_else(|| format!("0x{sr:04X}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trap_from_user_swaps_bank_and_iret_restores_it() {
        let mut state = SystemState::default();
        let mut reg = [0u32; 32];
        reg[28] = 0x1000; // S stack
        assert!(state.write_sr(SR_USP, 0x2000, &mut reg));
        state.pu = true;
        state.epc = 0x400;
        let (pc, _) = state.leave_trap(&mut reg);
        assert_eq!((pc, state.user, reg[28]), (0x400, true, 0x2000));
        reg[28] = 0x1FF0;
        state.enter_trap(
            &mut reg,
            0x404,
            Exception::Syscall(7).cause(),
            InstructionMode::Normal,
        );
        assert_eq!((state.user, state.pu, reg[28]), (false, true, 0x1000));
        assert_eq!(state.read_sr(SR_USP, &reg), Some(0x1FF0));
        assert_eq!(state.cause, 0x8003_0007);
    }

    #[test]
    fn user_access_needs_a_granting_region_for_every_byte() {
        let mut state = SystemState::default();
        let mut reg = [0u32; 32];
        state.user = true;
        assert!(!state.allows(0x100, 1, Access::Read));
        state.write_sr(SR_REGION_BASE, 0x100, &mut reg);
        state.write_sr(SR_REGION_BASE + 1, 0x104, &mut reg);
        state.write_sr(SR_REGION_BASE + 2, Access::Read as u32, &mut reg);
        assert!(state.allows(0x100, 4, Access::Read));
        assert!(!state.allows(0x101, 4, Access::Read));
        assert!(!state.allows(0x100, 4, Access::Write));
        state.write_sr(SR_REGION_BASE + 4, 0, &mut reg);
        state.write_sr(SR_REGION_BASE + 5, u32::MAX, &mut reg);
        state.write_sr(SR_REGION_BASE + 6, 7, &mut reg);
        assert!(!state.allows(CPU_RESERVED_BASE, 1, Access::Read));
        assert!(!state.allows(CPU_RESERVED_BASE - 2, 4, Access::Write));
    }

    #[test]
    fn unknown_sr_numbers_are_rejected() {
        let mut state = SystemState::default();
        let mut reg = [0u32; 32];
        assert_eq!(state.read_sr(0x03, &reg), None);
        assert_eq!(state.read_sr(SR_REGION_BASE + 3, &reg), None);
        assert_eq!(state.read_sr(SR_REGION_BASE + 4 * 8, &reg), None);
        assert!(!state.write_sr(0x07, 1, &mut reg));
        assert_eq!(sr_name(0x2E).as_deref(), Some("RGN7_FLAGS"));
    }

    #[test]
    fn status_u_bit_is_read_only() {
        let mut state = SystemState::default();
        state.set_status(STATUS_U | STATUS_PU);
        assert!(!state.user && state.pu && !state.ie);
        assert_eq!(state.status(), STATUS_PU);
    }
}
