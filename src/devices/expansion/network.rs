//! CNET physical layer (`doc/spec_NET.md` §7–§8).
//!
//! The host only moves raw link frames between the cards attached to a
//! segment. The network stack itself lives in the card. A segment is either
//! in-process only (`loopback`) or additionally bridged over UDP; a UDP
//! receive thread pushes decoded frames straight into the attached ports'
//! queues, and the Bus delivers them to the guest at its next event point.

use std::collections::{HashMap, VecDeque};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

/// Link frame header: DST_NODE, SRC_NODE, TYPE, RSV, LENGTH.
pub(crate) const FRAME_HEADER: usize = 8;
/// 8-byte link header + 16-byte datagram header + 1024-byte payload.
pub(crate) const MAX_FRAME: usize = FRAME_HEADER + 16 + 1024;
/// Frames buffered per attached card before new ones are dropped.
pub(crate) const PORT_QUEUE: usize = 64;

const UDP_MAGIC: &[u8; 4] = b"CNET";
const UDP_VERSION: u8 = 1;
const UDP_HEADER: usize = 12;

/// Receive side of one attached card.
pub(crate) struct PortShared {
    queue: Mutex<VecDeque<Vec<u8>>>,
    /// Set when a frame is queued; cleared by the host when it notifies the card.
    pending: AtomicBool,
}

impl PortShared {
    fn new() -> Self {
        Self {
            queue: Mutex::new(VecDeque::with_capacity(PORT_QUEUE)),
            pending: AtomicBool::new(false),
        }
    }

    fn push(&self, frame: &[u8]) {
        let mut queue = self
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if queue.len() < PORT_QUEUE {
            queue.push_back(frame.to_vec());
            self.pending.store(true, Ordering::Release);
        }
    }
}

struct UdpLink {
    socket: UdpSocket,
    targets: Vec<SocketAddr>,
    segment_id: u16,
    sender: u32,
}

pub(crate) struct Segment {
    name: String,
    network: u16,
    ports: Mutex<Vec<Arc<PortShared>>>,
    udp: Option<UdpLink>,
}

impl Segment {
    fn attach(self: &Arc<Self>) -> Arc<PortShared> {
        let port = Arc::new(PortShared::new());
        self.ports
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(Arc::clone(&port));
        port
    }

    fn deliver_local(&self, from: Option<&Arc<PortShared>>, frame: &[u8]) {
        let ports = self
            .ports
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for port in ports.iter() {
            if from.is_some_and(|from| Arc::ptr_eq(from, port)) {
                continue;
            }
            port.push(frame);
        }
    }

    fn send(&self, from: &Arc<PortShared>, frame: &[u8]) {
        self.deliver_local(Some(from), frame);
        if let Some(udp) = &self.udp {
            let mut packet = Vec::with_capacity(UDP_HEADER + frame.len());
            packet.extend_from_slice(UDP_MAGIC);
            packet.push(UDP_VERSION);
            packet.push(0);
            packet.extend_from_slice(&udp.segment_id.to_be_bytes());
            packet.extend_from_slice(&udp.sender.to_be_bytes());
            packet.extend_from_slice(frame);
            for target in &udp.targets {
                if let Err(error) = udp.socket.send_to(&packet, target) {
                    tracing::debug!(segment = %self.name, %target, %error, "CNET UDP send failed");
                }
            }
        }
    }
}

/// Validate a link frame's shape (not its content; the card does that).
pub(crate) fn frame_shape_ok(frame: &[u8]) -> bool {
    frame.len() >= FRAME_HEADER
        && frame.len() <= MAX_FRAME
        && usize::from(u16::from_be_bytes([frame[6], frame[7]])) == frame.len() - FRAME_HEADER
}

fn udp_receive_loop(segment: Weak<Segment>, socket: UdpSocket, segment_id: u16, sender: u32) {
    let mut buffer = vec![0u8; UDP_HEADER + MAX_FRAME + 1];
    loop {
        let received = socket.recv_from(&mut buffer);
        let Some(segment) = segment.upgrade() else {
            return;
        };
        let length = match received {
            Ok((length, _)) => length,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                continue;
            }
            // Windows reports ICMP port-unreachable for earlier sends as a receive error.
            Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => continue,
            Err(error) => {
                tracing::warn!(segment = %segment.name, %error, "CNET UDP receive stopped");
                return;
            }
        };
        let packet = &buffer[..length];
        if packet.len() < UDP_HEADER
            || &packet[0..4] != UDP_MAGIC
            || packet[4] != UDP_VERSION
            || u16::from_be_bytes([packet[6], packet[7]]) != segment_id
            || u32::from_be_bytes([packet[8], packet[9], packet[10], packet[11]]) == sender
        {
            continue;
        }
        let frame = &packet[UDP_HEADER..];
        if frame_shape_ok(frame) {
            segment.deliver_local(None, frame);
        }
    }
}

/// One card's connection to a segment, owned by the card's host state.
pub(crate) struct NetPort {
    segment: Arc<Segment>,
    port: Arc<PortShared>,
    pub(crate) static_node: Option<u16>,
}

impl NetPort {
    pub(crate) fn network(&self) -> u16 {
        self.segment.network
    }

    pub(crate) fn segment_name(&self) -> &str {
        &self.segment.name
    }

    pub(crate) fn send(&self, frame: &[u8]) {
        self.segment.send(&self.port, frame);
    }

    pub(crate) fn recv(&self) -> Option<Vec<u8>> {
        self.port
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pop_front()
    }

    pub(crate) fn pending(&self) -> bool {
        self.port.pending.load(Ordering::Acquire)
    }

    pub(crate) fn take_pending(&self) -> bool {
        self.port.pending.swap(false, Ordering::AcqRel)
    }

    /// Drop queued frames (card reset).
    pub(crate) fn clear(&self) {
        self.port
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
        self.port.pending.store(false, Ordering::Release);
    }
}

/// Parsed `network_segment` entry; sockets are opened only when a card attaches.
struct SegmentConfig {
    network: u16,
    udp: Option<UdpConfig>,
}

struct UdpConfig {
    bind: SocketAddr,
    multicast: Option<SocketAddrV4>,
    peers: Vec<SocketAddr>,
    segment_id: u16,
}

/// Segments declared in `config.yaml`, opened lazily on first attachment.
pub(crate) struct Segments {
    configs: HashMap<String, SegmentConfig>,
    opened: HashMap<String, Arc<Segment>>,
}

impl Segments {
    pub(crate) fn empty() -> Self {
        Self {
            configs: HashMap::new(),
            opened: HashMap::new(),
        }
    }

    pub(crate) fn parse(root: &yaml_serde::Value, config_path: &Path) -> Result<Self, String> {
        let mut segments = Self::empty();
        let Some(entries) = root.get("network_segment") else {
            return Ok(segments);
        };
        let where_ = config_path.display();
        let entries = entries
            .as_sequence()
            .ok_or_else(|| format!("`network_segment` in {where_} must be a sequence"))?;
        for (index, entry) in entries.iter().enumerate() {
            let context = |message: String| format!("network_segment entry {index}: {message}");
            let map = entry
                .as_mapping()
                .ok_or_else(|| context("must be a mapping".into()))?;
            for key in map.keys() {
                let key = key.as_str().unwrap_or("");
                if !matches!(key, "name" | "network" | "loopback" | "udp") {
                    return Err(context(format!("unknown key {key:?}")));
                }
            }
            let name = map
                .get("name")
                .and_then(yaml_serde::Value::as_str)
                .ok_or_else(|| context("needs a string name".into()))?
                .to_owned();
            let network = match map.get("network") {
                None => 0,
                Some(value) => u16_value(value)
                    .filter(|&network| network != 0xFFFF)
                    .ok_or_else(|| context("network must be 0..0xFFFE".into()))?,
            };
            let loopback = match map.get("loopback") {
                None => false,
                Some(value) => value
                    .as_bool()
                    .ok_or_else(|| context("loopback must be a boolean".into()))?,
            };
            let udp = match map.get("udp") {
                None => None,
                Some(value) => Some(parse_udp(value).map_err(context)?),
            };
            if loopback == udp.is_some() {
                return Err(context(
                    "set exactly one of `loopback: true` or `udp`".into(),
                ));
            }
            if segments
                .configs
                .insert(name.clone(), SegmentConfig { network, udp })
                .is_some()
            {
                return Err(context(format!("duplicate segment name {name:?}")));
            }
        }
        Ok(segments)
    }

    pub(crate) fn attach(
        &mut self,
        name: &str,
        static_node: Option<u16>,
    ) -> Result<NetPort, String> {
        let segment = match self.opened.get(name) {
            Some(segment) => Arc::clone(segment),
            None => {
                let config = self
                    .configs
                    .get(name)
                    .ok_or_else(|| format!("unknown network segment {name:?}"))?;
                let segment = open_segment(name, config)?;
                self.opened.insert(name.to_owned(), Arc::clone(&segment));
                segment
            }
        };
        let port = segment.attach();
        Ok(NetPort {
            segment,
            port,
            static_node,
        })
    }
}

fn open_segment(name: &str, config: &SegmentConfig) -> Result<Arc<Segment>, String> {
    let Some(udp) = &config.udp else {
        return Ok(Arc::new(Segment {
            name: name.to_owned(),
            network: config.network,
            ports: Mutex::new(Vec::new()),
            udp: None,
        }));
    };
    let context = |message: String| format!("network segment {name:?}: {message}");
    let socket = bind_udp(udp).map_err(|error| context(error.to_string()))?;
    let receiver = socket
        .try_clone()
        .map_err(|error| context(error.to_string()))?;
    receiver
        .set_read_timeout(Some(Duration::from_millis(200)))
        .map_err(|error| context(error.to_string()))?;
    let sender: u32 = rand::random();
    let mut targets: Vec<SocketAddr> = udp.peers.clone();
    if let Some(group) = udp.multicast {
        targets.push(SocketAddr::V4(group));
    }
    let segment = Arc::new(Segment {
        name: name.to_owned(),
        network: config.network,
        ports: Mutex::new(Vec::new()),
        udp: Some(UdpLink {
            socket,
            targets,
            segment_id: udp.segment_id,
            sender,
        }),
    });
    let weak = Arc::downgrade(&segment);
    let segment_id = udp.segment_id;
    std::thread::Builder::new()
        .name(format!("cnet-udp-{name}"))
        .spawn(move || udp_receive_loop(weak, receiver, segment_id, sender))
        .map_err(|error| context(error.to_string()))?;
    tracing::info!(segment = name, bind = %udp.bind, "CNET UDP segment opened");
    Ok(segment)
}

fn bind_udp(config: &UdpConfig) -> std::io::Result<UdpSocket> {
    use socket2::{Domain, Protocol, Socket, Type};
    let domain = if config.bind.is_ipv4() {
        Domain::IPV4
    } else {
        Domain::IPV6
    };
    let socket = Socket::new(domain, Type::DGRAM, Some(Protocol::UDP))?;
    if config.multicast.is_some() {
        // Several emulators on one host share the multicast port.
        socket.set_reuse_address(true)?;
    }
    socket.bind(&config.bind.into())?;
    if let Some(group) = config.multicast {
        let interface = match config.bind {
            SocketAddr::V4(bind) => *bind.ip(),
            SocketAddr::V6(_) => Ipv4Addr::UNSPECIFIED,
        };
        socket.join_multicast_v4(group.ip(), &interface)?;
        socket.set_multicast_loop_v4(true)?;
    }
    Ok(socket.into())
}

fn parse_udp(value: &yaml_serde::Value) -> Result<UdpConfig, String> {
    let map = value
        .as_mapping()
        .ok_or_else(|| "udp must be a mapping".to_owned())?;
    for key in map.keys() {
        let key = key.as_str().unwrap_or("");
        if !matches!(key, "bind" | "multicast" | "peers" | "segment_id") {
            return Err(format!("unknown udp key {key:?}"));
        }
    }
    let address = |value: &yaml_serde::Value, what: &str| -> Result<SocketAddr, String> {
        value
            .as_str()
            .and_then(|text| text.parse().ok())
            .ok_or_else(|| format!("udp {what} must be an \"address:port\" string"))
    };
    let bind = address(
        map.get("bind")
            .ok_or_else(|| "udp needs a bind address".to_owned())?,
        "bind",
    )?;
    let multicast = match map.get("multicast") {
        None => None,
        Some(value) => match address(value, "multicast")? {
            SocketAddr::V4(group) if group.ip().is_multicast() => Some(group),
            _ => return Err("udp multicast must be an IPv4 multicast address".into()),
        },
    };
    let peers = match map.get("peers") {
        None => Vec::new(),
        Some(value) => value
            .as_sequence()
            .ok_or_else(|| "udp peers must be a sequence".to_owned())?
            .iter()
            .map(|peer| address(peer, "peer"))
            .collect::<Result<_, _>>()?,
    };
    let segment_id = match map.get("segment_id") {
        None => 0,
        Some(value) => u16_value(value).ok_or_else(|| "segment_id must be 0..0xFFFF".to_owned())?,
    };
    Ok(UdpConfig {
        bind,
        multicast,
        peers,
        segment_id,
    })
}

pub(crate) fn u16_value(value: &yaml_serde::Value) -> Option<u16> {
    value.as_u64().and_then(|value| u16::try_from(value).ok())
}

/// Parse an `expansion_card[].network` mapping into (segment, static node).
pub(crate) fn parse_card_network(
    value: &yaml_serde::Value,
) -> Result<(String, Option<u16>), String> {
    let map = value
        .as_mapping()
        .ok_or_else(|| "network must be a mapping".to_owned())?;
    for key in map.keys() {
        let key = key.as_str().unwrap_or("");
        if !matches!(key, "segment" | "node") {
            return Err(format!("unknown network key {key:?}"));
        }
    }
    let segment = map
        .get("segment")
        .and_then(yaml_serde::Value::as_str)
        .ok_or_else(|| "network needs a segment name".to_owned())?
        .to_owned();
    let node = match map.get("node") {
        None => None,
        Some(value) => Some(
            u16_value(value)
                .filter(|node| (1..=0xFEFF).contains(node))
                .ok_or_else(|| "network node must be 0x0001..0xFEFF".to_owned())?,
        ),
    };
    Ok((segment, node))
}
