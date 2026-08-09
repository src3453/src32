-- SRC32 CPU (SRC32-ALM, Pure Lua)
-- Rust implementation in src/cpu.rs ported to Lua.

-- bit32 polyfill for baremetal Lua 5.1/5.2 environments
local bit32 = {}
bit32.lshift = function(value, shift)
    return value * (2 ^ shift)
end
bit32.rshift = function(value, shift)
    return math.floor(value / (2 ^ shift))
end
bit32.band = function(a, b)
    local result = 0
    local bit = 1
    while a > 0 and b > 0 do
        if (a % 2 == 1) and (b % 2 == 1) then
            result = result + bit
        end
        a = math.floor(a / 2)
        b = math.floor(b / 2)
        bit = bit * 2
    end
    return result
end
bit32.bor = function(a, b)
    local result = 0
    local bit = 1
    while a > 0 or b > 0 do
        if (a % 2 == 1) or (b % 2 == 1) then
            result = result + bit
        end
        a = math.floor(a / 2)
        b = math.floor(b / 2)
        bit = bit * 2
    end
    return result
end
bit32.bxor = function(a, b)
    local result = 0
    local bit = 1
    while a > 0 or b > 0 do
        if (a % 2) ~= (b % 2) then
            result = result + bit
        end
        a = math.floor(a / 2)
        b = math.floor(b / 2)
        bit = bit * 2
    end
    return result
end

bit = bit32

local Bus = {}
Bus.__index = Bus
Bus.memsize = 16 * 1024 * 1024 -- 16 MiB
Bus.membase = 0

Bus.new = function()
    local self = setmetatable({}, Bus)
    self._mem = {}
    for i = 0, Bus.memsize - 1 do
        self._mem[i] = 0
    end
    return self
end

Bus.read_u8 = function(self, addr)
    addr = addr - Bus.membase
    if addr < 0 or addr >= Bus.memsize then
        error(string.format("Bus read_u8 out of bounds: 0x%08X", addr + Bus.membase))
    end
    return self._mem[addr]
end

Bus.write_u8 = function(self, addr, value)
    addr = addr - Bus.membase
    if addr < 0 or addr >= Bus.memsize then
        error(string.format("Bus write_u8 out of bounds: 0x%08X", addr + Bus.membase))
    end
    self._mem[addr] = value % 256
end

local Cpu = {}
Cpu.__index = Cpu

Cpu.CPU_CLOCK = 48000000
Cpu.CYCLES_PER_FRAME = math.floor(Cpu.CPU_CLOCK / 60)
Cpu.CYCLES_PER_SCANLINE = math.floor(Cpu.CYCLES_PER_FRAME / 240)

local U32_MOD = 4294967296
local REG_ZERO = 0
local REG_CPUID = 1
local REG_FEATURES = 2
local REG_LR = 31

local EXT_BASE = 0x01
local EXT_A = 0x02
local EXT_L = 0x04
local EXT_M = 0x08
local CPU_FEATURES = bit.bor(EXT_BASE, EXT_A, EXT_L, EXT_M)
local CPU_ID = 0x53524332 -- "SRC2"

local INSN_SIZE = 4

local function u32(n)
    return n % U32_MOD
end

local function to_i32(n)
    n = u32(n)
    if n >= 0x80000000 then
        return n - U32_MOD
    end
    return n
end

local function sign16(v)
    v = bit.band(v, 0xFFFF)
    if v >= 0x8000 then
        return v - 0x10000
    end
    return v
end

local function idiv_trunc(a, b)
    if b == 0 then
        error("division by zero")
    end
    local q = a / b
    if q >= 0 then
        return math.floor(q)
    end
    return math.ceil(q)
end

local function imod_trunc(a, b)
    local q = idiv_trunc(a, b)
    return a - q * b
end

-- Exact 32x32->64 multiply using 16-bit limbs.
local function umul_32x32(a, b)
    local a0 = a % 0x10000
    local a1 = math.floor(a / 0x10000)
    local b0 = b % 0x10000
    local b1 = math.floor(b / 0x10000)

    local p0 = a0 * b0
    local p1 = a0 * b1 + a1 * b0
    local p2 = a1 * b1

    local carry = math.floor(p0 / 0x10000)
    local mid = p1 + carry

    local lo = (p0 % 0x10000) + ((mid % 0x10000) * 0x10000)
    local hi = (p2 + math.floor(mid / 0x10000)) % U32_MOD

    return u32(lo), u32(hi)
end

local function mulh_signed(a_u32, b_u32)
    local _, hi_u = umul_32x32(a_u32, b_u32)
    local a_s = to_i32(a_u32)
    local b_s = to_i32(b_u32)

    local hi_s = hi_u
    if a_s < 0 then
        hi_s = hi_s - b_u32
    end
    if b_s < 0 then
        hi_s = hi_s - a_u32
    end

    return u32(hi_s)
end

local function bus_read_u8(bus, addr)
    if bus.read_u8 then
        return bus:read_u8(addr)
    end
    error("bus.read_u8 is required")
end

local function bus_write_u8(bus, addr, value)
    if bus.write_u8 then
        bus:write_u8(addr, bit.band(value, 0xFF))
        return
    end
    error("bus.write_u8 is required")
end

local function bus_read_u32_be(bus, addr)
    if bus.read_u32_be then
        return u32(bus:read_u32_be(addr))
    end
    local b0 = bus_read_u8(bus, addr)
    local b1 = bus_read_u8(bus, u32(addr + 1))
    local b2 = bus_read_u8(bus, u32(addr + 2))
    local b3 = bus_read_u8(bus, u32(addr + 3))
    return u32(bit.lshift(b0, 24) + bit.lshift(b1, 16) + bit.lshift(b2, 8) + b3)
end

local function bus_write_u32_be(bus, addr, value)
    value = u32(value)
    if bus.write_u32_be then
        bus:write_u32_be(addr, value)
        return
    end

    bus_write_u8(bus, addr, bit.rshift(value, 24))
    bus_write_u8(bus, u32(addr + 1), bit.rshift(value, 16))
    bus_write_u8(bus, u32(addr + 2), bit.rshift(value, 8))
    bus_write_u8(bus, u32(addr + 3), value)
end

function Cpu.new(bus)
    local self = setmetatable({}, Cpu)
    self._reg = {}
    for i = 0, 31 do
        self._reg[i] = 0
    end
    self._pc = 0
    self._running = true
    self._bus = bus
    self._cycles = 0
    return self
end

function Cpu:reset(pc)
    for i = 0, 31 do
        self._reg[i] = 0
    end
    self._pc = u32(pc)
    self._running = true
end

function Cpu:load_program(base, image)
    base = u32(base)
    if type(image) == "string" then
        for i = 1, #image do
            local byte = string.byte(image, i)
            bus_write_u8(self._bus, u32(base + (i - 1)), byte)
        end
        return
    end

    for i = 1, #image do
        bus_write_u8(self._bus, u32(base + (i - 1)), image[i])
    end
end

function Cpu:pc()
    return self._pc
end

function Cpu:set_pc(pc)
    self._pc = u32(pc)
end

function Cpu:is_running()
    return self._running
end

function Cpu:cycles()
    return self._cycles
end

function Cpu:read_mem_u8(addr)
    return bus_read_u8(self._bus, u32(addr))
end

function Cpu:read_mem_u32_be(addr)
    return bus_read_u32_be(self._bus, u32(addr))
end

function Cpu:write_mem_u8(addr, value)
    bus_write_u8(self._bus, u32(addr), value)
end

function Cpu:write_mem_u32_be(addr, value)
    bus_write_u32_be(self._bus, u32(addr), value)
end

function Cpu:read_reg(reg)
    if reg < 0 or reg > 31 then
        error(string.format("Invalid register index: %d", reg))
    end
    if reg == REG_ZERO then
        return 0
    end
    return self._reg[reg]
end

function Cpu:write_reg(reg, value)
    if reg < 0 or reg > 31 then
        return nil, string.format("Invalid register index: %d", reg)
    end
    if reg == REG_ZERO then
        return true, "Warning: Writing to R0 has no effect"
    end
    self._reg[reg] = u32(value)
    return true, ""
end

function Cpu:fetch_u32()
    return bus_read_u32_be(self._bus, self._pc)
end

function Cpu:fetch_u32_at(addr)
    return bus_read_u32_be(self._bus, u32(addr))
end

function Cpu.decode(raw)
    raw = u32(raw)
    local op = bit.band(bit.rshift(raw, 26), 0x3F)
    local rd = bit.band(bit.rshift(raw, 21), 0x1F)
    local rs1 = bit.band(bit.rshift(raw, 16), 0x1F)
    local rs2 = bit.band(bit.rshift(raw, 11), 0x1F)
    local imm16 = sign16(bit.band(raw, 0xFFFF))
    local imm_u16 = bit.band(raw, 0xFFFF)

    if op == 0x00 then return { op = "Nop" } end
    if op == 0x01 then return { op = "Ld", rd = rd, base = rs1, offset = imm16 } end
    if op == 0x02 then return { op = "St", rd = rd, base = rs1, offset = imm16 } end
    if op == 0x03 then return { op = "Add", rd = rd, rs1 = rs1, rs2 = rs2 } end
    if op == 0x04 then return { op = "Addi", rd = rd, rs1 = rs1, imm = imm16 } end
    if op == 0x05 then return { op = "Sub", rd = rd, rs1 = rs1, rs2 = rs2 } end
    if op == 0x06 then return { op = "Slt", rd = rd, rs1 = rs1, rs2 = rs2 } end
    if op == 0x07 then return { op = "Beq", rs1 = rs1, rs2 = rd, offset = imm16 } end
    if op == 0x08 then return { op = "Bne", rs1 = rs1, rs2 = rd, offset = imm16 } end
    if op == 0x09 then return { op = "Jmp", offset = imm16 } end
    if op == 0x0A then return { op = "Jal", offset = imm16 } end
    if op == 0x0B then return { op = "Jr", rd = rd } end
    if op == 0x0C then return { op = "And", rd = rd, rs1 = rs1, rs2 = rs2 } end
    if op == 0x0D then return { op = "Or", rd = rd, rs1 = rs1, rs2 = rs2 } end
    if op == 0x0E then return { op = "Xor", rd = rd, rs1 = rs1, rs2 = rs2 } end
    if op == 0x0F then return { op = "Sll", rd = rd, rs1 = rs1, rs2 = rs2 } end
    if op == 0x10 then return { op = "Srl", rd = rd, rs1 = rs1, rs2 = rs2 } end
    if op == 0x11 then return { op = "Sla", rd = rd, rs1 = rs1, rs2 = rs2 } end
    if op == 0x12 then return { op = "Sra", rd = rd, rs1 = rs1, rs2 = rs2 } end
    if op == 0x13 then return { op = "Ldb", rd = rd, base = rs1, offset = imm16 } end
    if op == 0x14 then return { op = "Ldh", rd = rd, base = rs1, offset = imm16 } end
    if op == 0x15 then return { op = "Stb", rd = rd, base = rs1, offset = imm16 } end
    if op == 0x16 then return { op = "Sth", rd = rd, base = rs1, offset = imm16 } end
    if op == 0x17 then return { op = "Sltu", rd = rd, rs1 = rs1, rs2 = rs2 } end
    if op == 0x18 then return { op = "Mul", rd = rd, rs1 = rs1, rs2 = rs2 } end
    if op == 0x19 then return { op = "Div", rd = rd, rs1 = rs1, rs2 = rs2 } end
    if op == 0x1A then return { op = "Mod", rd = rd, rs1 = rs1, rs2 = rs2 } end
    if op == 0x1B then return { op = "Mulh", rd = rd, rs1 = rs1, rs2 = rs2 } end
    if op == 0x1C then return { op = "Divu", rd = rd, rs1 = rs1, rs2 = rs2 } end
    if op == 0x3C then return { op = "Ldil", rd = rd, imm = imm_u16 } end
    if op == 0x3D then return { op = "Ldih", rd = rd, imm = imm_u16 } end
    if op == 0x3E then return { op = "Cpuid" } end
    if op == 0x3F then return { op = "Halt" } end

    return { op = "Unknown", raw = raw }
end

function Cpu.format_instruction(insn)
    local op = insn.op

    if op == "Nop" then return "NOP" end
    if op == "Ld" then return string.format("LD R%d, [R%d + %d]", insn.rd, insn.base, insn.offset) end
    if op == "St" then return string.format("ST [R%d + %d], R%d", insn.base, insn.offset, insn.rd) end
    if op == "Add" then return string.format("ADD R%d, R%d, R%d", insn.rd, insn.rs1, insn.rs2) end
    if op == "Addi" then return string.format("ADDI R%d, R%d, %d", insn.rd, insn.rs1, insn.imm) end
    if op == "Sub" then return string.format("SUB R%d, R%d, R%d", insn.rd, insn.rs1, insn.rs2) end
    if op == "Slt" then return string.format("SLT R%d, R%d, R%d", insn.rd, insn.rs1, insn.rs2) end
    if op == "Beq" then return string.format("BEQ R%d, R%d, %d", insn.rs1, insn.rs2, insn.offset) end
    if op == "Bne" then return string.format("BNE R%d, R%d, %d", insn.rs1, insn.rs2, insn.offset) end
    if op == "Jmp" then return string.format("JMP %d", insn.offset) end
    if op == "Jal" then return string.format("JAL %d", insn.offset) end
    if op == "Jr" then return string.format("JR R%d", insn.rd) end
    if op == "Cpuid" then return "CPUID" end
    if op == "Halt" then return "HALT" end
    if op == "And" then return string.format("AND R%d, R%d, R%d", insn.rd, insn.rs1, insn.rs2) end
    if op == "Or" then return string.format("OR R%d, R%d, R%d", insn.rd, insn.rs1, insn.rs2) end
    if op == "Xor" then return string.format("XOR R%d, R%d, R%d", insn.rd, insn.rs1, insn.rs2) end
    if op == "Sll" then return string.format("SLL R%d, R%d, R%d", insn.rd, insn.rs1, insn.rs2) end
    if op == "Srl" then return string.format("SRL R%d, R%d, R%d", insn.rd, insn.rs1, insn.rs2) end
    if op == "Sla" then return string.format("SLA R%d, R%d, R%d", insn.rd, insn.rs1, insn.rs2) end
    if op == "Sra" then return string.format("SRA R%d, R%d, R%d", insn.rd, insn.rs1, insn.rs2) end
    if op == "Sltu" then return string.format("SLTU R%d, R%d, R%d", insn.rd, insn.rs1, insn.rs2) end
    if op == "Mul" then return string.format("MUL R%d, R%d, R%d", insn.rd, insn.rs1, insn.rs2) end
    if op == "Div" then return string.format("DIV R%d, R%d, R%d", insn.rd, insn.rs1, insn.rs2) end
    if op == "Mod" then return string.format("MOD R%d, R%d, R%d", insn.rd, insn.rs1, insn.rs2) end
    if op == "Mulh" then return string.format("MULH R%d, R%d, R%d", insn.rd, insn.rs1, insn.rs2) end
    if op == "Divu" then return string.format("DIVU R%d, R%d, R%d", insn.rd, insn.rs1, insn.rs2) end
    if op == "Ldb" then return string.format("LDB R%d, [R%d + %d]", insn.rd, insn.base, insn.offset) end
    if op == "Ldh" then return string.format("LDH R%d, [R%d + %d]", insn.rd, insn.base, insn.offset) end
    if op == "Stb" then return string.format("STB [R%d + %d], R%d", insn.base, insn.offset, insn.rd) end
    if op == "Sth" then return string.format("STH [R%d + %d], R%d", insn.base, insn.offset, insn.rd) end
    if op == "Ldil" then return string.format("LDIL R%d, 0x%04X", insn.rd, insn.imm) end
    if op == "Ldih" then return string.format("LDIH R%d, 0x%04X", insn.rd, insn.imm) end

    return string.format(".word 0x%08X", insn.raw)
end

function Cpu:disassemble_at(addr)
    local raw = self:fetch_u32_at(addr)
    local insn = Cpu.decode(raw)
    return Cpu.format_instruction(insn)
end

function Cpu:read_u32(addr)
    return self:fetch_u32_at(addr)
end

function Cpu:read_u40(addr)
    return self:fetch_u32_at(addr)
end

function Cpu.add_signed(base, offset)
    return u32(base + offset)
end

function Cpu.branch_target(next_pc, offset)
    return u32(next_pc + offset)
end

function Cpu:execute(insn)
    local next_pc = u32(self._pc + INSN_SIZE)
    self._pc = next_pc

    local op = insn.op

    if op == "Nop" then
        return
    elseif op == "Ld" then
        local addr = Cpu.add_signed(self:read_reg(insn.base), insn.offset)
        local value = bus_read_u32_be(self._bus, addr)
        self:write_reg(insn.rd, value)
    elseif op == "St" then
        local addr = Cpu.add_signed(self:read_reg(insn.base), insn.offset)
        local value = self:read_reg(insn.rd)
        bus_write_u32_be(self._bus, addr, value)
    elseif op == "Ldil" then
        local current = self:read_reg(insn.rd)
        local value = bit.bor(bit.band(current, 0xFFFF0000), insn.imm)
        self:write_reg(insn.rd, value)
    elseif op == "Ldih" then
        local current = self:read_reg(insn.rd)
        local value = bit.bor(bit.band(current, 0x0000FFFF), bit.lshift(insn.imm, 16))
        self:write_reg(insn.rd, value)
    elseif op == "Add" then
        local lhs = self:read_reg(insn.rs1)
        local rhs = self:read_reg(insn.rs2)
        self:write_reg(insn.rd, u32(lhs + rhs))
    elseif op == "Addi" then
        local lhs = self:read_reg(insn.rs1)
        self:write_reg(insn.rd, u32(lhs + insn.imm))
    elseif op == "Sub" then
        local lhs = self:read_reg(insn.rs1)
        local rhs = self:read_reg(insn.rs2)
        self:write_reg(insn.rd, u32(lhs - rhs))
    elseif op == "Slt" then
        local lhs = to_i32(self:read_reg(insn.rs1))
        local rhs = to_i32(self:read_reg(insn.rs2))
        self:write_reg(insn.rd, (lhs < rhs) and 1 or 0)
    elseif op == "Beq" then
        if self:read_reg(insn.rs1) == self:read_reg(insn.rs2) then
            self._pc = Cpu.branch_target(next_pc, insn.offset)
        end
    elseif op == "Bne" then
        if self:read_reg(insn.rs1) ~= self:read_reg(insn.rs2) then
            self._pc = Cpu.branch_target(next_pc, insn.offset)
        end
    elseif op == "Jmp" then
        self._pc = Cpu.branch_target(next_pc, insn.offset)
    elseif op == "Jal" then
        self:write_reg(REG_LR, next_pc)
        self._pc = Cpu.branch_target(next_pc, insn.offset)
    elseif op == "Jr" then
        self._pc = self:read_reg(insn.rd)
    elseif op == "Cpuid" then
        self:write_reg(REG_CPUID, CPU_ID)
        self:write_reg(REG_FEATURES, CPU_FEATURES)
    elseif op == "Halt" then
        self._running = false
    elseif op == "And" then
        self:write_reg(insn.rd, bit.band(self:read_reg(insn.rs1), self:read_reg(insn.rs2)))
    elseif op == "Or" then
        self:write_reg(insn.rd, bit.bor(self:read_reg(insn.rs1), self:read_reg(insn.rs2)))
    elseif op == "Xor" then
        self:write_reg(insn.rd, bit.bxor(self:read_reg(insn.rs1), self:read_reg(insn.rs2)))
    elseif op == "Sll" then
        local sh = bit.band(self:read_reg(insn.rs2), 0x1F)
        self:write_reg(insn.rd, u32(bit.lshift(self:read_reg(insn.rs1), sh)))
    elseif op == "Srl" then
        local sh = bit.band(self:read_reg(insn.rs2), 0x1F)
        self:write_reg(insn.rd, bit.rshift(self:read_reg(insn.rs1), sh))
    elseif op == "Sla" then
        local sh = bit.band(self:read_reg(insn.rs2), 0x1F)
        self:write_reg(insn.rd, u32(bit.lshift(self:read_reg(insn.rs1), sh)))
    elseif op == "Sra" then
        local sh = bit.band(self:read_reg(insn.rs2), 0x1F)
        local value = to_i32(self:read_reg(insn.rs1))
        self:write_reg(insn.rd, u32(bit.arshift(value, sh)))
    elseif op == "Sltu" then
        local lhs = self:read_reg(insn.rs1)
        local rhs = self:read_reg(insn.rs2)
        self:write_reg(insn.rd, (lhs < rhs) and 1 or 0)
    elseif op == "Mul" then
        local lhs = self:read_reg(insn.rs1)
        local rhs = self:read_reg(insn.rs2)
        local lo = umul_32x32(lhs, rhs)
        self:write_reg(insn.rd, lo)
    elseif op == "Div" then
        local lhs = to_i32(self:read_reg(insn.rs1))
        local rhs = to_i32(self:read_reg(insn.rs2))
        self:write_reg(insn.rd, u32(idiv_trunc(lhs, rhs)))
    elseif op == "Mod" then
        local lhs = to_i32(self:read_reg(insn.rs1))
        local rhs = to_i32(self:read_reg(insn.rs2))
        self:write_reg(insn.rd, u32(imod_trunc(lhs, rhs)))
    elseif op == "Mulh" then
        local lhs = self:read_reg(insn.rs1)
        local rhs = self:read_reg(insn.rs2)
        self:write_reg(insn.rd, mulh_signed(lhs, rhs))
    elseif op == "Divu" then
        local lhs = self:read_reg(insn.rs1)
        local rhs = self:read_reg(insn.rs2)
        if rhs == 0 then
            error("division by zero")
        end
        self:write_reg(insn.rd, math.floor(lhs / rhs))
    elseif op == "Ldb" then
        local addr = Cpu.add_signed(self:read_reg(insn.base), insn.offset)
        local value = bus_read_u8(self._bus, addr)
        self:write_reg(insn.rd, value)
    elseif op == "Ldh" then
        local addr = Cpu.add_signed(self:read_reg(insn.base), insn.offset)
        local b0 = bus_read_u8(self._bus, addr)
        local b1 = bus_read_u8(self._bus, u32(addr + 1))
        local value = bit.lshift(b0, 8) + b1
        self:write_reg(insn.rd, value)
    elseif op == "Stb" then
        local addr = Cpu.add_signed(self:read_reg(insn.base), insn.offset)
        local value = self:read_reg(insn.rd)
        bus_write_u8(self._bus, addr, value)
    elseif op == "Sth" then
        local addr = Cpu.add_signed(self:read_reg(insn.base), insn.offset)
        local value = self:read_reg(insn.rd)
        bus_write_u8(self._bus, addr, bit.rshift(value, 8))
        bus_write_u8(self._bus, u32(addr + 1), value)
    elseif op == "Unknown" then
        error(string.format("Illegal instruction at PC=0x%08X: 0x%08X", u32(next_pc - INSN_SIZE), insn.raw))
    else
        error("Unhandled instruction: " .. tostring(op))
    end
end

function Cpu:return_state_text()
    local pc = self._pc
    local op = self:fetch_u32()
    local out = string.format("PC=0x%08X OP=0x%08X\n", pc, op)
    for i = 0, 31 do
        out = out .. string.format(" R%-2d=0x%08X", i, self:read_reg(i))
        if i % 4 == 3 then
            out = out .. "\n"
        end
    end
    return out
end

function Cpu:step()
    if not self._running then
        return
    end

    local raw = self:fetch_u32()
    self._cycles = self._cycles + 1

    local insn = Cpu.decode(raw)
    self._cycles = self._cycles + 1

    self:execute(insn)
    self._cycles = self._cycles + 1
end

function Cpu:step_once()
    if not self._running then
        return false
    end
    self:step()
    return true
end

function Cpu:run(max_cycles)
    local start_cycles = self._cycles
    while self._running and self._cycles < (start_cycles + max_cycles) do
        self:step()
    end
end

return Cpu
