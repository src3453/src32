#![no_std]

//! CNET network card (`doc/spec_NET.md`). The data link and network layers
//! run here; the host only carries raw link frames over a segment.

use core::panic::PanicInfo;
use cpt32_wasm_expansion_card::{
    Card, NET_MAX_FRAME, NetSend, irq_set, net_link, net_network, net_recv, net_seed, net_send,
    net_static_node, schedule_tick,
};

// --- Protocol constants -----------------------------------------------------

const LINK_HEADER: usize = 8;
const DGRAM_HEADER: usize = 16;
const MTU: usize = 1024;

const TYPE_DATAGRAM: u8 = 0x01;
const TYPE_PROBE: u8 = 0x81;
const TYPE_IN_USE: u8 = 0x82;

const VERSION: u8 = 1;
const BROADCAST: u16 = 0xFFFF;
const ECHO_SOCKET: u8 = 4;
const PROTO_ECHO: u8 = 0x02;

/// System cycles are 48 MHz.
const CYCLES_PER_MS: u64 = 48_000;
const PROBE_INTERVAL: u64 = 20 * CYCLES_PER_MS;
const PROBE_COUNT: u8 = 4;
const MAX_RETRIES: u8 = 32;

// --- MMIO map ---------------------------------------------------------------

const HANDLES: usize = 16;
const QUEUE_DEPTH: usize = 8;
const HANDLE_BASE: u32 = 0x0100;
const HANDLE_STRIDE: u32 = 0x20;
const TX_BASE: u32 = 0x1_0000;
const RX_BASE: u32 = 0x2_0000;
const BUFFER_STRIDE: u32 = 0x400;

const STATUS_LINK: u16 = 1 << 0;
const STATUS_PROBING: u16 = 1 << 1;
const STATUS_ADDR_VALID: u16 = 1 << 2;
const STATUS_CONFLICT: u16 = 1 << 3;

const IRQ_ADDR: u16 = 1 << 0;
const IRQ_RX: u16 = 1 << 1;

const CMD_OPEN: u8 = 1;
const CMD_CLOSE: u8 = 2;
const CMD_SEND: u8 = 3;
const CMD_RX_POP: u8 = 4;

const R_OK: u8 = 0;
const R_BAD_STATE: u8 = 1;
const R_IN_USE: u8 = 2;
const R_NO_SOCKET: u8 = 3;
const R_BAD_LEN: u8 = 4;
const R_BAD_DEST: u8 = 5;
const R_NO_ADDR: u8 = 6;
const R_NO_LINK: u8 = 7;
const R_DROPPED: u8 = 8;
const R_NO_ROUTE: u8 = 9;

// --- State ------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Addr {
    Idle,
    Probing {
        candidate: u16,
        sent: u8,
        retries: u8,
    },
    Ready,
    Conflict,
}

#[derive(Clone, Copy)]
struct Datagram {
    src_net: u16,
    src_node: u16,
    src_socket: u8,
    proto: u8,
    len: u16,
    flags: u8,
    data: [u8; MTU],
}

const EMPTY_DATAGRAM: Datagram = Datagram {
    src_net: 0,
    src_node: 0,
    src_socket: 0,
    proto: 0,
    len: 0,
    flags: 0,
    data: [0; MTU],
};

#[derive(Clone, Copy)]
struct Handle {
    socket: u8,
    open: bool,
    result: u8,
    dropped: u8,
    tx_net: u16,
    tx_node: u16,
    tx_socket: u8,
    tx_proto: u8,
    tx_len: u16,
    tx: [u8; MTU],
    queue: [Datagram; QUEUE_DEPTH],
    head: usize,
    count: usize,
}

const CLOSED_HANDLE: Handle = Handle {
    socket: 0,
    open: false,
    result: 0,
    dropped: 0,
    tx_net: 0,
    tx_node: 0,
    tx_socket: 0,
    tx_proto: 0,
    tx_len: 0,
    tx: [0; MTU],
    queue: [EMPTY_DATAGRAM; QUEUE_DEPTH],
    head: 0,
    count: 0,
};

impl Handle {
    fn front(&self) -> Option<&Datagram> {
        (self.count > 0).then(|| &self.queue[self.head])
    }

    fn pop(&mut self) {
        if self.count > 0 {
            self.head = (self.head + 1) % QUEUE_DEPTH;
            self.count -= 1;
        }
    }

    /// Returns false when the queue is full.
    fn push(&mut self, datagram: &Datagram) -> bool {
        if self.count == QUEUE_DEPTH {
            self.dropped = self.dropped.saturating_add(1);
            return false;
        }
        self.queue[(self.head + self.count) % QUEUE_DEPTH] = *datagram;
        self.count += 1;
        true
    }
}

struct CnetCard {
    enabled: bool,
    addr: Addr,
    network: u16,
    node: u16,
    node_hint: u16,
    nonce: u32,
    rng: u32,
    irq_status: u16,
    irq_enable: u16,
    handles: [Handle; HANDLES],
    frame: [u8; NET_MAX_FRAME],
    scratch: Datagram,
}

impl Default for CnetCard {
    fn default() -> Self {
        Self {
            enabled: false,
            addr: Addr::Idle,
            network: 0,
            node: 0,
            node_hint: 0,
            nonce: 0,
            rng: 0,
            irq_status: 0,
            irq_enable: 0,
            handles: [CLOSED_HANDLE; HANDLES],
            frame: [0; NET_MAX_FRAME],
            scratch: EMPTY_DATAGRAM,
        }
    }
}

fn be16(bytes: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([bytes[at], bytes[at + 1]])
}

fn put16(bytes: &mut [u8], at: usize, value: u16) {
    bytes[at..at + 2].copy_from_slice(&value.to_be_bytes());
}

/// Byte `index` of a big-endian 16-bit register.
fn reg16(value: u16, index: u32) -> u8 {
    value.to_be_bytes()[index as usize & 1]
}

fn set_reg16(value: &mut u16, index: u32, byte: u8) {
    let mut bytes = value.to_be_bytes();
    bytes[index as usize & 1] = byte;
    *value = u16::from_be_bytes(bytes);
}

/// CRC-16/CCITT-FALSE.
fn crc16(crc: u16, bytes: &[u8]) -> u16 {
    let mut crc = crc;
    for &byte in bytes {
        crc ^= u16::from(byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// CRC over a datagram header (CRC field zero, hop count zero) and payload.
fn datagram_crc(header: &[u8], payload: &[u8]) -> u16 {
    let mut copy = [0u8; DGRAM_HEADER];
    copy.copy_from_slice(&header[..DGRAM_HEADER]);
    copy[0] &= 0xF0;
    copy[4] = 0;
    copy[5] = 0;
    crc16(crc16(0xFFFF, &copy), payload)
}

impl CnetCard {
    fn next_random(&mut self) -> u32 {
        // xorshift32
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        x
    }

    fn status(&self) -> u16 {
        let mut status = 0;
        if net_link() {
            status |= STATUS_LINK;
        }
        match self.addr {
            Addr::Probing { .. } => status |= STATUS_PROBING,
            Addr::Ready => status |= STATUS_ADDR_VALID,
            Addr::Conflict => status |= STATUS_CONFLICT,
            Addr::Idle => {}
        }
        status
    }

    fn update_irq(&self) {
        irq_set(self.irq_status & self.irq_enable != 0);
    }

    fn raise(&mut self, bits: u16) {
        self.irq_status |= bits;
        self.update_irq();
    }

    fn rx_ready(&self) -> u16 {
        let mut ready = 0;
        for (index, handle) in self.handles.iter().enumerate() {
            if handle.count > 0 {
                ready |= 1 << index;
            }
        }
        ready
    }

    // --- Link control and node acquisition ---------------------------------

    fn enable(&mut self) {
        if self.enabled {
            return;
        }
        self.enabled = true;
        if !net_link() {
            return;
        }
        self.network = net_network();
        let seed = net_seed();
        self.rng = if seed == 0 { 0x6C07_8965 } else { seed };
        self.nonce = self.next_random();
        let candidate = match net_static_node() {
            Some(node) => node,
            None if (1..=0xFEFF).contains(&self.node_hint) => self.node_hint,
            None => self.random_node(),
        };
        self.start_probe(candidate, 0);
    }

    fn disable(&mut self) {
        self.enabled = false;
        self.addr = Addr::Idle;
        self.node = 0;
        self.network = 0;
        let _ = schedule_tick(0);
        for handle in &mut self.handles {
            *handle = Handle {
                result: handle.result,
                ..CLOSED_HANDLE
            };
        }
    }

    fn random_node(&mut self) -> u16 {
        (self.next_random() % 0xFEFF) as u16 + 1
    }

    fn start_probe(&mut self, candidate: u16, retries: u8) {
        self.addr = Addr::Probing {
            candidate,
            sent: 0,
            retries,
        };
        self.probe_step();
    }

    /// Send the next probe, or confirm the address after the last interval.
    fn probe_step(&mut self) {
        let Addr::Probing {
            candidate,
            sent,
            retries,
        } = self.addr
        else {
            return;
        };
        if sent >= PROBE_COUNT {
            self.addr = Addr::Ready;
            self.node = candidate;
            self.raise(IRQ_ADDR);
            return;
        }
        let nonce = self.nonce.to_be_bytes();
        self.send_link(candidate, 0, TYPE_PROBE, &nonce);
        self.addr = Addr::Probing {
            candidate,
            sent: sent + 1,
            retries,
        };
        let _ = schedule_tick(PROBE_INTERVAL);
    }

    fn collision(&mut self) {
        let Addr::Probing { retries, .. } = self.addr else {
            return;
        };
        if net_static_node().is_some() || retries + 1 >= MAX_RETRIES {
            self.addr = Addr::Conflict;
            let _ = schedule_tick(0);
            self.raise(IRQ_ADDR);
            return;
        }
        self.nonce = self.next_random();
        let candidate = self.random_node();
        self.start_probe(candidate, retries + 1);
    }

    fn send_link(&mut self, dst: u16, src: u16, kind: u8, payload: &[u8]) -> NetSend {
        let frame = &mut self.frame;
        put16(frame, 0, dst);
        put16(frame, 2, src);
        frame[4] = kind;
        frame[5] = 0;
        put16(frame, 6, payload.len() as u16);
        frame[LINK_HEADER..LINK_HEADER + payload.len()].copy_from_slice(payload);
        net_send(&frame[..LINK_HEADER + payload.len()])
    }

    // --- Receive path -------------------------------------------------------

    fn receive_all(&mut self) {
        let mut frame = [0u8; NET_MAX_FRAME];
        while let Some(length) = net_recv(&mut frame) {
            self.receive_frame(&frame[..length]);
        }
    }

    fn receive_frame(&mut self, frame: &[u8]) {
        if !self.enabled
            || frame.len() < LINK_HEADER
            || usize::from(be16(frame, 6)) != frame.len() - LINK_HEADER
        {
            return;
        }
        let dst = be16(frame, 0);
        let payload = &frame[LINK_HEADER..];
        match frame[4] {
            TYPE_PROBE if payload.len() == 4 => {
                let nonce = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
                match self.addr {
                    Addr::Ready if dst == self.node => {
                        let node = self.node;
                        self.send_link(node, node, TYPE_IN_USE, &[]);
                    }
                    Addr::Probing { candidate, .. } if dst == candidate && nonce != self.nonce => {
                        self.collision();
                    }
                    _ => {}
                }
            }
            TYPE_IN_USE => {
                if let Addr::Probing { candidate, .. } = self.addr {
                    if dst == candidate {
                        self.collision();
                    }
                }
            }
            TYPE_DATAGRAM if self.addr == Addr::Ready => {
                if dst == self.node || dst == BROADCAST {
                    self.receive_datagram(payload, dst == BROADCAST);
                }
            }
            _ => {}
        }
    }

    /// Validate and deliver a network-layer datagram (link payload).
    fn receive_datagram(&mut self, bytes: &[u8], broadcast: bool) {
        if bytes.len() < DGRAM_HEADER || bytes[0] >> 4 != VERSION {
            return;
        }
        let len = usize::from(be16(bytes, 2));
        if len > MTU || bytes.len() != DGRAM_HEADER + len {
            return;
        }
        let payload = &bytes[DGRAM_HEADER..];
        if datagram_crc(bytes, payload) != be16(bytes, 4) {
            return;
        }
        let dst_net = be16(bytes, 6);
        let dst_node = be16(bytes, 8);
        if (dst_net != 0 && dst_net != self.network)
            || (dst_node != self.node && dst_node != BROADCAST)
        {
            return;
        }
        let src_net = be16(bytes, 10);
        let src_node = be16(bytes, 12);
        let dst_socket = bytes[14];
        let src_socket = bytes[15];
        let proto = bytes[1];
        if dst_socket == ECHO_SOCKET {
            if proto == PROTO_ECHO && payload.first() == Some(&1) {
                let mut reply = [0u8; MTU];
                reply[..len].copy_from_slice(payload);
                reply[0] = 2;
                let dst_net = if src_net == self.network { 0 } else { src_net };
                let _ = self.send_datagram(
                    dst_net,
                    src_node,
                    src_socket,
                    ECHO_SOCKET,
                    PROTO_ECHO,
                    &reply[..len],
                );
            }
            return;
        }
        let scratch = &mut self.scratch;
        scratch.src_net = src_net;
        scratch.src_node = src_node;
        scratch.src_socket = src_socket;
        scratch.proto = proto;
        scratch.len = len as u16;
        scratch.flags = u8::from(broadcast);
        scratch.data[..len].copy_from_slice(payload);
        self.deliver_scratch(dst_socket);
    }

    fn deliver_scratch(&mut self, dst_socket: u8) {
        let Some(index) = self
            .handles
            .iter()
            .position(|handle| handle.open && handle.socket == dst_socket)
        else {
            return;
        };
        let was_empty = self.handles[index].count == 0;
        let scratch = self.scratch;
        if self.handles[index].push(&scratch) && was_empty {
            self.raise(IRQ_RX);
        }
    }

    // --- Send path -----------------------------------------------------------

    /// Build and send one datagram. Returns a RESULT code.
    fn send_datagram(
        &mut self,
        dst_net: u16,
        dst_node: u16,
        dst_socket: u8,
        src_socket: u8,
        proto: u8,
        payload: &[u8],
    ) -> u8 {
        if !net_link() {
            return R_NO_LINK;
        }
        if self.addr != Addr::Ready {
            return R_NO_ADDR;
        }
        if payload.len() > MTU {
            return R_BAD_LEN;
        }
        if dst_node == 0 || dst_net == 0xFFFF || dst_socket == 0 || dst_socket == 0xFF {
            return R_BAD_DEST;
        }
        if dst_net != 0 && dst_net != self.network {
            // No routers yet (spec §9).
            return R_NO_ROUTE;
        }
        let mut datagram = [0u8; DGRAM_HEADER + MTU];
        datagram[0] = VERSION << 4;
        datagram[1] = proto;
        put16(&mut datagram, 2, payload.len() as u16);
        put16(&mut datagram, 6, dst_net);
        put16(&mut datagram, 8, dst_node);
        put16(&mut datagram, 10, self.network);
        put16(&mut datagram, 12, self.node);
        datagram[14] = dst_socket;
        datagram[15] = src_socket;
        datagram[DGRAM_HEADER..DGRAM_HEADER + payload.len()].copy_from_slice(payload);
        let crc = datagram_crc(&datagram, payload);
        put16(&mut datagram, 4, crc);
        let total = DGRAM_HEADER + payload.len();

        let to_self = dst_node == self.node;
        let broadcast = dst_node == BROADCAST;
        if to_self || broadcast {
            // Local delivery never goes through the host (spec §6.2).
            self.receive_datagram(&datagram[..total], broadcast);
        }
        if to_self {
            return R_OK;
        }
        let node = self.node;
        match self.send_link(dst_node, node, TYPE_DATAGRAM, &datagram[..total]) {
            NetSend::Accepted => R_OK,
            NetSend::NoLink => R_NO_LINK,
            NetSend::QueueFull => R_DROPPED,
        }
    }

    // --- Socket commands ----------------------------------------------------

    fn socket_in_use(&self, socket: u8) -> bool {
        self.handles
            .iter()
            .any(|handle| handle.open && handle.socket == socket)
    }

    fn command(&mut self, index: usize, cmd: u8) {
        let result = match cmd {
            CMD_OPEN => self.cmd_open(index),
            CMD_CLOSE => {
                if self.handles[index].open {
                    self.handles[index] = CLOSED_HANDLE;
                    R_OK
                } else {
                    R_BAD_STATE
                }
            }
            CMD_SEND => {
                let handle = &self.handles[index];
                if !handle.open {
                    R_BAD_STATE
                } else if usize::from(handle.tx_len) > MTU {
                    R_BAD_LEN
                } else {
                    let (net, node, socket, src, proto, len) = (
                        handle.tx_net,
                        handle.tx_node,
                        handle.tx_socket,
                        handle.socket,
                        handle.tx_proto,
                        usize::from(handle.tx_len),
                    );
                    let mut payload = [0u8; MTU];
                    payload[..len].copy_from_slice(&handle.tx[..len]);
                    self.send_datagram(net, node, socket, src, proto, &payload[..len])
                }
            }
            CMD_RX_POP => {
                if self.handles[index].open {
                    self.handles[index].pop();
                    R_OK
                } else {
                    R_BAD_STATE
                }
            }
            _ => R_BAD_STATE,
        };
        self.handles[index].result = result;
    }

    fn cmd_open(&mut self, index: usize) -> u8 {
        if self.handles[index].open {
            return R_BAD_STATE;
        }
        let requested = self.handles[index].socket;
        let socket = if requested == 0 {
            match (128..=254).find(|&socket| !self.socket_in_use(socket)) {
                Some(socket) => socket,
                None => return R_NO_SOCKET,
            }
        } else if requested == ECHO_SOCKET || requested == 0xFF || self.socket_in_use(requested) {
            return R_IN_USE;
        } else {
            requested
        };
        let handle = &mut self.handles[index];
        *handle = CLOSED_HANDLE;
        handle.socket = socket;
        handle.open = true;
        R_OK
    }

    // --- MMIO ----------------------------------------------------------------

    fn read(&self, offset: u32) -> u8 {
        match offset {
            0x0000..=0x0003 => b"CNET"[offset as usize],
            0x0004..=0x0005 => reg16(1, offset),
            0x0006..=0x0007 => reg16(self.status(), offset),
            0x0008..=0x0009 => reg16(u16::from(self.enabled), offset),
            0x000A..=0x000B => reg16(self.node_hint, offset),
            0x000C..=0x000D => reg16(self.network, offset),
            0x000E..=0x000F => reg16(
                if self.addr == Addr::Ready {
                    self.node
                } else {
                    0
                },
                offset,
            ),
            0x0010..=0x0011 => reg16(self.irq_status, offset),
            0x0012..=0x0013 => reg16(self.irq_enable, offset),
            0x0014..=0x0015 => reg16(self.rx_ready(), offset),
            0x0016..=0x0017 => reg16(MTU as u16, offset),
            HANDLE_BASE..0x0300 => {
                let index = ((offset - HANDLE_BASE) / HANDLE_STRIDE) as usize;
                self.read_handle(&self.handles[index], (offset - HANDLE_BASE) % HANDLE_STRIDE)
            }
            TX_BASE..0x1_4000 => {
                let index = ((offset - TX_BASE) / BUFFER_STRIDE) as usize;
                self.handles[index].tx[((offset - TX_BASE) % BUFFER_STRIDE) as usize]
            }
            RX_BASE..0x2_4000 => {
                let index = ((offset - RX_BASE) / BUFFER_STRIDE) as usize;
                let at = ((offset - RX_BASE) % BUFFER_STRIDE) as usize;
                self.handles[index]
                    .front()
                    .filter(|datagram| at < usize::from(datagram.len))
                    .map_or(0, |datagram| datagram.data[at])
            }
            _ => 0,
        }
    }

    fn read_handle(&self, handle: &Handle, register: u32) -> u8 {
        let front = handle.front();
        let rx16 = |get: fn(&Datagram) -> u16| reg16(front.map_or(0, get), register);
        match register {
            0x00 => handle.socket,
            0x01 => u8::from(handle.open),
            0x03 => handle.result,
            0x04 => handle.count as u8,
            0x05 => handle.dropped,
            0x08..=0x09 => reg16(handle.tx_net, register),
            0x0A..=0x0B => reg16(handle.tx_node, register),
            0x0C => handle.tx_socket,
            0x0D => handle.tx_proto,
            0x0E..=0x0F => reg16(handle.tx_len, register),
            0x10..=0x11 => rx16(|d| d.src_net),
            0x12..=0x13 => rx16(|d| d.src_node),
            0x14 => front.map_or(0, |d| d.src_socket),
            0x15 => front.map_or(0, |d| d.proto),
            0x16..=0x17 => rx16(|d| d.len),
            0x18 => front.map_or(0, |d| d.flags),
            _ => 0,
        }
    }

    fn write(&mut self, offset: u32, value: u8) {
        match offset {
            0x0008 => {
                if value & 0x80 != 0 {
                    self.reset();
                }
            }
            0x0009 => {
                if value & 1 != 0 {
                    self.enable();
                } else {
                    self.disable();
                }
            }
            0x000A..=0x000B => set_reg16(&mut self.node_hint, offset, value),
            0x0010..=0x0011 => {
                let mut clear = 0;
                set_reg16(&mut clear, offset, value);
                self.irq_status &= !clear;
                self.update_irq();
            }
            0x0012..=0x0013 => {
                set_reg16(&mut self.irq_enable, offset, value);
                self.update_irq();
            }
            HANDLE_BASE..0x0300 => {
                let index = ((offset - HANDLE_BASE) / HANDLE_STRIDE) as usize;
                let register = (offset - HANDLE_BASE) % HANDLE_STRIDE;
                let handle = &mut self.handles[index];
                match register {
                    0x00 if !handle.open => handle.socket = value,
                    0x02 => self.command(index, value),
                    0x05 => handle.dropped = 0,
                    0x08..=0x09 => set_reg16(&mut handle.tx_net, register, value),
                    0x0A..=0x0B => set_reg16(&mut handle.tx_node, register, value),
                    0x0C => handle.tx_socket = value,
                    0x0D => handle.tx_proto = value,
                    0x0E..=0x0F => set_reg16(&mut handle.tx_len, register, value),
                    _ => {}
                }
            }
            TX_BASE..0x1_4000 => {
                let index = ((offset - TX_BASE) / BUFFER_STRIDE) as usize;
                self.handles[index].tx[((offset - TX_BASE) % BUFFER_STRIDE) as usize] = value;
            }
            _ => {}
        }
    }
}

impl Card for CnetCard {
    fn reset(&mut self) {
        let _ = schedule_tick(0);
        *self = Self::default();
        irq_set(false);
    }

    fn mmio_read(&mut self, offset: u32) -> u8 {
        self.read(offset)
    }

    fn debug_read(&self, offset: u32) -> Option<u8> {
        Some(self.read(offset))
    }

    fn mmio_write(&mut self, offset: u32, value: u8) {
        self.write(offset, value);
    }

    fn tick(&mut self) {
        self.probe_step();
    }

    fn net_rx(&mut self) {
        self.receive_all();
    }
}

cpt32_wasm_expansion_card::export_card!(CnetCard);

#[panic_handler]
fn panic(_: &PanicInfo<'_>) -> ! {
    core::arch::wasm32::unreachable()
}
