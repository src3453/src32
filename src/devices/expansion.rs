mod package;
use std::cell::Cell;
use std::io::Cursor;
use std::path::Path;
use std::rc::Rc;

use wasmtime::{
    Caller, Config, Engine, ExternType, FuncType, Instance, Linker, Memory, Module, Store,
    StoreLimits, StoreLimitsBuilder, TypedFunc, ValType,
};

use crate::bus::Device;

pub const EXPANSION_SLOT_COUNT: u8 = 8;
pub const EXPANSION_SLOT_SIZE: u32 = 0x0200_0000;
pub const EXPANSION_BASE: u32 = 0xA000_0000;
const GUEST_FUEL: u64 = 10_000_000;
const MEMORY_LIMIT: usize = 64 * 1024 * 1024;
// Leave fuel headroom for polyphonic synthesis and resampling in each call.
const AUDIO_BLOCK_FRAMES: usize = 64;

/// Guest entry points metered separately in [`ExpansionCardDebugInfo`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExpansionCallKind {
    /// Module start function plus `cpt32_abi_version` during load.
    Start,
    Reset,
    MmioRead,
    MmioWrite,
    Tick,
    DebugRead,
    AudioRender,
}

impl ExpansionCallKind {
    pub const COUNT: usize = 7;
    pub const ALL: [Self; Self::COUNT] = [
        Self::Start,
        Self::Reset,
        Self::MmioRead,
        Self::MmioWrite,
        Self::Tick,
        Self::DebugRead,
        Self::AudioRender,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Start => "start + cpt32_abi_version",
            Self::Reset => "cpt32_reset",
            Self::MmioRead => "cpt32_mmio_read",
            Self::MmioWrite => "cpt32_mmio_write",
            Self::Tick => "cpt32_tick",
            Self::DebugRead => "cpt32_debug_read",
            Self::AudioRender => "cpt32_audio_render",
        }
    }
}

/// Cumulative fuel accounting for one guest entry point. A call that runs out
/// of fuel counts the whole per-call budget.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExpansionCallStats {
    pub calls: u64,
    pub fuel_total: u64,
    pub fuel_last: u64,
    pub fuel_peak: u64,
}

impl ExpansionCallStats {
    fn record(&mut self, fuel: u64) {
        self.calls += 1;
        self.fuel_total += fuel;
        self.fuel_last = fuel;
        self.fuel_peak = self.fuel_peak.max(fuel);
    }
}

/// Side-effect-free snapshot of one installed card for debuggers.
#[derive(Clone, Debug)]
pub struct ExpansionCardDebugInfo {
    pub slot: u8,
    pub label: String,
    pub faulted: bool,
    pub fault_reason: Option<String>,
    pub irq_asserted: bool,
    pub deadline: Option<u64>,
    /// Fuel granted to every guest call.
    pub fuel_limit: u64,
    pub calls: [ExpansionCallStats; ExpansionCallKind::COUNT],
    /// `cpt32_host` imports declared by the module.
    pub imports: Vec<String>,
    pub has_debug_read: bool,
    pub has_audio: bool,
    /// Current size of the exported `memory`, if the module exports one.
    pub memory_bytes: Option<usize>,
    pub memory_limit: usize,
    pub asset_count: i32,
}

impl ExpansionCardDebugInfo {
    pub fn call(&self, kind: ExpansionCallKind) -> &ExpansionCallStats {
        &self.calls[kind as usize]
    }

    /// Highest single-call fuel use as a fraction of the per-call budget.
    pub fn peak_fuel_ratio(&self) -> f64 {
        let peak = self
            .calls
            .iter()
            .map(|call| call.fuel_peak)
            .max()
            .unwrap_or(0);
        peak as f64 / self.fuel_limit as f64
    }

    /// Short human-readable list of the host features the card uses.
    pub fn features(&self) -> Vec<&'static str> {
        let imports = |names: &[&str]| self.imports.iter().any(|i| names.contains(&i.as_str()));
        let mut features = vec!["MMIO"];
        if imports(&["schedule_tick"]) {
            features.push("tick timer");
        }
        if imports(&["irq_set"]) {
            features.push("IRQ");
        }
        if imports(&["current_cycles"]) {
            features.push("cycle clock");
        }
        if imports(&["asset_count", "asset_size", "asset_read_byte"]) {
            features.push("assets");
        }
        if self.has_audio {
            features.push("audio");
        }
        if self.has_debug_read {
            features.push("debug read");
        }
        features
    }
}

#[derive(Clone)]
pub(crate) struct ExpansionRuntime {
    engine: Engine,
}

impl ExpansionRuntime {
    pub(crate) fn new() -> Result<Self, String> {
        let mut config = Config::new();
        config.consume_fuel(true);
        let engine = Engine::new(&config).map_err(|error| error.to_string())?;
        Ok(Self { engine })
    }

    pub(crate) fn load_card_package(
        &self,
        slot: u8,
        path: &Path,
        cycles: Rc<Cell<u64>>,
    ) -> Result<ExpansionCard, String> {
        let label = path.display().to_string();
        if slot >= EXPANSION_SLOT_COUNT {
            return Err(format!(
                "slot {slot} package {label}: slot is outside 0..{}",
                EXPANSION_SLOT_COUNT - 1
            ));
        }
        let context = |message: String| format!("slot {slot} package {label}: {message}");
        let (module_bytes, assets) = package::PackageAssets::load(path).map_err(context)?;
        self.load_card_components(slot, &label, &module_bytes, cycles, assets)
    }
    pub(crate) fn load_card_package_bytes(
        &self,
        slot: u8,
        label: &str,
        bytes: Vec<u8>,
        cycles: Rc<Cell<u64>>,
    ) -> Result<ExpansionCard, String> {
        let context = |message: String| format!("slot {slot} package {label}: {message}");
        let (module_bytes, assets) = package::PackageAssets::load_reader(Box::new(Cursor::new(
            bytes,
        ))
            as Box<dyn package::ReadSeek>)
        .map_err(context)?;
        self.load_card_components(slot, label, &module_bytes, cycles, assets)
    }

    fn load_card_components(
        &self,
        slot: u8,
        label: &str,
        bytes: &[u8],
        cycles: Rc<Cell<u64>>,
        assets: package::PackageAssets<Box<dyn package::ReadSeek>>,
    ) -> Result<ExpansionCard, String> {
        if slot >= EXPANSION_SLOT_COUNT {
            return Err(format!(
                "slot {slot} is outside 0..{} ({label})",
                EXPANSION_SLOT_COUNT - 1
            ));
        }
        let context = |message: String| format!("slot {slot} module {label}: {message}");
        let module =
            Module::new(&self.engine, bytes).map_err(|error| context(error.to_string()))?;
        validate_module(&module).map_err(context)?;

        let state = HostState {
            cycles,
            deadline: None,
            irq_asserted: false,
            faulted: false,
            assets,
            limits: StoreLimitsBuilder::new()
                .memory_size(MEMORY_LIMIT)
                .trap_on_grow_failure(true)
                .build(),
        };
        let mut store = Store::new(&self.engine, state);
        store.limiter(|state| &mut state.limits);
        store
            .set_fuel(GUEST_FUEL)
            .map_err(|error| context(error.to_string()))?;

        let mut linker = Linker::new(&self.engine);
        linker
            .func_wrap(
                "cpt32_host",
                "current_cycles",
                |caller: Caller<'_, HostState>| caller.data().cycles.get() as i64,
            )
            .map_err(|error| context(error.to_string()))?;
        linker
            .func_wrap(
                "cpt32_host",
                "schedule_tick",
                |mut caller: Caller<'_, HostState>, delay: i64| -> wasmtime::Result<()> {
                    if delay < 0 {
                        return Err(wasmtime::Error::msg("negative schedule_tick delay"));
                    }
                    let state = caller.data_mut();
                    if delay == 0 {
                        state.deadline = None;
                    } else {
                        state.deadline =
                            Some(state.cycles.get().checked_add(delay as u64).ok_or_else(
                                || wasmtime::Error::msg("schedule_tick deadline overflow"),
                            )?);
                    }
                    Ok(())
                },
            )
            .map_err(|error| context(error.to_string()))?;
        linker
            .func_wrap(
                "cpt32_host",
                "irq_set",
                |mut caller: Caller<'_, HostState>, asserted: i32| -> wasmtime::Result<()> {
                    match asserted {
                        0 => caller.data_mut().irq_asserted = false,
                        1 => caller.data_mut().irq_asserted = true,
                        _ => return Err(wasmtime::Error::msg("irq_set expects 0 or 1")),
                    }
                    Ok(())
                },
            )
            .map_err(|error| context(error.to_string()))?;

        linker
            .func_wrap(
                "cpt32_host",
                "asset_count",
                |caller: Caller<'_, HostState>| caller.data().assets.count(),
            )
            .map_err(|error| context(error.to_string()))?;
        linker
            .func_wrap(
                "cpt32_host",
                "asset_size",
                |caller: Caller<'_, HostState>, id: i32| caller.data().assets.size(id),
            )
            .map_err(|error| context(error.to_string()))?;
        linker
            .func_wrap(
                "cpt32_host",
                "asset_read_byte",
                |mut caller: Caller<'_, HostState>, id: i32, offset: i32| {
                    caller.data_mut().assets.read_byte(id, offset)
                },
            )
            .map_err(|error| context(error.to_string()))?;
        // Fuel is present before instantiation because a module start function is guest code.
        let instance = linker
            .instantiate(&mut store, &module)
            .map_err(|error| context(format!("instantiation failed: {error}")))?;
        let mut start_fuel = GUEST_FUEL - store.get_fuel().unwrap_or(0);
        let abi_version = instance
            .get_typed_func::<(), i32>(&mut store, "cpt32_abi_version")
            .map_err(|error| context(error.to_string()))?;
        store
            .set_fuel(GUEST_FUEL)
            .map_err(|error| context(error.to_string()))?;
        let version = abi_version
            .call(&mut store, ())
            .map_err(|error| context(format!("cpt32_abi_version trapped: {error}")))?;
        start_fuel += GUEST_FUEL - store.get_fuel().unwrap_or(0);
        if version != 1 {
            return Err(context(format!(
                "unsupported ABI version {version}; expected 1"
            )));
        }

        let reset = instance
            .get_typed_func::<(), ()>(&mut store, "cpt32_reset")
            .map_err(|error| context(error.to_string()))?;
        let mmio_read = instance
            .get_typed_func::<i32, i32>(&mut store, "cpt32_mmio_read")
            .map_err(|error| context(error.to_string()))?;
        let mmio_write = instance
            .get_typed_func::<(i32, i32), ()>(&mut store, "cpt32_mmio_write")
            .map_err(|error| context(error.to_string()))?;
        let tick = instance
            .get_typed_func::<(), ()>(&mut store, "cpt32_tick")
            .map_err(|error| context(error.to_string()))?;

        let debug_read = if module
            .exports()
            .any(|export| export.name() == "cpt32_debug_read")
        {
            Some(
                instance
                    .get_typed_func::<i32, i32>(&mut store, "cpt32_debug_read")
                    .map_err(|error| context(error.to_string()))?,
            )
        } else {
            None
        };

        let audio = if module
            .exports()
            .any(|export| export.name() == "cpt32_audio_render")
        {
            Some((
                instance
                    .get_typed_func::<(i32, i32), i32>(&mut store, "cpt32_audio_render")
                    .map_err(|error| context(error.to_string()))?,
                instance
                    .get_memory(&mut store, "memory")
                    .ok_or_else(|| context("audio requires exported memory".into()))?,
            ))
        } else {
            None
        };

        let memory = instance.get_memory(&mut store, "memory");
        let imports = module
            .imports()
            .map(|import| import.name().to_owned())
            .collect();
        let mut stats = [ExpansionCallStats::default(); ExpansionCallKind::COUNT];
        stats[ExpansionCallKind::Start as usize].record(start_fuel);

        Ok(ExpansionCard {
            slot,
            label: label.to_owned(),
            store,
            memory,
            imports,
            stats,
            fault_reason: None,
            _instance: instance,
            reset,
            mmio_read,
            debug_read,
            mmio_write,
            tick,
            audio,
        })
    }
}

#[derive(Debug)]
pub struct ExpansionCardPackage {
    pub slot: u8,
    pub label: String,
    pub bytes: Vec<u8>,
}

pub fn load_expansion_cards_from_bytes(
    packages: Vec<ExpansionCardPackage>,
    bus: &mut crate::bus::Bus,
) -> Result<(), String> {
    let mut slots = [false; EXPANSION_SLOT_COUNT as usize];
    for package in &packages {
        if package.slot >= EXPANSION_SLOT_COUNT {
            return Err(format!(
                "slot {}, package {}: slot must be 0..7",
                package.slot, package.label
            ));
        }
        if slots[usize::from(package.slot)] {
            return Err(format!(
                "slot {}, package {}: duplicate slot",
                package.slot, package.label
            ));
        }
        slots[usize::from(package.slot)] = true;
    }
    if packages.is_empty() {
        return Ok(());
    }

    let runtime = ExpansionRuntime::new()?;
    let cycle_clock = bus.cycle_clock();
    let mut cards = Vec::with_capacity(packages.len());
    for package in packages {
        let card = runtime
            .load_card_package_bytes(
                package.slot,
                &package.label,
                package.bytes,
                Rc::clone(&cycle_clock),
            )
            .map_err(|error| format!("package {}: {error}", package.label))?;
        cards.push((package.slot, card));
    }
    bus.install_expansion_cards(cards)
}

pub fn load_expansion_cards(config_path: &Path, bus: &mut crate::bus::Bus) -> Result<(), String> {
    let contents = match std::fs::read_to_string(config_path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("cannot read {}: {error}", config_path.display())),
    };
    let root: yaml_serde::Value = yaml_serde::from_str(&contents)
        .map_err(|error| format!("invalid YAML in {}: {error}", config_path.display()))?;
    let Some(entries) = root.get("expansion_card") else {
        return Ok(());
    };
    let entries = entries.as_sequence().ok_or_else(|| {
        format!(
            "`expansion_card` in {} must be a sequence of mappings",
            config_path.display()
        )
    })?;
    if entries.is_empty() {
        return Ok(());
    }

    let base = config_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut slots = [false; EXPANSION_SLOT_COUNT as usize];
    let mut modules = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        let table = entry.as_mapping().ok_or_else(|| {
            format!(
                "expansion_card entry {index} in {} must be a mapping",
                config_path.display()
            )
        })?;
        let package_value = table.get("package");
        let package_label = package_value
            .and_then(yaml_serde::Value::as_str)
            .unwrap_or("<invalid or missing package>");
        let slot_value = table.get("slot").ok_or_else(|| {
            format!("expansion_card entry {index} has no integer slot, package {package_label}")
        })?;
        let slot_integer = slot_value.as_i64().ok_or_else(|| {
            format!(
                "expansion_card entry {index} has invalid slot {:?}, package {package_label}",
                slot_value
            )
        })?;
        let slot = u8::try_from(slot_integer)
            .ok()
            .filter(|&slot| slot < EXPANSION_SLOT_COUNT)
            .ok_or_else(|| {
                format!(
                    "expansion_card slot {slot_integer}, package {package_label}: slot must be 0..7"
                )
            })?;
        let package = package_value
            .and_then(yaml_serde::Value::as_str)
            .ok_or_else(|| {
                format!("expansion_card slot {slot}: package {package_label} must be a path string")
            })?;
        if slots[usize::from(slot)] {
            return Err(format!(
                "expansion_card slot {slot}, package {package}: duplicate slot"
            ));
        }
        slots[usize::from(slot)] = true;
        let package_path = Path::new(package);
        let resolved = if package_path.is_absolute() {
            package_path.to_path_buf()
        } else {
            base.join(package_path)
        };
        modules.push((slot, package.to_owned(), resolved));
    }

    let runtime = ExpansionRuntime::new()?;
    let cycle_clock = bus.cycle_clock();
    let mut cards = Vec::with_capacity(modules.len());
    for (slot, label, path) in modules {
        let card = runtime
            .load_card_package(slot, &path, Rc::clone(&cycle_clock))
            .map_err(|error| format!("slot {slot} package {label}: {error}"))?;
        cards.push((slot, card));
        tracing::info!(slot, package = %label, path = %path.display(), "Loaded expansion card");
    }
    bus.install_expansion_cards(cards)
}

struct HostState {
    cycles: Rc<Cell<u64>>,
    deadline: Option<u64>,
    irq_asserted: bool,
    faulted: bool,
    assets: package::PackageAssets<Box<dyn package::ReadSeek>>,
    limits: StoreLimits,
}

pub(crate) struct ExpansionCard {
    slot: u8,
    label: String,
    store: Store<HostState>,
    memory: Option<Memory>,
    imports: Vec<String>,
    stats: [ExpansionCallStats; ExpansionCallKind::COUNT],
    fault_reason: Option<String>,
    // Keep the instance alive explicitly alongside its typed exports.
    _instance: Instance,
    reset: TypedFunc<(), ()>,
    mmio_read: TypedFunc<i32, i32>,
    debug_read: Option<TypedFunc<i32, i32>>,
    mmio_write: TypedFunc<(i32, i32), ()>,
    tick: TypedFunc<(), ()>,
    audio: Option<(TypedFunc<(i32, i32), i32>, Memory)>,
}

impl ExpansionCard {
    pub(crate) fn slot(&self) -> u8 {
        self.slot
    }

    pub fn is_faulted(&self) -> bool {
        self.store.data().faulted
    }

    pub fn debug_info(&self) -> ExpansionCardDebugInfo {
        let state = self.store.data();
        ExpansionCardDebugInfo {
            slot: self.slot,
            label: self.label.clone(),
            faulted: state.faulted,
            fault_reason: self.fault_reason.clone(),
            irq_asserted: state.irq_asserted,
            deadline: state.deadline,
            fuel_limit: GUEST_FUEL,
            calls: self.stats,
            imports: self.imports.clone(),
            has_debug_read: self.debug_read.is_some(),
            has_audio: self.audio.is_some(),
            memory_bytes: self.memory.map(|memory| memory.data_size(&self.store)),
            memory_limit: MEMORY_LIMIT,
            asset_count: state.assets.count(),
        }
    }

    pub fn reset_debug_stats(&mut self) {
        self.stats = Default::default();
    }

    pub fn irq_asserted(&self) -> bool {
        self.store.data().irq_asserted
    }

    pub fn next_event_cycle(&self) -> Option<u64> {
        if self.is_faulted() {
            None
        } else {
            self.store.data().deadline
        }
    }

    pub fn reset_card(&mut self) {
        {
            let state = self.store.data_mut();
            state.deadline = None;
            state.irq_asserted = false;
        }
        if self.is_faulted() {
            return;
        }
        let reset = self.reset.clone();
        if let Err(error) = self.call_guest(ExpansionCallKind::Reset, |store| reset.call(store, ()))
        {
            self.fault(format!("cpt32_reset trapped: {error}"));
        }
    }

    pub fn run_scheduled_event(&mut self, cycle: u64) {
        if self.is_faulted()
            || !self
                .store
                .data()
                .deadline
                .is_some_and(|deadline| deadline <= cycle)
        {
            return;
        }
        self.store.data_mut().deadline = None;
        let tick = self.tick.clone();
        if let Err(error) = self.call_guest(ExpansionCallKind::Tick, |store| tick.call(store, ())) {
            self.fault(format!("cpt32_tick trapped: {error}"));
        }
    }

    fn call_guest<T>(
        &mut self,
        kind: ExpansionCallKind,
        call: impl FnOnce(&mut Store<HostState>) -> wasmtime::Result<T>,
    ) -> wasmtime::Result<T> {
        self.store.set_fuel(GUEST_FUEL)?;
        let result = call(&mut self.store);
        let used = GUEST_FUEL - self.store.get_fuel().unwrap_or(0);
        self.stats[kind as usize].record(used);
        result
    }

    fn fault(&mut self, reason: String) {
        if self.store.data().faulted {
            return;
        }
        let state = self.store.data_mut();
        state.faulted = true;
        state.deadline = None;
        state.irq_asserted = false;
        self.fault_reason = Some(reason.clone());
        tracing::error!(slot = self.slot, label = %self.label, %reason, "WASM expansion card faulted");
    }
}

impl Device for ExpansionCard {
    fn render_audio(&mut self, sample_rate: u32, left: &mut [i16], right: &mut [i16]) {
        let Some((render, memory)) = self.audio.clone() else {
            return;
        };
        if self.is_faulted() {
            return;
        }
        let frames = left.len().min(right.len());
        // About 1 ms, capped at 64 frames. Low-rate outputs may resample a
        // higher-rate chip internally, so they need proportionally smaller calls.
        let block_frames = AUDIO_BLOCK_FRAMES.min((sample_rate as usize / 1000).max(1));
        for start in (0..frames).step_by(block_frames) {
            let count = (frames - start).min(block_frames);
            let mut bytes = [0u8; AUDIO_BLOCK_FRAMES * 4];
            let result = self
                .call_guest(ExpansionCallKind::AudioRender, |store| {
                    render.call(store, (count as i32, sample_rate as i32))
                })
                .and_then(|ptr| {
                    memory
                        .read(&self.store, ptr as u32 as usize, &mut bytes[..count * 4])
                        .map_err(|error| wasmtime::Error::msg(error.to_string()))
                });
            if let Err(error) = result {
                self.fault(format!("cpt32_audio_render failed: {error:#}"));
                return;
            }
            for (i, frame) in bytes[..count * 4].chunks_exact(4).enumerate() {
                left[start + i] =
                    left[start + i].saturating_add(i16::from_le_bytes([frame[0], frame[1]]));
                right[start + i] =
                    right[start + i].saturating_add(i16::from_le_bytes([frame[2], frame[3]]));
            }
        }
    }

    fn read(&mut self, offset: u32) -> u8 {
        if self.is_faulted() || offset >= EXPANSION_SLOT_SIZE {
            return 0;
        }
        let Ok(offset) = i32::try_from(offset) else {
            self.fault(format!("MMIO read offset out of range: 0x{offset:X}"));
            return 0;
        };
        let mmio_read = self.mmio_read.clone();
        match self.call_guest(ExpansionCallKind::MmioRead, |store| {
            mmio_read.call(store, offset)
        }) {
            Ok(value) if (0..=255).contains(&value) => value as u8,
            Ok(value) => {
                self.fault(format!("cpt32_mmio_read returned non-byte value {value}"));
                0
            }
            Err(error) => {
                self.fault(format!("cpt32_mmio_read trapped: {error}"));
                0
            }
        }
    }

    fn debug_read(&mut self, offset: u32) -> Option<u8> {
        if self.is_faulted() || offset >= EXPANSION_SLOT_SIZE {
            return None;
        }
        let debug_read = self.debug_read.clone()?;
        let offset = i32::try_from(offset).ok()?;
        match self.call_guest(ExpansionCallKind::DebugRead, |store| {
            debug_read.call(store, offset)
        }) {
            Ok(value) if (0..=255).contains(&value) => Some(value as u8),
            Ok(-1) => None,
            Ok(value) => {
                self.fault(format!("cpt32_debug_read returned invalid value {value}"));
                None
            }
            Err(error) => {
                self.fault(format!("cpt32_debug_read trapped: {error}"));
                None
            }
        }
    }

    fn write(&mut self, offset: u32, value: u8) {
        if self.is_faulted() || offset >= EXPANSION_SLOT_SIZE {
            return;
        }
        let Ok(offset) = i32::try_from(offset) else {
            self.fault(format!("MMIO write offset out of range: 0x{offset:X}"));
            return;
        };
        let mmio_write = self.mmio_write.clone();
        if let Err(error) = self.call_guest(ExpansionCallKind::MmioWrite, |store| {
            mmio_write.call(store, (offset, i32::from(value)))
        }) {
            self.fault(format!("cpt32_mmio_write trapped: {error}"));
        }
    }

    fn size(&self) -> u32 {
        EXPANSION_SLOT_SIZE
    }

    fn next_event_cycle(&self) -> Option<u64> {
        ExpansionCard::next_event_cycle(self)
    }

    fn run_scheduled_event(&mut self, cycle: u64) {
        ExpansionCard::run_scheduled_event(self, cycle);
    }

    fn is_faulted(&self) -> bool {
        ExpansionCard::is_faulted(self)
    }

    fn irq_asserted(&self) -> bool {
        ExpansionCard::irq_asserted(self)
    }

    fn reset(&mut self) {
        self.reset_card();
    }

    fn expansion_card_debug_info(&self) -> Option<ExpansionCardDebugInfo> {
        Some(self.debug_info())
    }

    fn reset_expansion_card_debug_stats(&mut self) {
        self.reset_debug_stats();
    }
}

fn function_type_matches(
    engine: &Engine,
    ty: &ExternType,
    params: &[ValType],
    results: &[ValType],
) -> bool {
    let ExternType::Func(actual) = ty else {
        return false;
    };
    let expected = FuncType::new(engine, params.iter().cloned(), results.iter().cloned());
    actual.matches(&expected)
}

fn validate_module(module: &Module) -> Result<(), String> {
    let engine = module.engine();
    if let Some(export) = module
        .exports()
        .find(|export| export.name() == "cpt32_debug_read")
    {
        if !function_type_matches(engine, &export.ty(), &[ValType::I32], &[ValType::I32]) {
            return Err("wrong type for export cpt32_debug_read".into());
        }
    }
    if let Some(export) = module
        .exports()
        .find(|export| export.name() == "cpt32_audio_render")
    {
        if !function_type_matches(
            engine,
            &export.ty(),
            &[ValType::I32, ValType::I32],
            &[ValType::I32],
        ) {
            return Err("wrong type for export cpt32_audio_render".into());
        }
        if !module
            .exports()
            .any(|export| export.name() == "memory" && matches!(export.ty(), ExternType::Memory(_)))
        {
            return Err("audio requires exported memory".into());
        }
    }
    for import in module.imports() {
        if import.module() != "cpt32_host" {
            return Err(format!("unsupported import namespace {}", import.module()));
        }
        let valid = match import.name() {
            "current_cycles" => function_type_matches(engine, &import.ty(), &[], &[ValType::I64]),
            "schedule_tick" => function_type_matches(engine, &import.ty(), &[ValType::I64], &[]),
            "irq_set" => function_type_matches(engine, &import.ty(), &[ValType::I32], &[]),
            "asset_count" => function_type_matches(engine, &import.ty(), &[], &[ValType::I32]),
            "asset_size" => {
                function_type_matches(engine, &import.ty(), &[ValType::I32], &[ValType::I32])
            }
            "asset_read_byte" => function_type_matches(
                engine,
                &import.ty(),
                &[ValType::I32, ValType::I32],
                &[ValType::I32],
            ),
            name => return Err(format!("unsupported import cpt32_host::{name}")),
        };
        if !valid {
            return Err(format!(
                "wrong type for import cpt32_host::{}",
                import.name()
            ));
        }
    }

    let exports = [
        ("cpt32_abi_version", vec![], vec![ValType::I32]),
        ("cpt32_reset", vec![], vec![]),
        ("cpt32_mmio_read", vec![ValType::I32], vec![ValType::I32]),
        ("cpt32_mmio_write", vec![ValType::I32, ValType::I32], vec![]),
        ("cpt32_tick", vec![], vec![]),
    ];
    for (name, params, results) in exports {
        let export = module
            .exports()
            .find(|export| export.name() == name)
            .ok_or_else(|| format!("missing export {name}"))?;
        if !function_type_matches(engine, &export.ty(), &params, &results) {
            return Err(format!("wrong type for export {name}"));
        }
    }
    Ok(())
}
