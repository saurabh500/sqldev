//! Telemetry recording (opt-in, anonymous, local-only stub).
//!
//! Privacy contract:
//! * Telemetry is **off by default**. The user must run
//!   `sqldev telemetry enable` to opt in. There is no install-time prompt.
//! * `SQLDEV_TELEMETRY=0` and `DO_NOT_TRACK=1` (industry-standard
//!   <https://consoledonottrack.com>) both force telemetry off, regardless
//!   of stored consent.
//! * Recorded payload contains only:
//!     - command name (top-level subcommand, e.g. `query`, never the SQL),
//!     - exit status (success / failure),
//!     - duration bucket (`<100ms`, `<1s`, `<10s`, `<60s`, `>=60s`),
//!     - sqldev version, OS family, anonymous client UUID,
//!     - UTC timestamp.
//!
//!   No SQL text, no schema names, no connection strings, no user paths.
//! * Until an upload endpoint is selected (issue #12 leaves it TBD),
//!   events are appended to a local NDJSON queue and never leave the
//!   machine. The queue file is the same shape that a future uploader
//!   would consume, so wiring transport later is a one-file change.
//! * Crash reports follow the same rules: backtrace + version + OS only.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Persisted user consent state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Consent {
    /// Default — user has neither opted in nor out. Treat as disabled.
    Unset,
    /// User explicitly enabled telemetry.
    Enabled,
    /// User explicitly disabled telemetry.
    Disabled,
}

impl Consent {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unset => "unset",
            Self::Enabled => "enabled",
            Self::Disabled => "disabled",
        }
    }
}

/// Persisted file at `<config>/sqldev/telemetry.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct TelemetryState {
    consent: Consent,
    /// Stable anonymous identifier minted on first state-file write.
    /// Generated from system time + pid; not derived from anything
    /// user-identifying.
    client_id: String,
}

impl Default for TelemetryState {
    fn default() -> Self {
        Self {
            consent: Consent::Unset,
            client_id: random_client_id(),
        }
    }
}

fn random_client_id() -> String {
    // Lightweight uuidv4-shaped id without pulling in the `uuid` crate.
    // This is purely an opaque identifier so we don't need
    // cryptographic strength.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0u128, |d| d.as_nanos());
    let pid = u128::from(std::process::id());
    let mixed = nanos
        .wrapping_mul(0x9E37_79B9_7F4A_7C15_9E37_79B9_7F4A_7C15)
        .wrapping_add(pid);
    let mut bytes = mixed.to_le_bytes();
    bytes[6] = (bytes[6] & 0x0F) | 0x40;
    bytes[8] = (bytes[8] & 0x3F) | 0x80;
    // Canonical UUID string: 8-4-4-4-12, big-endian within each group so
    // the version nibble (bytes[6] high) lands at character 14.
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15],
    )
}

/// Snapshot of every input the telemetry recorder needs. Extracting
/// these out of process-globals keeps the unit tests free of env
/// mutation.
pub struct Recorder {
    dir: PathBuf,
    env_disabled: bool,
}

impl Recorder {
    /// Builds a recorder from the current environment. Returns `None`
    /// if no config directory can be located (e.g. headless CI without
    /// `$HOME`); telemetry then becomes a no-op.
    pub fn from_env() -> Option<Self> {
        let dir = if let Ok(p) = std::env::var("SQLDEV_CONFIG_DIR") {
            PathBuf::from(p)
        } else {
            dirs::config_dir()?.join("sqldev")
        };
        Some(Self {
            dir,
            env_disabled: env_disables_telemetry(),
        })
    }

    /// Test-only constructor that pins both the config directory and
    /// whether the env-var override is in effect, with no process-env
    /// mutation.
    #[cfg(test)]
    fn for_test(dir: PathBuf, env_disabled: bool) -> Self {
        Self { dir, env_disabled }
    }

    fn state_path(&self) -> PathBuf {
        self.dir.join("telemetry.json")
    }

    fn queue_path(&self) -> PathBuf {
        self.dir.join("telemetry-queue.ndjson")
    }

    fn crash_path(&self) -> PathBuf {
        self.dir.join("crash-reports.ndjson")
    }

    fn load_state(&self) -> TelemetryState {
        let p = self.state_path();
        let Ok(bytes) = fs::read(&p) else {
            return TelemetryState::default();
        };
        serde_json::from_slice::<TelemetryState>(&bytes).unwrap_or_default()
    }

    fn save_state(&self, state: &TelemetryState) -> Result<()> {
        let p = self.state_path();
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        }
        let json = serde_json::to_vec_pretty(state).context("serialize telemetry state")?;
        fs::write(&p, json).with_context(|| format!("write {}", p.display()))?;
        Ok(())
    }

    pub fn effective_consent(&self) -> Consent {
        if self.env_disabled {
            return Consent::Disabled;
        }
        self.load_state().consent
    }

    pub fn set_consent(&self, consent: Consent) -> Result<Consent> {
        let mut state = self.load_state();
        state.consent = consent;
        self.save_state(&state)?;
        Ok(consent)
    }

    pub fn status_report(&self) -> String {
        let stored = self.load_state().consent;
        let effective = if self.env_disabled {
            Consent::Disabled
        } else {
            stored
        };
        let env = if self.env_disabled {
            "disabled (SQLDEV_TELEMETRY/DO_NOT_TRACK)"
        } else {
            "none"
        };
        let mut s = String::new();
        s.push_str("stored consent : ");
        s.push_str(stored.as_str());
        s.push('\n');
        s.push_str("env override   : ");
        s.push_str(env);
        s.push('\n');
        s.push_str("effective state: ");
        s.push_str(effective.as_str());
        s.push('\n');
        s.push_str("state file     : ");
        s.push_str(&self.state_path().display().to_string());
        s.push('\n');
        s.push_str("event queue    : ");
        s.push_str(&self.queue_path().display().to_string());
        s.push('\n');
        s
    }

    pub fn record_command(&self, command: &str, exit_code: i32, duration: Duration) {
        if self.effective_consent() != Consent::Enabled {
            return;
        }
        let state = self.load_state();
        let event = CommandEvent {
            kind: "command",
            client_id: state.client_id,
            sqldev_version: env!("CARGO_PKG_VERSION"),
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
            command: command.to_string(),
            exit_code,
            duration_bucket: bucket_duration(duration),
            ts_unix: now_unix_seconds(),
        };
        if let Ok(s) = serde_json::to_string(&event) {
            let _ = append_event(&self.queue_path(), &s);
        }
    }

    pub fn record_crash(&self, message: &str, backtrace: &str) {
        if self.effective_consent() != Consent::Enabled {
            return;
        }
        let state = self.load_state();
        let event = CrashEvent {
            kind: "crash",
            client_id: state.client_id,
            sqldev_version: env!("CARGO_PKG_VERSION"),
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
            message: message.to_string(),
            backtrace: backtrace.to_string(),
            ts_unix: now_unix_seconds(),
        };
        if let Ok(s) = serde_json::to_string(&event) {
            let _ = append_event(&self.crash_path(), &s);
        }
    }
}

fn env_disables_telemetry() -> bool {
    matches!(
        std::env::var("SQLDEV_TELEMETRY").as_deref(),
        Ok("0" | "false" | "off" | "no")
    ) || matches!(
        std::env::var("DO_NOT_TRACK").as_deref(),
        Ok("1" | "true" | "on" | "yes")
    )
}

fn append_event(path: &Path, line: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(f, "{line}")?;
    Ok(())
}

fn now_unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Coarse buckets so individual run durations aren't fingerprintable.
#[must_use]
pub fn bucket_duration(d: Duration) -> &'static str {
    let ms = d.as_millis();
    if ms < 100 {
        "<100ms"
    } else if ms < 1_000 {
        "<1s"
    } else if ms < 10_000 {
        "<10s"
    } else if ms < 60_000 {
        "<60s"
    } else {
        ">=60s"
    }
}

#[derive(Serialize)]
struct CommandEvent {
    kind: &'static str,
    client_id: String,
    sqldev_version: &'static str,
    os: &'static str,
    arch: &'static str,
    command: String,
    exit_code: i32,
    duration_bucket: &'static str,
    ts_unix: u64,
}

#[derive(Serialize)]
struct CrashEvent {
    kind: &'static str,
    client_id: String,
    sqldev_version: &'static str,
    os: &'static str,
    arch: &'static str,
    message: String,
    backtrace: String,
    ts_unix: u64,
}

/// Installs the global panic hook that funnels into the recorder.
pub fn install_panic_hook() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let msg = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "panic".to_string());
        let bt = std::backtrace::Backtrace::force_capture().to_string();
        if let Some(rec) = Recorder::from_env() {
            rec.record_crash(&msg, &bt);
        }
        prev(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bucket_duration_boundaries() {
        assert_eq!(bucket_duration(Duration::from_millis(0)), "<100ms");
        assert_eq!(bucket_duration(Duration::from_millis(99)), "<100ms");
        assert_eq!(bucket_duration(Duration::from_millis(100)), "<1s");
        assert_eq!(bucket_duration(Duration::from_millis(999)), "<1s");
        assert_eq!(bucket_duration(Duration::from_secs(1)), "<10s");
        assert_eq!(bucket_duration(Duration::from_millis(9_999)), "<10s");
        assert_eq!(bucket_duration(Duration::from_secs(10)), "<60s");
        assert_eq!(bucket_duration(Duration::from_secs(59)), "<60s");
        assert_eq!(bucket_duration(Duration::from_secs(60)), ">=60s");
        assert_eq!(bucket_duration(Duration::from_secs(3600)), ">=60s");
    }

    #[test]
    fn random_client_id_is_uuid_shaped() {
        let id = random_client_id();
        assert_eq!(id.len(), 36);
        let parts: Vec<&str> = id.split('-').collect();
        assert_eq!(parts.len(), 5);
        assert_eq!(parts[0].len(), 8);
        assert_eq!(parts[1].len(), 4);
        assert_eq!(parts[2].len(), 4);
        assert_eq!(parts[3].len(), 4);
        assert_eq!(parts[4].len(), 12);
        assert_eq!(&parts[2][..1], "4");
        let v = parts[3].chars().next().unwrap();
        assert!(matches!(v, '8' | '9' | 'a' | 'b'));
    }

    #[test]
    fn consent_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let rec = Recorder::for_test(dir.path().to_path_buf(), false);

        assert_eq!(rec.effective_consent(), Consent::Unset);
        rec.set_consent(Consent::Enabled).unwrap();
        assert_eq!(rec.effective_consent(), Consent::Enabled);
        rec.set_consent(Consent::Disabled).unwrap();
        assert_eq!(rec.effective_consent(), Consent::Disabled);
    }

    #[test]
    fn env_override_forces_disabled_even_when_stored_enabled() {
        let dir = tempfile::tempdir().unwrap();
        let rec = Recorder::for_test(dir.path().to_path_buf(), false);
        rec.set_consent(Consent::Enabled).unwrap();
        let blocked = Recorder::for_test(dir.path().to_path_buf(), true);
        assert_eq!(blocked.effective_consent(), Consent::Disabled);
    }

    #[test]
    fn record_command_writes_only_when_enabled() {
        let dir = tempfile::tempdir().unwrap();
        let rec = Recorder::for_test(dir.path().to_path_buf(), false);

        rec.record_command("introspect", 0, Duration::from_millis(50));
        let q = dir.path().join("telemetry-queue.ndjson");
        assert!(!q.exists());

        rec.set_consent(Consent::Enabled).unwrap();
        rec.record_command("introspect", 0, Duration::from_millis(50));
        let body = std::fs::read_to_string(&q).unwrap();
        assert!(body.contains("\"command\":\"introspect\""));
        assert!(body.contains("\"duration_bucket\":\"<100ms\""));
        // Sanity: no SQL text leaks.
        assert!(!body.to_lowercase().contains("select"));

        let blocked = Recorder::for_test(dir.path().to_path_buf(), true);
        let before_len = std::fs::metadata(&q).unwrap().len();
        blocked.record_command("introspect", 0, Duration::from_millis(50));
        let after_len = std::fs::metadata(&q).unwrap().len();
        assert_eq!(before_len, after_len);
    }

    #[test]
    fn status_report_shows_state() {
        let dir = tempfile::tempdir().unwrap();
        let rec = Recorder::for_test(dir.path().to_path_buf(), false);
        rec.set_consent(Consent::Enabled).unwrap();
        let report = rec.status_report();
        assert!(report.contains("stored consent : enabled"));
        assert!(report.contains("env override   : none"));
        assert!(report.contains("effective state: enabled"));

        let blocked = Recorder::for_test(dir.path().to_path_buf(), true);
        let report = blocked.status_report();
        assert!(report.contains("env override   : disabled"));
        assert!(report.contains("effective state: disabled"));
    }
}
