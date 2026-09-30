//! The line-based JSON adapter.
//!
//! One request per line on stdin, one reply per line on stdout; a request is
//! `{"id", "method", "params"}` and the reply carries the same `id` back. This is
//! the protocol an app uses when it drives the component as a child process —
//! the macOS player does exactly that — and it is also the easiest way to probe
//! the component by hand:
//!
//! ```text
//! $ echo '{"id":"1","method":"get_helper_info","params":{}}' | qqmusic-helper-next
//! {"id":"1","ok":true,"helper":{"helperVersion":"0.1.0",…}}
//! ```
//!
//! stdout is reserved for protocol JSON; diagnostics go to stderr. The
//! credential directory comes from `QQMUSIC_HELPER_NEXT_DIR`, or the host's
//! `configure()` call when the component is embedded rather than spawned.

use qqmusic_api_helper_next::{configure, methods, Configuration, CredentialStore, Upstream};
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::sync::Arc;

fn log(message: &str) {
    eprintln!("[qqmusic-helper-next] {message}");
}

/// Where the credential lives when this adapter is spawned as a process.
///
/// `QQMUSIC_HELPER_NEXT_DIR` is the explicit override (tests, and a host that
/// wants to point at its own directory); otherwise the macOS
/// application-support folder this build is deployed into. An Android host
/// configures the library directly instead of spawning it.
fn credential_directory() -> std::path::PathBuf {
    if let Ok(explicit) = std::env::var("QQMUSIC_HELPER_NEXT_DIR") {
        if !explicit.is_empty() {
            return std::path::PathBuf::from(explicit);
        }
    }
    let helper_dir = std::env::var("QQMUSIC_HELPER_DIR").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        format!("{home}/Library/Application Support/kmgccc.player/QQMusicHelperNext")
    });
    std::path::PathBuf::from(helper_dir)
}

fn main() {
    let data_dir = credential_directory();
    configure(Configuration {
        data_dir: data_dir.to_string_lossy().to_string(),
    });
    log(&format!(
        "version={} protocol={} data={}",
        methods::COMPONENT_VERSION,
        methods::PROTOCOL_VERSION,
        data_dir.display()
    ));

    let upstream = Arc::new(Upstream::new());
    let stdin = std::io::stdin();
    let mut workers = Vec::new();

    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let request: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(error) => {
                emit(json!({ "ok": false, "error": format!("请求不是合法 JSON：{error}") }));
                continue;
            }
        };

        // One thread per request: a host issues several reads at once and they
        // must not queue behind each other. The rate limiter and the breaker are
        // shared, so concurrency is bounded at the upstream, not here.
        let upstream = Arc::clone(&upstream);
        workers.retain(|worker: &std::thread::JoinHandle<()>| !worker.is_finished());
        workers.push(std::thread::spawn(move || emit(serve(&upstream, &request))));
    }

    for worker in workers {
        let _ = worker.join();
    }
}

fn serve(upstream: &Upstream, request: &Value) -> Value {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let method = request.get("method").and_then(Value::as_str).unwrap_or("");
    let params = request.get("params").cloned().unwrap_or(json!({}));

    // Credential work is the library's, and it never travels through a reply:
    // a credential in a log or a JSON line is a credential leaked.
    if method == "import_cookies" {
        return match methods::credential_from_params(&params) {
            Ok(credential) => match CredentialStore::for_directory(&credential_directory())
            .store(&credential)
            {
                Ok(()) => with_id(id, json!({ "login": { "loggedIn": true } })),
                Err(error) => with_id(id, json!({ "ok": false, "error": format!("写入凭据失败：{error}") })),
            },
            Err(error) => with_id(id, json!({ "ok": false, "error": error.to_string() })),
        };
    }
    if method == "logout" {
        let _ =
            CredentialStore::for_directory(&credential_directory())
            .clear();
        return with_id(id, json!({ "login": { "loggedIn": false } }));
    }

    if method.is_empty() || !methods::is_known(method) {
        return with_id(
            id,
            json!({ "ok": false, "error": format!("不支持的方法：{method}") }),
        );
    }

    let credential = CredentialStore::for_directory(&credential_directory())
    .load();
    let started = std::time::Instant::now();
    let result = methods::dispatch(upstream, credential.as_ref(), method, &params);
    log(&format!(
        "method={method} durationMs={}",
        started.elapsed().as_millis()
    ));
    match result {
        Ok(value) => with_id(id, value),
        Err(error) => with_id(id, json!({ "ok": false, "error": error.to_string() })),
    }
}

/// Attach `ok: true` and the request id.
///
/// The id is not decoration: a host matches a reply to its request by it, so a
/// reply without one is waited on until the host's timeout. (The Python helper
/// this replaces shipped one method without it, and the symptom was a
/// fifteen-second hang with nothing in the logs.)
fn with_id(id: Value, mut value: Value) -> Value {
    if let Some(object) = value.as_object_mut() {
        object.insert("id".into(), id);
        object.entry("ok").or_insert(json!(true));
        value
    } else {
        json!({ "id": id, "ok": true, "value": value })
    }
}

/// Serialize one reply to stdout under a lock, so two threads cannot interleave
/// within a line.
fn emit(value: Value) {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK.lock().expect("stdout");
    let line = serde_json::to_string(&value).unwrap_or_else(|_| "{\"ok\":false}".into());
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    let _ = writeln!(handle, "{line}");
    let _ = handle.flush();
}
