#![no_std]

//! Minimal `no_std` support for implementing CPT32 WASM expansion cards.
//!
//! Implement [`Card`] for the card state and invoke [`export_card!`] exactly once
//! to provide the ABI v1 exports. Host functions trap on invalid asset IDs,
//! offsets, or ABI arguments; keep those arguments within the declared package
//! and host contract.

/// Stateful card logic invoked by the CPT32 host.
pub trait Card: Default {
    /// Restore power-on state. The host has already cleared pending ticks and IRQ.
    fn reset(&mut self);

    /// Read one byte at a slot-relative MMIO offset.
    fn mmio_read(&mut self, offset: u32) -> u8;

    /// Write one byte at a slot-relative MMIO offset.
    fn mmio_write(&mut self, offset: u32, value: u8);

    /// Handle a scheduled virtual-cycle deadline.
    fn tick(&mut self);
}

#[link(wasm_import_module = "cpt32_host")]
unsafe extern "C" {
    #[link_name = "current_cycles"]
    fn host_current_cycles() -> i64;
    #[link_name = "schedule_tick"]
    fn host_schedule_tick(delay: i64);
    #[link_name = "irq_set"]
    fn host_irq_set(asserted: i32);
    #[link_name = "asset_count"]
    fn host_asset_count() -> i32;
    #[link_name = "asset_size"]
    fn host_asset_size(id: i32) -> i32;
    #[link_name = "asset_read_byte"]
    fn host_asset_read_byte(id: i32, offset: i32) -> i32;
}

/// Return the host's monotonic virtual-cycle counter as an unsigned bit pattern.
pub fn current_cycles() -> u64 {
    // SAFETY: This import is defined by ABI v1 and takes no arguments.
    unsafe { host_current_cycles() as u64 }
}

/// Schedule a tick after `delay` virtual cycles, or cancel it with zero.
///
/// Delays larger than `i64::MAX` cannot be represented by ABI v1 and are
/// rejected without calling the host.
pub fn schedule_tick(delay: u64) -> Result<(), DelayOutOfRange> {
    let Ok(delay) = i64::try_from(delay) else {
        return Err(DelayOutOfRange);
    };
    // SAFETY: The conversion above guarantees the ABI's nonnegative i64 input.
    unsafe { host_schedule_tick(delay) };
    Ok(())
}

/// Set the card's IRQ level.
pub fn irq_set(asserted: bool) {
    // SAFETY: bool maps exactly to the ABI's accepted 0/1 values.
    unsafe { host_irq_set(i32::from(asserted)) }
}

/// Number of package assets declared in `manifest.yaml`.
pub fn asset_count() -> u32 {
    // SAFETY: Valid packages expose a nonnegative count representable by i32.
    unsafe { host_asset_count() as u32 }
}

/// Uncompressed size of an asset. An unknown ID traps in the host.
pub fn asset_size(id: u32) -> u32 {
    // SAFETY: Valid package asset sizes and IDs are representable by i32.
    unsafe { host_asset_size(id as i32) as u32 }
}

/// Read one byte from a declared package asset. Invalid IDs or offsets trap.
pub fn asset_read_byte(id: u32, offset: u32) -> u8 {
    // SAFETY: This forwards the arguments to the ABI-defined checked host import.
    unsafe { host_asset_read_byte(id as i32, offset as i32) as u8 }
}

/// Error returned when a requested cycle delay is not representable by ABI v1.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DelayOutOfRange;

/// Generate the five required `cpt32_*` exports for one card type.
///
/// The module is single-threaded by the CPT32 ABI (shared memory is forbidden).
/// This macro owns the sole mutable card instance and calls `Default` lazily
/// from `cpt32_abi_version`; all later entry points reuse that instance.
#[macro_export]
macro_rules! export_card {
    ($card:ty) => {
        struct __Cpt32CardState {
            initialized: bool,
            card: core::mem::MaybeUninit<$card>,
        }

        struct __Cpt32CardCell(core::cell::UnsafeCell<__Cpt32CardState>);

        // CPT32 rejects shared WASM memory and calls each card serially.
        unsafe impl Sync for __Cpt32CardCell {}

        static __CPT32_CARD: __Cpt32CardCell =
            __Cpt32CardCell(core::cell::UnsafeCell::new(__Cpt32CardState {
                initialized: false,
                card: core::mem::MaybeUninit::uninit(),
            }));

        fn __cpt32_card_mut() -> &'static mut $card {
            // SAFETY: A card instance is called serially by the single-threaded
            // CPT32 ABI. Shared memory and concurrent host calls are unsupported.
            unsafe {
                let state = &mut *__CPT32_CARD.0.get();
                if !state.initialized {
                    state
                        .card
                        .write(<$card as core::default::Default>::default());
                    state.initialized = true;
                }
                state.card.assume_init_mut()
            }
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn cpt32_abi_version() -> i32 {
            let _ = __cpt32_card_mut();
            1
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn cpt32_reset() {
            <$card as $crate::Card>::reset(__cpt32_card_mut());
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn cpt32_mmio_read(offset: i32) -> i32 {
            <$card as $crate::Card>::mmio_read(__cpt32_card_mut(), offset as u32) as i32
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn cpt32_mmio_write(offset: i32, value: i32) {
            <$card as $crate::Card>::mmio_write(__cpt32_card_mut(), offset as u32, value as u8);
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn cpt32_tick() {
            <$card as $crate::Card>::tick(__cpt32_card_mut());
        }
    };
}
