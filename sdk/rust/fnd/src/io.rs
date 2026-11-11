//! STDIN / STDOUT / STDERR streams and the print macros.

use crate::{Result, check, sys};

/// Writes all of `bytes` to stream `h`.
pub fn write_all(h: sys::fnd_handle, mut bytes: &[u8]) -> Result<()> {
    while !bytes.is_empty() {
        let mut put = 0u32;
        check(unsafe { sys::fnd_write(h, bytes.as_ptr(), bytes.len() as u32, &mut put) })?;
        if put == 0 {
            return Err(crate::Error(-16)); // FND_E_IO: the stream took nothing
        }
        bytes = &bytes[put as usize..];
    }
    Ok(())
}

/// Reads up to `buf.len()` bytes from stream `h`; `Ok(0)` at the end.
pub fn read(h: sys::fnd_handle, buf: &mut [u8]) -> Result<usize> {
    let mut got = 0u32;
    let status = check(unsafe { sys::fnd_read(h, buf.as_mut_ptr(), buf.len() as u32, &mut got) })?;
    Ok(if status == sys::FND_END {
        0
    } else {
        got as usize
    })
}

macro_rules! stream {
    ($name:ident, $handle:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Copy)]
        pub struct $name;

        impl $name {
            pub fn handle(self) -> sys::fnd_handle {
                unsafe { sys::$handle() }
            }

            pub fn write(self, bytes: &[u8]) -> Result<()> {
                write_all(self.handle(), bytes)
            }
        }

        impl core::fmt::Write for $name {
            fn write_str(&mut self, text: &str) -> core::fmt::Result {
                write_all(self.handle(), text.as_bytes()).map_err(|_| core::fmt::Error)
            }
        }
    };
}

stream!(Stdout, fnd_std_out, "The process's STDOUT.");
stream!(
    Stderr,
    fnd_std_err,
    "The process's STDERR (STDOUT when it has none)."
);

/// The process's STDIN.
#[derive(Clone, Copy)]
pub struct Stdin;

impl Stdin {
    pub fn handle(self) -> sys::fnd_handle {
        unsafe { sys::fnd_std_in() }
    }

    pub fn read(self, buf: &mut [u8]) -> Result<usize> {
        read(self.handle(), buf)
    }
}

#[doc(hidden)]
pub fn _print(args: core::fmt::Arguments, err: bool) {
    use core::fmt::Write;
    let _ = if err {
        Stderr.write_fmt(args)
    } else {
        Stdout.write_fmt(args)
    };
}

/// `print!` to STDOUT.
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => { $crate::io::_print(format_args!($($arg)*), false) };
}

/// `println!` to STDOUT.
#[macro_export]
macro_rules! println {
    () => { $crate::print!("\n") };
    ($($arg:tt)*) => { $crate::io::_print(format_args!("{}\n", format_args!($($arg)*)), false) };
}

/// `eprint!` to STDERR.
#[macro_export]
macro_rules! eprint {
    ($($arg:tt)*) => { $crate::io::_print(format_args!($($arg)*), true) };
}

/// `eprintln!` to STDERR.
#[macro_export]
macro_rules! eprintln {
    () => { $crate::eprint!("\n") };
    ($($arg:tt)*) => { $crate::io::_print(format_args!("{}\n", format_args!($($arg)*)), true) };
}
