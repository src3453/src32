//! `rsdemo [word...]`: a Foundation command in Rust. It prints its arguments,
//! sorts them with `alloc` (Vec / String on the process heap), writes them to
//! a file and reads it back. `rsdemo --panic` shows the panic handler of the
//! fnd crate (message on STDERR, exit code 101).
//! foundation/tests/rust runs it in the shell on src32_testbench.
#![no_std]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use fnd::fs::File;

fnd::main!(run);

fn run() -> i32 {
    if fnd::arg(1) == Some("--panic") {
        panic!("requested by {}", "--panic");
    }
    fnd::println!("Hello from Rust on Foundation!");
    let mut words: Vec<&str> = fnd::args().collect();
    fnd::println!("{} argument(s)", words.len());
    words.sort_unstable();
    let joined: String = words.join(",");
    fnd::println!("sorted: {}", joined);

    let path = c"/rsdemo.txt";
    let result = File::create(path).and_then(|mut file| file.write_all(joined.as_bytes()));
    if let Err(error) = result {
        fnd::eprintln!("rsdemo: write: {}", error);
        return 1;
    }
    let mut buffer = [0u8; 128];
    let read = File::open(path, fnd::fs::FND_OPEN_READ).and_then(|mut file| file.read(&mut buffer));
    match read {
        Ok(n) => fnd::println!(
            "file: {} bytes, {}",
            n,
            core::str::from_utf8(&buffer[..n]).unwrap_or("?")
        ),
        Err(error) => {
            fnd::eprintln!("rsdemo: read: {}", error);
            return 1;
        }
    }
    let _ = fnd::fs::remove_file(path);
    0
}
