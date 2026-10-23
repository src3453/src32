#ifndef FOUNDATION_CHIP8_CORE_H
#define FOUNDATION_CHIP8_CORE_H
#include <stdint.h>
#include <string.h>
/* Original CHIP-8 language, not a COSMAC 1802 emulator. All guest memory
 * accesses are bounded; unsupported machine-code calls fail explicitly. */
typedef struct {
    uint8_t mem[4096], v[16], pixels[64 * 32];
    uint16_t stack[12], pc, i, keys;
    uint8_t sp, delay, sound, sound_gate, dirty, draw_ready;
    int8_t wait_reg, wait_key;
    uint32_t random, timer_phase;
} chip8_vm;

static void chip8_init(chip8_vm *c, uint32_t seed) {
    static const uint8_t font[80] = {
        0xF0,0x90,0x90,0x90,0xF0, 0x20,0x60,0x20,0x20,0x70,
        0xF0,0x10,0xF0,0x80,0xF0, 0xF0,0x10,0xF0,0x10,0xF0,
        0x90,0x90,0xF0,0x10,0x10, 0xF0,0x80,0xF0,0x10,0xF0,
        0xF0,0x80,0xF0,0x90,0xF0, 0xF0,0x10,0x20,0x40,0x40,
        0xF0,0x90,0xF0,0x90,0xF0, 0xF0,0x90,0xF0,0x10,0xF0,
        0xF0,0x90,0xF0,0x90,0x90, 0xE0,0x90,0xE0,0x90,0xE0,
        0xF0,0x80,0x80,0x80,0xF0, 0xE0,0x90,0x90,0x90,0xE0,
        0xF0,0x80,0xF0,0x80,0xF0, 0xF0,0x80,0xF0,0x80,0x80
    };
    memset(c, 0, sizeof *c);
    memcpy(c->mem + 0x50, font, sizeof font);
    c->pc = 0x200; c->dirty = 1;
    c->wait_reg = c->wait_key = -1;
    c->random = seed ? seed : 1;
}

/* Called for each Foundation scheduler tick, even while DXYN/FX0A waits.
 * Fractional accumulator preserves 60 Hz over a 100 Hz scheduler. */
static void chip8_tick(chip8_vm *c, uint32_t hz) {
    c->timer_phase += 60;
    while (c->timer_phase >= hz) {
        c->timer_phase -= hz;
        if (c->delay) --c->delay;
        if (c->sound) --c->sound;
        if (!c->sound) c->sound_gate = 0;
        c->draw_ready = 1;
    }
}

static void chip8_keys(chip8_vm *c, uint16_t down, uint16_t pressed, uint16_t released) {
    c->keys = down;
    if (c->wait_reg < 0) return;
    if (c->wait_key < 0) {
        uint16_t candidates = down | pressed;
        for (uint32_t k = 0; k < 16; ++k)
            if (candidates & (1u << k)) { c->wait_key = (int8_t)k; break; }
    }
    if (c->wait_key >= 0 && (released & (1u << c->wait_key))) {
        c->v[(uint32_t)c->wait_reg] = (uint8_t)c->wait_key;
        c->wait_reg = c->wait_key = -1;
    }
}

/* 0: executed; 1: waiting; -1: invalid instruction/bounds/stack. */
static int chip8_step(chip8_vm *c) {
    if (c->wait_reg >= 0) return 1;
    if (c->pc > 4094) return -1;
    uint32_t op = ((uint32_t)c->mem[c->pc] << 8) | c->mem[c->pc + 1];
    uint32_t x = (op >> 8) & 15, y = (op >> 4) & 15, n = op & 15;
    uint32_t nn = op & 255, addr = op & 4095;
    uint32_t a = c->v[x], b = c->v[y], value, flag;
    if ((op >> 12) == 13 && !c->draw_ready) return 1;
    c->pc += 2;
    switch (op >> 12) {
    case 0:
        if (op == 0x00E0) { memset(c->pixels, 0, sizeof c->pixels); c->dirty = 1; }
        else if (op == 0x00EE) { if (!c->sp) return -1; c->pc = c->stack[--c->sp]; }
        else return -1;
        break;
    case 1: c->pc = (uint16_t)addr; break;
    case 2:
        if (c->sp == 12) return -1;
        c->stack[c->sp++] = c->pc; c->pc = (uint16_t)addr; break;
    case 3: if (a == nn) c->pc += 2; break;
    case 4: if (a != nn) c->pc += 2; break;
    case 5: if (n) return -1; if (a == b) c->pc += 2; break;
    case 6: c->v[x] = (uint8_t)nn; break;
    case 7: c->v[x] = (uint8_t)(a + nn); break;
    case 8:
        switch (n) {
        case 0: c->v[x] = (uint8_t)b; break;
        case 1: c->v[x] = (uint8_t)(a | b); c->v[15] = 0; break;
        case 2: c->v[x] = (uint8_t)(a & b); c->v[15] = 0; break;
        case 3: c->v[x] = (uint8_t)(a ^ b); c->v[15] = 0; break;
        case 4: value = a + b; flag = value > 255; goto arithmetic;
        case 5: value = a - b; flag = a >= b; goto arithmetic;
        case 6: value = b >> 1; flag = b & 1; goto arithmetic;
        case 7: value = b - a; flag = b >= a; goto arithmetic;
        case 14: value = b << 1; flag = b >> 7; goto arithmetic;
        default: return -1;
        }
        break;
    arithmetic:
        c->v[x] = (uint8_t)value; c->v[15] = (uint8_t)flag; break;
    case 9: if (n) return -1; if (a != b) c->pc += 2; break;
    case 10: c->i = (uint16_t)addr; break;
    case 11: c->pc = (uint16_t)(addr + c->v[0]); break;
    case 12:
        c->random ^= c->random << 13; c->random ^= c->random >> 17; c->random ^= c->random << 5;
        c->v[x] = (uint8_t)(c->random & nn); break;
    case 13: {
        if (!n || c->i > 4096u - n) return -1;
        a &= 63; b &= 31; c->v[15] = 0;
        for (uint32_t row = 0; row < n && b + row < 32; ++row)
            for (uint32_t col = 0; col < 8 && a + col < 64; ++col)
                if (c->mem[c->i + row] & (128u >> col)) {
                    uint32_t p = (b + row) * 64 + a + col;
                    c->v[15] |= c->pixels[p]; c->pixels[p] ^= 1;
                }
        c->draw_ready = 0; c->dirty = 1; break;
    }
    case 14:
        flag = (c->keys >> (a & 15)) & 1;
        if (nn == 0x9E) { if (flag) c->pc += 2; }
        else if (nn == 0xA1) { if (!flag) c->pc += 2; }
        else return -1;
        break;
    case 15:
        switch (nn) {
        case 0x07: c->v[x] = c->delay; break;
        case 0x0A:
            c->wait_reg = (int8_t)x; c->wait_key = -1;
            chip8_keys(c, c->keys, 0, 0); return 1;
        case 0x15: c->delay = (uint8_t)a; break;
        case 0x18: c->sound = (uint8_t)a; c->sound_gate = a >= 2; break;
        case 0x1E: c->i = (uint16_t)(c->i + a); break;
        case 0x29: c->i = (uint16_t)(0x50 + (a & 15) * 5); break;
        case 0x33:
            if (c->i > 4093) return -1;
            c->mem[c->i] = (uint8_t)(a / 100);
            c->mem[c->i + 1] = (uint8_t)(a / 10 % 10);
            c->mem[c->i + 2] = (uint8_t)(a % 10); break;
        case 0x55: case 0x65:
            if (c->i > 4095u - x) return -1;
            for (uint32_t j = 0; j <= x; ++j)
                if (nn == 0x55) c->mem[c->i + j] = c->v[j];
                else c->v[j] = c->mem[c->i + j];
            c->i += (uint16_t)(x + 1); break;
        default: return -1;
        }
        break;
    }
    return 0;
}
#endif
