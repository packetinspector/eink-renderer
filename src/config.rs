//! Configuration. Layers, lowest to highest precedence:
//! built-in defaults < settings stored by Home Assistant pairing (`/data/state.json`) < TOML file < environment.
//! Keys set by the file or the environment are reported as `locked` (Home Assistant cannot change them).
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::path::Path;

pub const DEFAULT_CONFIG_PATH: &str = "/etc/eink-renderer/config.toml";
pub const DEFAULT_DATA_DIR: &str = "/data";

/// Settings Home Assistant may set through pairing / `POST /api/settings`. All optional.
#[derive(Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ha_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dashboard_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub header_crop: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub threshold: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invert: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dither: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dark: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_hinting: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_resources: Option<Vec<String>>,
    /// IANA time zone for the rendered page (clock cards); Home Assistant sends its own.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
}

impl fmt::Debug for Settings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Settings")
            .field("ha_url", &self.ha_url)
            .field("token", &self.token.as_ref().map(|_| "<redacted>"))
            .field("dashboard_path", &self.dashboard_path)
            .finish_non_exhaustive()
    }
}

impl Settings {
    /// Overlay `other` onto `self` (fields present in `other` win).
    pub fn merge(&mut self, other: &Settings) {
        macro_rules! m { ($($k:ident),*) => { $( if other.$k.is_some() { self.$k = other.$k.clone(); } )* } }
        m!(
            ha_url,
            token,
            dashboard_path,
            width,
            height,
            header_crop,
            threshold,
            invert,
            dither,
            theme,
            dark,
            font_hinting,
            block_resources,
            timezone
        );
    }
}

/// Effective settings. The token is kept out of `Debug` output and never logged.
#[derive(Clone)]
pub struct Config {
    pub listen: String,
    pub ha_url: String,
    token: Option<String>,
    pub token_file: Option<String>,
    pub dashboard_path: String,
    pub width: u32,
    pub height: u32,
    pub header_crop: u32,
    pub threshold: u8,
    pub invert: bool,
    pub dither: String,
    pub theme: String,
    pub dark: bool,
    pub font_hinting: String,
    pub block_resources: Vec<String>,
    /// IANA zone applied to the page ("" = the process time zone, i.e. TZ).
    pub timezone: String,
    pub chrome_path: String,
    pub chrome_extra_flags: Vec<String>,
    pub stale_after_secs: u64,
    pub ping_interval_secs: u64,
    pub ping_timeout_secs: u64,
    pub cache_file: Option<String>,
    /// Directory for state.json / id / last.png (None: nothing persisted, pairing unavailable).
    pub data_dir: Option<String>,
    /// Name announced over mDNS and shown in Home Assistant.
    pub name: String,
    /// Announce over mDNS (`ADVERTISE=false` to disable).
    pub advertise: bool,
    /// Restrict the mDNS announcement to this address.
    pub advertise_ip: Option<String>,
    /// Config keys fixed by the TOML file or the environment.
    pub locked: Vec<String>,
}

impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("listen", &self.listen)
            .field("ha_url", &self.ha_url)
            .field("token", &self.token.as_ref().map(|_| "<redacted>"))
            .field("token_file", &self.token_file)
            .field("dashboard_path", &self.dashboard_path)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("threshold", &self.threshold)
            .field("invert", &self.invert)
            .field("dither", &self.dither)
            .field("theme", &self.theme)
            .field("locked", &self.locked)
            .finish_non_exhaustive()
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            listen: "0.0.0.0:10001".into(),
            ha_url: String::new(),
            token: None,
            token_file: None,
            dashboard_path: "/lovelace/0".into(),
            width: 800,
            height: 480,
            header_crop: 56,
            threshold: 220,
            invert: true,
            dither: "none".into(),
            theme: String::new(),
            dark: false,
            font_hinting: "slight".into(),
            block_resources: Vec::new(),
            timezone: String::new(),
            chrome_path: "/opt/chrome/chrome-headless-shell".into(),
            chrome_extra_flags: Vec::new(),
            stale_after_secs: 300,
            ping_interval_secs: 30,
            ping_timeout_secs: 10,
            cache_file: None,
            data_dir: None,
            name: "E-ink Renderer".into(),
            advertise: true,
            advertise_ip: None,
            locked: Vec::new(),
        }
    }
}

/// The TOML file: every key optional.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    listen: Option<String>,
    ha_url: Option<String>,
    token_file: Option<String>,
    dashboard_path: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    header_crop: Option<u32>,
    threshold: Option<u8>,
    invert: Option<bool>,
    dither: Option<String>,
    theme: Option<String>,
    dark: Option<bool>,
    font_hinting: Option<String>,
    block_resources: Option<Vec<String>>,
    chrome_path: Option<String>,
    chrome_extra_flags: Option<Vec<String>>,
    stale_after_secs: Option<u64>,
    ping_interval_secs: Option<u64>,
    ping_timeout_secs: Option<u64>,
    cache_file: Option<String>,
    data_dir: Option<String>,
    name: Option<String>,
    advertise: Option<bool>,
    advertise_ip: Option<String>,
}

fn parse_bool(k: &str, v: &str) -> Result<bool, String> {
    match v.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(format!("{k}: expected true/false, got {v:?}")),
    }
}

fn parse_num<T: std::str::FromStr>(k: &str, v: &str) -> Result<T, String> {
    v.trim()
        .parse()
        .map_err(|_| format!("{k}: expected a number, got {v:?}"))
}

/// What the process was started with (file text + environment), kept so the effective config can be
/// recomputed whenever Home Assistant changes the stored settings.
#[derive(Clone, Default)]
pub struct Sources {
    pub file: Option<String>,
    pub env: HashMap<String, String>,
}

impl Sources {
    /// Read CONFIG_FILE (or the first CLI argument, or the default path if it exists) and the environment.
    pub fn from_process() -> Result<Sources, String> {
        let env: HashMap<String, String> = std::env::vars().collect();
        let explicit = env
            .get("CONFIG_FILE")
            .cloned()
            .filter(|s| !s.is_empty())
            .or_else(|| std::env::args().nth(1).filter(|a| !a.starts_with("--")));
        let read = |p: &str| std::fs::read_to_string(p).map_err(|e| format!("config file {p}: {e}"));
        let file = match &explicit {
            Some(p) => Some(read(p)?),
            None if Path::new(DEFAULT_CONFIG_PATH).exists() => Some(read(DEFAULT_CONFIG_PATH)?),
            None => None,
        };
        Ok(Sources { file, env })
    }

    /// The data directory: DATA_DIR / data_dir, else /data if it exists.
    pub fn data_dir(&self) -> Option<String> {
        if let Some(d) = self.env.get("DATA_DIR").filter(|s| !s.trim().is_empty()) {
            return Some(d.trim().into());
        }
        if let Some(text) = &self.file {
            if let Ok(f) = toml::from_str::<FileConfig>(text) {
                if let Some(d) = f.data_dir {
                    return Some(d);
                }
            }
        }
        Path::new(DEFAULT_DATA_DIR)
            .is_dir()
            .then(|| DEFAULT_DATA_DIR.to_string())
    }

    pub fn build(&self, state: Option<&Settings>) -> Result<Config, String> {
        Config::from_sources(self.file.as_deref(), &self.env, state)
    }
}

impl Config {
    /// Build from an optional TOML document, an environment map and optional stored settings.
    pub fn from_sources(
        file: Option<&str>,
        env: &HashMap<String, String>,
        state: Option<&Settings>,
    ) -> Result<Config, String> {
        let mut c = Config::default();
        let mut locked: Vec<&str> = Vec::new();
        if let Some(s) = state {
            macro_rules! st { ($($k:ident),*) => { $( if let Some(v) = &s.$k { c.$k = v.clone(); } )* } }
            st!(
                ha_url,
                dashboard_path,
                width,
                height,
                header_crop,
                threshold,
                invert,
                dither,
                theme,
                dark,
                font_hinting,
                block_resources,
                timezone
            );
            if s.token.is_some() {
                c.token = s.token.clone();
            }
        }
        if let Some(text) = file {
            let f: FileConfig = toml::from_str(text).map_err(|e| format!("config file: {e}"))?;
            macro_rules! take {
                ($($k:ident),*) => { $( if let Some(v) = f.$k { c.$k = v; locked.push(stringify!($k)); } )* }
            }
            take!(
                listen,
                ha_url,
                dashboard_path,
                width,
                height,
                header_crop,
                threshold,
                invert,
                dither,
                theme,
                dark,
                font_hinting,
                block_resources,
                chrome_path,
                chrome_extra_flags,
                stale_after_secs,
                ping_interval_secs,
                ping_timeout_secs,
                name,
                advertise
            );
            if f.token_file.is_some() {
                c.token_file = f.token_file;
                c.token = None;
                locked.push("token");
            }
            if f.cache_file.is_some() {
                c.cache_file = f.cache_file;
            }
            if f.data_dir.is_some() {
                c.data_dir = f.data_dir;
            }
            if f.advertise_ip.is_some() {
                c.advertise_ip = f.advertise_ip;
            }
        }
        // Empty values are treated as unset, except THEME and BLOCK_RESOURCES where "" is meaningful.
        let get = |k: &str| env.get(k).map(|s| s.trim()).filter(|s| !s.is_empty());
        macro_rules! env_str {
            ($var:literal, $k:ident) => {
                if let Some(v) = get($var) {
                    c.$k = v.into();
                    locked.push(stringify!($k));
                }
            };
        }
        macro_rules! env_num {
            ($var:literal, $k:ident) => {
                if let Some(v) = get($var) {
                    c.$k = parse_num($var, v)?;
                    locked.push(stringify!($k));
                }
            };
        }
        env_str!("HA_URL", ha_url);
        if let Some(v) = get("HA_TOKEN_FILE") {
            c.token_file = Some(v.into());
            c.token = None;
            locked.push("token");
        }
        if let Some(v) = get("HA_TOKEN") {
            c.token = Some(v.into());
            locked.push("token");
        }
        env_str!("DASHBOARD_PATH", dashboard_path);
        env_num!("WIDTH", width);
        env_num!("HEIGHT", height);
        env_num!("HEADER_CROP", header_crop);
        env_num!("THRESHOLD", threshold);
        if let Some(v) = get("INVERT") {
            c.invert = parse_bool("INVERT", v)?;
            locked.push("invert");
        }
        if let Some(v) = get("DITHER") {
            c.dither = v.to_ascii_lowercase();
            locked.push("dither");
        }
        if let Some(v) = env.get("THEME") {
            c.theme = v.trim().into();
            locked.push("theme");
        }
        if let Some(v) = get("DARK") {
            c.dark = parse_bool("DARK", v)?;
            locked.push("dark");
        }
        if let Some(v) = get("FONT_HINTING") {
            c.font_hinting = v.to_ascii_lowercase();
            locked.push("font_hinting");
        }
        if let Some(v) = env.get("BLOCK_RESOURCES") {
            c.block_resources = v
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            locked.push("block_resources");
        }
        if let Some(v) = get("TZ") {
            c.timezone = v.into();
            locked.push("timezone");
        }
        if let Some(v) = get("LISTEN") {
            c.listen = v.into();
        }
        if let Some(v) = get("PORT") {
            let p: u16 = parse_num("PORT", v)?;
            let host = c
                .listen
                .rsplit_once(':')
                .map(|(h, _)| h.to_string())
                .unwrap_or_else(|| "0.0.0.0".into());
            c.listen = format!("{host}:{p}");
        }
        if let Some(v) = get("CACHE_FILE") {
            c.cache_file = Some(v.into());
        }
        if let Some(v) = get("DATA_DIR") {
            c.data_dir = Some(v.into());
        }
        if let Some(v) = get("NAME") {
            c.name = v.into();
        }
        if let Some(v) = get("ADVERTISE") {
            c.advertise = parse_bool("ADVERTISE", v)?;
        }
        if let Some(v) = get("ADVERTISE_IP") {
            c.advertise_ip = Some(v.into());
        }
        if let Some(v) = get("CHROME_PATH") {
            c.chrome_path = v.into();
        }
        if let Some(v) = get("CHROME_FLAGS") {
            c.chrome_extra_flags = v.split_whitespace().map(String::from).collect();
        }
        if let Some(v) = get("STALE_AFTER_SECS") {
            c.stale_after_secs = parse_num("STALE_AFTER_SECS", v)?;
        }
        if c.cache_file.is_none() {
            if let Some(d) = &c.data_dir {
                c.cache_file = Some(format!("{}/last.png", d.trim_end_matches('/')));
            }
        }
        // only report keys Home Assistant could otherwise set
        const SETTING_KEYS: &[&str] = &[
            "ha_url",
            "token",
            "dashboard_path",
            "width",
            "height",
            "header_crop",
            "threshold",
            "invert",
            "dither",
            "theme",
            "dark",
            "font_hinting",
            "block_resources",
            "timezone",
        ];
        locked.retain(|k| SETTING_KEYS.contains(k));
        locked.sort_unstable();
        locked.dedup();
        c.locked = locked.into_iter().map(String::from).collect();
        c.validate()?;
        Ok(c)
    }

    /// Value checks. A missing HA_URL or token is not an error: the renderer then waits to be set up.
    fn validate(&self) -> Result<(), String> {
        if !self.ha_url.is_empty() && !(self.ha_url.starts_with("http://") || self.ha_url.starts_with("https://")) {
            return Err(format!(
                "HA_URL must start with http:// or https://, got {:?}",
                self.ha_url
            ));
        }
        if !self.dashboard_path.starts_with('/') {
            return Err(format!(
                "DASHBOARD_PATH must start with '/', got {:?}",
                self.dashboard_path
            ));
        }
        if self.timezone.len() > 64
            || self
                .timezone
                .chars()
                .any(|ch| !(ch.is_ascii_alphanumeric() || "/_+-".contains(ch)))
        {
            return Err(format!(
                "timezone must be an IANA name like Europe/Berlin, got {:?}",
                self.timezone
            ));
        }
        if !matches!(self.dither.as_str(), "none" | "atkinson" | "floyd-steinberg") {
            return Err(format!(
                "DITHER must be none, atkinson or floyd-steinberg, got {:?}",
                self.dither
            ));
        }
        if !matches!(self.font_hinting.as_str(), "none" | "slight" | "medium" | "full") {
            return Err(format!(
                "FONT_HINTING must be none, slight, medium or full, got {:?}",
                self.font_hinting
            ));
        }
        if self.width == 0 || self.height == 0 || self.width > 4096 || self.height > 4096 {
            return Err(format!("WIDTH/HEIGHT out of range: {}x{}", self.width, self.height));
        }
        Ok(())
    }

    /// True when there is enough to render: an HA URL and a token source.
    pub fn is_configured(&self) -> bool {
        !self.ha_url.is_empty() && (self.token.is_some() || self.token_file.is_some())
    }

    /// The token, from HA_TOKEN, the token file, or pairing. Never log the result.
    pub fn read_token(&self) -> Result<String, String> {
        if let Some(t) = &self.token {
            return Ok(t.clone());
        }
        let Some(path) = self.token_file.as_deref() else {
            return Err("no token configured".into());
        };
        use std::os::unix::fs::PermissionsExt;
        let md = std::fs::metadata(path).map_err(|e| format!("token file {path}: {e}"))?;
        if md.permissions().mode() & 0o077 != 0 {
            crate::log(&format!(
                "warning: token file {path} is readable by group/other; chmod 600 it"
            ));
        }
        let t = std::fs::read_to_string(path).map_err(|e| format!("token file {path}: {e}"))?;
        let t = t.trim().to_string();
        if t.is_empty() {
            return Err(format!("token file {path} is empty"));
        }
        Ok(t)
    }

    pub fn page_url(&self) -> String {
        format!("{}{}", self.ha_url.trim_end_matches('/'), self.dashboard_path)
    }

    pub fn port(&self) -> u16 {
        self.listen
            .rsplit(':')
            .next()
            .and_then(|p| p.parse().ok())
            .unwrap_or(10001)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn env_only_minimal() {
        let c = Config::from_sources(None, &env(&[("HA_URL", "http://ha:8123"), ("HA_TOKEN", "abc")]), None).unwrap();
        assert_eq!(c.page_url(), "http://ha:8123/lovelace/0");
        assert_eq!((c.width, c.height, c.threshold, c.invert), (800, 480, 220, true));
        assert_eq!(c.dither, "none");
        assert!(c.block_resources.is_empty());
        assert_eq!(c.read_token().unwrap(), "abc");
        assert_eq!(c.listen, "0.0.0.0:10001");
        assert_eq!(c.cache_file, None);
        assert!(c.is_configured());
        assert_eq!(c.locked, vec!["ha_url", "token"]);
    }

    #[test]
    fn nothing_set_means_waiting_for_setup() {
        let c = Config::from_sources(None, &env(&[]), None).unwrap();
        assert!(!c.is_configured());
        assert!(c.locked.is_empty());
    }

    #[test]
    fn precedence_state_then_file_then_env() {
        let state = Settings {
            ha_url: Some("http://state:8123".into()),
            token: Some("statetoken".into()),
            dashboard_path: Some("/state/0".into()),
            threshold: Some(50),
            theme: Some("StateTheme".into()),
            ..Default::default()
        };
        let file = "dashboard_path = \"/file/0\"\nthreshold = 100\ntoken_file = \"/run/secrets/t\"";
        let e = env(&[("THRESHOLD", "200")]);
        let c = Config::from_sources(Some(file), &e, Some(&state)).unwrap();
        assert_eq!(
            c.ha_url, "http://state:8123",
            "state value used when nothing overrides it"
        );
        assert_eq!(c.dashboard_path, "/file/0", "file beats state");
        assert_eq!(c.threshold, 200, "env beats file");
        assert_eq!(c.theme, "StateTheme");
        assert_eq!(c.token_file.as_deref(), Some("/run/secrets/t"));
        assert!(
            c.token.is_none(),
            "a token file from file/env replaces the paired token"
        );
        assert_eq!(c.locked, vec!["dashboard_path", "threshold", "token"]);
    }

    #[test]
    fn timezone_from_home_assistant_unless_tz_is_set() {
        let state = Settings {
            timezone: Some("Europe/Berlin".into()),
            ..Default::default()
        };
        let c = Config::from_sources(None, &env(&[]), Some(&state)).unwrap();
        assert_eq!(c.timezone, "Europe/Berlin");
        let c = Config::from_sources(None, &env(&[("TZ", "Asia/Tokyo")]), Some(&state)).unwrap();
        assert_eq!(c.timezone, "Asia/Tokyo");
        assert!(c.locked.contains(&"timezone".to_string()));
        let bad = Settings {
            timezone: Some("x; rm -rf".into()),
            ..Default::default()
        };
        assert!(Config::from_sources(None, &env(&[]), Some(&bad)).is_err());
    }

    #[test]
    fn paired_state_alone_is_configured() {
        let state = Settings {
            ha_url: Some("http://h:8123".into()),
            token: Some("t".into()),
            ..Default::default()
        };
        let c = Config::from_sources(None, &env(&[]), Some(&state)).unwrap();
        assert!(c.is_configured());
        assert_eq!(c.read_token().unwrap(), "t");
        assert!(c.locked.is_empty());
    }

    #[test]
    fn env_overrides_file() {
        let file = r#"
            ha_url = "http://file:8123"
            dashboard_path = "/file/0"
            threshold = 100
            invert = false
            token_file = "/run/secrets/t"
            block_resources = ["/a/"]
        "#;
        let e = env(&[
            ("HA_URL", "http://env:8123/"),
            ("THRESHOLD", "200"),
            ("INVERT", "yes"),
            ("PORT", "9000"),
        ]);
        let c = Config::from_sources(Some(file), &e, None).unwrap();
        assert_eq!(c.ha_url, "http://env:8123/");
        assert_eq!(c.page_url(), "http://env:8123/file/0");
        assert_eq!(c.threshold, 200);
        assert!(c.invert);
        assert_eq!(c.block_resources, vec!["/a/".to_string()]);
        assert_eq!(c.token_file.as_deref(), Some("/run/secrets/t"));
        assert_eq!(c.listen, "0.0.0.0:9000");
        assert_eq!(c.port(), 9000);
    }

    #[test]
    fn data_dir_gives_default_cache_file() {
        let c = Config::from_sources(None, &env(&[("DATA_DIR", "/data/")]), None).unwrap();
        assert_eq!(c.cache_file.as_deref(), Some("/data/last.png"));
        let c = Config::from_sources(None, &env(&[("DATA_DIR", "/data"), ("CACHE_FILE", "/x.png")]), None).unwrap();
        assert_eq!(c.cache_file.as_deref(), Some("/x.png"));
    }

    #[test]
    fn empty_env_values() {
        let file = "ha_url = \"http://f:1\"\ntoken_file = \"/t\"\nblock_resources = [\"/x/\"]\ntheme = \"T\"";
        let c = Config::from_sources(
            Some(file),
            &env(&[("HA_URL", ""), ("BLOCK_RESOURCES", ""), ("THEME", "")]),
            None,
        )
        .unwrap();
        assert_eq!(c.ha_url, "http://f:1", "empty HA_URL is ignored");
        assert!(c.block_resources.is_empty(), "BLOCK_RESOURCES=\"\" clears the list");
        assert_eq!(c.theme, "", "THEME=\"\" selects the HA default theme");
    }

    #[test]
    fn hinting_flag_and_list_parsing() {
        let e = env(&[
            ("HA_URL", "http://h"),
            ("HA_TOKEN", "x"),
            ("BLOCK_RESOURCES", " /a/ ,, /b/ "),
            ("CHROME_FLAGS", "--foo  --bar=1"),
            ("FONT_HINTING", "FULL"),
        ]);
        let c = Config::from_sources(None, &e, None).unwrap();
        assert_eq!(c.block_resources, vec!["/a/", "/b/"]);
        assert_eq!(c.chrome_extra_flags, vec!["--foo", "--bar=1"]);
        assert_eq!(c.font_hinting, "full");
    }

    #[test]
    fn errors_are_explicit() {
        let e = Config::from_sources(None, &env(&[("HA_URL", "h:8123"), ("HA_TOKEN", "x")]), None).unwrap_err();
        assert!(e.contains("http://"), "{e}");
        let e = Config::from_sources(None, &env(&[("DITHER", "bayer")]), None).unwrap_err();
        assert!(e.contains("DITHER"), "{e}");
        let e = Config::from_sources(None, &env(&[("INVERT", "maybe")]), None).unwrap_err();
        assert!(e.contains("INVERT"), "{e}");
        assert!(Config::from_sources(Some("bogus_key = 1"), &env(&[]), None).is_err());
        let bad = Settings {
            dither: Some("bayer".into()),
            ..Default::default()
        };
        assert!(Config::from_sources(None, &env(&[]), Some(&bad)).is_err());
    }

    #[test]
    fn debug_redacts_token() {
        let c = Config::from_sources(None, &env(&[("HA_URL", "http://h"), ("HA_TOKEN", "supersecret")]), None).unwrap();
        assert!(!format!("{c:?}").contains("supersecret"));
        let s = Settings {
            token: Some("supersecret".into()),
            ..Default::default()
        };
        assert!(!format!("{s:?}").contains("supersecret"));
    }

    #[test]
    fn settings_merge_and_json() {
        let mut a = Settings {
            threshold: Some(1),
            theme: Some("A".into()),
            ..Default::default()
        };
        a.merge(&Settings {
            threshold: Some(2),
            ..Default::default()
        });
        assert_eq!((a.threshold, a.theme.as_deref()), (Some(2), Some("A")));
        let j = serde_json::to_string(&a).unwrap();
        assert_eq!(j, r#"{"threshold":2,"theme":"A"}"#);
        assert!(serde_json::from_str::<Settings>(r#"{"nope":1}"#).is_err());
    }
}
