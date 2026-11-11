//! Rust programs for Foundation OS on libfnd (`/lib/fnd.1.wal`).
//!
//! A program is a `no_std` library crate built by
//! `tools/llvm-src32/cargo_src32.py --foundation-wal <OUT>/root/lib/fnd.1.wal`
//! into a relocatable WAX v3 that imports libfnd, like the C commands in
//! `foundation/bin`. Declare the entry with [`main!`]:
//!
//! ```ignore
//! #![no_std]
//! fnd::main!(run);
//! fn run() -> i32 {
//!     fnd::println!("hello, {}", fnd::arg(1).unwrap_or("world"));
//!     0
//! }
//! ```
//!
//! [`sys`] has the raw bindings; the rest wraps them: [`args`], [`print!`] /
//! [`println!`] / [`eprintln!`] on STDOUT / STDERR, [`fs::File`], [`console`],
//! and with the `global-allocator` feature, `alloc` on the process heap.
#![no_std]

pub mod console;
pub mod fs;
pub mod io;
pub mod sys;

#[cfg(feature = "global-allocator")]
mod heap;

use core::ffi::{CStr, c_char};

/// A negative libfnd status (`FND_E_*`, `foundation/include/foundation/errors.h`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Error(pub i32);

impl Error {
    pub const BAD_ARG: Error = Error(-2);
    pub const NO_MEMORY: Error = Error(-5);
    pub const NOT_FOUND: Error = Error(-10);
    pub const EXISTS: Error = Error(-11);

    /// The message `fnd_strerror` gives for the code.
    pub fn message(self) -> &'static str {
        unsafe { c_str(sys::fnd_strerror(self.0)) }.unwrap_or("unknown error")
    }
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        write!(f, "{} ({})", self.message(), self.0)
    }
}

pub type Result<T> = core::result::Result<T, Error>;

/// FND_OK and FND_END are successes (returned as is); negative codes are errors.
pub fn check(status: i32) -> Result<i32> {
    if status < 0 {
        Err(Error(status))
    } else {
        Ok(status)
    }
}

/// # Safety
/// `p` is null or a NUL-terminated string that outlives the program.
unsafe fn c_str(p: *const c_char) -> Option<&'static str> {
    if p.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(p) }.to_str().ok()
}

/// `fnd_init`; [`main!`] calls it before the program body.
pub fn init() -> Result<i32> {
    check(unsafe { sys::fnd_init() })
}

/// Ends the process with `code` (`fnd_exit`).
pub fn exit(code: i32) -> ! {
    unsafe { sys::fnd_exit(code) }
}

/// The number of arguments, `argv[0]` (the command) included.
pub fn arg_count() -> u32 {
    unsafe { sys::fnd_argc() }
}

/// Argument `i` (0 is the command); `None` past the end or if it is not UTF-8.
pub fn arg(i: u32) -> Option<&'static str> {
    unsafe { c_str(sys::fnd_argv(i)) }
}

/// Argument `i` as a C string, for libfnd calls that take paths.
pub fn arg_c(i: u32) -> Option<&'static CStr> {
    let p = unsafe { sys::fnd_argv(i) };
    if p.is_null() {
        None
    } else {
        Some(unsafe { CStr::from_ptr(p) })
    }
}

/// The arguments after the command.
pub fn args() -> impl Iterator<Item = &'static str> {
    (1..arg_count()).filter_map(arg)
}

/// Environment entries, `NAME=value`.
pub fn env() -> impl Iterator<Item = &'static str> {
    (0..unsafe { sys::fnd_envc() }).filter_map(|i| unsafe { c_str(sys::fnd_env(i)) })
}

/// The value of environment variable `name`.
pub fn var(name: &str) -> Option<&'static str> {
    env().find_map(|entry| {
        let (key, value) = entry.split_once('=')?;
        (key == name).then_some(value)
    })
}

/// Defines the process entry: `fnd_init`, then `$body()` (a `fn() -> i32`),
/// whose result is the exit code, as `CMD_MAIN` does for the C commands.
#[macro_export]
macro_rules! main {
    ($body:path) => {
        #[unsafe(no_mangle)]
        pub extern "C" fn main() -> i32 {
            let _ = $crate::init();
            let body: fn() -> i32 = $body;
            $crate::exit(body())
        }
    };
}

#[cfg(feature = "panic-handler")]
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    use core::fmt::Write;
    let mut err = io::Stderr;
    let _ = err.write_str("panic");
    if let Some(location) = info.location() {
        let _ = write!(err, " at {}:{}", location.file(), location.line());
    }
    let _ = write!(err, ": {}\n", info.message());
    exit(101)
}
