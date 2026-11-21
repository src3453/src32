//! CNET network (`doc/spec_NET.md`): host physical layer and the card in
//! `examples/wasm-cnet`. The card tests need the built module:
//! `cargo build --manifest-path examples/wasm-cnet/Cargo.toml --release --target wasm32-unknown-unknown`
//! (or `CPT32_CNET_CARD_WASM=<path>`).

use cpt32::bus::Bus;
use cpt32::devices::expansion::{EXPANSION_BASE, EXPANSION_SLOT_SIZE, load_expansion_cards};
use std::fs;
use std::io::{Cursor, Write};
use std::net::UdpSocket;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

fn temporary_directory() -> PathBuf {
    let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("cpt32-cnet-{}-{id}", std::process::id()));
    fs::create_dir_all(&path).unwrap();
    path
}

fn write_package(path: &Path, wasm: &[u8], network: bool) {
    let mut archive = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let manifest = format!(
        "abi_version: 1\nmodule: card.wasm\n{}",
        if network {
            "capabilities: [network]\n"
        } else {
            ""
        }
    );
    archive.start_file("manifest.yaml", options).unwrap();
    archive.write_all(manifest.as_bytes()).unwrap();
    archive.start_file("card.wasm", options).unwrap();
    archive.write_all(wasm).unwrap();
    fs::write(path, archive.finish().unwrap().into_inner()).unwrap();
}

/// Write `config.yaml` with the given extra YAML and return its path.
fn write_config(dir: &Path, yaml: &str) -> PathBuf {
    let path = dir.join("config.yaml");
    fs::write(&path, yaml).unwrap();
    path
}

// --- Host-level tests with hand-written WAT cards --------------------------

/// Writing any byte sends a fixed 10-byte link frame; `cpt32_net_rx` drains
/// frames and counts them. MMIO 0 = frames received, 1 = last length,
/// 2 = last net_send result, 3 = net_info(0), 4 = low byte of net_info(1).
const RELAY: &str = r#"
(module
  (import "cpt32_host" "net_info" (func $info (param i32) (result i32)))
  (import "cpt32_host" "net_send" (func $send (param i32 i32) (result i32)))
  (import "cpt32_host" "net_recv" (func $recv (param i32 i32) (result i32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "\00\20\00\10\01\00\00\02\ab\cd")
  (global $count (mut i32) (i32.const 0))
  (global $last (mut i32) (i32.const 0))
  (global $sent (mut i32) (i32.const 255))
  (func (export "cpt32_abi_version") (result i32) i32.const 1)
  (func (export "cpt32_reset"))
  (func (export "cpt32_tick"))
  (func (export "cpt32_mmio_read") (param $o i32) (result i32)
    (if (i32.eq (local.get $o) (i32.const 0)) (then (return (global.get $count))))
    (if (i32.eq (local.get $o) (i32.const 1)) (then (return (global.get $last))))
    (if (i32.eq (local.get $o) (i32.const 2)) (then (return (global.get $sent))))
    (if (i32.eq (local.get $o) (i32.const 3)) (then (return (call $info (i32.const 0)))))
    (i32.and (call $info (i32.const 1)) (i32.const 255)))
  (func (export "cpt32_mmio_write") (param i32 i32)
    (global.set $sent (call $send (i32.const 0) (i32.const 10))))
  (func (export "cpt32_net_rx")
    (local $n i32)
    (block $done
      (loop $more
        (local.set $n (call $recv (i32.const 4096) (i32.const 1048)))
        (br_if $done (i32.lt_s (local.get $n) (i32.const 0)))
        (global.set $last (local.get $n))
        (global.set $count (i32.add (global.get $count) (i32.const 1)))
        (br $more)))))
"#;

fn slot_base(slot: u8) -> u32 {
    EXPANSION_BASE + u32::from(slot) * EXPANSION_SLOT_SIZE
}

#[test]
fn loopback_segment_carries_frames_between_cards_at_the_next_event_point() {
    let dir = temporary_directory();
    write_package(
        &dir.join("relay.cpt32card"),
        &wat::parse_str(RELAY).unwrap(),
        true,
    );
    let config = write_config(
        &dir,
        r#"
network_segment:
  - name: test
    network: 0x0102
    loopback: true
expansion_card:
  - slot: 0
    package: relay.cpt32card
    network: { segment: test }
  - slot: 1
    package: relay.cpt32card
    network: { segment: test }
  - slot: 2
    package: relay.cpt32card
"#,
    );
    let mut bus = Bus::new();
    load_expansion_cards(&config, &mut bus).unwrap();
    assert_eq!(bus.read_u8(slot_base(0) + 3), 1, "attached card has link");
    assert_eq!(
        bus.read_u8(slot_base(0) + 4),
        0x02,
        "network number low byte"
    );
    assert_eq!(
        bus.read_u8(slot_base(2) + 3),
        0,
        "unattached card has no link"
    );

    bus.write_u8(slot_base(0), 1);
    assert_eq!(bus.read_u8(slot_base(0) + 2), 0, "accepted");
    // Delivery waits for the Bus event point; nothing is called reentrantly.
    assert_eq!(bus.read_u8(slot_base(1)), 0);
    bus.advance_system_cycles(1);
    assert_eq!(bus.read_u8(slot_base(1)), 1);
    assert_eq!(bus.read_u8(slot_base(1) + 1), 10);
    assert_eq!(bus.read_u8(slot_base(0)), 0, "sender does not hear itself");
    assert_eq!(
        bus.read_u8(slot_base(2)),
        0,
        "unattached card hears nothing"
    );

    bus.write_u8(slot_base(2), 1);
    assert_eq!(
        bus.read_u8(slot_base(2) + 2),
        1,
        "unattached send reports no link"
    );
    bus.advance_system_cycles(1);
    assert_eq!(bus.read_u8(slot_base(0)), 0);
    assert_eq!(bus.read_u8(slot_base(1)), 1);

    // Reset drops frames the card has not taken yet.
    bus.write_u8(slot_base(0), 1);
    bus.reset_expansion_cards();
    bus.advance_system_cycles(1);
    assert_eq!(bus.read_u8(slot_base(1)), 1);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn network_imports_and_attachment_require_the_manifest_capability() {
    let dir = temporary_directory();
    let relay = wat::parse_str(RELAY).unwrap();
    write_package(&dir.join("undeclared.cpt32card"), &relay, false);
    let config = write_config(
        &dir,
        "expansion_card:\n  - slot: 0\n    package: undeclared.cpt32card\n",
    );
    let error = load_expansion_cards(&config, &mut Bus::new()).unwrap_err();
    assert!(
        error.contains("requires the \"network\" capability"),
        "{error}"
    );

    let plain = wat::parse_str(
        r#"(module
          (func (export "cpt32_abi_version") (result i32) i32.const 1)
          (func (export "cpt32_reset")) (func (export "cpt32_tick"))
          (func (export "cpt32_mmio_read") (param i32) (result i32) i32.const 0)
          (func (export "cpt32_mmio_write") (param i32 i32)))"#,
    )
    .unwrap();
    write_package(&dir.join("plain.cpt32card"), &plain, false);
    let config = write_config(
        &dir,
        "network_segment:\n  - { name: t, loopback: true }\nexpansion_card:\n  - slot: 0\n    package: plain.cpt32card\n    network: { segment: t }\n",
    );
    let error = load_expansion_cards(&config, &mut Bus::new()).unwrap_err();
    assert!(error.contains("does not declare"), "{error}");

    let no_memory = RELAY.replace("(memory (export \"memory\") 1)", "(memory 1)");
    write_package(
        &dir.join("nomem.cpt32card"),
        &wat::parse_str(&no_memory).unwrap(),
        true,
    );
    let config = write_config(
        &dir,
        "expansion_card:\n  - slot: 0\n    package: nomem.cpt32card\n",
    );
    let error = load_expansion_cards(&config, &mut Bus::new()).unwrap_err();
    assert!(error.contains("require exported memory"), "{error}");
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn invalid_network_configuration_fails_before_registration() {
    let dir = temporary_directory();
    write_package(
        &dir.join("relay.cpt32card"),
        &wat::parse_str(RELAY).unwrap(),
        true,
    );
    let card = "expansion_card:\n  - slot: 0\n    package: relay.cpt32card\n";
    for (yaml, expected) in [
        (
            format!("{card}    network: {{ segment: missing }}\n"),
            "unknown network segment",
        ),
        (
            format!(
                "network_segment:\n  - {{ name: t, loopback: true }}\n{card}    network: {{ segment: t, node: 0 }}\n"
            ),
            "0x0001..0xFEFF",
        ),
        (
            format!("network_segment:\n  - {{ name: t }}\n{card}"),
            "exactly one",
        ),
        (
            format!("network_segment:\n  - {{ name: t, loopback: true, network: 0xFFFF }}\n{card}"),
            "network must be",
        ),
        (
            format!(
                "network_segment:\n  - {{ name: t, loopback: true }}\n  - {{ name: t, loopback: true }}\n{card}"
            ),
            "duplicate segment",
        ),
        (
            format!(
                "network_segment:\n  - {{ name: t, udp: {{ bind: \"127.0.0.1:0\", multicast: \"10.0.0.1:1\" }} }}\n{card}"
            ),
            "multicast",
        ),
    ] {
        let config = write_config(&dir, &yaml);
        let mut bus = Bus::new();
        let error = load_expansion_cards(&config, &mut bus).unwrap_err();
        assert!(error.contains(expected), "{yaml}\n=> {error}");
        assert!(!bus.memory_map().any(|r| r.start == EXPANSION_BASE));
    }
    fs::remove_dir_all(dir).unwrap();
}

// --- The CNET card ------------------------------------------------------------

fn cnet_wasm() -> Vec<u8> {
    let path = std::env::var_os("CPT32_CNET_CARD_WASM")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("examples/wasm-cnet/target/wasm32-unknown-unknown/release/cpt32_wasm_cnet_card.wasm")
        });
    fs::read(&path).unwrap_or_else(|error| {
        panic!(
            "cannot read the CNET card module {} ({error}); build it with \
             `cargo build --manifest-path examples/wasm-cnet/Cargo.toml --release --target wasm32-unknown-unknown`",
            path.display()
        )
    })
}

/// Registers (`doc/spec_NET.md` §6).
const STATUS: u32 = 0x06;
const CONTROL: u32 = 0x08;
const NODE_HINT: u32 = 0x0A;
const NET: u32 = 0x0C;
const NODE: u32 = 0x0E;
const IRQ_STATUS: u32 = 0x10;
const IRQ_ENABLE: u32 = 0x12;
const RX_READY: u32 = 0x14;
const ST_LINK: u16 = 1;
const ST_ADDR_VALID: u16 = 4;
const ST_CONFLICT: u16 = 8;

const CMD_OPEN: u8 = 1;
const CMD_CLOSE: u8 = 2;
const CMD_SEND: u8 = 3;
const CMD_RX_POP: u8 = 4;

/// Enough virtual time for the five 20 ms probe intervals at 48 MHz.
const ACQUIRE_CYCLES: u64 = 6 * 20 * 48_000;

struct Card<'a> {
    bus: &'a mut Bus,
    base: u32,
}

impl Card<'_> {
    fn r8(&mut self, offset: u32) -> u8 {
        self.bus.read_u8(self.base + offset)
    }
    fn r16(&mut self, offset: u32) -> u16 {
        u16::from_be_bytes([self.r8(offset), self.r8(offset + 1)])
    }
    fn w8(&mut self, offset: u32, value: u8) {
        self.bus.write_u8(self.base + offset, value);
    }
    fn w16(&mut self, offset: u32, value: u16) {
        let [high, low] = value.to_be_bytes();
        self.w8(offset, high);
        self.w8(offset + 1, low);
    }
    fn h(handle: u32) -> u32 {
        0x100 + handle * 0x20
    }
    /// Open `handle` on `socket` (0 = dynamic); returns (RESULT, socket).
    fn open(&mut self, handle: u32, socket: u8) -> (u8, u8) {
        self.w8(Self::h(handle), socket);
        self.w8(Self::h(handle) + 2, CMD_OPEN);
        (self.r8(Self::h(handle) + 3), self.r8(Self::h(handle)))
    }
    fn send(&mut self, handle: u32, net: u16, node: u16, socket: u8, proto: u8, data: &[u8]) -> u8 {
        let h = Self::h(handle);
        for (i, &byte) in data.iter().enumerate() {
            self.w8(0x1_0000 + handle * 0x400 + i as u32, byte);
        }
        self.w16(h + 0x08, net);
        self.w16(h + 0x0A, node);
        self.w8(h + 0x0C, socket);
        self.w8(h + 0x0D, proto);
        self.w16(h + 0x0E, data.len() as u16);
        self.w8(h + 2, CMD_SEND);
        self.r8(h + 3)
    }
    /// (src net, src node, src socket, proto, flags, payload) of the queue head.
    fn head(&mut self, handle: u32) -> (u16, u16, u8, u8, u8, Vec<u8>) {
        let h = Self::h(handle);
        let len = self.r16(h + 0x16);
        let data = (0..u32::from(len))
            .map(|i| self.r8(0x2_0000 + handle * 0x400 + i))
            .collect();
        (
            self.r16(h + 0x10),
            self.r16(h + 0x12),
            self.r8(h + 0x14),
            self.r8(h + 0x15),
            self.r8(h + 0x18),
            data,
        )
    }
    fn rx_count(&mut self, handle: u32) -> u8 {
        self.r8(Self::h(handle) + 4)
    }
}

fn card(bus: &mut Bus, slot: u8) -> Card<'_> {
    Card {
        bus,
        base: slot_base(slot),
    }
}

fn load_cnet(dir: &Path, yaml: &str) -> Bus {
    write_package(&dir.join("cnet.cpt32card"), &cnet_wasm(), true);
    let config = write_config(dir, yaml);
    let mut bus = Bus::new();
    load_expansion_cards(&config, &mut bus).unwrap();
    bus
}

const TWO_CARDS: &str = r#"
network_segment:
  - { name: test, network: 0x0001, loopback: true }
expansion_card:
  - { slot: 0, package: cnet.cpt32card, network: { segment: test, node: 0x0010 } }
  - { slot: 1, package: cnet.cpt32card, network: { segment: test, node: 0x0020 } }
  - { slot: 2, package: cnet.cpt32card }
"#;

fn enable_and_acquire(bus: &mut Bus, slots: &[u8]) {
    for &slot in slots {
        card(bus, slot).w16(CONTROL, 1);
    }
    bus.advance_system_cycles(ACQUIRE_CYCLES);
}

#[test]
fn cnet_cards_acquire_static_nodes_and_exchange_datagrams() {
    let dir = temporary_directory();
    let mut bus = load_cnet(&dir, TWO_CARDS);
    {
        let mut a = card(&mut bus, 0);
        assert_eq!(&[a.r8(0), a.r8(1), a.r8(2), a.r8(3)], b"CNET");
        assert_eq!(a.r16(0x16), 1024, "MTU");
        assert_eq!(a.r16(STATUS), ST_LINK);
    }
    enable_and_acquire(&mut bus, &[0, 1]);
    for (slot, node) in [(0, 0x10), (1, 0x20)] {
        let mut c = card(&mut bus, slot);
        assert_eq!(c.r16(STATUS), ST_LINK | ST_ADDR_VALID, "slot {slot}");
        assert_eq!(c.r16(NET), 1);
        assert_eq!(c.r16(NODE), node);
        assert_eq!(c.r16(IRQ_STATUS) & 1, 1, "ADDR event");
        c.w16(IRQ_STATUS, 1);
        assert_eq!(c.r16(IRQ_STATUS), 0);
    }

    assert_eq!(card(&mut bus, 1).open(0, 0x40), (0, 0x40));
    assert_eq!(card(&mut bus, 1).open(1, 0x40).0, 2, "socket in use");
    assert_eq!(card(&mut bus, 1).open(1, 4).0, 2, "echo socket is reserved");
    assert_eq!(card(&mut bus, 0).open(0, 0), (0, 128), "dynamic socket");
    card(&mut bus, 1).w16(IRQ_ENABLE, 2);

    let result = card(&mut bus, 0).send(0, 0, 0x20, 0x40, 0x80, b"hello");
    assert_eq!(result, 0);
    bus.advance_system_cycles(1);
    {
        let mut b = card(&mut bus, 1);
        assert_eq!(b.rx_count(0), 1);
        assert_eq!(b.r16(RX_READY), 1);
        assert_eq!(b.head(0), (1, 0x10, 128, 0x80, 0, b"hello".to_vec()));
        assert_eq!(b.r16(IRQ_STATUS), 2, "RX event");
    }
    let irq = bus
        .expansion_card_debug_info()
        .into_iter()
        .find(|info| info.slot == 1)
        .unwrap()
        .irq_asserted;
    assert!(irq, "RX IRQ is enabled on slot 1");
    {
        let mut b = card(&mut bus, 1);
        b.w8(Card::h(0) + 2, CMD_RX_POP);
        assert_eq!(b.rx_count(0), 0);
        assert_eq!(b.head(0).5, Vec::<u8>::new());
    }

    // Echo: the card answers socket 4 by itself.
    assert_eq!(
        card(&mut bus, 0).send(0, 0, 0x20, 4, 0x02, &[1, b'x', b'y']),
        0
    );
    bus.advance_system_cycles(1);
    bus.advance_system_cycles(1);
    assert_eq!(
        card(&mut bus, 0).head(0),
        (1, 0x20, 4, 0x02, 0, vec![2, b'x', b'y'])
    );
    card(&mut bus, 0).w8(Card::h(0) + 2, CMD_RX_POP);

    // Broadcast reaches other nodes and the sender's own matching socket.
    assert_eq!(card(&mut bus, 0).open(1, 0x40), (0, 0x40));
    assert_eq!(card(&mut bus, 0).send(0, 0, 0xFFFF, 0x40, 0x81, b"all"), 0);
    bus.advance_system_cycles(1);
    assert_eq!(
        card(&mut bus, 1).head(0),
        (1, 0x10, 128, 0x81, 1, b"all".to_vec())
    );
    assert_eq!(
        card(&mut bus, 0).head(1),
        (1, 0x10, 128, 0x81, 1, b"all".to_vec())
    );

    // Sending to oneself stays inside the card.
    assert_eq!(card(&mut bus, 0).send(0, 1, 0x10, 0x40, 0x82, b"me"), 0);
    assert_eq!(card(&mut bus, 0).rx_count(1), 2);

    // Errors.
    let mut a = card(&mut bus, 0);
    assert_eq!(a.send(0, 0, 0x20, 0x40, 0x80, &[0; 0]), 0);
    a.w16(Card::h(0) + 0x0E, 1025);
    a.w8(Card::h(0) + 2, CMD_SEND);
    assert_eq!(a.r8(Card::h(0) + 3), 4, "BAD_LEN");
    assert_eq!(a.send(0, 0, 0, 0x40, 0x80, b""), 5, "BAD_DEST node 0");
    assert_eq!(a.send(0, 0x0099, 0x20, 0x40, 0x80, b""), 9, "NO_ROUTE");
    assert_eq!(
        a.send(5, 0, 0x20, 0x40, 0x80, b""),
        1,
        "BAD_STATE closed handle"
    );
    a.w8(Card::h(0) + 2, CMD_CLOSE);
    assert_eq!(a.r8(Card::h(0) + 1), 0);

    // An unattached card reports no link and refuses to send.
    let mut c = card(&mut bus, 2);
    c.w16(CONTROL, 1);
    assert_eq!(c.r16(STATUS), 0);
    assert_eq!(c.open(0, 0x40).0, 0);
    assert_eq!(c.send(0, 0, 0x20, 0x40, 0x80, b""), 7, "NO_LINK");
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn cnet_receive_queue_overflow_counts_drops() {
    let dir = temporary_directory();
    let mut bus = load_cnet(&dir, TWO_CARDS);
    enable_and_acquire(&mut bus, &[0, 1]);
    card(&mut bus, 1).open(0, 0x40);
    card(&mut bus, 0).open(0, 0x41);
    for i in 0..10u8 {
        assert_eq!(card(&mut bus, 0).send(0, 0, 0x20, 0x40, 0x80, &[i]), 0);
        bus.advance_system_cycles(1);
    }
    let mut b = card(&mut bus, 1);
    assert_eq!(b.rx_count(0), 8);
    assert_eq!(b.r8(Card::h(0) + 5), 2);
    assert_eq!(b.head(0).5, vec![0]);
    b.w8(Card::h(0) + 5, 0);
    assert_eq!(b.r8(Card::h(0) + 5), 0);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn cnet_node_acquisition_detects_conflicts_and_retries_dynamic_nodes() {
    let dir = temporary_directory();
    let mut bus = load_cnet(
        &dir,
        r#"
network_segment:
  - { name: test, loopback: true }
expansion_card:
  - { slot: 0, package: cnet.cpt32card, network: { segment: test, node: 0x0010 } }
  - { slot: 1, package: cnet.cpt32card, network: { segment: test, node: 0x0010 } }
  - { slot: 2, package: cnet.cpt32card, network: { segment: test } }
"#,
    );
    enable_and_acquire(&mut bus, &[0]);
    assert_eq!(card(&mut bus, 0).r16(NODE), 0x10);

    // A fixed node that is already taken stops with CONFLICT.
    enable_and_acquire(&mut bus, &[1]);
    let mut b = card(&mut bus, 1);
    assert_eq!(b.r16(STATUS), ST_LINK | ST_CONFLICT);
    assert_eq!(b.r16(NODE), 0);
    assert_eq!(b.r16(IRQ_STATUS) & 1, 1);

    // A dynamic node whose hint is taken picks another one.
    card(&mut bus, 2).w16(NODE_HINT, 0x10);
    enable_and_acquire(&mut bus, &[2]);
    let mut c = card(&mut bus, 2);
    assert_eq!(c.r16(STATUS), ST_LINK | ST_ADDR_VALID);
    let node = c.r16(NODE);
    assert!(node != 0x10 && (1..=0xFEFF).contains(&node), "{node:#x}");
    assert_eq!(c.r16(NET), 0, "segment without a network number");

    // Disabling the link forgets the address and closes sockets.
    c.open(0, 0x40);
    c.w16(CONTROL, 0);
    assert_eq!(c.r16(STATUS), ST_LINK);
    assert_eq!(c.r8(Card::h(0) + 1), 0);
    fs::remove_dir_all(dir).unwrap();
}

fn free_udp_port() -> u16 {
    UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[test]
fn cnet_cards_in_two_emulators_talk_over_a_udp_segment() {
    let (port_a, port_b) = (free_udp_port(), free_udp_port());
    let make = |port: u16, peer: u16, node: u16| {
        let dir = temporary_directory();
        let bus = load_cnet(
            &dir,
            &format!(
                "network_segment:\n  - name: lan\n    network: 7\n    udp:\n      bind: \"127.0.0.1:{port}\"\n      peers: [\"127.0.0.1:{peer}\"]\n      segment_id: 3\nexpansion_card:\n  - {{ slot: 0, package: cnet.cpt32card, network: {{ segment: lan, node: {node} }} }}\n"
            ),
        );
        (dir, bus)
    };
    let (dir_a, mut a) = make(port_a, port_b, 0x11);
    let (dir_b, mut b) = make(port_b, port_a, 0x22);
    card(&mut a, 0).w16(CONTROL, 1);
    card(&mut b, 0).w16(CONTROL, 1);
    a.advance_system_cycles(ACQUIRE_CYCLES);
    b.advance_system_cycles(ACQUIRE_CYCLES);
    assert_eq!(card(&mut a, 0).r16(STATUS), ST_LINK | ST_ADDR_VALID);
    assert_eq!(card(&mut b, 0).r16(STATUS), ST_LINK | ST_ADDR_VALID);
    card(&mut b, 0).open(0, 0x40);
    card(&mut a, 0).open(0, 0x41);
    assert_eq!(card(&mut a, 0).send(0, 7, 0x22, 0x40, 0x80, b"over udp"), 0);

    let deadline = Instant::now() + Duration::from_secs(5);
    while card(&mut b, 0).rx_count(0) == 0 {
        assert!(
            Instant::now() < deadline,
            "datagram did not arrive over UDP"
        );
        std::thread::sleep(Duration::from_millis(5));
        b.advance_system_cycles(1);
    }
    assert_eq!(
        card(&mut b, 0).head(0),
        (7, 0x11, 0x41, 0x80, 0, b"over udp".to_vec())
    );
    drop((a, b));
    fs::remove_dir_all(dir_a).unwrap();
    fs::remove_dir_all(dir_b).unwrap();
}
