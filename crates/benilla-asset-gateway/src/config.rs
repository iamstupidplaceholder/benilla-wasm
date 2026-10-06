//! Optional TOML settings: CLI > config > built-in defaults/discovery.
use std::{net::SocketAddr, path::PathBuf};

use anyhow::{Context, Result};
use clap::{ArgMatches, parser::ValueSource};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    data: Option<PathBuf>,
    addons: Option<PathBuf>,
    web: Option<PathBuf>,
    bind: Option<String>,
    auth_upstream: Option<SocketAddr>,
    world_upstream: Option<SocketAddr>,
    workers: Option<usize>,
    queue: Option<usize>,
    max_asset_mib: Option<u64>,
    cache_mib: Option<usize>,
    asset_max_age_secs: Option<u64>,
}

pub(super) fn apply(args: &mut super::Args, matches: &ArgMatches) -> Result<()> {
    let explicit = args.config.is_some();
    let path = args.config.clone().unwrap_or_else(|| "gateway.toml".into());
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if !explicit && error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
    };
    let config: Config =
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    macro_rules! setting {
        ($field:ident) => {
            if matches.value_source(stringify!($field)) != Some(ValueSource::CommandLine) {
                if let Some(value) = config.$field {
                    args.$field = value;
                }
            }
        };
    }
    // Data is optional in Args because installed-client discovery is the final fallback.
    if matches.value_source("data") != Some(ValueSource::CommandLine) {
        args.data = config.data;
    }
    if matches.value_source("addons") != Some(ValueSource::CommandLine) {
        args.addons = config.addons;
    }
    setting!(web);
    setting!(bind);
    setting!(auth_upstream);
    setting!(world_upstream);
    setting!(workers);
    setting!(queue);
    setting!(max_asset_mib);
    setting!(cache_mib);
    setting!(asset_max_age_secs);
    println!("Gateway config: {}", path.display());
    Ok(())
}
