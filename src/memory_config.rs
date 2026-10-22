// Memory capacities configured by config.yaml (doc/spec.md §2.4).

use std::path::Path;

use crate::devices::ram::DEFAULT_RAM_SIZE;
use crate::devices::vdp::vdp::DEFAULT_VDP_VRAM_SIZE;

/// Main RAM and VRAM sizes from the `memory` section of `config_path`
/// (doc/spec.md §2.4). A missing file or section keeps the defaults.
pub fn memory_sizes_from_config(config_path: &Path) -> Result<(usize, u32), String> {
    let contents = match std::fs::read_to_string(config_path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(format!("cannot read {}: {error}", config_path.display())),
    };
    memory_sizes_from_yaml(&contents)
}

pub fn memory_sizes_from_yaml(contents: &str) -> Result<(usize, u32), String> {
    let root: yaml_serde::Value = if contents.is_empty() {
        yaml_serde::Value::Null
    } else {
        yaml_serde::from_str(contents).map_err(|error| format!("invalid YAML: {error}"))?
    };
    let Some(memory) = root.get("memory") else {
        return Ok((DEFAULT_RAM_SIZE, DEFAULT_VDP_VRAM_SIZE));
    };
    let table = memory.as_mapping().ok_or("`memory` must be a mapping")?;
    let parse = |key: &str, default: u64, minimum: u64, maximum: u64| -> Result<u64, String> {
        let Some(value) = table.get(key) else {
            return Ok(default);
        };
        let mib = value
            .as_u64()
            .ok_or_else(|| format!("`memory.{key}` must be an integer MiB value"))?;
        if !(minimum..=maximum).contains(&mib) {
            return Err(format!(
                "`memory.{key}` must be between {minimum} and {maximum} MiB"
            ));
        }
        Ok(mib)
    };
    let ram_mib = parse("ram_mib", 16, 1, 256)?;
    let vram_mib = parse("vram_mib", 4, 4, 128)?;
    Ok((
        (ram_mib * 1024 * 1024) as usize,
        (vram_mib * 1024 * 1024) as u32,
    ))
}
