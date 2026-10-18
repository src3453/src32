use std::io::{self, Write};
use std::path::Path;

use cpt32::bus::Bus;
use cpt32::cpu::InstructionMode;
use cpt32::cpu_select::{AnyCpu, CpuKind, resolve_cpu_kind};
use cpt32::devices::pec::idc::idc::connect_idc_from_config;
use cpt32::devices::pec::rng::connect_rng;
use cpt32::devices::pec::serial::connect_uart_with_stdin;
use cpt32::devices::pec::timing::connect_pec_timer_rtc;
use cpt32::devices::ram::{DEFAULT_RAM_SIZE, connect_ram_with_size};
use cpt32::devices::sgu::sgu::connect_sgu;

fn load_binary_data(path: &str, cpu: &mut AnyCpu, base: u32) {
    let data = std::fs::read(path).expect("Failed to read binary file");
    if cpt32::wax::is_wax(&data) {
        let img = cpt32::wax::parse(&data).expect("Invalid WAX file");
        assert!(!img.is_lib, ".wal library is not runnable");
        for block in &img.blocks {
            cpu.load_program(block.load_address, block.data);
        }
        cpu.set_pc(img.entry);
    } else {
        cpu.load_program(base, &data);
    }
}

fn parse_u32(token: &str) -> Result<u32, String> {
    if let Some(hex) = token
        .strip_prefix("0x")
        .or_else(|| token.strip_prefix("0X"))
    {
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

fn print_state(cpu: &mut AnyCpu) {
    let state = cpu.return_state_text();
    println!("{}", state);
    let mode = cpu.instruction_mode();
    let disasm = cpu.decode_at(cpu.pc(), mode);
    println!("next: {}", disasm.text);
}

fn print_disassembly(cpu: &mut AnyCpu, mut addr: u32, count: usize) {
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

fn step_cpu(cpu: &mut AnyCpu, count: usize) {
    for _ in 0..count {
        if !cpu.step_once() {
            println!("CPU halted.");
            break;
        }
    }
    print_state(cpu);
}

fn print_memory(cpu: &mut AnyCpu, mut addr: u32, count: usize) {
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

#[derive(Default)]
pub struct CommandOutput {
    pub lines: Vec<String>,
}

pub fn execute_command(cpu: &mut AnyCpu, command_line: &str) -> CommandOutput {
    let mut output = CommandOutput::default();
    let mut parts = command_line.split_whitespace();
    let Some(command) = parts.next() else {
        return output;
    };
    let push_state = |cpu: &mut AnyCpu, output: &mut CommandOutput| {
        output.lines.push(cpu.return_state_text());
        let decoded = cpu.decode_at(cpu.pc(), cpu.instruction_mode());
        output.lines.push(format!("next: {}", decoded.text));
    };
    match command {
        "help" | "h" | "?" => output.lines.extend([
            "Commands:".to_string(),
            "  help(h|?)               show this help".to_string(),
            "  load(l) <path> [addr]   load a binary image into memory".to_string(),
            "  step(s) [n]             execute n instructions (default 1)".to_string(),
            "  run(r) [n]              execute n instructions (default 1000)".to_string(),
            "  regs(state)(rs)         print CPU state".to_string(),
            "  setr(sr) <reg> <value>  set register to value".to_string(),
            "  goto(setpc|g) <addr>    set PC to addr".to_string(),
            "  disasm(u) [addr] [n]    show n decoded instructions".to_string(),
            "  mem(x) <addr> [n]       dump n bytes from memory".to_string(),
            "  poke(p) <addr> <value>  write one byte to memory".to_string(),
            "  reset(rst) [pc]         reset CPU and set PC".to_string(),
            "  pause / continue       control real-time emulation in the debug GUI".to_string(),
            "  quit(exit|q)            unavailable in the embedded monitor".to_string(),
        ]),
        "load" | "l" => {
            let Some(path) = parts.next() else {
                output.lines.push("Usage: load <path> [addr]".into());
                return output;
            };
            let addr = match parts.next().map(parse_u32).transpose() {
                Ok(Some(addr)) => addr,
                Ok(None) => 0,
                Err(error) => {
                    output.lines.push(format!("Invalid address: {error}"));
                    return output;
                }
            };
            match std::fs::read(path) {
                Ok(data) => {
                    if cpt32::wax::is_wax(&data) {
                        match cpt32::wax::parse(&data) {
                            Ok(img) if !img.is_lib => {
                                for block in &img.blocks {
                                    cpu.load_program(block.load_address, block.data);
                                }
                                cpu.set_pc(img.entry);
                                let loaded_bytes: usize =
                                    img.blocks.iter().map(|block| block.data.len()).sum();
                                output.lines.push(format!(
                                    "Loaded WAX {} ({} blocks, {} bytes, entry 0x{:08X})",
                                    path,
                                    img.blocks.len(),
                                    loaded_bytes,
                                    img.entry
                                ));
                            }
                            _ => output.lines.push(format!("Invalid WAX file {path}")),
                        }
                    } else {
                        cpu.load_program(addr, &data);
                        output
                            .lines
                            .push(format!("Loaded {} bytes at 0x{:08X}", data.len(), addr));
                    }
                }
                Err(error) => output.lines.push(format!("Failed to read {path}: {error}")),
            }
        }
        "step" | "s" | "run" | "r" => {
            let default_count = if matches!(command, "run" | "r") {
                1000
            } else {
                1
            };
            let count = match parts.next().map(str::parse::<usize>).transpose() {
                Ok(Some(count)) => count,
                Ok(None) => default_count,
                Err(error) => {
                    output.lines.push(format!("Invalid step count: {error}"));
                    return output;
                }
            };
            for _ in 0..count {
                if !cpu.step_once() {
                    output.lines.push("CPU halted.".into());
                    break;
                }
            }
            push_state(cpu, &mut output);
        }
        "regs" | "state" | "rs" => push_state(cpu, &mut output),
        "disasm" | "u" => {
            let addr = match parts.next().map(parse_u32).transpose() {
                Ok(Some(addr)) => addr,
                Ok(None) => cpu.pc(),
                Err(error) => {
                    output.lines.push(format!("Invalid address: {error}"));
                    return output;
                }
            };
            let count = match parts.next().map(str::parse::<usize>).transpose() {
                Ok(Some(count)) => count,
                Ok(None) => 8,
                Err(error) => {
                    output.lines.push(format!("Invalid count: {error}"));
                    return output;
                }
            };
            let mut addr = addr;
            let mut mode = if addr == cpu.pc() {
                cpu.instruction_mode()
            } else {
                InstructionMode::Normal
            };
            for _ in 0..count {
                let marker = if addr == cpu.pc() { "=>" } else { "  " };
                let decoded = cpu.decode_at(addr, mode);
                let raw = if decoded.size == 2 {
                    format!("{:04X}     ", cpu.read_u16_be(addr))
                } else {
                    format!("{:08X} ", cpu.read_u32(addr))
                };
                output.lines.push(format!(
                    "{} 0x{:08X}: {} [{:?}] {}",
                    marker, addr, raw, mode, decoded.text
                ));
                addr = addr.wrapping_add(decoded.size as u32);
                mode = decoded.next_mode;
            }
        }
        "mem" | "x" => {
            let addr = match parts.next().map(parse_u32).transpose() {
                Ok(Some(addr)) => addr,
                Ok(None) => cpu.pc(),
                Err(error) => {
                    output.lines.push(format!("Invalid address: {error}"));
                    return output;
                }
            };
            let count = match parts.next().map(str::parse::<usize>).transpose() {
                Ok(Some(count)) => count,
                Ok(None) => 64,
                Err(error) => {
                    output.lines.push(format!("Invalid count: {error}"));
                    return output;
                }
            };
            let mut addr = addr;
            for row_start in (0..count).step_by(16) {
                let row_count = (count - row_start).min(16);
                let mut hex = String::new();
                let mut ascii = String::new();
                for _ in 0..row_count {
                    let byte = cpu.read_mem_u8(addr);
                    hex.push_str(&format!("{:02X} ", byte));
                    ascii.push(if byte.is_ascii_graphic() || byte == b' ' {
                        byte as char
                    } else {
                        '.'
                    });
                    addr = addr.wrapping_add(1);
                }
                output.lines.push(format!(
                    "0x{:08X}: {}|{}|",
                    addr.wrapping_sub(row_count as u32),
                    hex,
                    ascii
                ));
            }
        }
        "poke" | "p" => {
            let (Some(addr_token), Some(value_token)) = (parts.next(), parts.next()) else {
                output.lines.push("Usage: poke <addr> <value>".into());
                return output;
            };
            let addr = match parse_u32(addr_token) {
                Ok(addr) => addr,
                Err(error) => {
                    output.lines.push(format!("Invalid address: {error}"));
                    return output;
                }
            };
            let value = match parse_u32(value_token) {
                Ok(value) => value as u8,
                Err(error) => {
                    output.lines.push(format!("Invalid value: {error}"));
                    return output;
                }
            };
            cpu.write_mem_u8(addr, value);
            output
                .lines
                .push(format!("Wrote 0x{:02X} to 0x{:08X}", value, addr));
        }
        "reset" | "rst" => {
            let pc = match parts.next().map(parse_u32).transpose() {
                Ok(Some(pc)) => pc,
                Ok(None) => 0,
                Err(error) => {
                    output.lines.push(format!("Invalid PC: {error}"));
                    return output;
                }
            };
            cpu.reset(pc);
            push_state(cpu, &mut output);
        }
        "setr" | "sr" => {
            let (Some(reg_token), Some(value_token)) = (parts.next(), parts.next()) else {
                output.lines.push("Usage: setr <reg> <value>".into());
                return output;
            };
            let reg = match parse_u32(reg_token) {
                Ok(reg) => reg as usize,
                Err(error) => {
                    output.lines.push(format!("Invalid register: {error}"));
                    return output;
                }
            };
            if reg >= 32 {
                output
                    .lines
                    .push("Register index out of range (0-31)".into());
                return output;
            }
            let value = match parse_u32(value_token) {
                Ok(value) => value,
                Err(error) => {
                    output.lines.push(format!("Invalid value: {error}"));
                    return output;
                }
            };
            let message = cpu.write_reg(reg, value);
            output
                .lines
                .push(format!("Set R{} to 0x{:08X}", reg, value));
            match message {
                Ok(message) if !message.is_empty() => output.lines.push(message),
                Err(error) => output.lines.push(format!("Error: {error}")),
                _ => {}
            }
        }
        "goto" | "setpc" | "g" => {
            let Some(addr_token) = parts.next() else {
                output.lines.push(format!("Usage: {} <addr>", command));
                return output;
            };
            match parse_u32(addr_token) {
                Ok(addr) => {
                    cpu.set_pc(addr);
                    output.lines.push(format!("Set PC to 0x{:08X}", addr));
                }
                Err(error) => output.lines.push(format!("Invalid address: {error}")),
            }
        }
        "quit" | "exit" | "q" => output
            .lines
            .push("quit is unavailable in the embedded monitor".into()),
        other => output.lines.push(format!("Unknown command: {other}")),
    }
    output
}

fn configured_ram_size(config_path: &Path) -> Result<usize, String> {
    let contents = match std::fs::read_to_string(config_path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(format!("cannot read {}: {error}", config_path.display())),
    };
    let config: yaml_serde::Value = if contents.is_empty() {
        yaml_serde::Value::Null
    } else {
        yaml_serde::from_str(&contents)
            .map_err(|error| format!("invalid YAML in {}: {error}", config_path.display()))?
    };
    let Some(memory) = config.get("memory") else {
        return Ok(DEFAULT_RAM_SIZE);
    };
    let table = memory.as_mapping().ok_or("`memory` must be a mapping")?;
    let Some(value) = table.get("ram_mib") else {
        return Ok(DEFAULT_RAM_SIZE);
    };
    let mib = value
        .as_u64()
        .ok_or("`memory.ram_mib` must be an integer MiB value")?;
    if !(1..=256).contains(&mib) {
        return Err("`memory.ram_mib` must be between 1 and 256 MiB".into());
    }
    Ok((mib * 1024 * 1024) as usize)
}

pub fn run(program_path: Option<&str>, cli_cpu: Option<CpuKind>) {
    let cpu_kind = resolve_cpu_kind(cli_cpu, Path::new("config.yaml"))
        .unwrap_or_else(|error| panic!("Invalid CPU configuration: {error}"));
    println!("CPU micro-architecture: {}", cpu_kind.as_str());
    let mut bus = Bus::new();
    let ram_size = configured_ram_size(Path::new("config.yaml"))
        .unwrap_or_else(|error| panic!("Invalid memory configuration: {error}"));
    connect_ram_with_size(&mut bus, ram_size);
    cpt32::bus::connect_bmc_dmac(&mut bus);
    connect_idc_from_config(&mut bus)
        .unwrap_or_else(|error| panic!("Invalid disk configuration: {error}"));
    connect_pec_timer_rtc(&mut bus);
    connect_uart_with_stdin(&mut bus, false);
    connect_rng(&mut bus);
    let _sgu = connect_sgu(&mut bus);
    cpt32::devices::expansion::load_expansion_cards(Path::new("config.yaml"), &mut bus)
        .unwrap_or_else(|error| panic!("Invalid expansion-card configuration: {error}"));
    let bios_entry = cpt32::devices::bios::load_bios(Path::new("config.yaml"), &mut bus)
        .unwrap_or_else(|error| panic!("Invalid BIOS configuration: {error}"));
    let mut cpu = AnyCpu::new(cpu_kind, bus);

    if let Some(path) = program_path {
        load_binary_data(path, &mut cpu, 0);
        println!("Loaded {} at 0x00000000", path);
    }
    if let Some(entry) = bios_entry {
        cpu.set_pc(entry);
        println!("BIOS ROM configured: booting from 0x{entry:08X}");
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
                        if cpt32::wax::is_wax(&data) {
                            match cpt32::wax::parse(&data) {
                                Ok(img) if !img.is_lib => {
                                    for block in &img.blocks {
                                        cpu.load_program(block.load_address, block.data);
                                    }
                                    cpu.set_pc(img.entry);
                                    let loaded_bytes: usize =
                                        img.blocks.iter().map(|block| block.data.len()).sum();
                                    println!(
                                        "Loaded WAX {} ({} blocks, {} bytes, entry 0x{:08X})",
                                        path,
                                        img.blocks.len(),
                                        loaded_bytes,
                                        img.entry
                                    );
                                }
                                _ => println!("Invalid WAX file {path}"),
                            }
                        } else {
                            cpu.load_program(addr, &data);
                            println!("Loaded {} bytes at 0x{:08X}", data.len(), addr);
                        }
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

#[cfg(test)]
mod tests {
    use super::execute_command;
    use cpt32::bus::Bus;
    use cpt32::cpu_select::{AnyCpu, CpuKind};
    use cpt32::devices::ram::connect_ram;

    #[test]
    fn console_set_register_command_updates_cpu_and_returns_feedback() {
        for kind in [CpuKind::Scalar, CpuKind::Cached, CpuKind::Pipelined] {
            let mut bus = Bus::new();
            connect_ram(&mut bus);
            let mut cpu = AnyCpu::new(kind, bus);

            let output = execute_command(&mut cpu, "setr 1 0x1234");

            assert_eq!(cpu.read_reg(1), 0x1234);
            assert!(output.lines.iter().any(|line| line.contains("Set R1")));
        }
    }
}
