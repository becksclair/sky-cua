use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};
use std::os::unix::fs::FileTypeExt;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex as StdMutex};
use std::time::{Duration, Instant, SystemTime};

use sky_cua_platform::BROWSER_ENV_HEALTH_KEYS;
use sky_cua_platform::model::DiagnosticEntry;

use super::diagnostics::browser_bridge_disconnected_diagnostic;

pub(super) use sky_cua_platform::config::BROWSER_USE_SOCKET_DIR_ENV as SKY_CUA_SOCKET_DIR_ENV;
pub(super) const CODEX_SOCKET_DIR_ENV: &str = "CODEX_BROWSER_USE_SOCKET_DIR";
pub(super) use sky_cua_platform::config::BROWSER_SELECTION_ENV as SKY_CUA_BROWSER_ENV;
const DEFAULT_SOCKET_DIR: &str = "/tmp/codex-browser-use";
pub(super) const MAX_BRIDGE_SOCKET_CANDIDATES: usize = 32;
const SOCKET_FAMILY_CACHE_TTL: Duration = Duration::from_secs(10);
const STALE_SOCKET_FAILURE_TTL: Duration = Duration::from_secs(30);

static BRIDGE_SOCKET_INVENTORY: LazyLock<StdMutex<BridgeSocketInventory>> =
    LazyLock::new(|| StdMutex::new(BridgeSocketInventory::default()));

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BrowserSocketSelection {
    All,
    Browser(BrowserFamily),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BrowserFamily {
    Brave,
    Chrome,
    Chromium,
}

impl BrowserFamily {
    fn label(self) -> &'static str {
        match self {
            Self::Brave => "brave",
            Self::Chrome => "chrome",
            Self::Chromium => "chromium",
        }
    }
}

#[derive(Debug, Clone)]
struct SocketCandidate {
    path: PathBuf,
    modified: SystemTime,
}

#[derive(Debug, Default)]
struct BridgeSocketInventory {
    families: HashMap<PathBuf, CachedSocketFamily>,
    recent_failures: HashMap<PathBuf, Instant>,
}

#[derive(Debug, Clone)]
struct CachedSocketFamily {
    family: Option<BrowserFamily>,
    modified: SystemTime,
    checked_at: Instant,
}

impl BridgeSocketInventory {
    fn find_bridge_sockets(
        &mut self,
        dirs: Vec<PathBuf>,
        selection: BrowserSocketSelection,
        now: Instant,
    ) -> Vec<PathBuf> {
        self.prune(now);
        let mut candidates = Vec::new();
        for dir in dirs {
            self.collect_sockets_in_dir(&dir, selection, now, &mut candidates);
        }
        candidates.sort_by(compare_socket_candidates);
        candidates.truncate(MAX_BRIDGE_SOCKET_CANDIDATES);
        candidates
            .into_iter()
            .map(|candidate| candidate.path)
            .collect()
    }

    fn collect_sockets_in_dir(
        &mut self,
        dir: &Path,
        selection: BrowserSocketSelection,
        now: Instant,
        candidates: &mut Vec<SocketCandidate>,
    ) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if !file_name.starts_with("extension-") || !file_name.ends_with(".sock") {
                continue;
            }
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if !metadata.file_type().is_socket() {
                continue;
            }
            let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let candidate = SocketCandidate { path, modified };
            if self.recently_failed(&candidate.path, now)
                || !self.matches_selection(&candidate, selection, now)
            {
                continue;
            }
            push_socket_candidate(candidates, candidate);
        }
    }

    fn matches_selection(
        &mut self,
        candidate: &SocketCandidate,
        selection: BrowserSocketSelection,
        now: Instant,
    ) -> bool {
        match selection {
            BrowserSocketSelection::All => true,
            BrowserSocketSelection::Browser(browser) => {
                self.browser_family(candidate, now) == Some(browser)
            }
        }
    }

    fn browser_family(
        &mut self,
        candidate: &SocketCandidate,
        now: Instant,
    ) -> Option<BrowserFamily> {
        if let Some(cached) = self.families.get(&candidate.path)
            && cached.modified == candidate.modified
            && now.duration_since(cached.checked_at) < SOCKET_FAMILY_CACHE_TTL
        {
            return cached.family;
        }

        let family = socket_browser_family(&candidate.path);
        self.families.insert(
            candidate.path.clone(),
            CachedSocketFamily {
                family,
                modified: candidate.modified,
                checked_at: now,
            },
        );
        family
    }

    fn record_result<T>(&mut self, socket: &Path, result: Result<&T, &DiagnosticEntry>) {
        match result {
            Ok(_) => {
                self.recent_failures.remove(socket);
            }
            Err(diagnostic) if stale_socket_diagnostic(&diagnostic.code) => {
                self.recent_failures
                    .insert(socket.to_path_buf(), Instant::now());
            }
            Err(_) => {}
        }
    }

    fn recently_failed(&self, socket: &Path, now: Instant) -> bool {
        self.recent_failures
            .get(socket)
            .is_some_and(|failed_at| now.duration_since(*failed_at) < STALE_SOCKET_FAILURE_TTL)
    }

    fn prune(&mut self, now: Instant) {
        self.families
            .retain(|_, cached| now.duration_since(cached.checked_at) < SOCKET_FAMILY_CACHE_TTL);
        self.recent_failures
            .retain(|_, failed_at| now.duration_since(*failed_at) < STALE_SOCKET_FAILURE_TTL);
    }
}

/// Newest `extension-*.sock` across the candidate dirs, ignoring the
/// stale-failure quarantine and browser selection. The heartbeat keepalive
/// must keep re-establishing against the current host socket even if a per-op
/// probe transiently quarantined it, and the heartbeat is per-host (not scoped
/// to the operator's browser selection).
pub(super) fn newest_bridge_socket_path() -> Option<PathBuf> {
    let mut newest: Option<(SystemTime, PathBuf)> = None;
    for dir in candidate_socket_dirs() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if !name.starts_with("extension-") || !name.ends_with(".sock") {
                continue;
            }
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if !metadata.file_type().is_socket() {
                continue;
            }
            let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            if newest
                .as_ref()
                .is_none_or(|(newest_modified, _)| modified > *newest_modified)
            {
                newest = Some((modified, path));
            }
        }
    }
    newest.map(|(_, path)| path)
}

pub(super) fn find_bridge_sockets(selection: BrowserSocketSelection) -> Vec<PathBuf> {
    BRIDGE_SOCKET_INVENTORY
        .lock()
        .expect("browser socket inventory mutex poisoned")
        .find_bridge_sockets(candidate_socket_dirs(), selection, Instant::now())
}

pub(super) fn record_bridge_socket_result<T>(socket: &Path, result: Result<&T, &DiagnosticEntry>) {
    BRIDGE_SOCKET_INVENTORY
        .lock()
        .expect("browser socket inventory mutex poisoned")
        .record_result(socket, result);
}

fn stale_socket_diagnostic(code: &str) -> bool {
    matches!(
        code,
        "BrowserBridgeDisconnected" | "BrowserBridgeRequestTimedOut"
    )
}

#[cfg(test)]
pub(super) fn reset_socket_inventory_for_tests() {
    *BRIDGE_SOCKET_INVENTORY
        .lock()
        .expect("browser socket inventory mutex poisoned") = BridgeSocketInventory::default();
}

#[cfg(test)]
pub(super) fn cache_socket_family_for_tests(socket: &Path, family: Option<BrowserFamily>) {
    let modified = socket
        .metadata()
        .and_then(|metadata| metadata.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH);
    BRIDGE_SOCKET_INVENTORY
        .lock()
        .expect("browser socket inventory mutex poisoned")
        .families
        .insert(
            socket.to_path_buf(),
            CachedSocketFamily {
                family,
                modified,
                checked_at: Instant::now(),
            },
        );
}

pub(super) fn browser_socket_selection_from_env() -> Result<BrowserSocketSelection, DiagnosticEntry>
{
    // Env override first, then the machine config file
    // (~/.config/sky-cua/sky-cua.toml `browser` key), then no selection.
    let resolved = sky_cua_platform::config::resolved_browser_selection().map_err(|message| {
        DiagnosticEntry {
            code: "MachineConfigInvalid".to_string(),
            message,
            details: None,
        }
    })?;
    browser_socket_selection_from_value(resolved.as_deref())
}

pub(super) fn browser_socket_selection_from_value(
    value: Option<&str>,
) -> Result<BrowserSocketSelection, DiagnosticEntry> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(BrowserSocketSelection::All);
    };
    let normalized = value.to_ascii_lowercase().replace('_', "-");
    match normalized.as_str() {
        "all" | "any" | "*" => Ok(BrowserSocketSelection::All),
        "brave" | "brave-browser" => Ok(BrowserSocketSelection::Browser(BrowserFamily::Brave)),
        "chrome" | "google-chrome" => Ok(BrowserSocketSelection::Browser(BrowserFamily::Chrome)),
        "chromium" | "chromium-browser" => {
            Ok(BrowserSocketSelection::Browser(BrowserFamily::Chromium))
        }
        _ => Err(DiagnosticEntry {
            code: "BrowserSelectionInvalid".to_string(),
            message: format!(
                "Unsupported browser selection {value:?} (from {SKY_CUA_BROWSER_ENV} or the machine config `browser` key); use brave, chrome, chromium, or all."
            ),
            details: None,
        }),
    }
}

pub(super) fn browser_env_values_present() -> BTreeMap<String, String> {
    BROWSER_ENV_HEALTH_KEYS
        .iter()
        .filter_map(|key| {
            std::env::var(key)
                .ok()
                .filter(|value| !value.is_empty())
                .map(|value| ((*key).to_string(), value))
        })
        .collect()
}

fn candidate_socket_dirs() -> Vec<PathBuf> {
    if let Some(path) = env_socket_dir(SKY_CUA_SOCKET_DIR_ENV) {
        return vec![path];
    }
    if let Some(path) = env_socket_dir(CODEX_SOCKET_DIR_ENV) {
        return vec![path];
    }
    vec![PathBuf::from(DEFAULT_SOCKET_DIR)]
}

fn env_socket_dir(key: &str) -> Option<PathBuf> {
    let path = std::env::var_os(key)?;
    (!path.as_os_str().is_empty()).then(|| PathBuf::from(path))
}

fn push_socket_candidate(candidates: &mut Vec<SocketCandidate>, candidate: SocketCandidate) {
    if candidates.len() < MAX_BRIDGE_SOCKET_CANDIDATES {
        candidates.push(candidate);
        candidates.sort_by(compare_socket_candidates);
        return;
    }

    let Some(worst_candidate) = candidates.last() else {
        candidates.push(candidate);
        return;
    };
    if compare_socket_candidates(&candidate, worst_candidate) != Ordering::Less {
        return;
    }

    let insert_at = candidates
        .binary_search_by(|existing| compare_socket_candidates(existing, &candidate))
        .unwrap_or_else(|index| index);
    candidates.insert(insert_at, candidate);
    candidates.truncate(MAX_BRIDGE_SOCKET_CANDIDATES);
}

fn compare_socket_candidates(left: &SocketCandidate, right: &SocketCandidate) -> Ordering {
    right
        .modified
        .cmp(&left.modified)
        .then_with(|| left.path.cmp(&right.path))
}

fn socket_browser_family(socket: &Path) -> Option<BrowserFamily> {
    #[cfg(target_os = "linux")]
    {
        let mut pid = socket_host_pid(socket)?;
        for _ in 0..4 {
            pid = process_parent_pid(pid)?;
            if let Some(family) = process_executable(pid)
                .and_then(|executable| browser_family_from_executable(&executable))
            {
                return Some(family);
            }
        }
        None
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = socket;
        None
    }
}

pub(super) fn socket_host_pid(socket: &Path) -> Option<u32> {
    let file_name = socket.file_name()?.to_str()?;
    let rest = file_name.strip_prefix("extension-")?;
    let pid = rest.split('-').next()?;
    pid.parse().ok()
}

#[cfg(target_os = "linux")]
fn process_parent_pid(pid: u32) -> Option<u32> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    status.lines().find_map(|line| {
        let value = line.strip_prefix("PPid:")?.trim();
        value.parse().ok()
    })
}

#[cfg(target_os = "linux")]
fn process_executable(pid: u32) -> Option<String> {
    let bytes = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    bytes
        .split(|byte| *byte == 0)
        .find(|arg| !arg.is_empty())
        .map(|arg| String::from_utf8_lossy(arg).into_owned())
}

#[cfg(test)]
pub(super) fn browser_family_from_cmdline(cmdline: &str) -> Option<BrowserFamily> {
    browser_family_from_executable(cmdline.split_whitespace().next()?)
}

fn browser_family_from_executable(executable: &str) -> Option<BrowserFamily> {
    let executable = executable.to_ascii_lowercase();
    if executable.contains("brave") {
        return Some(BrowserFamily::Brave);
    }
    if executable.contains("chromium") {
        return Some(BrowserFamily::Chromium);
    }
    if executable.contains("chrome") || executable.contains("google-chrome") {
        return Some(BrowserFamily::Chrome);
    }
    None
}

pub(super) fn browser_bridge_disconnected_for_selection(
    selection: BrowserSocketSelection,
) -> DiagnosticEntry {
    match selection {
        BrowserSocketSelection::All => browser_bridge_disconnected_diagnostic(),
        BrowserSocketSelection::Browser(browser) => DiagnosticEntry {
            code: "BrowserBridgeDisconnected".to_string(),
            message: format!(
                "No Chrome extension/native-host browser socket is available for selected browser {SKY_CUA_BROWSER_ENV}={}",
                browser.label()
            ),
            details: None,
        },
    }
}
