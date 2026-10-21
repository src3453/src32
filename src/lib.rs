// Expose the main components of the CPT32 emulator as a library

pub mod bus;
pub mod cartridge;
pub mod cow;
pub mod cpu;
pub mod cpu_pipeline;
pub mod cpu_privilege;
pub mod cpu_select;
pub mod music;
pub mod sys;
pub mod wav;
pub mod wax;
pub mod devices {
    pub mod bios;
    pub mod bmc;
    pub mod cpu;
    pub mod expansion;
    pub mod dmac {
        pub mod dmac;
    }
    pub mod ram;
    pub mod irqc {
        pub mod irqc;
    }
    pub mod sgu {
        pub mod s3w2;
        pub mod sgu;
        pub mod sqv4;
    }
    pub mod vdp {
        pub mod chr_rom;
        pub mod clut;
        pub mod compositor;
        pub mod gp;
        pub mod pcg;
        pub mod reg;
        pub mod vdp;
    }
    pub mod sgc {
        pub mod sgc;
    }
    pub mod vpu {
        pub mod vpu;
    }
    pub mod pec {
        pub mod pec;
        pub mod rng;
        pub mod serial;
        pub mod timing;
        pub mod idc {
            pub mod audio;
            pub mod idc;
        }
    }
}

/// Initialize the process-wide tracing subscriber for command-line binaries.
/// `RUST_LOG` takes precedence over the `logging.level` value in config.yaml.
pub fn init_logging() {
    use std::io::IsTerminal;
    use tracing_subscriber::filter::LevelFilter;

    let (default_level, config_warning) = match config_log_level() {
        Ok(Some(level)) => (level, None),
        Ok(None) => (LevelFilter::INFO, None),
        Err(error) => (LevelFilter::INFO, Some(error)),
    };
    let mut env_warning = None;
    let filter = match std::env::var("RUST_LOG") {
        Ok(value) => match tracing_subscriber::EnvFilter::try_new(value) {
            Ok(filter) => filter,
            Err(error) => {
                env_warning = Some(error.to_string());
                tracing_subscriber::EnvFilter::new(default_level.to_string())
            }
        },
        Err(std::env::VarError::NotPresent) => {
            tracing_subscriber::EnvFilter::new(default_level.to_string())
        }
        Err(error) => {
            env_warning = Some(error.to_string());
            tracing_subscriber::EnvFilter::new(default_level.to_string())
        }
    };

    if tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .try_init()
        .is_ok()
    {
        if let Some(error) = config_warning {
            tracing::warn!(%error, "Invalid logging configuration; using INFO");
        }
        if let Some(error) = env_warning {
            tracing::warn!(%error, "Invalid RUST_LOG value; using configured level");
        }
    }
}

fn config_log_level() -> Result<Option<tracing_subscriber::filter::LevelFilter>, String> {
    let contents = match std::fs::read_to_string("config.yaml") {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("cannot read config.yaml: {error}")),
    };
    let config: yaml_serde::Value = yaml_serde::from_str(&contents)
        .map_err(|error| format!("invalid YAML in config.yaml: {error}"))?;
    parse_config_log_level(&config)
}

fn parse_config_log_level(
    config: &yaml_serde::Value,
) -> Result<Option<tracing_subscriber::filter::LevelFilter>, String> {
    use tracing_subscriber::filter::LevelFilter;

    let Some(logging) = config.get("logging") else {
        return Ok(None);
    };
    let table = logging.as_mapping().ok_or("`logging` must be a mapping")?;
    let Some(level) = table.get("level") else {
        return Ok(None);
    };
    let level = level
        .as_str()
        .ok_or("`logging.level` must be a string")?
        .to_ascii_lowercase();
    match level.as_str() {
        "trace" => Ok(Some(LevelFilter::TRACE)),
        "debug" => Ok(Some(LevelFilter::DEBUG)),
        "info" => Ok(Some(LevelFilter::INFO)),
        "warn" => Ok(Some(LevelFilter::WARN)),
        "error" => Ok(Some(LevelFilter::ERROR)),
        "off" => Ok(Some(LevelFilter::OFF)),
        _ => Err("`logging.level` must be trace, debug, info, warn, error, or off".into()),
    }
}

#[cfg(test)]
mod logging_tests {
    use super::parse_config_log_level;
    use tracing_subscriber::filter::LevelFilter;

    #[test]
    fn reads_configured_log_level_and_defaults_when_absent() {
        let config: yaml_serde::Value = yaml_serde::from_str("logging:\n  level: DEBUG\n").unwrap();
        assert_eq!(
            parse_config_log_level(&config).unwrap(),
            Some(LevelFilter::DEBUG)
        );

        let config: yaml_serde::Value = yaml_serde::from_str("pcg:\n  font: font.bin\n").unwrap();
        assert_eq!(parse_config_log_level(&config).unwrap(), None);
    }

    #[test]
    fn rejects_unknown_configured_log_level() {
        let config: yaml_serde::Value =
            yaml_serde::from_str("logging:\n  level: verbose\n").unwrap();
        assert!(parse_config_log_level(&config).is_err());
    }
}
