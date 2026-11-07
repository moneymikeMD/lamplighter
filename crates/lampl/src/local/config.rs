// SPDX-License-Identifier: Apache-2.0

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub(crate) const DB_FILE: &str = "lampl.db";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "lowercase")]
pub(crate) enum Config {
    Local { db_path: PathBuf },
    Server { url: String },
}

pub(crate) fn config_dir() -> Result<PathBuf, String> {
    resolve_dir("LAMPL_CONFIG_DIR", dirs::config_dir())
}

pub(crate) fn data_dir() -> Result<PathBuf, String> {
    resolve_dir("LAMPL_DATA_DIR", dirs::data_dir())
}

fn resolve_dir(var: &str, platform: Option<PathBuf>) -> Result<PathBuf, String> {
    match std::env::var_os(var) {
        Some(v) if !v.is_empty() => Ok(PathBuf::from(v)),
        _ => platform
            .map(|p| p.join("lampl"))
            .ok_or_else(|| format!("cannot determine a directory; set {var}")),
    }
}

pub(crate) fn config_path() -> Result<PathBuf, String> {
    Ok(config_dir()?.join("config.toml"))
}

/// Reads the config file. A missing file is an error naming `lampl init`; nothing is created.
pub(crate) fn load() -> Result<Config, String> {
    let path = config_path()?;
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err("not configured: run `lampl init`".to_owned());
        }
        Err(e) => return Err(format!("reading {}: {e}", path.display())),
    };
    toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

pub(crate) fn save(path: &Path, config: &Config) -> Result<(), String> {
    let text = toml::to_string_pretty(config).map_err(|e| e.to_string())?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    }
    std::fs::write(path, text).map_err(|e| format!("writing {}: {e}", path.display()))
}
