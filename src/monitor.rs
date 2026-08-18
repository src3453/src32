use std::io::{self, Write};

use cpt32::bus::Bus;
use cpt32::cpu::{Cpu, InstructionMode};
use cpt32::devices::ram::connect_ram;
use cpt32::devices::pec::serial::connect_uart_with_stdin;
use cpt32::devices::pec::rng::connect_rng;
use cpt32::devices::sgu::sgu::connect_sgu;

fn load_binary_data(path: &str, cpu: &mut Cpu, base: u32) {
    let data = std::fs::read(path).expect("Failed to read binary file");
    cpu.load_program(base, &data);
}

fn parse_u32(token: &str) -> Result<u32, String> {
    if let Some(hex) = token.strip_prefix("0x").or_else(|| token.strip_prefix("0X")) {
        u32::from_str_radix(hex, 16).map_err(|err| err.to_string())
    } else {
        token.parse::<u32>().map_err(|err| err.to_string())
    }
}

fn print_help() {
    println!("Commands:");
    println!("  COMMAND(ALIAS) [ARGS...]  DESCRIPTION\n");
    println!("  help(h|?)               show this help");
    println!("  load(l) <path> [addr]   load a binary image into memory");
    println!("  step(s) [n]             execute n instructions (default 1)");
    println!("  run(r) [n]              same as step, but intended for longer runs");
    println!("  regs(state)(rs)         print CPU state");
    println!("  setr(sr) <reg> <value>  set register to value");
    println!("  goto(setpc|g) <addr>    set PC to addr");
    println!("  disasm(u) [addr] [n]    show n decoded instructions");
    println!("  mem(x) <addr> [n]       dump n bytes from memory");
    println!("  poke(p) <addr> <value>  write one byte to memory");
    println!("  reset(rst) [pc]         reset CPU and set PC");
    println!("  quit(exit|q)            exit the monitor");
}

fn print_state(cpu: &mut Cpu) {
    let state = cpu.return_state_text();
    println!("{}", state);
    let mode = cpu.instruction_mode();
    let disasm = cpu.decode_at(cpu.pc(), mode);
    println!("next: {}", disasm.text);
}

fn print_disassembly(cpu: &mut Cpu, mut addr: u32, count: usize) {
    let mut mode = if addr == cpu.pc() {
        cpu.instruction_mode()
    } else {
        InstructionMode::Normal
    };

    for _ in 0..count {
        let marker = if addr == cpu.pc() { "=>" } else { "  " };
        let decoded = cpu.decode_at(addr, mode);
        if decoded.size == 2 {
            let raw = cpu.read_u16_be(addr);
            println!(
                "{} 0x{:08X}: {:04X}      [{:?}] {}",
                marker, addr, raw, mode, decoded.text
            );
        } else {
            let raw = cpu.read_u32(addr);
            println!(
                "{} 0x{:08X}: {:08X}  [{:?}] {}",
                marker, addr, raw, mode, decoded.text
            );
        }
        addr = addr.wrapping_add(decoded.size as u32);
        mode = decoded.next_mode;
    }
}

fn step_cpu(cpu: &mut Cpu, count: usize) {
    for _ in 0..count {
        if !cpu.step_once() {
            println!("CPU halted.");
            break;
        }
    }
    print_state(cpu);
}

fn print_memory(cpu: &mut Cpu, mut addr: u32, count: usize) {
    for i in 0..count {
        if i % 16 == 0 {
            print!("0x{:08X}: ", addr);
        }
        print!("{:02X} ", cpu.read_mem_u8(addr));
        if i % 16 == 15 {
            if i != 0 {
                print!(" | ");
                for j in 0..16 {
                    let byte = cpu.read_mem_u8(addr.wrapping_sub(15).wrapping_add(j));
                    if byte.is_ascii_graphic() || byte == b' ' {
                        print!("{}", byte as char);
                    } else {
                        print!(".");
                    }
                }
                println!();
            }
        }
        addr = addr.wrapping_add(1);
    }
    println!();
}

pub fn run(program_path: Option<&str>) {
    let mut bus = Bus::new();
    connect_ram(&mut bus);
    connect_uart_with_stdin(&mut bus, false);
    connect_rng(&mut bus);
    let _sgu = connect_sgu(&mut bus);
    let mut cpu = Cpu::new(bus);

    if let Some(path) = program_path {
        load_binary_data(path, &mut cpu, 0);
        println!("Loaded {} at 0x00000000", path);
    }
    println!("-------------------------");
    println!("CPT32 Debug Monitor");
    print_help();
    print_state(&mut cpu);

    let stdin = io::stdin();
    let mut line = String::new();

    loop {
        print!("src32> ");
        io::stdout().flush().expect("Failed to flush stdout");

        line.clear();
        if stdin.read_line(&mut line).expect("Failed to read command") == 0 {
            break;
        }

        let mut parts = line.split_whitespace();
        let Some(command) = parts.next() else {
            continue;
        };

        match command {
            "help" | "h" | "?" => print_help(),
            "load" | "l" => {
                let Some(path) = parts.next() else {
                    println!("Usage: load <path> [addr]");
                    continue;
                };
                let addr = match parts.next() {
                    Some(token) => match parse_u32(token) {
                        Ok(value) => value,
                        Err(err) => {
                            println!("Invalid address: {}", err);
                            continue;
                        }
                    },
                    None => 0,
                };
                match std::fs::read(path) {
                    Ok(data) => {
                        cpu.load_program(addr, &data);
                        println!("Loaded {} bytes at 0x{:08X}", data.len(), addr);
                    }
                    Err(err) => println!("Failed to read {}: {}", path, err),
                }
            }
            "step" | "s" => {
                let count = match parts.next() {
                    Some(token) => match token.parse::<usize>() {
                        Ok(value) => value,
                        Err(err) => {
                            println!("Invalid step count: {}", err);
                            continue;
                        }
                    },
                    None => 1,
                };
                step_cpu(&mut cpu, count);
            }
            "run" | "r" => {
                let count = match parts.next() {
                    Some(token) => match token.parse::<usize>() {
                        Ok(value) => value,
                        Err(err) => {
                            println!("Invalid run count: {}", err);
                            continue;
                        }
                    },
                    None => 1000,
                };
                step_cpu(&mut cpu, count);
            }
            "regs" | "state" | "rs" => print_state(&mut cpu),
            "disasm" | "u" => {
                let addr = match parts.next() {
                    Some(token) => match parse_u32(token) {
                        Ok(value) => value,
                        Err(err) => {
                            println!("Invalid address: {}", err);
                            continue;
                        }
                    },
                    None => cpu.pc(),
                };
                let count = match parts.next() {
                    Some(token) => match token.parse::<usize>() {
                        Ok(value) => value,
                        Err(err) => {
                            println!("Invalid count: {}", err);
                            continue;
                        }
                    },
                    None => 8,
                };
                print_disassembly(&mut cpu, addr, count);
            }
            "mem" | "x" => {
                let addr = match parts.next() {
                    Some(token) => match parse_u32(token) {
                        Ok(value) => value,
                        Err(err) => {
                            println!("Invalid address: {}", err);
                            continue;
                        }
                    },
                    None => cpu.pc(),
                };
                let count = match parts.next() {
                    Some(token) => match token.parse::<usize>() {
                        Ok(value) => value,
                        Err(err) => {
                            println!("Invalid count: {}", err);
                            continue;
                        }
                    },
                    None => 64,
                };
                print_memory(&mut cpu, addr, count);
            }
            "poke" | "p" => {
                let Some(addr_token) = parts.next() else {
                    println!("Usage: poke <addr> <value>");
                    continue;
                };
                let Some(value_token) = parts.next() else {
                    println!("Usage: poke <addr> <value>");
                    continue;
                };
                let addr = match parse_u32(addr_token) {
                    Ok(value) => value,
                    Err(err) => {
                        println!("Invalid address: {}", err);
                        continue;
                    }
                };
                let value = match parse_u32(value_token) {
                    Ok(value) => value as u8,
                    Err(err) => {
                        println!("Invalid value: {}", err);
                        continue;
                    }
                };
                cpu.write_mem_u8(addr, value);
                println!("Wrote 0x{:02X} to 0x{:08X}", value, addr);
            }
            "reset" | "rst" => {
                let pc = match parts.next() {
                    Some(token) => match parse_u32(token) {
                        Ok(value) => value,
                        Err(err) => {
                            println!("Invalid PC: {}", err);
                            continue;
                        }
                    },
                    None => 0,
                };
                cpu.reset(pc);
                print_state(&mut cpu);
            }
            "setr" | "sr" => {
                let Some(reg_token) = parts.next() else {
                    println!("Usage: setr <reg> <value>");
                    continue;
                };
                let Some(value_token) = parts.next() else {
                    println!("Usage: setr <reg> <value>");
                    continue;
                };
                let reg = match parse_u32(reg_token) {
                    Ok(value) => value as usize,
                    Err(err) => {
                        println!("Invalid register: {}", err);
                        continue;
                    }
                };
                if reg >= 32 {
                    println!("Register index out of range (0-31)");
                    continue;
                }
                let value = match parse_u32(value_token) {
                    Ok(value) => value,
                    Err(err) => {
                        println!("Invalid value: {}", err);
                        continue;
                    }
                };
                let message = cpu.write_reg(reg, value);
                println!("Set R{} to 0x{:08X}", reg, value);
                match message {
                    Ok(msg) if !msg.is_empty() => println!("{}", msg),
                    Err(err) => println!("Error: {}", err),
                    _ => {}
                }
            }
            "goto" | "setpc" | "g" => {
                let Some(addr_token) = parts.next() else {
                    println!("Usage: {} <addr>", command);
                    continue;
                };
                let addr = match parse_u32(addr_token) {
                    Ok(value) => value,
                    Err(err) => {
                        println!("Invalid address: {}", err);
                        continue;
                    }
                };
                cpu.set_pc(addr);
                println!("Set PC to 0x{:08X}", addr);
            }
            "quit" | "exit" | "q" => break,
            other => println!("Unknown command: {}", other),
        }
    }
}