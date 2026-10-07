//! Layered `config.toml`: where the layers live and how they combine.
//!
//! Layers, lowest precedence first:
//! 1. the system file: `/etc/rekindle/config.toml` on Unix,
//!    `%ProgramData%\rekindle\config.toml` on Windows (`FOLDERID_ProgramData`);
//! 2. system drop-ins, `config.d/*.toml`, in lexicographic order;
//! 3. the user file in the data root's config dir
//!    ([`crate::paths::DataRoot::config`]);
//! 4. user drop-ins, in lexicographic order;
//! 5. `$REKINDLE_CONFIG`;
//! 6. an explicit path (`--config`), which must exist.
//!
//! Layers merge by **key presence**, as systemd drop-ins do: a key set in a
//! later layer replaces the earlier value (even with the default), a key a
//! layer leaves out is untouched, and tables merge key by key. The merged
//! table is then deserialized once, so the schema's strictness applies to
//! the result.

use std::fmt;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;

/// The environment variable naming one more config file.
pub const CONFIG_ENV: &str = "REKINDLE_CONFIG";

const FILE: &str = "config.toml";
const DROPINS: &str = "config.d";

/// One place a layer may come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Layer {
    /// A file read if it exists.
    File(PathBuf),
    /// A directory whose `*.toml` files are read in lexicographic order.
    DropIns(PathBuf),
    /// A file that must exist (`--config`).
    Required(PathBuf),
}

impl Layer {
    /// The path this layer reads.
    #[must_use]
    pub fn path(&self) -> &Path {
        match self {
            Self::File(p) | Self::DropIns(p) | Self::Required(p) => p,
        }
    }
}

/// Why the config could not be loaded.
#[derive(Debug)]
pub enum LayerError {
    /// The platform's system or user config directory is unknown.
    NoDir(&'static str),
    /// A layer exists but cannot be read.
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    /// A layer is not valid TOML.
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    /// The `--config` file does not exist.
    Missing(PathBuf),
    /// The merged config does not fit the schema.
    Schema(toml::de::Error),
}

impl fmt::Display for LayerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoDir(which) => write!(f, "cannot determine the {which} config directory"),
            Self::Read { path, source } => write!(f, "cannot read {}: {source}", path.display()),
            Self::Parse { path, source } => write!(f, "{}: {source}", path.display()),
            Self::Missing(path) => write!(f, "config file not found: {}", path.display()),
            Self::Schema(source) => write!(f, "config: {source}"),
        }
    }
}

impl std::error::Error for LayerError {}

/// The machine-wide config directory.
///
/// # Errors
/// On Windows, `%ProgramData%` is unset.
pub fn system_dir() -> Result<PathBuf, LayerError> {
    if cfg!(windows) {
        std::env::var_os("ProgramData")
            .map(|dir| PathBuf::from(dir).join("rekindle"))
            .ok_or(LayerError::NoDir("system"))
    } else {
        Ok(PathBuf::from("/etc/rekindle"))
    }
}

/// The user's config directory: the data root's `config`, the folder the
/// desktop keeps its files in (plan C5).
///
/// # Errors
/// The platform has no config directory for this user.
pub fn user_dir() -> Result<PathBuf, LayerError> {
    crate::paths::DataRoot::resolve()
        .map(|root| root.config)
        .map_err(|_| LayerError::NoDir("user"))
}

/// The layers, lowest precedence first.
///
/// # Errors
/// [`system_dir`] or [`user_dir`] cannot be resolved.
pub fn layers(explicit: Option<&Path>) -> Result<Vec<Layer>, LayerError> {
    let system = system_dir()?;
    let user = user_dir()?;
    let mut layers = vec![
        Layer::File(system.join(FILE)),
        Layer::DropIns(system.join(DROPINS)),
        Layer::File(user.join(FILE)),
        Layer::DropIns(user.join(DROPINS)),
    ];
    if let Some(env) = std::env::var_os(CONFIG_ENV) {
        layers.push(Layer::File(PathBuf::from(env)));
    }
    if let Some(path) = explicit {
        layers.push(Layer::Required(path.to_path_buf()));
    }
    Ok(layers)
}

/// Load every layer, merge them and deserialize the result as `T`.
///
/// # Errors
/// A layer cannot be read or parsed, `--config` is missing, or the merged
/// config does not fit `T`.
pub fn load<T: DeserializeOwned>(explicit: Option<&Path>) -> Result<T, LayerError> {
    load_layers(&layers(explicit)?)
}

/// Load and merge the given layers, then deserialize the result as `T`.
///
/// # Errors
/// As [`load`].
pub fn load_layers<T: DeserializeOwned>(layers: &[Layer]) -> Result<T, LayerError> {
    let mut merged = toml::Table::new();
    for layer in layers {
        for table in read_layer(layer)? {
            merge(&mut merged, table);
        }
    }
    toml::Value::Table(merged)
        .try_into()
        .map_err(LayerError::Schema)
}

fn read_layer(layer: &Layer) -> Result<Vec<toml::Table>, LayerError> {
    match layer {
        Layer::File(path) => Ok(read_file(path)?.into_iter().collect()),
        Layer::Required(path) => read_file(path)?
            .map(|table| vec![table])
            .ok_or_else(|| LayerError::Missing(path.clone())),
        Layer::DropIns(dir) => {
            let entries = match std::fs::read_dir(dir) {
                Ok(entries) => entries,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
                Err(source) => {
                    return Err(LayerError::Read {
                        path: dir.clone(),
                        source,
                    })
                }
            };
            let mut paths = Vec::new();
            for entry in entries {
                let path = entry
                    .map_err(|source| LayerError::Read {
                        path: dir.clone(),
                        source,
                    })?
                    .path();
                if path.extension().is_some_and(|ext| ext == "toml") {
                    paths.push(path);
                }
            }
            paths.sort();
            let mut tables = Vec::with_capacity(paths.len());
            for path in paths {
                tables.extend(read_file(&path)?);
            }
            Ok(tables)
        }
    }
}

/// A TOML file's table, or `None` if the file does not exist.
fn read_file(path: &Path) -> Result<Option<toml::Table>, LayerError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(LayerError::Read {
                path: path.to_path_buf(),
                source,
            })
        }
    };
    text.parse::<toml::Table>()
        .map(Some)
        .map_err(|source| LayerError::Parse {
            path: path.to_path_buf(),
            source,
        })
}

/// Merge `overlay` into `base` by key presence: tables merge recursively,
/// any other value present in `overlay` replaces the one in `base`.
pub fn merge(base: &mut toml::Table, overlay: toml::Table) {
    for (key, value) in overlay {
        match (base.get_mut(&key), value) {
            (Some(toml::Value::Table(existing)), toml::Value::Table(incoming)) => {
                merge(existing, incoming);
            }
            (_, value) => {
                base.insert(key, value);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(text: &str) -> toml::Table {
        text.parse().unwrap()
    }

    #[test]
    fn a_later_layer_can_restore_a_default() {
        let mut merged = table("[network]\nrpc_timeout_ms = 15000\ngossip_ttl = 7");
        merge(&mut merged, table("[network]\nrpc_timeout_ms = 8000"));
        assert_eq!(
            merged.to_string(),
            table("[network]\nrpc_timeout_ms = 8000\ngossip_ttl = 7").to_string()
        );
    }

    #[test]
    fn nested_tables_merge_key_by_key() {
        let mut merged = table("[network.safety.voice]\nstability = \"reliable\"\nhop_count = 4");
        merge(
            &mut merged,
            table("[network.safety.voice]\nstability = \"low_latency\""),
        );
        let voice = &merged["network"]["safety"]["voice"];
        assert_eq!(voice["stability"].as_str(), Some("low_latency"));
        assert_eq!(voice["hop_count"].as_integer(), Some(4));
    }

    #[test]
    fn drop_ins_apply_in_lexicographic_order_after_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let dropins = dir.path().join("config.d");
        std::fs::create_dir(&dropins).unwrap();
        std::fs::write(dir.path().join("config.toml"), "a = 1\nb = 1").unwrap();
        std::fs::write(dropins.join("20-late.toml"), "b = 3").unwrap();
        std::fs::write(dropins.join("10-early.toml"), "a = 2\nb = 2").unwrap();
        std::fs::write(dropins.join("ignored.txt"), "a = 9").unwrap();
        let loaded: toml::Table = load_layers(&[
            Layer::File(dir.path().join("config.toml")),
            Layer::DropIns(dropins),
        ])
        .unwrap();
        assert_eq!(loaded["a"].as_integer(), Some(2));
        assert_eq!(loaded["b"].as_integer(), Some(3));
    }

    #[test]
    fn missing_optional_layers_are_skipped_but_explicit_ones_are_required() {
        let dir = tempfile::tempdir().unwrap();
        let none: toml::Table = load_layers(&[
            Layer::File(dir.path().join("absent.toml")),
            Layer::DropIns(dir.path().join("absent.d")),
        ])
        .unwrap();
        assert!(none.is_empty());
        assert!(matches!(
            load_layers::<toml::Table>(&[Layer::Required(dir.path().join("absent.toml"))]),
            Err(LayerError::Missing(_))
        ));
    }

    #[test]
    fn a_broken_layer_names_its_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "network = [").unwrap();
        let err = load_layers::<toml::Table>(&[Layer::File(path.clone())]).unwrap_err();
        assert!(err.to_string().starts_with(&path.display().to_string()));
    }
}
