//! Aria2 Next: the download engine the component hands byte-shuffling to.
//!
//! Why a second process: the component's job is talking to QQ Music's endpoints —
//! signing, cookies, rate limits, parsing. Moving bytes is a different job with a
//! mature specialist implementation (aria2-next: multi-connection, resume,
//! per-host concurrency, rate limits), and the app's own download path can then
//! be one line of "ask for this file" instead of a progress-reporting HTTP client.
//!
//! The binary travels next to this component (`<dir>/aria2-next`, shipped in the
//! same release package), so "install the component" installs its downloader.
//! It is started **on first use**, not at launch: a component that spawns a
//! daemon just because the app opened would be rude.
//!
//! The RPC is JSON-RPC 2.0 over HTTP on a loopback port, exactly as aria2 defines
//! it (`aria2.addUri`, `aria2.tellStatus`, …), with a per-session secret. Keeping
//! the wire format standard is what lets someone point AriaNg at the port to see
//! what is going on.

use crate::upstream::UpstreamError;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The port the component's own aria2 listens on.
///
/// Not 6800: that is aria2's default and therefore exactly the port a user's own
/// daemon already holds. This is a private instance.
const DEFAULT_PORT: u16 = 16_800;

/// aria2's own knobs that the settings page exposes, with the range each accepts
/// and what happens when the user types something silly.
#[derive(Debug, Clone, PartialEq)]
pub struct Aria2Options {
    /// `--split`: how many connections one file is cut into.
    pub split: u32,
    /// `--max-connection-per-server`.
    pub max_connection_per_server: u32,
    /// `--max-concurrent-downloads`.
    pub max_concurrent_downloads: u32,
    /// `--min-split-size` in MiB (aria2 wants bytes; the page shows MiB).
    pub min_split_size_mib: u32,
    /// `--max-overall-download-limit`, in KiB/s; 0 means no limit.
    pub max_overall_download_limit_kib: u32,
    /// The loopback port the RPC listens on. Not something a running aria2 can
    /// change — a listener cannot move — so a new port applies the next time the
    /// engine starts (重启引擎, or the next download once it has stopped).
    pub port: u16,
}

impl Default for Aria2Options {
    fn default() -> Self {
        Self {
            // aria2's own defaults, except the rate: a music player downloading a
            // handful of files should not saturate a connection by default.
            split: 5,
            max_connection_per_server: 5,
            max_concurrent_downloads: 1,
            min_split_size_mib: 1,
            max_overall_download_limit_kib: 0,
            port: DEFAULT_PORT,
        }
    }
}

impl Aria2Options {
    /// Clamp everything into what aria2 will accept.
    ///
    /// A zero where aria2 divides or multiplies is not just ignored — aria2
    /// rejects the whole option set — so the clamps are part of the contract, not
    /// politeness.
    pub fn sanitised(self) -> Self {
        Self {
            split: self.split.clamp(1, 16),
            max_connection_per_server: self.max_connection_per_server.clamp(1, 16),
            max_concurrent_downloads: self.max_concurrent_downloads.clamp(1, 10),
            min_split_size_mib: self.min_split_size_mib.clamp(1, 1024),
            max_overall_download_limit_kib: self.max_overall_download_limit_kib.min(1_048_576),
            // Below 1024 needs root and collides with system services; above is
            // the user's business.
            port: self.port.clamp(1024, 65_535),
        }
    }

    /// The `aria2.changeGlobalOption` payload (and the flags the process starts on).
    pub fn to_rpc(&self) -> Value {
        let options = self.clone().sanitised();
        json!({
            "split": options.split.to_string(),
            "max-connection-per-server": options.max_connection_per_server.to_string(),
            "max-concurrent-downloads": options.max_concurrent_downloads.to_string(),
            "min-split-size": format!("{}M", options.min_split_size_mib),
            "max-overall-download-limit": if options.max_overall_download_limit_kib == 0 {
                "0".to_string()
            } else {
                format!("{}K", options.max_overall_download_limit_kib)
            },
        })
    }

    /// The same values as command-line flags, for the process this component
    /// starts itself.
    fn to_flags(&self) -> Vec<String> {
        let options = self.clone().sanitised();
        vec![
            "--split".into(),
            options.split.to_string(),
            "--max-connection-per-server".into(),
            options.max_connection_per_server.to_string(),
            "--max-concurrent-downloads".into(),
            options.max_concurrent_downloads.to_string(),
            "--min-split-size".into(),
            format!("{}M", options.min_split_size_mib),
            "--max-overall-download-limit".into(),
            if options.max_overall_download_limit_kib == 0 {
                "0".into()
            } else {
                format!("{}K", options.max_overall_download_limit_kib)
            },
        ]
    }
}

#[derive(Debug)]
struct Running {
    child: Child,
    port: u16,
    secret: String,
    started: Instant,
}

#[derive(Debug, Default)]
pub struct Aria2 {
    directory: PathBuf,
    options: Mutex<Aria2Options>,
    running: Mutex<Option<Running>>,
}

impl Aria2 {
    pub fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            options: Mutex::new(Aria2Options::default()),
            running: Mutex::new(None),
        }
    }

    pub fn options(&self) -> Aria2Options {
        self.options.lock().expect("aria2 options").clone()
    }

    /// The binary that travels with this component.
    pub fn binary(&self) -> PathBuf {
        self.directory.join("aria2-next")
    }

    pub fn is_installed(&self) -> bool {
        self.binary().is_file()
    }

    /// Apply the user's numbers. They take effect immediately when the process is
    /// already up (through `aria2.changeGlobalOption`), so a change does not
    /// require a restart — but the ones that shape a *new* download (split) are
    /// also passed on the command line for the next start.
    pub fn configure(&self, options: Aria2Options) {
        let options = options.sanitised();
        *self.options.lock().expect("aria2 options") = options.clone();
        if let Some(running) = self.running.lock().expect("aria2").as_ref() {
            // The port is deliberately not pushed: a listening socket cannot move,
            // so the new value applies the next time the engine starts.
            let _ = self.call_on(
                running,
                "aria2.changeGlobalOption",
                json!([options.to_rpc()]),
            );
        }
    }

    /// Is the process up? Prunes a dead child rather than reporting it alive.
    pub fn is_running(&self) -> bool {
        let mut slot = self.running.lock().expect("aria2");
        match slot.as_mut() {
            Some(running) => match running.child.try_wait() {
                Ok(None) => true,
                _ => {
                    *slot = None;
                    false
                }
            },
            None => false,
        }
    }

    /// Start the daemon if it is not already up.
    pub fn ensure_running(&self) -> Result<(), UpstreamError> {
        if self.is_running() {
            return Ok(());
        }
        if !self.is_installed() {
            return Err(UpstreamError::Upstream(format!(
                "没有随组件携带的 aria2-next（期望在 {}）",
                self.binary().display()
            )));
        }
        let secret = session_secret();
        // The user's port when it is free, otherwise whichever one is: a leftover
        // instance (the component was killed, not stopped) must not stop this one
        // from starting, and the port reported back is the one in use.
        let preferred = self.options().port;
        let port = if std::net::TcpListener::bind(("127.0.0.1", preferred)).is_ok() {
            preferred
        } else {
            free_port(preferred).unwrap_or(preferred)
        };
        let download_dir = self.directory.join("Downloads");
        std::fs::create_dir_all(&download_dir)
            .map_err(|error| UpstreamError::Upstream(format!("创建下载目录失败：{error}")))?;

        let mut command = Command::new(self.binary());
        command
            .arg("--no-conf")
            .arg("--enable-rpc")
            .arg("--rpc-listen-all=false")
            .arg(format!("--rpc-listen-port={port}"))
            .arg(format!("--rpc-secret={secret}"))
            .arg(format!("--dir={}", download_dir.display()))
            // The app imports the file itself and decides what it is called, so
            // aria2 must not rename it out from under the import.
            .arg("--auto-file-renaming=false")
            .arg("--allow-overwrite=true")
            .arg("--continue=true")
            .arg("--file-allocation=none")
            // A CDN that refuses the app's HTTP client refuses this one too: the
            // referer is what makes the request look like the player's.
            .arg("--referer=https://y.qq.com/")
            .arg("--user-agent=QQMusic/1.0 (macOS)")
            .args(self.options().to_flags())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        let child = command
            .spawn()
            .map_err(|error| UpstreamError::Upstream(format!("启动 aria2-next 失败：{error}")))?;
        let running = Running {
            child,
            port,
            secret,
            started: Instant::now(),
        };
        {
            let mut slot = self.running.lock().expect("aria2");
            *slot = Some(running);
        }
        self.wait_until_ready()
    }

    /// Stop the daemon, leaving any partial downloads resumable.
    pub fn shutdown(&self) {
        if let Some(running) = self.running.lock().expect("aria2").take() {
            let _ = self.call_on(&running, "aria2.forceShutdown", json!([]));
            let mut child = running.child;
            let deadline = Instant::now() + Duration::from_secs(2);
            while Instant::now() < deadline {
                if matches!(child.try_wait(), Ok(Some(_))) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    pub fn restart(&self) -> Result<(), UpstreamError> {
        self.shutdown();
        self.ensure_running()
    }

    /// Poll the RPC until it answers or the deadline passes.
    fn wait_until_ready(&self) -> Result<(), UpstreamError> {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut last = String::new();
        while Instant::now() < deadline {
            match self.call("aria2.getVersion", json!([])) {
                Ok(_) => return Ok(()),
                Err(error) => last = error.to_string(),
            }
            std::thread::sleep(Duration::from_millis(120));
        }
        Err(UpstreamError::Upstream(format!(
            "aria2-next 启动后没有应答复 RPC：{last}"
        )))
    }

    /// A global summary: what is running, how fast, and how the options stand.
    pub fn status(&self) -> Value {
        let installed = self.is_installed();
        let running = self.is_running();
        let options = self.options();
        if !running {
            return json!({
                "installed": installed,
                "running": false,
                "binary": self.binary().display().to_string(),
                "options": options_to_json(&options),
            });
        }
        let stat = self.call("aria2.getGlobalStat", json!([])).ok();
        let version = self
            .call("aria2.getVersion", json!([]))
            .ok()
            .and_then(|value| value.get("version").cloned());
        let active = self
            .call("aria2.tellActive", json!([]))
            .ok()
            .and_then(|value| value.as_array().map(Vec::len))
            .unwrap_or(0);
        json!({
            "installed": installed,
            "running": true,
            "binary": self.binary().display().to_string(),
            "port": self.port(),
            "version": version,
            "active": active,
            "downloadSpeed": stat.as_ref().and_then(|s| s.get("downloadSpeed")).and_then(value_as_i64),
            "downloads": stat.as_ref().and_then(|s| s.get("numActive")).and_then(value_as_i64),
            "waiting": stat.as_ref().and_then(|s| s.get("numWaiting")).and_then(value_as_i64),
            "stopped": stat.as_ref().and_then(|s| s.get("numStopped")).and_then(value_as_i64),
            "options": options_to_json(&options),
        })
    }

    pub fn port(&self) -> u16 {
        self.running
            .lock()
            .expect("aria2")
            .as_ref()
            .map(|running| running.port)
            .unwrap_or(DEFAULT_PORT)
    }

    /// Queue one file. `out` is the file name inside the download directory,
    /// which is what makes the result importable under a name the app chose.
    pub fn add(&self, url: &str, out: &str) -> Result<String, UpstreamError> {
        self.ensure_running()?;
        let result = self.call(
            "aria2.addUri",
            json!([[url], { "out": out, "continue": "true" }]),
        )?;
        result
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| UpstreamError::Upstream("aria2 没有返回 gid".into()))
    }

    /// One download's state, in the shape the app polls.
    pub fn tell(&self, gid: &str) -> Result<Value, UpstreamError> {
        let status = self.call("aria2.tellStatus", json!([gid]))?;
        let get = |key: &str| status.get(key).cloned().unwrap_or(Value::Null);
        Ok(json!({
            "gid": get("gid"),
            "status": get("status"),
            "completed": get("completedLength").as_str().and_then(|value| value.parse::<i64>().ok()).unwrap_or(0),
            "total": get("totalLength").as_str().and_then(|value| value.parse::<i64>().ok()).unwrap_or(0),
            "speed": get("downloadSpeed").as_str().and_then(|value| value.parse::<i64>().ok()).unwrap_or(0),
            "path": get("files").get(0).and_then(|file| file.get("path")).cloned().unwrap_or(Value::Null),
            "error": get("errorMessage"),
            // aria2 spells it `errorCode`; the app shows it verbatim.
            "errorCode": get("errorCode"),
        }))
    }

    /// Every task the engine knows: active, waiting and recently stopped.
    ///
    /// The engine is the source of truth for the list the user sees — it knows
    /// about tasks this component did not queue (a restart, a second caller) and
    /// about the ones that stopped while nobody was looking.
    pub fn list(&self) -> Result<Vec<Value>, UpstreamError> {
        if !self.is_running() {
            return Ok(Vec::new());
        }
        let mut tasks = Vec::new();
        for method in ["aria2.tellActive", "aria2.tellWaiting", "aria2.tellStopped"] {
            let params = if method == "aria2.tellActive" {
                json!([])
            } else {
                // The two paging calls need (offset, num); a hundred is more than
                // a music queue ever holds.
                json!([0, 100])
            };
            let value = self.call(method, params)?;
            if let Some(items) = value.as_array() {
                tasks.extend(items.iter().cloned());
            }
        }
        Ok(tasks.iter().map(summarise_task).collect())
    }

    /// Pause one task, or all of them when `gid` is None.
    ///
    /// aria2 spells the two cases as different methods — `aria2.forcePause(gid)`
    /// against `aria2.forcePauseAll()` — and calling the single-task one with an
    /// empty argument list is an error, not "all".
    pub fn pause(&self, gid: Option<&str>) -> Result<(), UpstreamError> {
        match gid {
            Some(gid) => self.control("aria2.forcePause", json!([gid])),
            None => self.control("aria2.forcePauseAll", json!([])),
        }
    }

    /// Resume one task, or all of them.
    pub fn unpause(&self, gid: Option<&str>) -> Result<(), UpstreamError> {
        match gid {
            Some(gid) => self.control("aria2.unpause", json!([gid])),
            None => self.control("aria2.unpauseAll", json!([])),
        }
    }

    /// Cancel: stop the task **and delete the partial file**.
    ///
    /// Two steps, because aria2 keeps a task's record after it is removed: the
    /// file is deleted while the status still knows where it is, then the record
    /// is dropped so the list does not fill up with cancelled entries.
    pub fn cancel(&self, gid: Option<&str>) -> Result<Vec<String>, UpstreamError> {
        let targets: Vec<String> = match gid {
            Some(gid) => vec![gid.to_string()],
            None => self
                .list()?
                .iter()
                .filter_map(|task| text_of(task, "gid"))
                .collect(),
        };
        let mut removed = Vec::new();
        for target in targets {
            // Where the file is has to be read before the removal, and a failure
            // here must not stop the removal — a task with no readable path is
            // exactly the kind that needs cancelling.
            let path = self
                .call("aria2.tellStatus", json!([target]))
                .ok()
                .and_then(|status| {
                    status
                        .get("files")
                        .and_then(Value::as_array)
                        .and_then(|files| files.first())
                        .cloned()
                })
                .and_then(|file| text_of(&file, "path"));
            let _ = self.call("aria2.forceRemove", json!([target]));
            if let Some(path) = path {
                if !path.is_empty() {
                    let _ = std::fs::remove_file(&path);
                    // aria2's own control file, if it made one.
                    let _ = std::fs::remove_file(format!("{path}.aria2"));
                }
            }
            let _ = self.call("aria2.removeDownloadResult", json!([target]));
            removed.push(target);
        }
        Ok(removed)
    }

    fn control(&self, method: &str, params: Value) -> Result<(), UpstreamError> {
        self.ensure_running()?;
        self.call(method, params).map(|_| ())
    }

    /// One JSON-RPC call against the running process.
    fn call(&self, method: &str, params: Value) -> Result<Value, UpstreamError> {
        let slot = self.running.lock().expect("aria2");
        let running = slot
            .as_ref()
            .ok_or_else(|| UpstreamError::Upstream("aria2-next 没有在运行".into()))?;
        self.call_on(running, method, params)
    }

    fn call_on(
        &self,
        running: &Running,
        method: &str,
        params: Value,
    ) -> Result<Value, UpstreamError> {
        let mut params = params.as_array().cloned().unwrap_or_default();
        params.insert(0, json!(format!("token:{}", running.secret)));
        let body = json!({
            "jsonrpc": "2.0",
            "id": "qqmusic-helper-next",
            "method": method,
            "params": params,
        });
        let url = format!("http://127.0.0.1:{}/jsonrpc", running.port);
        let response = ureq::post(&url)
            .header("Content-Type", "application/json")
            .send_json(&body)
            .map_err(|error| UpstreamError::Transport(format!("aria2 RPC 失败：{error}")))?;
        let mut response = response;
        let value: Value = response.body_mut().read_json().map_err(|error| {
            UpstreamError::Transport(format!("aria2 RPC 返回不是 JSON：{error}"))
        })?;
        if let Some(error) = value.get("error") {
            // aria2 answers `{"error":{"code":1,"message":"..."}}` for a refused
            // call — most often a bad option, which is worth reading verbatim.
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("未知错误");
            return Err(UpstreamError::Upstream(format!(
                "aria2 拒绝了 {method}：{message}"
            )));
        }
        Ok(value.get("result").cloned().unwrap_or(Value::Null))
    }
}

/// One task, in the shape the app's list shows.
fn summarise_task(task: &Value) -> Value {
    let file = task
        .get("files")
        .and_then(Value::as_array)
        .and_then(|files| files.first())
        .cloned()
        .unwrap_or(json!({}));
    let name = text_of(&file, "path")
        .map(|path| {
            std::path::Path::new(&path)
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or(path)
        })
        .unwrap_or_default();
    json!({
        "gid": task.get("gid").cloned().unwrap_or(Value::Null),
        "status": task.get("status").cloned().unwrap_or(Value::Null),
        "completed": number_of(task, "completedLength"),
        "total": number_of(task, "totalLength"),
        "speed": number_of(task, "downloadSpeed"),
        "name": name,
        "path": text_of(&file, "path").unwrap_or_default(),
        "error": task.get("errorMessage").and_then(Value::as_str).unwrap_or(""),
    })
}

fn number_of(value: &Value, key: &str) -> i64 {
    value
        .get(key)
        .and_then(|inner| {
            inner
                .as_str()
                .and_then(|text| text.parse().ok())
                .or_else(|| inner.as_i64())
        })
        .unwrap_or(0)
}

fn text_of(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

/// A loopback port nothing is listening on, preferring the user's.
fn free_port(preferred: u16) -> Option<u16> {
    if std::net::TcpListener::bind(("127.0.0.1", preferred)).is_ok() {
        return Some(preferred);
    }
    // Any port will do; ask the OS for one.
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .ok()
        .and_then(|listener| listener.local_addr().ok())
        .map(|address| address.port())
}

/// A per-session secret.
///
/// The port is loopback-only, but an RPC that can write files anywhere the
/// process can reach should not be open to anything on the machine that happens
/// to guess the port — and the secret is regenerated on every start, so a leak
/// from a previous run is worthless.
fn session_secret() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let bytes: [u8; 16] = rng.gen();
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn options_to_json(options: &Aria2Options) -> Value {
    json!({
        "split": options.split,
        "maxConnectionPerServer": options.max_connection_per_server,
        "maxConcurrentDownloads": options.max_concurrent_downloads,
        "minSplitSizeMiB": options.min_split_size_mib,
        "maxOverallDownloadLimitKiB": options.max_overall_download_limit_kib,
        "port": options.port,
    })
}

fn value_as_i64(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
}

/// Parse the app's configure payload into options, defaulting what it omits.
pub fn options_from_params(params: &Value) -> Aria2Options {
    let defaults = Aria2Options::default();
    let int = |key: &str, fallback: u32| -> u32 {
        params
            .get(key)
            .and_then(|value| {
                value
                    .as_i64()
                    .or_else(|| value.as_str().and_then(|t| t.parse().ok()))
            })
            .map(|value| value.max(0) as u32)
            .unwrap_or(fallback)
    };
    Aria2Options {
        split: int("split", defaults.split),
        max_connection_per_server: int(
            "maxConnectionPerServer",
            defaults.max_connection_per_server,
        ),
        max_concurrent_downloads: int("maxConcurrentDownloads", defaults.max_concurrent_downloads),
        min_split_size_mib: int("minSplitSizeMiB", defaults.min_split_size_mib),
        max_overall_download_limit_kib: int(
            "maxOverallDownloadLimitKiB",
            defaults.max_overall_download_limit_kib,
        ),
        port: int("port", u32::from(defaults.port)) as u16,
    }
    .sanitised()
}

/// Where the component's aria2 lives, given the component's own directory.
pub fn binary_in(directory: &Path) -> PathBuf {
    directory.join("aria2-next")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_are_clamped_into_what_aria2_accepts() {
        let wild = Aria2Options {
            split: 0,
            max_connection_per_server: 999,
            max_concurrent_downloads: 0,
            min_split_size_mib: 0,
            max_overall_download_limit_kib: 999_999_999,
            port: 80,
        }
        .sanitised();
        assert_eq!(wild.split, 1);
        assert_eq!(wild.max_connection_per_server, 16);
        assert_eq!(wild.max_concurrent_downloads, 1);
        assert_eq!(wild.min_split_size_mib, 1);
        assert_eq!(wild.max_overall_download_limit_kib, 1_048_576);
        assert_eq!(
            wild.port, 1024,
            "a privileged port is not the user's to take"
        );
    }

    #[test]
    fn the_rpc_payload_uses_aria2s_own_spellings() {
        let options = Aria2Options {
            split: 4,
            max_connection_per_server: 2,
            max_concurrent_downloads: 3,
            min_split_size_mib: 2,
            max_overall_download_limit_kib: 0,
            port: 16_900,
        };
        let payload = options.to_rpc();
        assert_eq!(payload["split"], "4");
        assert_eq!(payload["min-split-size"], "2M");
        assert_eq!(
            payload["max-overall-download-limit"], "0",
            "0 means no limit to aria2"
        );
        assert_eq!(payload["max-concurrent-downloads"], "3");
    }

    #[test]
    fn a_partial_configure_keeps_the_other_defaults() {
        let options = options_from_params(&json!({ "split": 8 }));
        assert_eq!(options.split, 8);
        assert_eq!(
            options.max_concurrent_downloads,
            Aria2Options::default().max_concurrent_downloads
        );
    }

    #[test]
    fn a_task_is_summarised_with_the_name_it_will_have_on_disk() {
        let task = json!({
            "gid": "abc", "status": "active", "completedLength": "1024",
            "totalLength": "4096", "downloadSpeed": "512",
            "files": [{ "path": "/tmp/Downloads/0039MnYb0qxYhV-1a2b3c4d.flac" }]
        });
        let summary = summarise_task(&task);
        assert_eq!(summary["gid"], "abc");
        assert_eq!(summary["completed"], 1024);
        assert_eq!(summary["total"], 4096);
        assert_eq!(summary["name"], "0039MnYb0qxYhV-1a2b3c4d.flac");
    }

    #[test]
    fn a_task_with_no_file_yet_summarises_empty_rather_than_failing() {
        let summary = summarise_task(&json!({ "gid": "g", "status": "waiting" }));
        assert_eq!(summary["name"], "");
        assert_eq!(summary["total"], 0);
    }

    #[test]
    fn the_binary_sits_next_to_the_component() {
        assert_eq!(
            binary_in(Path::new("/tmp/x")).to_str(),
            Some("/tmp/x/aria2-next")
        );
    }
}
