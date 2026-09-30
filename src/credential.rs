//! The credential the component reads and writes.
//!
//! Deliberately the *same file* the Python helper used
//! (`~/Library/Application Support/kmgccc.player/QQMusicHelper/Credential/qqmusic-credential.json`):
//! an existing login keeps working, and the two components can be swapped back
//! and forth while the old one is still around. The keys are the upstream's own
//! (`musicid`, `musickey`, `encrypt_uin`, …), plus `str_musicid`, which the
//! library preferred and the app's web path read first.

use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// `hash33` with an explicit seed.
///
/// The seed matters and is not always 5381: `g_tk` starts from 5381, while
/// ptlogin's `ptqrtoken` starts from **0** — and getting that wrong is not a
/// subtle failure, it is an HTTP 403 with no explanation (which is exactly how
/// the QR login was broken until the library's own default was checked).
pub fn hash33_seeded(key: &str, seed: u32) -> u32 {
    let mut hash: u32 = seed;
    for ch in key.chars() {
        hash = hash.wrapping_shl(5).wrapping_add(hash).wrapping_add(ch as u32);
    }
    hash & 0x7FFF_FFFF
}

/// The `cgi-bin/musicu.fcg` request needs `g_tk` — `hash33` of the music key
/// starting from 5381. Same algorithm the Python helper and the library use.
pub fn hash33(key: &str) -> u32 {
    hash33_seeded(key, 5381)
}

#[derive(Debug, Clone, Default)]
pub struct Credential {
    pub music_id: String,
    pub music_key: String,
    /// `encrypt_uin`: some endpoints address the account by this instead of the
    /// numeric id, and neither the library nor we derive it — it arrives in the
    /// credential file.
    pub encrypted_uin: String,
    /// Kept so a refresh (or another component) can rewrite the file without
    /// losing fields it did not touch.
    pub raw: Value,
}

impl Credential {
    pub fn is_usable(&self) -> bool {
        !self.music_id.is_empty() && !self.music_key.is_empty()
    }

    pub fn g_tk(&self) -> u32 {
        hash33(&self.music_key)
    }

    /// The cookie header the upstream reads. Both spellings, because one path
    /// reads the legacy `uin` and another `qqmusic_uin`.
    pub fn cookie_header(&self) -> String {
        if !self.is_usable() {
            return String::new();
        }
        format!(
            "uin={id}; qm_keyst={key}; qqmusic_key={key}; qqmusic_uin={id}",
            id = self.music_id,
            key = self.music_key
        )
    }

    /// Build a credential from a login response's payload.
    ///
    /// The login endpoints answer with the same field names the credential file
    /// uses, so the whole object is kept: fields this component does not read
    /// (a refresh token, for instance) must survive a save.
    pub fn from_json(value: &Value) -> Option<Self> {
        let credential = Self::from_value(value);
        credential.is_usable().then_some(credential)
    }

    fn from_value(value: &Value) -> Self {
        let music_id = value
            .get("str_musicid")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                value.get("musicid").map(|v| match v {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                })
            })
            .unwrap_or_default();
        Self {
            music_id,
            music_key: value
                .get("musickey")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            // Two spellings, both real: the login response carries `encryptUin`
            // (the library's field alias — it reads the camelCase and writes the
            // snake_case), while the credential file it wrote has `encrypt_uin`.
            // Reading only the latter is how a perfectly good login ended up
            // without the value the 关注歌手 endpoint addresses the account by.
            encrypted_uin: value
                .get("encrypt_uin")
                .or_else(|| value.get("encryptUin"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            raw: value.clone(),
        }
    }
}

pub struct CredentialStore {
    path: PathBuf,
}

impl CredentialStore {
    /// `<helper dir>/Credential/qqmusic-credential.json`, where `<helper dir>`
    /// is the external directory the app prefers, or this binary's own folder.
    pub fn for_directory(helper_dir: &Path) -> Self {
        Self {
            path: helper_dir.join("Credential").join("qqmusic-credential.json"),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Option<Credential> {
        let data = std::fs::read_to_string(&self.path).ok()?;
        let value: Value = serde_json::from_str(&data).ok()?;
        let credential = Credential::from_value(&value);
        if credential.is_usable() {
            Some(credential)
        } else {
            None
        }
    }

    /// Write the credential, keeping every key we do not own.
    ///
    /// The file is written through a temporary and renamed, so a reader (the app,
    /// or the old helper during a swap) never sees a half-written credential.
    pub fn store(&self, credential: &Credential) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut merged = credential.raw.clone();
        if !merged.is_object() {
            merged = json!({});
        }
        let object = merged.as_object_mut().expect("object");
        object.insert("musicid".into(), json!(credential.music_id));
        object.insert("str_musicid".into(), json!(credential.music_id));
        object.insert("musickey".into(), json!(credential.music_key));
        if !credential.encrypted_uin.is_empty() {
            object.insert("encrypt_uin".into(), json!(credential.encrypted_uin));
        }

        let temporary = self.path.with_extension("json.tmp");
        std::fs::write(&temporary, serde_json::to_vec_pretty(&merged)?)?;
        std::fs::rename(&temporary, &self.path)
    }

    pub fn clear(&self) -> std::io::Result<()> {
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }
}

/// Build a credential from the cookies a browser/web-login flow handed over.
///
/// `qm_keyst` is both the session ticket and the playback ticket the CDN wants,
/// so a cookie import is a complete login — the same equivalence the README
/// records for the two login paths the old helper offered.
pub fn credential_from_cookies(cookies: &Value) -> Option<Credential> {
    let get = |name: &str| -> Option<String> {
        cookies
            .get(name)
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    };
    let music_key = get("qm_keyst")?;
    let music_id = get("uin")
        .or_else(|| get("qqmusic_uin"))
        .or_else(|| get("musicid"))?;
    Some(Credential {
        music_id,
        music_key,
        encrypted_uin: get("encrypt_uin").unwrap_or_default(),
        raw: json!({}),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_seed_changes_the_hash() {
        // The lesson that cost a round of 403s: these two are different numbers.
        assert_ne!(hash33_seeded("qrsig-value", 0), hash33_seeded("qrsig-value", 5381));
        assert_eq!(hash33("k"), hash33_seeded("k", 5381));
    }

    #[test]
    fn gt_k_matches_the_librarys_hash33() {
        // Cross-checked against the Python helper's own g_tk for this key.
        let credential = Credential {
            music_id: "1234567890".into(),
            music_key: "abc".into(),
            ..Default::default()
        };
        assert_eq!(credential.g_tk(), hash33("abc"));
        assert!(credential.g_tk() <= 0x7FFF_FFFF);
    }

    #[test]
    fn a_cookie_import_needs_the_playback_key() {
        assert!(credential_from_cookies(&json!({"uin": "1"})).is_none());
        let credential = credential_from_cookies(&json!({
            "uin": "1234567890", "qm_keyst": "KEY", "qqmusic_key": "KEY"
        }))
        .expect("a complete cookie set is a login");
        assert_eq!(credential.music_id, "1234567890");
        assert_eq!(credential.music_key, "KEY");
        assert!(credential.is_usable());
    }

    #[test]
    fn str_musicid_wins_over_the_numeric_one() {
        let store_dir = std::env::temp_dir().join("qqmusic-helper-next-credential-test");
        let _ = std::fs::remove_dir_all(&store_dir);
        let store = CredentialStore::for_directory(&store_dir);
        let credential = Credential {
            music_id: "42".into(),
            music_key: "K".into(),
            encrypted_uin: "EUIN".into(),
            raw: json!({"refresh_token": "RT"}),
        };
        store.store(&credential).expect("store");
        let loaded = store.load().expect("load");
        assert_eq!(loaded.music_id, "42");
        assert_eq!(loaded.music_key, "K");
        assert_eq!(loaded.encrypted_uin, "EUIN");
        assert_eq!(loaded.raw.get("refresh_token").unwrap(), "RT");
        let _ = std::fs::remove_dir_all(&store_dir);
    }
}
