//! Extension P: privilege levels and system calls (doc/spec_CPU_priv.md).
//!
//! Every CPU kind keeps its trap state in one `SystemState` so trap entry,
//! IRET, system registers and user-mode protection behave identically.

use crate::bus::Bus;
use crate::cpu::InstructionMode;
use crate::devices::cpu::{BUS_ERROR_VECTOR, ILLEGAL_INSTRUCTION_VECTOR, SYSCALL_VECTOR};

/// CPUID feature bit for Extension P.
pub const EXT_P: u32 = 0x100;
/// CPUID feature bit for Extension V (doc/spec_CPU_vm.md).
pub const EXT_V: u32 = 0x200;

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

/// Extension V system registers.
pub const SR_VCTL: u16 = 0x40;
pub const SR_PTBR: u16 = 0x41;
pub const SR_VFAULT: u16 = 0x42;
pub const SR_TLBFLUSH: u16 = 0x43;
pub const SR_TLBINV: u16 = 0x44;
pub const VCTL_EN: u32 = 1;

/// Set in CAUSE for every exception; IRQs leave it clear (CAUSE = 0..15).
pub const CAUSE_EXCEPTION: u32 = 0x8000_0000;

/// User code can never touch vectors, IRQC or CPU state at or above this.
pub const CPU_RESERVED_BASE: u32 = 0xFFFF_0000;

const PAGE_SHIFT: u32 = 12;
const PAGE_SIZE: u32 = 1 << PAGE_SHIFT;
const TLB_ENTRIES: usize = 16;

const PTE_V: u32 = 1 << 0;
const PTE_R: u32 = 1 << 1;
const PTE_W: u32 = 1 << 2;
const PTE_X: u32 = 1 << 3;
const PTE_U: u32 = 1 << 4;
const PTE_RESERVED: u32 = 0xC0;
const PTE_RWX: u32 = PTE_R | PTE_W | PTE_X;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct TlbEntry {
    valid: bool,
    vpn: u32,
    ppn: u32,
    /// `R`, `W`, `X` and `U` bits of the leaf PTE.
    flags: u32,
}

/// Extension V state (doc/spec_CPU_vm.md): control registers and the TLB.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Vm {
    pub enabled: bool,
    pub ptbr: u32,
    pub vfault: u32,
    tlb: [TlbEntry; TLB_ENTRIES],
    next_victim: usize,
}

impl Default for Vm {
    fn default() -> Self {
        Self {
            enabled: false,
            ptbr: 0,
            vfault: 0,
            tlb: [TlbEntry::default(); TLB_ENTRIES],
            next_victim: 0,
        }
    }
}

impl Vm {
    pub fn flush(&mut self) {
        self.tlb = [TlbEntry::default(); TLB_ENTRIES];
        self.next_victim = 0;
    }

    fn invalidate(&mut self, va: u32) {
        let vpn = va >> PAGE_SHIFT;
        for entry in self.tlb.iter_mut().filter(|entry| entry.vpn == vpn) {
            entry.valid = false;
        }
    }

    fn lookup(&self, vpn: u32) -> Option<TlbEntry> {
        self.tlb
            .iter()
            .copied()
            .find(|entry| entry.valid && entry.vpn == vpn)
    }

    fn fill(&mut self, entry: TlbEntry) {
        self.tlb[self.next_victim] = entry;
        self.next_victim = (self.next_victim + 1) % TLB_ENTRIES;
    }

    /// Hardware table walk. `Ok(None)` is a missing translation; `Err(())` a
    /// bus error while reading a PTE.
    fn walk(
        &self,
        va: u32,
        read_word: &mut impl FnMut(u32) -> Option<u32>,
    ) -> Result<Option<TlbEntry>, ()> {
        let l1 = read_word((self.ptbr & !(PAGE_SIZE - 1)).wrapping_add((va >> 22) * 4)).ok_or(())?;
        if l1 & PTE_V == 0 || l1 & PTE_RESERVED != 0 || l1 & PTE_RWX != 0 {
            return Ok(None);
        }
        let l2_base = l1 & !(PAGE_SIZE - 1);
        let l2 = read_word(l2_base.wrapping_add(((va >> PAGE_SHIFT) & 0x3FF) * 4)).ok_or(())?;
        let leaf_ok = l2 & PTE_V != 0
            && l2 & PTE_RESERVED == 0
            && l2 & PTE_RWX != 0
            && (l2 & PTE_W == 0 || l2 & PTE_R != 0);
        Ok(leaf_ok.then_some(TlbEntry {
            valid: true,
            vpn: va >> PAGE_SHIFT,
            ppn: l2 >> PAGE_SHIFT,
            flags: l2 & (PTE_RWX | PTE_U),
        }))
    }
}

/// Reads one big-endian word from physical memory for the table walk.
fn read_pte(bus: &mut Bus, addr: u32) -> Option<u32> {
    let value = bus.read_u32_be(addr);
    (!bus.take_bus_error()).then_some(value)
}

/// One TLB slot as shown by the debugger.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TlbEntryView {
    pub index: usize,
    pub valid: bool,
    /// Page-aligned virtual and physical addresses.
    pub virtual_page: u32,
    pub physical_page: u32,
    pub flags: TlbFlags,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TlbFlags {
    pub read: bool,
    pub write: bool,
    pub execute: bool,
    pub user: bool,
}

impl TlbFlags {
    fn from_pte(pte: u32) -> Self {
        Self {
            read: pte & PTE_R != 0,
            write: pte & PTE_W != 0,
            execute: pte & PTE_X != 0,
            user: pte & PTE_U != 0,
        }
    }
}

/// Extension V state for the debugger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MmuSnapshot {
    pub enabled: bool,
    /// Translation is applied right now (enabled and in U mode).
    pub translating: bool,
    pub ptbr: u32,
    pub vfault: u32,
    pub next_victim: usize,
    pub entries: Vec<TlbEntryView>,
}

/// Result of a side-effect-free translation for the debugger.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugTranslation {
    Mapped {
        physical: u32,
        flags: TlbFlags,
        /// Served by a TLB entry (otherwise by a table walk).
        in_tlb: bool,
    },
    /// No valid leaf (V = 0, malformed PTE or unsupported huge page).
    Unmapped,
    /// A PTE could not be read from the bus.
    BusError,
}

/// Every Extension P/V CPU register as the debugger sees it (read only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemSnapshot {
    pub status: u32,
    pub ie: bool,
    pub pie: bool,
    /// Protection mode: true = User, false = Supervisor.
    pub user: bool,
    pub pu: bool,
    pub psm: InstructionMode,
    pub epc: u32,
    pub cause: u32,
    /// User copies of R28..R30 (`USP`, `UFP`, `UGP`).
    pub user_bank: [u32; BANK_SIZE],
    pub regions: [Region; REGION_COUNT],
    pub mmu: MmuSnapshot,
    /// IRQ number latched but not yet taken. Filled in by the CPU.
    pub irq_pending: Option<u8>,
}

/// Human-readable form of a CAUSE value (IRQ number or exception).
pub fn describe_cause(cause: u32) -> String {
    if cause & CAUSE_EXCEPTION == 0 {
        return format!("IRQ{}", cause & 0xF);
    }
    let detail = cause & 0xFFFF;
    let access = |bits: u32| {
        format!(
            "{}{}{}",
            if bits & 1 != 0 { 'R' } else { '-' },
            if bits & 2 != 0 { 'W' } else { '-' },
            if bits & 4 != 0 { 'X' } else { '-' },
        )
    };
    match (cause >> 16) & 0xFF {
        1 => "Illegal instruction".to_string(),
        2 => "Bus error".to_string(),
        3 => format!("SYSCALL #{} ctx 0x{:02X}", detail & 0xFF, detail >> 8),
        4 => "Privileged instruction".to_string(),
        5 => format!("Protection fault ({})", access(detail)),
        6 => format!(
            "Page fault ({}, {})",
            access(detail),
            if detail & 8 != 0 { "protection" } else { "not present" }
        ),
        code => format!("Unknown exception {code}"),
    }
}

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
    /// Extension V page fault in U mode. `protection` separates a present
    /// leaf that denies the access from a missing translation; `addr` goes
    /// to `VFAULT`.
    PageFault {
        access: Access,
        protection: bool,
        addr: u32,
    },
}

impl Exception {
    pub fn cause(self) -> u32 {
        let (code, detail) = match self {
            Exception::IllegalInstruction => (1, 0),
            Exception::BusError => (2, 0),
            Exception::Syscall(imm) => (3, u32::from(imm)),
            Exception::PrivilegedInstruction => (4, 0),
            Exception::ProtectionFault(access) => (5, access as u32),
            Exception::PageFault {
                access, protection, ..
            } => (6, access as u32 | if protection { 8 } else { 0 }),
        };
        CAUSE_EXCEPTION | (code << 16) | detail
    }

    /// Address of the vector word holding the handler address.
    pub fn vector(self) -> u32 {
        match self {
            Exception::IllegalInstruction | Exception::PrivilegedInstruction => {
                ILLEGAL_INSTRUCTION_VECTOR
            }
            Exception::BusError | Exception::ProtectionFault(_) | Exception::PageFault { .. } => {
                BUS_ERROR_VECTOR
            }
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
    pub vm: Vm,
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
            vm: Vm::default(),
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
            SR_VCTL => Some(u32::from(self.vm.enabled)),
            SR_PTBR => Some(self.vm.ptbr),
            SR_VFAULT => Some(self.vm.vfault),
            SR_TLBFLUSH | SR_TLBINV => Some(0),
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
            SR_VCTL => self.vm.enabled = value & VCTL_EN != 0,
            SR_PTBR => {
                self.vm.ptbr = value & !(PAGE_SIZE - 1);
                self.vm.flush();
            }
            SR_VFAULT => self.vm.vfault = value,
            SR_TLBFLUSH => self.vm.flush(),
            SR_TLBINV => self.vm.invalidate(value),
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

    /// Read-only view of all system registers; `reg` is the live register file.
    pub fn snapshot(&self, reg: &[u32; 32]) -> SystemSnapshot {
        let mut user_bank = self.other_bank;
        if self.user {
            user_bank.copy_from_slice(&reg[BANK_FIRST..BANK_FIRST + BANK_SIZE]);
        }
        SystemSnapshot {
            status: self.status(),
            ie: self.ie,
            pie: self.pie,
            user: self.user,
            pu: self.pu,
            psm: self.psm,
            epc: self.epc,
            cause: self.cause,
            user_bank,
            regions: self.regions,
            mmu: self.mmu_snapshot(),
            irq_pending: None,
        }
    }

    pub fn mmu_snapshot(&self) -> MmuSnapshot {
        MmuSnapshot {
            enabled: self.vm.enabled,
            translating: self.translating(),
            ptbr: self.vm.ptbr,
            vfault: self.vm.vfault,
            next_victim: self.vm.next_victim,
            entries: self
                .vm
                .tlb
                .iter()
                .enumerate()
                .map(|(index, entry)| TlbEntryView {
                    index,
                    valid: entry.valid,
                    virtual_page: entry.vpn << PAGE_SHIFT,
                    physical_page: entry.ppn << PAGE_SHIFT,
                    flags: TlbFlags::from_pte(entry.flags),
                })
                .collect(),
        }
    }

    /// Translate `va` like the hardware would, without filling the TLB or
    /// checking permissions. `read_word` reads one physical word.
    pub fn debug_translate(
        &self,
        va: u32,
        mut read_word: impl FnMut(u32) -> Option<u32>,
    ) -> DebugTranslation {
        let (entry, in_tlb) = match self.vm.lookup(va >> PAGE_SHIFT) {
            Some(entry) => (entry, true),
            None => match self.vm.walk(va, &mut read_word) {
                Err(()) => return DebugTranslation::BusError,
                Ok(None) => return DebugTranslation::Unmapped,
                Ok(Some(entry)) => (entry, false),
            },
        };
        DebugTranslation::Mapped {
            physical: (entry.ppn << PAGE_SHIFT) | (va & (PAGE_SIZE - 1)),
            flags: TlbFlags::from_pte(entry.flags),
            in_tlb,
        }
    }

    /// Whether U-mode accesses are translated (Extension V).
    pub fn translating(&self) -> bool {
        self.user && self.vm.enabled
    }

    /// Whether the current mode may perform this access. With Extension V
    /// translation active this walks the page table instead of consulting the
    /// regions. A denied access yields the exception to raise; nothing has
    /// been read or written yet.
    pub fn check_access(
        &mut self,
        bus: &mut Bus,
        addr: u32,
        len: u32,
        access: Access,
    ) -> Result<(), Exception> {
        if !self.translating() {
            return if self.allows(addr, len, access) {
                Ok(())
            } else {
                Err(Exception::ProtectionFault(access))
            };
        }
        let last = addr.wrapping_add(len - 1);
        let mut page = addr >> PAGE_SHIFT;
        loop {
            let fault_addr = if page == addr >> PAGE_SHIFT {
                addr
            } else {
                page << PAGE_SHIFT
            };
            let fault = |protection| Exception::PageFault {
                access,
                protection,
                addr: fault_addr,
            };
            if page << PAGE_SHIFT >= CPU_RESERVED_BASE {
                return Err(fault(true));
            }
            let entry = match self.vm.lookup(page) {
                Some(entry) => entry,
                None => match self.vm.walk(fault_addr, &mut |pa| read_pte(bus, pa)) {
                    Err(()) => return Err(Exception::BusError),
                    Ok(None) => return Err(fault(false)),
                    Ok(Some(entry)) => {
                        self.vm.fill(entry);
                        entry
                    }
                },
            };
            if entry.flags & PTE_U == 0
                || entry.flags & Self::pte_bit(access) == 0
                || entry.ppn << PAGE_SHIFT >= CPU_RESERVED_BASE
            {
                return Err(fault(true));
            }
            if page == last >> PAGE_SHIFT || page.wrapping_add(1) == 0 {
                return Ok(());
            }
            page += 1;
        }
    }

    fn pte_bit(access: Access) -> u32 {
        match access {
            Access::Read => PTE_R,
            Access::Write => PTE_W,
            Access::Fetch => PTE_X,
        }
    }

    /// Physical address of a byte that `check_access` just validated.
    fn physical(&self, va: u32) -> u32 {
        if !self.translating() {
            return va;
        }
        let ppn = self
            .vm
            .lookup(va >> PAGE_SHIFT)
            .map_or(0, |entry| entry.ppn);
        (ppn << PAGE_SHIFT) | (va & (PAGE_SIZE - 1))
    }

    /// Big-endian load of 1, 2 or 4 bytes after a successful `check_access`.
    pub fn read_data(&self, bus: &mut Bus, va: u32, size: u32) -> u32 {
        if size == 4 && (!self.translating() || va & (PAGE_SIZE - 1) <= PAGE_SIZE - 4) {
            return bus.read_u32_be(self.physical(va));
        }
        (0..size).fold(0, |acc, index| {
            let byte = self.physical(va.wrapping_add(index));
            (acc << 8) | u32::from(bus.read_u8(byte))
        })
    }

    /// Big-endian store of 1, 2 or 4 bytes after a successful `check_access`.
    pub fn write_data(&self, bus: &mut Bus, va: u32, size: u32, value: u32) {
        if size == 4 && (!self.translating() || va & (PAGE_SIZE - 1) <= PAGE_SIZE - 4) {
            bus.write_u32_be(self.physical(va), value);
            return;
        }
        for index in 0..size {
            let shift = 8 * (size - 1 - index);
            let byte = self.physical(va.wrapping_add(index));
            bus.write_u8(byte, (value >> shift) as u8);
        }
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
        SR_VCTL => Some("VCTL"),
        SR_PTBR => Some("PTBR"),
        SR_VFAULT => Some("VFAULT"),
        SR_TLBFLUSH => Some("TLBFLUSH"),
        SR_TLBINV => Some("TLBINV"),
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
