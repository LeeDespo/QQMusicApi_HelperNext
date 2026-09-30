//! The device identity.
//!
//! Some upstream interfaces — search, the recommendation feed, the artist header
//! and every write — answer an empty (or refused) payload unless the request
//! carries a device identity. QQMusicApi obtains one by asking Tencent's QIMEI
//! service: it wraps a random AES key with RSA, encrypts a JSON device profile
//! with that key, signs both, and keeps the `q16`/`q36` pair it gets back.
//! That protocol is reproduced here, in Rust, so the component can be the only
//! thing that talks to the upstream.
//!
//! The device is generated once and kept next to the credential: the service
//! hands out an identity per device, and a fresh one on every launch would look
//! like a fleet of new devices rather than one user.

use base64::Engine;
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::{Path, PathBuf};

/// QIMEI service. It is not the music API: it hands out the device identity.
const QIMEI_URL: &str = "https://api.tencentmusic.com/tme/trpc/proxy";
/// The service's own public key (1024-bit), as QQMusicApi ships it.
/// The QIMEI service's public key, as DER.
///
/// DER rather than the PEM text the library stores: the same bytes, with no
/// PEM reader in the way (the `rsa` crate's PEM path rejected this key while
/// `openssl` and the base64 both accept it).
const PUBLIC_KEY_DER_HEX: &str = "30819f300d06092a864886f70d010101050003818d0030818902818100c4231830a2eb5fc2827170641e79d80fec51bda9a22e4b4ab37d1f205a4ae44d928cda25879f66a3429051663312a127faf8a246bdaaf63918417e90d7c95b5908aa6a2d0f852e4a6770294a548ac1c2fe8f1f252fb826f4ac86ab9a00e7ce47d002a56e7c4b51eb889acc60ca6adbc9f72e81f4d31b1dd7464805264530ab1d0203010001";

const SECRET: &str = "ZdJqM15EeO2zWc08";
const APP_KEY: &str = "0AND0HD6FE4HY80F";
const CHANNEL_ID: &str = "10003505";
const PACKAGE_ID: &str = "com.tencent.qqmusic";
/// Versions the QIMEI SDK reports. They are the android profile's own, from the
/// library's version policy — the service is not fussy about them, but they are
/// part of what makes the request look like the client it is imitating.
const QIMEI_APP_VERSION: &str = "14.9.0.8";
const QIMEI_SDK_VERSION: &str = "1.2.13.6";
const HEX: &[u8] = b"0123456789abcdef";
const IDENTITY_TTL_SECONDS: i64 = 86_400;

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn random_hex(len: usize) -> String {
    let mut rng = rand::thread_rng();
    (0..len)
        .map(|_| HEX[rng.gen_range(0..HEX.len())] as char)
        .collect()
}

/// A Luhn-valid 15-digit IMEI, the same kind of number the library makes up.
fn random_imei() -> String {
    let mut rng = rand::thread_rng();
    let mut digits: Vec<u32> = (0..14).map(|_| rng.gen_range(0..10)).collect();
    let mut sum = 0u32;
    for (index, digit) in digits.iter().enumerate() {
        let mut value = *digit;
        if index % 2 == 1 {
            value *= 2;
            if value > 9 {
                value -= 9;
            }
        }
        sum += value;
    }
    digits.push((10 - (sum % 10)) % 10);
    digits.iter().map(|digit| digit.to_string()).collect()
}

/// The device the component presents. Persisted, so the identity is stable.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    pub brand: String,
    pub model: String,
    pub android_id: String,
    pub imei: String,
    pub open_udid: String,
    pub os_release: String,
    pub os_sdk: String,
    pub proc_version: String,
    pub qimei: Option<String>,
    pub qimei36: Option<String>,
    pub qimei_at: Option<i64>,
}

impl Default for Device {
    fn default() -> Self {
        Self {
            brand: "Xiaomi".into(),
            model: "M2102J2SC".into(),
            android_id: random_hex(16),
            imei: random_imei(),
            open_udid: format!("{}{}", random_hex(16), random_hex(16)),
            os_release: "12".into(),
            os_sdk: "31".into(),
            proc_version: format!(
                "Linux 5.4.0-54-generic-{} (android-build@google.com)",
                random_hex(8)
            ),
            qimei: None,
            qimei36: None,
            qimei_at: None,
        }
    }
}

impl Device {
    pub fn os_version_text(&self) -> String {
        format!("Android {},level {}", self.os_release, self.os_sdk)
    }

    fn has_fresh_identity(&self) -> bool {
        matches!(
            (&self.qimei, &self.qimei36, self.qimei_at),
            (Some(_), Some(_), Some(at)) if now() - at < IDENTITY_TTL_SECONDS
        )
    }
}

pub struct DeviceStore {
    path: PathBuf,
}

impl DeviceStore {
    pub fn for_directory(directory: &Path) -> Self {
        Self {
            path: directory.join("device.json"),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load_or_create(&self) -> Device {
        if let Ok(data) = std::fs::read_to_string(&self.path) {
            if let Ok(device) = serde_json::from_str::<Device>(&data) {
                return device;
            }
        }
        let device = Device::default();
        self.save(&device);
        device
    }

    fn save(&self, device: &Device) {
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(data) = serde_json::to_vec_pretty(device) {
            let _ = std::fs::write(&self.path, data);
        }
    }

    /// The `q16`/`q36` pair, from the cache when it is still valid and from the
    /// service otherwise.
    ///
    /// Returns `None` rather than failing: a request without a device identity
    /// still works for every interface that does not demand one, so the caller
    /// sends what it has.
    pub fn qimei(&self, agent: &ureq::Agent) -> Option<(String, String)> {
        let mut device = self.load_or_create();
        if device.has_fresh_identity() {
            return device.qimei.clone().zip(device.qimei36.clone());
        }
        let (q16, q36) = fetch_qimei(agent, &device).ok()?;
        device.qimei = Some(q16.clone());
        device.qimei36 = Some(q36.clone());
        device.qimei_at = Some(now());
        self.save(&device);
        Some((q16, q36))
    }

    /// What the component last obtained, without making a request.
    pub fn cached_identity(&self) -> Option<(String, String)> {
        let device = self.load_or_create();
        if device.has_fresh_identity() {
            device.qimei.clone().zip(device.qimei36.clone())
        } else {
            None
        }
    }
}

/// Beacon id: the SDK's own device fingerprint string, forty `k<n>:<value>`
/// pairs. Its shape is reproduced from the library, which reproduces the SDK.
fn random_beacon_id() -> String {
    let mut rng = rand::thread_rng();
    let month = format!("{}-01", beacon_month());
    let rand1 = rng.gen_range(100_000..1_000_000);
    let rand2 = rng.gen_range(100_000_000..1_000_000_000);
    let mut beacon = String::new();
    for index in 1..=40 {
        let value = match index {
            _ if [1, 2, 13, 14, 17, 18, 21, 22, 25, 26, 29, 30, 33, 34, 37, 38].contains(&index) => {
                format!("{month}{rand1}.{rand2}")
            }
            3 => "0000000000000000".to_string(),
            4 => random_hex(16),
            _ => rng.gen_range(0..10_000).to_string(),
        };
        beacon.push_str(&format!("k{index}:{value};"));
    }
    beacon
}

/// The beacon id's month, as `YYYY-MM`. The service treats it as an opaque part
/// of the fingerprint, so the only requirement is that it looks like a month.
fn beacon_month() -> String {
    let days = now() / 86_400;
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    format!("{year}-{month:02}")
}

fn hex_bytes(text: &str) -> Vec<u8> {
    text.as_bytes()
        .chunks(2)
        .filter_map(|pair| std::str::from_utf8(pair).ok())
        .filter_map(|pair| u8::from_str_radix(pair, 16).ok())
        .collect()
}

/// MD5 over the concatenation of the parts, which is how the library signs.
fn md5_hex(parts: &[&str]) -> String {
    let joined: String = parts.concat();
    format!("{:x}", md5::compute(joined.as_bytes()))
}

fn aes_cbc_encrypt(key: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, String> {
    use aes::cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyIvInit};
    let encryptor = cbc::Encryptor::<aes::Aes128>::new(key.into(), key.into());
    Ok(encryptor.encrypt_padded_vec_mut::<Pkcs7>(plaintext))
}

fn rsa_encrypt(plaintext: &[u8]) -> Result<Vec<u8>, String> {
    use rsa::pkcs8::DecodePublicKey;
    use rsa::Pkcs1v15Encrypt;
    let der = hex_bytes(PUBLIC_KEY_DER_HEX);
    let key = rsa::RsaPublicKey::from_public_key_der(&der).map_err(|error| error.to_string())?;
    let mut rng = rand::thread_rng();
    key.encrypt(&mut rng, Pkcs1v15Encrypt, plaintext)
        .map_err(|error| error.to_string())
}

fn device_profile(device: &Device) -> serde_json::Value {
    let reserved = json!({
        "harmony": "0",
        "clone": "0",
        "containe": "",
        "oz": "UhYmelwouA+V2nPWbOvLTgN2/m8jwGB+yUB5v9tysQg=",
        "oo": "Xecjt+9S1+f8Pz2VLSxgpw==",
        "kelong": "0",
        "uptimes": "",
        "multiUser": "0",
        "bod": device.brand,
        "dv": device.brand,
        "firstLevel": "",
        "manufact": device.brand,
        "name": device.model,
        "host": "se.infra",
        "kernel": device.proc_version,
    });
    json!({
        "androidId": device.android_id,
        "platformId": 1,
        "appKey": APP_KEY,
        "appVersion": QIMEI_APP_VERSION,
        "beaconIdSrc": random_beacon_id(),
        "brand": device.brand,
        "channelId": CHANNEL_ID,
        "cid": "",
        "imei": device.imei,
        "imsi": "",
        "mac": "",
        "model": device.model,
        "networkType": "unknown",
        "oaid": "",
        "osVersion": device.os_version_text(),
        "qimei": "",
        "qimei36": "",
        "sdkVersion": QIMEI_SDK_VERSION,
        "targetSdkVersion": "33",
        "audit": "",
        "userId": "{}",
        "packageId": PACKAGE_ID,
        "deviceType": "Phone",
        "sdkName": "",
        "reserved": reserved.to_string(),
    })
}

/// Ask the QIMEI service for a device identity.
fn fetch_qimei(agent: &ureq::Agent, device: &Device) -> Result<(String, String), String> {
    let engine = base64::engine::general_purpose::STANDARD;
    let payload = device_profile(device);
    let crypt_key = random_hex(16);
    let nonce = random_hex(16);
    let timestamp = now();
    let key = engine.encode(rsa_encrypt(crypt_key.as_bytes())?);
    let params = engine.encode(
        aes_cbc_encrypt(crypt_key.as_bytes(), payload.to_string().as_bytes()).map_err(|error| error)?,
    );
    let extra = format!("{{\"appKey\":\"{APP_KEY}\"}}");
    let request_sign = md5_hex(&[
        &key,
        &params,
        &(timestamp * 1000).to_string(),
        &nonce,
        SECRET,
        &extra,
    ]);
    let header_sign = md5_hex(&[
        "qimei_qq_androidpzAuCmaFAaFaHrdakPjLIEqKrGnSOOvH",
        &timestamp.to_string(),
    ]);

    let body = json!({
        "app": 0,
        "os": 1,
        "qimeiParams": {
            "key": key,
            "params": params,
            "time": timestamp.to_string(),
            "nonce": nonce,
            "sign": request_sign,
            "extra": extra,
        },
    });

    let mut response = agent
        .post(QIMEI_URL)
        .header("Host", "api.tencentmusic.com")
        .header("method", "GetQimei")
        .header("service", "trpc.tme_datasvr.qimeiproxy.QimeiProxy")
        .header("appid", "qimei_qq_android")
        .header("user-agent", "QQMusic")
        .header("timestamp", timestamp.to_string())
        .header("sign", header_sign)
        .send_json(&body)
        .map_err(|error| error.to_string())?;
    let value: serde_json::Value = response
        .body_mut()
        .read_json()
        .map_err(|error| error.to_string())?;

    // The answer nests JSON inside JSON: `{"data": "{\"data\": {\"q16\":…}}"}`.
    let inner = value
        .get("data")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "QIMEI 响应缺少 data".to_string())?;
    let decoded: serde_json::Value =
        serde_json::from_str(inner).map_err(|error| error.to_string())?;
    let identity = decoded.get("data").unwrap_or(&decoded);
    let q16 = identity
        .get("q16")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "QIMEI 响应缺少 q16".to_string())?;
    let q36 = identity
        .get("q36")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "QIMEI 响应缺少 q36".to_string())?;
    Ok((q16.to_string(), q36.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_devices_are_plausible_and_persist() {
        let directory = std::env::temp_dir().join("qqmusic-helper-next-device-test");
        let _ = std::fs::remove_dir_all(&directory);
        let store = DeviceStore::for_directory(&directory);
        let first = store.load_or_create();
        assert_eq!(first.imei.len(), 15, "an IMEI is fifteen digits");
        assert!(first.imei.chars().all(|c| c.is_ascii_digit()));
        assert_eq!(first.android_id.len(), 16);
        let second = store.load_or_create();
        assert_eq!(first.android_id, second.android_id, "the device is kept");
        assert!(store.cached_identity().is_none(), "no identity before asking");
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn md5_matches_the_known_digest() {
        // The library concatenates its arguments before hashing.
        assert_eq!(md5_hex(&["abc"]), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(md5_hex(&["a", "bc"]), "900150983cd24fb0d6963f7d28e17f72");
    }

    #[test]
    fn aes_cbc_matches_a_known_vector() {
        // NIST SP 800-38A, AES-128-CBC, first block.
        let key = hex_bytes("2b7e151628aed2a6abf7158809cf4f3c");
        let iv = hex_bytes("000102030405060708090a0b0c0d0e0f");
        let plaintext = hex_bytes("6bc1bee22e409f96e93d7e117393172a");
        let encrypted = {
            use aes::cipher::{block_padding::NoPadding, BlockEncryptMut, KeyIvInit};
            let encryptor = cbc::Encryptor::<aes::Aes128>::new(
                key.as_slice().into(),
                iv.as_slice().into(),
            );
            encryptor.encrypt_padded_vec_mut::<NoPadding>(&plaintext)
        };
        assert_eq!(hex_string(&encrypted), "7649abac8119b246cee98e9b12e9197d");
    }

    fn hex_bytes(text: &str) -> Vec<u8> {
        text.as_bytes()
            .chunks(2)
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect()
    }

    fn hex_string(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}

#[cfg(test)]
mod live_tests {
    use super::*;

    /// The QIMEI handshake, against the real service.
    ///
    /// `cargo test -- --ignored --nocapture live_qimei` — it needs the network,
    /// and it prints why it failed rather than asserting, because the failure
    /// text is the thing worth reading.
    #[test]
    #[ignore]
    fn live_qimei_handshake() {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(std::time::Duration::from_secs(20)))
            .build()
            .into();
        let device = Device::default();
        match fetch_qimei(&agent, &device) {
            Ok((q16, q36)) => println!("qimei ok: q16={q16} q36={}", &q36[..q36.len().min(40)]),
            Err(error) => println!("qimei failed: {error}"),
        }
    }
}
