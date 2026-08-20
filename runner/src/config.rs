//! Configuration model: defaults, `~/.cellular/config.json`, project
//! `.cellular/config.json` and CLI arguments, merged field by field.
//!
//! A project may still carry a `.cellular` directory for its own
//! `config.json`; generated index data always lives under
//! `~/.cellular/index/`.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

pub const CELLULAR_DIR: &str = ".cellular";
pub const CONFIG_FILE: &str = "config.json";
pub const INDEX_DIR: &str = "index";

/// Every configurable field, in display order (used by the TUI list).
pub const FIELDS: &[&str] = &[
    "index_depth",
    "index_exclude",
    "index_detect_as_module",
    "ignoring_extensions",
    "ignoring_files",
    "metric",
    "select_date_query_result_is_multiple",
    "select_date_query_result_is_none",
    "threads",
];

/// Where a resolved value came from. Shown next to each field in the TUI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    Default,
    UserConfig(PathBuf),
    ProjectConfig(PathBuf),
    Args,
    /// Changed in this terminal session and not written to a file yet.
    Session,
}

impl Origin {
    /// Used by the terminal interface to label each field.
    #[allow(dead_code)]
    pub fn path(&self) -> Option<&Path> {
        match self {
            Origin::UserConfig(p) | Origin::ProjectConfig(p) => Some(p.as_path()),
            _ => None,
        }
    }
}

macro_rules! str_enum {
    ($name:ident { $( $variant:ident => $canon:literal $(| $alias:literal)* ),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub enum $name { $( $variant ),+ }

        impl $name {
            pub fn as_str(&self) -> &'static str {
                match self { $( $name::$variant => $canon ),+ }
            }
            pub fn parse(s: &str) -> Option<Self> {
                let s = s.trim().to_ascii_lowercase();
                match s.as_str() {
                    $( $canon $(| $alias)* => Some($name::$variant), )+
                    _ => None,
                }
            }
            pub fn variants() -> &'static [&'static str] { &[ $( $canon ),+ ] }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(self.as_str()) }
        }

        impl TryFrom<String> for $name {
            type Error = String;
            fn try_from(s: String) -> std::result::Result<Self, Self::Error> {
                $name::parse(&s).ok_or_else(|| {
                    format!("invalid value {s:?}, expected one of {:?}", $name::variants())
                })
            }
        }

        impl From<$name> for String {
            fn from(v: $name) -> String { v.as_str().to_string() }
        }
    };
}

str_enum!(DateMultiple {
    Latest => "latest",
    Oldest => "oldest",
    Median => "median",
});

str_enum!(DateNone {
    FastForward => "fast-forward" | "ff",
    Rewind => "rewind" | "rw",
});

/// How many threads a build measures commits with.
///
/// `Auto` is not one number decided up front: the build starts from the cores
/// the machine has and lowers the count while it runs when the volume holding
/// the index is filling up (see [`crate::disk`]). A `Fixed` count is taken as
/// given, cap included, because an explicit choice is the point of setting it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Threads {
    Auto,
    Fixed(usize),
}

impl Threads {
    pub const AUTO: &'static str = "auto";

    /// Parses `auto` or a positive integer. Zero threads is not a build.
    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim();
        if value.eq_ignore_ascii_case(Threads::AUTO) {
            return Some(Threads::Auto);
        }
        match value.parse::<usize>() {
            Ok(count) if count > 0 => Some(Threads::Fixed(count)),
            _ => None,
        }
    }

    pub fn is_auto(self) -> bool {
        matches!(self, Threads::Auto)
    }
}

impl fmt::Display for Threads {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Threads::Auto => f.write_str(Threads::AUTO),
            Threads::Fixed(count) => write!(f, "{count}"),
        }
    }
}

/// `"auto"` stays a string in `config.json`; a fixed count stays a number.
impl Serialize for Threads {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        match self {
            Threads::Auto => serializer.serialize_str(Threads::AUTO),
            Threads::Fixed(count) => serializer.serialize_u64(*count as u64),
        }
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ThreadsRepr {
    Word(String),
    Count(i64),
}

impl<'de> Deserialize<'de> for Threads {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        use serde::de::Error;
        // The file holds the type as written: the word `auto`, or a number.
        // A quoted number is neither, and typing one is a mistake worth
        // hearing about rather than guessing at.
        match ThreadsRepr::deserialize(deserializer)? {
            ThreadsRepr::Word(word) if word == Threads::AUTO => Ok(Threads::Auto),
            ThreadsRepr::Word(word) => Err(D::Error::custom(format!(
                "threads must be {:?} or a positive integer, not {word:?}",
                Threads::AUTO
            ))),
            ThreadsRepr::Count(count) if count > 0 => Ok(Threads::Fixed(count as usize)),
            ThreadsRepr::Count(count) => Err(D::Error::custom(format!(
                "threads must be {:?} or a positive integer, not {count}",
                Threads::AUTO
            ))),
        }
    }
}

/// The on-disk shape of a `config.json`. Absent fields do not override.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConfigFile {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index_depth: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index_exclude: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index_detect_as_module: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ignoring_extensions: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ignoring_files: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metric: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub select_date_query_result_is_multiple: Option<DateMultiple>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub select_date_query_result_is_none: Option<DateNone>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub threads: Option<Threads>,
    /// Anything else the file holds: comments a user added, or fields a newer
    /// version writes. Saving must not silently delete them.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl ConfigFile {
    pub fn load(path: &Path) -> Result<Option<Self>> {
        if !path.exists() {
            return Ok(None);
        }
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read config file {}", path.display()))?;
        let parsed: Self = serde_json::from_str(&text)
            .with_context(|| format!("failed to parse config file {}", path.display()))?;
        Ok(Some(parsed))
    }

    /// Copy one field out of a resolved config, so saving writes back only the
    /// fields the user actually changed and leaves the rest of the file alone.
    pub fn take_field(&mut self, config: &Config, field: &str) {
        match field {
            "index_depth" => self.index_depth = config.index_depth,
            "index_exclude" => self.index_exclude = Some(config.index_exclude.clone()),
            "index_detect_as_module" => {
                self.index_detect_as_module = Some(config.index_detect_as_module.clone())
            }
            "ignoring_extensions" => {
                self.ignoring_extensions = Some(config.ignoring_extensions.clone())
            }
            "ignoring_files" => self.ignoring_files = Some(config.ignoring_files.clone()),
            "metric" => self.metric = Some(config.metric.clone()),
            "select_date_query_result_is_multiple" => {
                self.select_date_query_result_is_multiple =
                    Some(config.select_date_query_result_is_multiple)
            }
            "select_date_query_result_is_none" => {
                self.select_date_query_result_is_none =
                    Some(config.select_date_query_result_is_none)
            }
            "threads" => self.threads = Some(config.threads),
            _ => {}
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let text = serde_json::to_string_pretty(self)?;
        std::fs::write(path, text + "\n")
            .with_context(|| format!("failed to write config file {}", path.display()))?;
        Ok(())
    }
}

/// A fully resolved configuration plus the origin of every field.
#[derive(Debug, Clone)]
pub struct Config {
    /// No default: the build refuses to run until this is set.
    pub index_depth: Option<u32>,
    pub index_exclude: Vec<String>,
    pub index_detect_as_module: Vec<String>,
    pub ignoring_extensions: Vec<String>,
    pub ignoring_files: Vec<String>,
    pub metric: Vec<String>,
    pub select_date_query_result_is_multiple: DateMultiple,
    pub select_date_query_result_is_none: DateNone,
    pub threads: Threads,
    pub origins: BTreeMap<String, Origin>,
}

fn to_vec(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_string()).collect()
}

impl Default for Config {
    fn default() -> Self {
        let mut origins = BTreeMap::new();
        for field in FIELDS {
            origins.insert((*field).to_string(), Origin::Default);
        }
        Config {
            index_depth: None,
            index_exclude: to_vec(&[".git*"]),
            index_detect_as_module: Vec::new(),
            ignoring_extensions: to_vec(&[
                ".md",
                ".markdown",
                ".txt",
                ".json",
                ".csv",
                ".yml",
                ".yaml",
                ".git*",
            ]),
            ignoring_files: to_vec(&["README.md", "LICENSE"]),
            metric: to_vec(&["lines", "chars", "languages"]),
            select_date_query_result_is_multiple: DateMultiple::Latest,
            select_date_query_result_is_none: DateNone::FastForward,
            threads: Threads::Auto,
            origins,
        }
    }
}

macro_rules! apply {
    ($self:ident, $file:ident, $origin:expr, $( $field:ident ),+ $(,)?) => {
        $(
            if let Some(value) = $file.$field.clone() {
                $self.$field = value;
                $self.origins.insert(stringify!($field).to_string(), $origin.clone());
            }
        )+
    };
}

impl Config {
    /// Overlay a config file; only fields present in it are overridden.
    pub fn overlay(&mut self, file: &ConfigFile, origin: Origin) {
        if let Some(value) = file.index_depth {
            self.index_depth = Some(value);
            self.origins
                .insert("index_depth".to_string(), origin.clone());
        }
        apply!(
            self,
            file,
            origin,
            index_exclude,
            index_detect_as_module,
            ignoring_extensions,
            ignoring_files,
            metric,
            select_date_query_result_is_multiple,
            select_date_query_result_is_none,
            threads,
        );
    }

    pub fn origin_of(&self, field: &str) -> &Origin {
        self.origins.get(field).unwrap_or(&Origin::Default)
    }

    /// Human-readable value of a field, for the TUI config list and `get`.
    pub fn display_value(&self, field: &str) -> Option<String> {
        Some(match field {
            "index_depth" => self
                .index_depth
                .map(|v| v.to_string())
                .unwrap_or_else(|| "(unset)".to_string()),
            "index_exclude" => format_list(&self.index_exclude),
            "index_detect_as_module" => format_list(&self.index_detect_as_module),
            "ignoring_extensions" => format_list(&self.ignoring_extensions),
            "ignoring_files" => format_list(&self.ignoring_files),
            "metric" => format_list(&self.metric),
            "select_date_query_result_is_multiple" => {
                self.select_date_query_result_is_multiple.to_string()
            }
            "select_date_query_result_is_none" => self.select_date_query_result_is_none.to_string(),
            "threads" => self.threads.to_string(),
            _ => return None,
        })
    }

    /// The current value of a list-valued field.
    pub fn list_value(&self, field: &str) -> Option<&[String]> {
        Some(match field {
            "index_exclude" => &self.index_exclude,
            "index_detect_as_module" => &self.index_detect_as_module,
            "ignoring_extensions" => &self.ignoring_extensions,
            "ignoring_files" => &self.ignoring_files,
            "metric" => &self.metric,
            _ => return None,
        })
    }

    /// Candidate values offered by completion, for fields that are enums.
    pub fn value_candidates(field: &str) -> &'static [&'static str] {
        match field {
            "select_date_query_result_is_multiple" => DateMultiple::variants(),
            "select_date_query_result_is_none" => DateNone::variants(),
            "threads" => &[Threads::AUTO],
            _ => &[],
        }
    }

    /// List-valued fields open the multi-line editor in the terminal interface.
    pub fn is_list_field(field: &str) -> bool {
        matches!(
            field,
            "index_exclude"
                | "index_detect_as_module"
                | "ignoring_extensions"
                | "ignoring_files"
                | "metric"
        )
    }

    pub fn wants_metric(&self, name: &str) -> bool {
        crate::filters::any_glob_matches(&self.metric, name)
    }
}

fn format_list(items: &[String]) -> String {
    format!("[{}]", items.join(", "))
}

/// `~/.cellular`
pub fn profile_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("could not determine the home directory")?;
    Ok(home.join(CELLULAR_DIR))
}

/// `~/.cellular/index`, the parent of every project's index directory.
pub fn profile_index_dir() -> Result<PathBuf> {
    Ok(profile_dir()?.join(INDEX_DIR))
}

/// `~/.cellular/config.json`
pub fn user_config_path() -> Result<PathBuf> {
    Ok(profile_dir()?.join(CONFIG_FILE))
}

/// `<project>/.cellular/config.json`
pub fn project_config_path(project_root: &Path) -> PathBuf {
    project_root.join(CELLULAR_DIR).join(CONFIG_FILE)
}

/// Merge defaults, the user config and the project config. CLI arguments are
/// applied on top of this by the caller.
pub fn load_layered(project_root: &Path) -> Result<Config> {
    let mut config = Config::default();

    if let Ok(user_path) = user_config_path()
        && let Some(file) = ConfigFile::load(&user_path)?
    {
        config.overlay(&file, Origin::UserConfig(user_path));
    }

    let project_path = project_config_path(project_root);
    if let Some(file) = ConfigFile::load(&project_path)? {
        config.overlay(&file, Origin::ProjectConfig(project_path));
    }

    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saving_keeps_fields_the_runner_does_not_know() {
        let text = r#"{
            "index_depth": 1,
            "_comment": "why depth 1",
            "future_option": true
        }"#;
        let mut file: ConfigFile = serde_json::from_str(text).unwrap();
        assert_eq!(file.index_depth, Some(1));
        assert_eq!(file.extra.len(), 2);

        let config = Config {
            metric: vec!["lines".to_string()],
            ..Config::default()
        };
        file.take_field(&config, "metric");

        let written = serde_json::to_string(&file).unwrap();
        let round_trip: serde_json::Value = serde_json::from_str(&written).unwrap();
        assert_eq!(round_trip["_comment"], "why depth 1");
        assert_eq!(round_trip["future_option"], true);
        assert_eq!(round_trip["index_depth"], 1);
        assert_eq!(round_trip["metric"][0], "lines");
    }

    #[test]
    fn threads_accepts_auto_and_a_positive_count() {
        assert_eq!(Threads::parse("auto"), Some(Threads::Auto));
        assert_eq!(Threads::parse(" AUTO "), Some(Threads::Auto));
        assert_eq!(Threads::parse("4"), Some(Threads::Fixed(4)));
        assert_eq!(Threads::parse("0"), None);
        assert_eq!(Threads::parse("-2"), None);
        assert_eq!(Threads::parse("many"), None);
    }

    #[test]
    fn threads_round_trips_through_the_config_file() {
        let file: ConfigFile = serde_json::from_str(r#"{"threads": "auto"}"#).unwrap();
        assert_eq!(file.threads, Some(Threads::Auto));
        let file: ConfigFile = serde_json::from_str(r#"{"threads": 3}"#).unwrap();
        assert_eq!(file.threads, Some(Threads::Fixed(3)));
        assert!(serde_json::from_str::<ConfigFile>(r#"{"threads": 0}"#).is_err());
        // A count is a number in the file, not a string that looks like one.
        assert!(serde_json::from_str::<ConfigFile>(r#"{"threads": "4"}"#).is_err());
        assert!(serde_json::from_str::<ConfigFile>(r#"{"threads": "AUTO"}"#).is_err());

        let written = serde_json::to_string(&file).unwrap();
        assert!(written.contains(r#""threads":3"#), "{written}");
        let auto = ConfigFile {
            threads: Some(Threads::Auto),
            ..ConfigFile::default()
        };
        let written = serde_json::to_string(&auto).unwrap();
        assert!(written.contains(r#""threads":"auto""#), "{written}");
    }

    #[test]
    fn an_absent_field_does_not_override() {
        let file: ConfigFile = serde_json::from_str(r#"{"index_depth": 4}"#).unwrap();
        let mut config = Config::default();
        let before = config.ignoring_files.clone();
        config.overlay(&file, Origin::Args);
        assert_eq!(config.index_depth, Some(4));
        assert_eq!(config.ignoring_files, before);
    }
}
