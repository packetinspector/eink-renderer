//! Persistent state in the data directory: renderer id, pairing (API key) and settings set by Home Assistant.
use crate::config::Settings;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct State {
    /// Stable id, used as the Home Assistant unique id and mDNS instance suffix.
    pub id: String,
    /// Key Home Assistant must present to change settings. None = not paired.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paired_with: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paired_at: Option<u64>,
    #[serde(default)]
    pub settings: Settings,
}

impl State {
    pub fn paired(&self) -> bool {
        self.api_key.is_some()
    }
}

pub struct Store {
    pub dir: Option<PathBuf>,
    pub state: State,
}

pub fn random_hex(bytes: usize) -> String {
    use std::io::Read;
    let mut b = vec![0u8; bytes];
    if std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut b))
        .is_err()
    {
        // extremely unlikely; fall back to time + pid mixing (still unique, not secret)
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        for (i, x) in b.iter_mut().enumerate() {
            *x = ((t >> ((i % 16) * 8)) as u8) ^ (std::process::id() as u8).wrapping_mul(i as u8 + 1);
        }
    }
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Constant-time comparison for the API key.
pub fn key_matches(expected: &str, given: &str) -> bool {
    let (a, b) = (expected.as_bytes(), given.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

impl Store {
    fn path(dir: &Path) -> PathBuf {
        dir.join("state.json")
    }

    /// Load (or create) the state. Without a usable directory the state lives in memory only.
    pub fn open(dir: Option<&str>) -> Result<Store, String> {
        let Some(d) = dir else {
            return Ok(Store {
                dir: None,
                state: State {
                    id: random_hex(6),
                    ..Default::default()
                },
            });
        };
        let dir = PathBuf::from(d);
        let p = Self::path(&dir);
        let mut st: State = match std::fs::read_to_string(&p) {
            Ok(s) => serde_json::from_str(&s).map_err(|e| format!("{}: {e}", p.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => State::default(),
            Err(e) => return Err(format!("{}: {e}", p.display())),
        };
        let mut store = Store {
            dir: Some(dir),
            state: State::default(),
        };
        if st.id.is_empty() {
            st.id = random_hex(6);
            store.state = st;
            if let Err(e) = store.save() {
                crate::log(&format!(
                    "warning: data directory {d} is not writable ({e}); pairing will not persist"
                ));
                store.dir = None;
            }
        } else {
            store.state = st;
        }
        Ok(store)
    }

    pub fn save(&self) -> Result<(), String> {
        let Some(dir) = &self.dir else {
            return Err("no data directory (mount a volume at /data)".into());
        };
        let p = Self::path(dir);
        let tmp = dir.join(".state.json.tmp");
        let body = serde_json::to_vec_pretty(&self.state).map_err(|e| e.to_string())?;
        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&tmp)
                .map_err(|e| format!("{}: {e}", tmp.display()))?;
            f.write_all(&body).map_err(|e| e.to_string())?;
            f.sync_all().ok();
        }
        std::fs::rename(&tmp, &p).map_err(|e| format!("{}: {e}", p.display()))
    }

    pub fn persistent(&self) -> bool {
        self.dir.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_save_reload_with_mode_600() {
        let d = std::env::temp_dir().join(format!("eink-store-{}", random_hex(4)));
        std::fs::create_dir_all(&d).unwrap();
        let ds = d.to_str().unwrap();
        let mut s = Store::open(Some(ds)).unwrap();
        assert_eq!(s.state.id.len(), 12);
        assert!(!s.state.paired());
        s.state.api_key = Some("k".into());
        s.state.settings.token = Some("t".into());
        s.save().unwrap();
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(d.join("state.json")).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let again = Store::open(Some(ds)).unwrap();
        assert_eq!(again.state.id, s.state.id);
        assert!(again.state.paired());
        assert_eq!(again.state.settings.token.as_deref(), Some("t"));
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn no_dir_is_memory_only() {
        let s = Store::open(None).unwrap();
        assert!(!s.persistent());
        assert!(s.save().is_err());
    }

    #[test]
    fn keys() {
        assert_eq!(random_hex(16).len(), 32);
        assert_ne!(random_hex(16), random_hex(16));
        assert!(key_matches("abc", "abc"));
        assert!(!key_matches("abc", "abd"));
        assert!(!key_matches("abc", "ab"));
    }
}
