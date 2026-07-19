#[cfg(test)]
use crate::app_server::AppServerControlError;
use crate::app_server::{AppServerControlResult, AppServerManager};
use crate::frame::{read_frame, write_frame};
mod control_plane;
mod control_plane_settlement;

use anyhow::{Context, Result, bail};
use control_plane::*;
use control_plane_settlement::*;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    env, fs,
    fs::File,
    io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write},
    net::Shutdown,
    os::unix::{
        fs::{MetadataExt, PermissionsExt},
        io::AsRawFd,
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    process,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use sky_cua_platform::config::BROWSER_USE_SOCKET_DIR_ENV as SKY_CUA_SOCKET_DIR_ENV;
const CODEX_SOCKET_DIR_ENV: &str = "CODEX_BROWSER_USE_SOCKET_DIR";
const SKY_CUA_SESSIONS_DIR_ENV: &str = "SKY_CUA_BROWSER_USE_SESSIONS_DIR";
const CODEX_SESSIONS_DIR_ENV: &str = "CODEX_BROWSER_USE_SESSIONS_DIR";
const TRACE_ENV: &str = "SKY_CUA_CHROME_HOST_TRACE";
const DEFAULT_SOCKET_DIR: &str = "/tmp/codex-browser-use";
const ROLLOUT_POLL_INTERVAL: Duration = Duration::from_millis(500);
const OBSERVED_TURN_TTL: Duration = Duration::from_secs(6 * 60 * 60);
const ROLLOUT_SEARCH_MAX_DEPTH: usize = 5;
const SKY_CUA_MCP_SESSION_ID: &str = "sky-cua-mcp";
const SKY_CUA_CLIENT_ROLE_PARAM: &str = "_sky_cua_client_role";
const SKY_CUA_OBSERVE_TURNS_PARAM: &str = "_sky_cua_observe_turns";
const MAX_NON_PRIMARY_CLIENTS: usize = 16;
const MAX_PENDING_REQUESTS: usize = 100;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

type SharedState = Arc<Mutex<HostState>>;
type SharedClientWriter = Arc<Mutex<UnixStream>>;

#[derive(Clone)]
struct Client {
    writer: SharedClientWriter,
    role: ClientRole,
    daemon_generation: Option<String>,
    capabilities: HashSet<String>,
    connected_at: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClientRole {
    Unknown,
    Primary,
    ControlPlane,
    Heartbeat,
    Ephemeral,
}

struct PendingChromeRequest {
    client_id: usize,
    client_request_id: Value,
    created_at: Instant,
    settlement: Option<SettlementMetadata>,
    state: PendingRequestState,
}

#[derive(Clone)]
struct PendingClientRequest {
    client_id: usize,
    chrome_request_id: Value,
    created_at: Instant,
}

#[derive(Debug, PartialEq, Eq)]
enum ChromeClientRouteError {
    NoClients,
    MultipleClients,
}

impl ChromeClientRouteError {
    fn message(&self) -> &'static str {
        match self {
            Self::NoClients => "No primary Codex browser client is connected",
            Self::MultipleClients => {
                "Multiple primary Codex browser clients are connected; Chrome requests require exactly one"
            }
        }
    }
}

struct HostState {
    host_name: String,
    stdout: Arc<Mutex<io::Stdout>>,
    rollout_tracker: RolloutTracker,
    clients: HashMap<usize, Client>,
    pending_chrome_requests: HashMap<String, PendingChromeRequest>,
    pending_client_requests: HashMap<String, PendingClientRequest>,
    pending_id_tombstones: HashMap<String, Instant>,
    queued_settlements: VecDeque<Value>,
    settlement_delivery_in_progress: bool,
    next_client_id: usize,
    next_chrome_id: u64,
    next_client_request_id: u64,
    app_server_manager: Arc<AppServerManager>,
    host_instance_id: String,
    owner_mode: OwnerMode,
    owner_daemon_generation: Option<String>,
    strict_legacy_clients_evicted: u64,
    strict_legacy_requests_rejected: u64,
}

impl HostState {
    #[cfg(test)]
    fn new(
        host_name: impl Into<String>,
        stdout: Arc<Mutex<io::Stdout>>,
        rollout_tracker: RolloutTracker,
    ) -> Self {
        Self::with_app_server_manager(
            host_name,
            stdout,
            rollout_tracker,
            Arc::new(AppServerManager::new(None, None)),
        )
    }

    fn with_app_server_manager(
        host_name: impl Into<String>,
        stdout: Arc<Mutex<io::Stdout>>,
        rollout_tracker: RolloutTracker,
        app_server_manager: Arc<AppServerManager>,
    ) -> Self {
        Self {
            host_name: host_name.into(),
            stdout,
            rollout_tracker,
            clients: HashMap::new(),
            pending_chrome_requests: HashMap::new(),
            pending_client_requests: HashMap::new(),
            pending_id_tombstones: HashMap::new(),
            queued_settlements: VecDeque::new(),
            settlement_delivery_in_progress: false,
            next_client_id: 1,
            next_chrome_id: 1,
            next_client_request_id: 1,
            app_server_manager,
            host_instance_id: format!(
                "native-host-{}-{}",
                process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            ),
            owner_mode: OwnerMode::Hybrid,
            owner_daemon_generation: None,
            strict_legacy_clients_evicted: 0,
            strict_legacy_requests_rejected: 0,
        }
    }

    fn add_client(&mut self, writer: SharedClientWriter) -> usize {
        let id = self.next_client_id;
        self.next_client_id += 1;
        self.clients.insert(
            id,
            Client {
                writer,
                role: ClientRole::Unknown,
                daemon_generation: None,
                capabilities: HashSet::new(),
                connected_at: Instant::now(),
            },
        );
        id
    }

    fn accept_client(&mut self, writer: SharedClientWriter) -> (usize, Vec<(usize, Client)>) {
        let id = self.add_client(writer);
        let evicted_clients = self.prune_excess_non_primary_clients();
        (id, evicted_clients)
    }

    fn update_client_role_for_message(
        &mut self,
        client_id: usize,
        message: &Value,
    ) -> Vec<(usize, Client)> {
        let Some(client) = self.clients.get(&client_id) else {
            return Vec::new();
        };

        let role = client_role_for_message(message);
        match (client.role, role) {
            (
                ClientRole::Primary
                | ClientRole::ControlPlane
                | ClientRole::Heartbeat
                | ClientRole::Ephemeral,
                _,
            ) => Vec::new(),
            (ClientRole::Unknown, ClientRole::Ephemeral) => {
                self.clients
                    .get_mut(&client_id)
                    .expect("client exists")
                    .role = ClientRole::Ephemeral;
                Vec::new()
            }
            (ClientRole::Unknown, ClientRole::Primary) => self.promote_primary_client(client_id),
            (ClientRole::Unknown, ClientRole::Heartbeat) => {
                self.clients
                    .get_mut(&client_id)
                    .expect("client exists")
                    .role = ClientRole::Heartbeat;
                Vec::new()
            }
            (ClientRole::Unknown, ClientRole::ControlPlane) => Vec::new(),
            (ClientRole::Unknown, ClientRole::Unknown) => Vec::new(),
        }
    }

    fn promote_primary_client(&mut self, client_id: usize) -> Vec<(usize, Client)> {
        let evict_ids = self
            .clients
            .iter()
            .filter_map(|(id, client)| {
                (*id != client_id && client.role == ClientRole::Primary).then_some(*id)
            })
            .collect::<Vec<_>>();
        let mut evicted_clients = Vec::new();
        for evict_id in evict_ids {
            if let Some(client) = self.clients.remove(&evict_id) {
                remove_pending_requests_for_client(
                    &mut self.pending_chrome_requests,
                    &mut self.pending_client_requests,
                    evict_id,
                );
                evicted_clients.push((evict_id, client));
            }
        }
        if let Some(client) = self.clients.get_mut(&client_id) {
            client.role = ClientRole::Primary;
        }
        evicted_clients
    }

    fn remove_client(&mut self, client_id: usize) {
        if let Some(client) = self.clients.remove(&client_id) {
            self.cleanup_pending_for_removed_client(client_id, client.role);
        }
    }

    fn prune_excess_non_primary_clients(&mut self) -> Vec<(usize, Client)> {
        let mut evicted_clients = Vec::new();
        while self
            .clients
            .values()
            .filter(|client| client_role_is_prunable(client.role))
            .count()
            > MAX_NON_PRIMARY_CLIENTS
        {
            let Some(oldest_id) = self
                .clients
                .iter()
                .filter(|(_, client)| client_role_is_prunable(client.role))
                .min_by_key(|(id, client)| (client.connected_at, *id))
                .map(|(id, _)| *id)
            else {
                break;
            };
            if let Some(client) = self.clients.remove(&oldest_id) {
                self.cleanup_pending_for_removed_client(oldest_id, client.role);
                evicted_clients.push((oldest_id, client));
            }
        }
        evicted_clients
    }

    /// Snapshot the Chrome-stdout writer handle and host name so the caller can
    /// write a frame AFTER dropping the state lock. Writing while the state lock
    /// is held lets a blocked pipe — a wedged service worker that has stopped
    /// reading its native port — freeze every other client thread and the Chrome
    /// reader thread along with it.
    fn chrome_writer(&self) -> (Arc<Mutex<io::Stdout>>, String) {
        (Arc::clone(&self.stdout), self.host_name.clone())
    }

    /// Clone a client's writer handle for a post-unlock write, or `None` when the
    /// client is gone.
    fn client_writer(&self, client_id: usize) -> Option<SharedClientWriter> {
        self.clients
            .get(&client_id)
            .map(|client| Arc::clone(&client.writer))
    }

    /// Clone the current control-plane and legacy primary writers for a
    /// post-unlock notification broadcast during hybrid migration.
    fn notification_client_writers(&self) -> Vec<SharedClientWriter> {
        self.clients
            .values()
            .filter(|client| matches!(client.role, ClientRole::ControlPlane | ClientRole::Primary))
            .map(|client| Arc::clone(&client.writer))
            .collect()
    }
}

fn client_role_is_prunable(role: ClientRole) -> bool {
    matches!(role, ClientRole::Unknown | ClientRole::Ephemeral)
}

/// Write a frame to the Chrome native-messaging pipe. Must be called WITHOUT the
/// host state lock held: a write to a pipe the service worker has stopped reading
/// blocks, and holding the state lock across that block would stall every client
/// thread and the Chrome reader. A hard write error means the Chrome port is dead
/// — terminal for the host — so exit and let socket removal surface a clean
/// disconnect rather than a hang.
fn write_chrome_frame(stdout: &Arc<Mutex<io::Stdout>>, host_name: &str, message: &Value) {
    let mut stdout = stdout.lock().expect("stdout mutex poisoned");
    if let Err(error) = write_frame(&mut *stdout, message) {
        log(host_name, &format!("native stdout error: {error}"));
        process::exit(1);
    }
}

/// Write a frame to a client socket. Must be called WITHOUT the host state lock
/// held, for the same head-of-line reason as [`write_chrome_frame`]. A write
/// error is non-fatal: the client's own reader thread observes the broken socket
/// and deregisters it.
fn write_client_frame(writer: &SharedClientWriter, host_name: &str, message: &Value) -> bool {
    let mut writer = writer.lock().expect("client writer mutex poisoned");
    if let Err(error) = write_frame(&mut *writer, message) {
        log(host_name, &format!("client socket write error: {error}"));
        return false;
    }
    true
}

#[derive(Clone)]
struct RolloutTracker {
    host_name: String,
    inner: Arc<Mutex<RolloutTrackerState>>,
    stdout: Arc<Mutex<io::Stdout>>,
    sessions_root: Option<PathBuf>,
}

struct RolloutTrackerState {
    observed: HashMap<String, ObservedTurn>,
}

struct ObservedTurn {
    session_id: String,
    turn_id: String,
    path: Option<PathBuf>,
    offset: u64,
    created_at: Instant,
}

impl RolloutTracker {
    fn new(host_name: String, stdout: Arc<Mutex<io::Stdout>>) -> Self {
        let tracker = Self::without_worker(host_name, stdout, sessions_root());
        let worker = tracker.clone();
        if let Err(error) = thread::Builder::new()
            .name("sky-cua-chrome-rollout-tracker".to_string())
            .spawn(move || worker.watch_loop())
        {
            log(
                &tracker.host_name,
                &format!("rollout watcher error: {error}"),
            );
        }
        tracker
    }

    fn without_worker(
        host_name: String,
        stdout: Arc<Mutex<io::Stdout>>,
        sessions_root: Option<PathBuf>,
    ) -> Self {
        Self {
            host_name,
            inner: Arc::new(Mutex::new(RolloutTrackerState {
                observed: HashMap::new(),
            })),
            stdout,
            sessions_root,
        }
    }

    fn observe_turn(&self, session_id: String, turn_id: String) {
        let key = observed_turn_key(&session_id, &turn_id);
        let mut state = self.inner.lock().expect("rollout watcher mutex poisoned");
        if state.observed.contains_key(&key) {
            return;
        }

        let (path, offset) = self
            .sessions_root
            .as_deref()
            .and_then(|root| find_rollout_path(root, &session_id))
            .map(|path| {
                let offset = file_len(&path).unwrap_or_default();
                (Some(path), offset)
            })
            .unwrap_or((None, 0));

        trace(
            &self.host_name,
            &format!("observed browser turn session={session_id} turn={turn_id}"),
        );
        state.observed.insert(
            key,
            ObservedTurn {
                session_id,
                turn_id,
                path,
                offset,
                created_at: Instant::now(),
            },
        );
    }

    fn watch_loop(self) {
        loop {
            thread::sleep(ROLLOUT_POLL_INTERVAL);
            match self.process_rollouts() {
                Ok(completed) => {
                    for (session_id, turn_id) in completed {
                        self.emit_turn_ended(&session_id, &turn_id);
                    }
                }
                Err(error) => log(&self.host_name, &format!("rollout watcher error: {error}")),
            }
        }
    }

    fn process_rollouts(&self) -> Result<Vec<(String, String)>> {
        let Some(sessions_root) = self.sessions_root.as_deref() else {
            return Ok(Vec::new());
        };

        let mut completed = Vec::new();
        let mut expired = Vec::new();
        {
            let mut state = self.inner.lock().expect("tracker mutex poisoned");
            for (key, observed) in &mut state.observed {
                if observed.created_at.elapsed() >= OBSERVED_TURN_TTL {
                    expired.push(key.clone());
                    continue;
                }

                if observed.path.is_none()
                    && let Some(path) = find_rollout_path(sessions_root, &observed.session_id)
                {
                    observed.offset = 0;
                    observed.path = Some(path);
                }

                let Some(path) = observed.path.as_ref() else {
                    continue;
                };
                let (offset, is_complete) =
                    drain_rollout_file(path, observed.offset, &observed.turn_id).with_context(
                        || format!("failed to drain rollout file {}", path.display()),
                    )?;
                observed.offset = offset;
                if is_complete {
                    completed.push((
                        key.clone(),
                        observed.session_id.clone(),
                        observed.turn_id.clone(),
                    ));
                }
            }

            for key in expired {
                state.observed.remove(&key);
            }
            for (key, _, _) in &completed {
                state.observed.remove(key);
            }
        }

        Ok(completed
            .into_iter()
            .map(|(_, session_id, turn_id)| (session_id, turn_id))
            .collect())
    }

    fn emit_turn_ended(&self, session_id: &str, turn_id: &str) {
        let message = turn_ended_message(session_id, turn_id);
        trace(
            &self.host_name,
            &format!("emitting turnEnded session={session_id} turn={turn_id}"),
        );
        let mut stdout = self.stdout.lock().expect("stdout writer mutex poisoned");
        if let Err(error) = write_frame(&mut *stdout, &message) {
            log(
                &self.host_name,
                &format!("failed to emit turnEnded for session {session_id}: {error}"),
            );
        }
    }
}

pub fn serve(host_name: String, extension_id: Option<String>) -> Result<()> {
    let socket_dir = socket_dir();
    prepare_socket_dir(&socket_dir)?;
    let socket_path = socket_path(&socket_dir);
    remove_socket_if_present(&socket_path)?;

    #[cfg(unix)]
    let old_umask = unsafe { libc::umask(0o077) };
    let listener = UnixListener::bind(&socket_path)
        .with_context(|| format!("failed to bind {}", socket_path.display()))?;
    #[cfg(unix)]
    unsafe {
        libc::umask(old_umask);
    }

    log(
        &host_name,
        &format!("listening on {}", socket_path.display()),
    );
    let stdout = Arc::new(Mutex::new(io::stdout()));
    let rollout_tracker = RolloutTracker::new(host_name.clone(), Arc::clone(&stdout));
    let app_server_manager = Arc::new(AppServerManager::new(Some(host_name.clone()), extension_id));
    let state = Arc::new(Mutex::new(HostState::with_app_server_manager(
        host_name,
        stdout,
        rollout_tracker,
        app_server_manager,
    )));
    {
        let state = Arc::clone(&state);
        thread::spawn(move || accept_clients(listener, state));
    }
    {
        let state = Arc::clone(&state);
        thread::spawn(move || settlement_maintenance_loop(state));
    }

    let result = read_chrome_messages(Arc::clone(&state));
    remove_socket_if_present(&socket_path)?;
    result
}

fn socket_dir() -> PathBuf {
    std::env::var_os(SKY_CUA_SOCKET_DIR_ENV)
        .or_else(|| std::env::var_os(CODEX_SOCKET_DIR_ENV))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_SOCKET_DIR))
}

fn sessions_root() -> Option<PathBuf> {
    if let Some(path) = env::var_os(SKY_CUA_SESSIONS_DIR_ENV).map(PathBuf::from) {
        return Some(path);
    }
    if let Some(path) = env::var_os(CODEX_SESSIONS_DIR_ENV).map(PathBuf::from) {
        return Some(path);
    }
    if let Some(path) = env::var_os("CODEX_HOME").map(PathBuf::from) {
        return Some(path.join("sessions"));
    }
    env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join(".codex").join("sessions"))
}

fn socket_path(socket_dir: &Path) -> PathBuf {
    let mut nonce = [0u8; 8];
    if let Ok(mut file) = fs::File::open("/dev/urandom") {
        let _ = file.read_exact(&mut nonce);
    }
    let nonce_hex = nonce
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<String>();
    socket_dir.join(format!("extension-{}-{nonce_hex}.sock", process::id()))
}

fn prepare_socket_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path).with_context(|| format!("failed to create {}", path.display()))?;
    let metadata =
        fs::symlink_metadata(path).with_context(|| format!("failed to stat {}", path.display()))?;
    if !metadata.file_type().is_dir() {
        bail!(
            "unix socket directory path is not a directory: {}",
            path.display()
        );
    }

    let current_uid = users_uid();
    if metadata.uid() != current_uid {
        bail!(
            "unix socket directory is owned by uid {}, expected {}: {}",
            metadata.uid(),
            current_uid,
            path.display()
        );
    }

    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .with_context(|| format!("failed to chmod {}", path.display()))
}

fn users_uid() -> u32 {
    fs::metadata("/proc/self")
        .map(|metadata| metadata.uid())
        // SAFETY: libc::getuid has no preconditions and cannot fail.
        .unwrap_or_else(|_| unsafe { libc::getuid() })
}

fn remove_socket_if_present(path: &Path) -> Result<()> {
    if path.exists() || path.is_symlink() {
        fs::remove_file(path).with_context(|| format!("failed to remove {}", path.display()))?;
    }
    Ok(())
}

fn get_host_name(state: &SharedState) -> String {
    state
        .lock()
        .expect("host state mutex poisoned")
        .host_name
        .clone()
}

fn accept_clients(listener: UnixListener, state: SharedState) {
    for stream in listener.incoming() {
        let Ok(stream) = stream else {
            log(&get_host_name(&state), "client socket accept error");
            continue;
        };
        if let Err(error) = authorize_client_peer(&stream) {
            log(
                &get_host_name(&state),
                &format!("rejecting client socket: {error}"),
            );
            let _ = stream.shutdown(Shutdown::Both);
            continue;
        }

        let writer = match stream.try_clone() {
            Ok(writer) => Arc::new(Mutex::new(writer)),
            Err(error) => {
                log(
                    &get_host_name(&state),
                    &format!("client socket clone error: {error}"),
                );
                continue;
            }
        };

        let (client_id, evicted_clients) = {
            let mut state = state.lock().expect("host state mutex poisoned");
            state.accept_client(writer)
        };
        for (evicted_id, evicted_client) in evicted_clients {
            log(
                &get_host_name(&state),
                &format!("evicting non-primary browser client {evicted_id} after the client cap"),
            );
            close_client_socket(&evicted_client);
        }

        let state = Arc::clone(&state);
        thread::spawn(move || read_client_messages(state, client_id, stream));
    }
}

fn authorize_client_peer(stream: &UnixStream) -> Result<()> {
    let peer = peer_cred(stream).context("failed to read client socket peer credentials")?;
    let current_uid = users_uid();
    if peer.uid != current_uid {
        bail!(
            "client uid {} does not match current uid {}",
            peer.uid,
            current_uid
        );
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PeerCred {
    uid: u32,
}

fn peer_cred(stream: &UnixStream) -> Result<PeerCred> {
    let mut credential = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            std::ptr::addr_of_mut!(credential).cast(),
            &mut length,
        )
    };
    if result == -1 {
        return Err(io::Error::last_os_error()).context("SO_PEERCRED getsockopt failed");
    }
    Ok(PeerCred {
        uid: credential.uid,
    })
}

fn close_client_socket(client: &Client) {
    if let Ok(writer) = client.writer.lock() {
        let _ = writer.shutdown(Shutdown::Both);
    }
}

fn read_chrome_messages(state: SharedState) -> Result<()> {
    let mut stdin = io::stdin();
    while let Some(message) =
        read_frame(&mut stdin).context("failed to read Chrome native frame")?
    {
        handle_chrome_message(&state, message);
    }
    Ok(())
}

fn read_client_messages(state: SharedState, client_id: usize, mut stream: UnixStream) {
    loop {
        match read_frame(&mut stream) {
            Ok(Some(message)) => handle_client_message(&state, client_id, message),
            Ok(None) => break,
            Err(error) => {
                log(
                    &get_host_name(&state),
                    &format!("client socket read error: {error}"),
                );
                break;
            }
        }
    }

    let mut state = state.lock().expect("host state mutex poisoned");
    state.remove_client(client_id);
}

fn handle_client_message(state: &SharedState, client_id: usize, message: Value) {
    {
        let state = state.lock().expect("host state mutex poisoned");
        if !state.clients.contains_key(&client_id) {
            return;
        }
    }

    if is_response(&message) {
        let Some(id) = message_id_as_string(&message) else {
            return;
        };

        let (stdout, host_name, outbound) = {
            let mut state = state.lock().expect("host state mutex poisoned");
            let Some(pending) = state.pending_client_requests.get(&id).cloned() else {
                return;
            };
            if pending.client_id != client_id {
                return;
            }
            state.pending_client_requests.remove(&id);
            let (stdout, host_name) = state.chrome_writer();
            (
                stdout,
                host_name,
                strip_host_private_params(with_id(message, pending.chrome_request_id)),
            )
        };
        write_chrome_frame(&stdout, &host_name, &outbound);
        return;
    }

    if message.get("method").and_then(Value::as_str) == Some(SKY_CUA_HOST_HELLO_METHOD) {
        let (writer, host_name, outcome) = {
            let mut state = state.lock().expect("host state mutex poisoned");
            let Some(writer) = state.client_writer(client_id) else {
                return;
            };
            let host_name = state.host_name.clone();
            let outcome = state.handle_host_hello(client_id, &message);
            (writer, host_name, outcome)
        };
        for (fenced_id, fenced_client) in &outcome.fenced_clients {
            log(
                &host_name,
                &format!(
                    "fencing stale control-plane client {fenced_id} after a newer daemon generation connected"
                ),
            );
            close_client_socket(fenced_client);
        }
        for (legacy_id, legacy_client) in &outcome.rejected_legacy_clients {
            log(
                &host_name,
                &format!(
                    "closing legacy browser-operation client {legacy_id} after strict control-plane activation"
                ),
            );
            close_client_socket(legacy_client);
        }
        if write_client_frame(&writer, &host_name, &outcome.response) {
            deliver_queued_settlements(state);
        }
        return;
    }

    if message.get("method").and_then(Value::as_str).is_some()
        && message.get("method").and_then(Value::as_str) != Some("ping")
    {
        let rejection = {
            let mut state = state.lock().expect("host state mutex poisoned");
            let client = state.clients.get(&client_id);
            if state.owner_mode == OwnerMode::Strict
                && client.is_some_and(|client| client.role != ClientRole::ControlPlane)
            {
                state.strict_legacy_requests_rejected =
                    state.strict_legacy_requests_rejected.saturating_add(1);
                Some((
                    state.client_writer(client_id),
                    state.host_name.clone(),
                    strict_legacy_request_error(
                        message.get("id").cloned().unwrap_or(Value::Null),
                        state.strict_legacy_requests_rejected,
                    ),
                ))
            } else {
                None
            }
        };
        if let Some((Some(writer), host_name, response)) = rejection {
            write_client_frame(&writer, &host_name, &response);
            return;
        }
    }

    let control_plane_marker_requires_hello = message
        .pointer(&format!("/params/{SKY_CUA_CLIENT_ROLE_PARAM}"))
        .and_then(Value::as_str)
        == Some(CONTROL_PLANE_ROLE)
        && {
            let state = state.lock().expect("host state mutex poisoned");
            state
                .clients
                .get(&client_id)
                .is_none_or(|client| client.role != ClientRole::ControlPlane)
        };
    if control_plane_marker_requires_hello {
        let Some((writer, host_name)) = ({
            let state = state.lock().expect("host state mutex poisoned");
            state
                .client_writer(client_id)
                .map(|writer| (writer, state.host_name.clone()))
        }) else {
            return;
        };
        write_client_frame(
            &writer,
            &host_name,
            &json!({
                "jsonrpc": "2.0",
                "id": message.get("id").cloned().unwrap_or(Value::Null),
                "error": {
                    "code": -32001,
                    "message": "control_plane role requires skyCuaHost/hello before extension dispatch",
                    "data": {
                        "type": "sky_cua_host_hello_required",
                        "host_protocol_version": SKY_CUA_HOST_PROTOCOL_VERSION,
                    }
                }
            }),
        );
        return;
    }

    let evicted_clients = {
        let mut state = state.lock().expect("host state mutex poisoned");
        if !state.clients.contains_key(&client_id) {
            return;
        }
        state.update_client_role_for_message(client_id, &message)
    };
    for (evicted_id, evicted_client) in evicted_clients {
        log(
            &get_host_name(state),
            &format!(
                "evicting stale primary browser client {evicted_id} after a newer primary connected"
            ),
        );
        close_client_socket(&evicted_client);
    }

    if !is_request(&message) {
        let Some((stdout, host_name)) = ({
            let state = state.lock().expect("host state mutex poisoned");
            state
                .clients
                .contains_key(&client_id)
                .then(|| state.chrome_writer())
        }) else {
            return;
        };
        write_chrome_frame(&stdout, &host_name, &strip_host_private_params(message));
        return;
    }

    if message.get("method").and_then(Value::as_str) == Some("ping") {
        let Some(id) = message.get("id").cloned() else {
            return;
        };
        let Some((writer, host_name)) = ({
            let state = state.lock().expect("host state mutex poisoned");
            state
                .client_writer(client_id)
                .map(|writer| (writer, state.host_name.clone()))
        }) else {
            return;
        };
        write_client_frame(
            &writer,
            &host_name,
            &json!({ "jsonrpc": "2.0", "id": id, "result": "pong" }),
        );
        return;
    }

    let Some(client_request_id) = message.get("id").cloned() else {
        return;
    };

    let settlement = {
        let guard = state.lock().expect("host state mutex poisoned");
        let Some(client) = guard.clients.get(&client_id) else {
            return;
        };
        if client.role != ClientRole::ControlPlane {
            None
        } else {
            match settlement_metadata(&message) {
                Ok(metadata)
                    if metadata.daemon_generation
                        == client.daemon_generation.as_deref().unwrap_or_default()
                        && (!metadata.operation_class.requires_settlement()
                            || guard.settlement_capacity_available()) =>
                {
                    Some(metadata)
                }
                Ok(metadata)
                    if metadata.daemon_generation
                        != client.daemon_generation.as_deref().unwrap_or_default() =>
                {
                    drop(guard);
                    reject_control_plane_request(
                        state,
                        client_id,
                        &client_request_id,
                        "sky_cua_host_generation_mismatch",
                        "request daemon_generation does not match the accepted control-plane hello",
                    );
                    return;
                }
                Ok(_) => {
                    drop(guard);
                    reject_control_plane_request(
                        state,
                        client_id,
                        &client_request_id,
                        "sky_cua_host_settlement_capacity",
                        "native-host settlement retention is full",
                    );
                    return;
                }
                Err(error) => {
                    drop(guard);
                    reject_control_plane_request(
                        state,
                        client_id,
                        &client_request_id,
                        "sky_cua_host_invalid_request_metadata",
                        error,
                    );
                    return;
                }
            }
        }
    };

    let observed_turn = session_turn_from_message(&message);
    let (tracker, observes_rollout_turns) = {
        let state = state.lock().expect("host state mutex poisoned");
        let Some(client) = state.clients.get(&client_id) else {
            return;
        };
        (
            state.rollout_tracker.clone(),
            client_observes_rollout_turns(client.role, &message),
        )
    };
    if observes_rollout_turns && let Some((session_id, turn_id)) = observed_turn {
        tracker.observe_turn(session_id, turn_id);
    }

    let (stdout, host_name, outbound) = {
        let mut state = state.lock().expect("host state mutex poisoned");
        if !state.clients.contains_key(&client_id) {
            return;
        }
        state.cleanup_old_requests();
        let chrome_id = state.allocate_chrome_id();
        state.pending_chrome_requests.insert(
            chrome_id.clone(),
            PendingChromeRequest {
                client_id,
                client_request_id,
                created_at: Instant::now(),
                settlement,
                state: PendingRequestState::Active,
            },
        );
        let (stdout, host_name) = state.chrome_writer();
        (
            stdout,
            host_name,
            strip_host_private_params(with_id(message, Value::String(chrome_id))),
        )
    };
    write_chrome_frame(&stdout, &host_name, &outbound);
}

fn handle_chrome_message(state: &SharedState, message: Value) {
    if is_response(&message) {
        let Some(id) = message_id_as_string(&message) else {
            return;
        };

        enum ResponseRoute {
            Client {
                writer: SharedClientWriter,
                outbound: Value,
                failed_delivery_settlement: Option<Value>,
            },
            Queued,
        }

        let (route, host_name) = {
            let mut state = state.lock().expect("host state mutex poisoned");
            let Some(pending) = state.pending_chrome_requests.remove(&id) else {
                trace(
                    &state.host_name,
                    &unmatched_chrome_response_trace(&id, &message),
                );
                return;
            };
            state.tombstone_pending_id(id.clone());
            let host_name = state.host_name.clone();
            let original_writer = state.client_writer(pending.client_id);
            let orphaned =
                pending.state == PendingRequestState::OrphanedPending || original_writer.is_none();
            let route = if !orphaned {
                let failed_delivery_settlement = pending
                    .settlement
                    .as_ref()
                    .filter(|metadata| metadata.operation_class.requires_settlement())
                    .map(|metadata| {
                        settlement_message(
                            "completed",
                            &id,
                            &pending.client_request_id,
                            metadata,
                            Some(message.clone()),
                        )
                    });
                ResponseRoute::Client {
                    writer: original_writer.expect("active pending client has writer"),
                    outbound: with_id(message, pending.client_request_id),
                    failed_delivery_settlement,
                }
            } else if let Some(metadata) = pending.settlement {
                let settlement = settlement_message(
                    "completed",
                    &id,
                    &pending.client_request_id,
                    &metadata,
                    Some(message),
                );
                if let Some(writer) = state.active_control_plane_writer() {
                    ResponseRoute::Client {
                        writer,
                        outbound: settlement.clone(),
                        failed_delivery_settlement: Some(settlement),
                    }
                } else {
                    state.queue_settlement(settlement);
                    ResponseRoute::Queued
                }
            } else {
                ResponseRoute::Queued
            };
            (route, host_name)
        };
        match route {
            ResponseRoute::Client {
                writer,
                outbound,
                failed_delivery_settlement,
            } => {
                if !write_client_frame(&writer, &host_name, &outbound)
                    && let Some(settlement) = failed_delivery_settlement
                {
                    state
                        .lock()
                        .expect("host state mutex poisoned")
                        .queue_settlement(settlement);
                }
            }
            ResponseRoute::Queued => {}
        }
        return;
    }

    if !is_request(&message) {
        let (writers, host_name) = {
            let state = state.lock().expect("host state mutex poisoned");
            (state.notification_client_writers(), state.host_name.clone())
        };
        for writer in writers {
            write_client_frame(&writer, &host_name, &message);
        }
        return;
    }

    let app_server_method = message.get("method").and_then(Value::as_str);
    if app_server_method.is_some_and(is_app_server_local_method) {
        let (manager, stdout, host_name) = {
            let state = state.lock().expect("host state mutex poisoned");
            let (stdout, host_name) = state.chrome_writer();
            (Arc::clone(&state.app_server_manager), stdout, host_name)
        };
        let params = message.get("params");
        let response = app_server_control_response(&message, || match app_server_method {
            Some("codexRuntime/hello") => manager.hello(params),
            Some("codexRuntime/restart") => manager.restart(params),
            Some("codexRuntime/ensure") => manager.ensure(params),
            Some("ensureCodexAppServer") => manager.ensure_legacy(),
            _ => unreachable!("app-server method was filtered above"),
        });
        write_chrome_frame(&stdout, &host_name, &response);
        return;
    }

    let chrome_request_id = message.get("id").cloned().unwrap_or(Value::Null);

    enum ChromeRoute {
        ToClient(SharedClientWriter, Value),
        RouteError(Arc<Mutex<io::Stdout>>, Value),
    }

    let (route, host_name) = {
        let mut state = state.lock().expect("host state mutex poisoned");
        let host_name = state.host_name.clone();
        let request_route = if app_server_method == Some("ping") {
            ChromeRequestRoute::Ping
        } else {
            ChromeRequestRoute::SidePanel
        };
        let route = match select_chrome_request_client_id(
            &state.clients,
            request_route,
            state.owner_mode,
        ) {
            Ok(client_id) => {
                let client_request_id =
                    format!("chrome-{}-{}", process::id(), state.next_client_request_id);
                state.next_client_request_id += 1;
                state.cleanup_old_requests();
                state.pending_client_requests.insert(
                    client_request_id.clone(),
                    PendingClientRequest {
                        client_id,
                        chrome_request_id,
                        created_at: Instant::now(),
                    },
                );
                let Some(writer) = state.client_writer(client_id) else {
                    return;
                };
                ChromeRoute::ToClient(writer, with_id(message, Value::String(client_request_id)))
            }
            Err(error) => {
                let (stdout, _) = state.chrome_writer();
                ChromeRoute::RouteError(
                    stdout,
                    json!({
                        "jsonrpc": "2.0",
                        "id": chrome_request_id,
                        "error": {
                            "code": -32000,
                            "message": error.message(),
                            "data": {
                                "type": "sky_cua_host_route_unavailable",
                                "route": match request_route {
                                    ChromeRequestRoute::Ping => "ping",
                                    ChromeRequestRoute::SidePanel => "side_panel",
                                }
                            }
                        }
                    }),
                )
            }
        };
        (route, host_name)
    };

    match route {
        ChromeRoute::ToClient(writer, outbound) => {
            write_client_frame(&writer, &host_name, &outbound);
        }
        ChromeRoute::RouteError(stdout, outbound) => {
            write_chrome_frame(&stdout, &host_name, &outbound)
        }
    }
}

fn is_app_server_local_method(method: &str) -> bool {
    matches!(
        method,
        "ensureCodexAppServer"
            | "codexRuntime/hello"
            | "codexRuntime/ensure"
            | "codexRuntime/restart"
    )
}

fn app_server_control_response(
    message: &Value,
    ensure: impl FnOnce() -> AppServerControlResult,
) -> Value {
    let id = message.get("id").cloned().unwrap_or(Value::Null);
    match ensure() {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err(error) => {
            let mut payload = json!({ "code": -32000, "message": error.to_string() });
            if let Some(error_type) = error.error_type() {
                payload["data"] = json!({ "type": error_type });
            }
            json!({ "jsonrpc": "2.0", "id": id, "error": payload })
        }
    }
}

fn select_primary_client_id(
    clients: &HashMap<usize, Client>,
) -> std::result::Result<usize, ChromeClientRouteError> {
    let mut primary_client_id = None;
    let mut unknown_client_id = None;
    let mut unknown_count = 0;
    for (id, client) in clients {
        match client.role {
            ClientRole::Primary => {
                if primary_client_id.is_some() {
                    return Err(ChromeClientRouteError::MultipleClients);
                }
                primary_client_id = Some(*id);
            }
            ClientRole::Unknown => {
                unknown_count += 1;
                unknown_client_id = Some(*id);
            }
            ClientRole::ControlPlane | ClientRole::Heartbeat | ClientRole::Ephemeral => {}
        }
    }
    if let Some(primary_client_id) = primary_client_id {
        return Ok(primary_client_id);
    }
    let heartbeat_client_id = clients
        .iter()
        .filter(|(_, client)| client.role == ClientRole::Heartbeat)
        .max_by_key(|(id, client)| (client.connected_at, *id))
        .map(|(id, _)| *id);
    if let Some(heartbeat_client_id) = heartbeat_client_id {
        return Ok(heartbeat_client_id);
    }
    if unknown_count > 1 {
        return Err(ChromeClientRouteError::MultipleClients);
    }
    unknown_client_id.ok_or(ChromeClientRouteError::NoClients)
}

fn client_role_for_message(message: &Value) -> ClientRole {
    match message
        .pointer(&format!("/params/{SKY_CUA_CLIENT_ROLE_PARAM}"))
        .and_then(Value::as_str)
    {
        Some("ephemeral") => return ClientRole::Ephemeral,
        Some("heartbeat") => return ClientRole::Heartbeat,
        _ => {}
    }
    if session_id_from_message(message) == Some(SKY_CUA_MCP_SESSION_ID) {
        ClientRole::Ephemeral
    } else {
        ClientRole::Primary
    }
}

fn client_observes_rollout_turns(role: ClientRole, message: &Value) -> bool {
    role == ClientRole::Primary
        || message
            .pointer(&format!("/params/{SKY_CUA_OBSERVE_TURNS_PARAM}"))
            .and_then(Value::as_bool)
            == Some(true)
}

fn session_id_from_message(message: &Value) -> Option<&str> {
    message
        .get("params")
        .and_then(|params| params.get("session_id"))
        .and_then(Value::as_str)
}

fn remove_pending_requests_for_client(
    pending_chrome_requests: &mut HashMap<String, PendingChromeRequest>,
    pending_client_requests: &mut HashMap<String, PendingClientRequest>,
    client_id: usize,
) {
    pending_chrome_requests.retain(|_, pending| pending.client_id != client_id);
    pending_client_requests.retain(|_, pending| pending.client_id != client_id);
}

fn is_request(message: &Value) -> bool {
    message.get("id").is_some() && message.get("method").and_then(Value::as_str).is_some()
}

fn is_response(message: &Value) -> bool {
    message.get("id").is_some() && message.get("method").and_then(Value::as_str).is_none()
}

fn message_id_as_string(message: &Value) -> Option<String> {
    message.get("id").and_then(|id| match id {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    })
}

fn with_id(mut message: Value, id: Value) -> Value {
    if let Value::Object(ref mut object) = message {
        object.insert("id".to_string(), id);
    }
    message
}

fn turn_ended_message(session_id: &str, turn_id: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": format!("native-turn-ended:{session_id}:{turn_id}"),
        "method": "turnEnded",
        "params": {
            "session_id": session_id,
            "turn_id": turn_id,
        },
    })
}

fn session_turn_from_message(message: &Value) -> Option<(String, String)> {
    let params = message.get("params")?;
    let session_id = non_empty_string(params.get("session_id")?)?;
    let turn_id = non_empty_string(params.get("turn_id")?)?;
    Some((session_id.to_string(), turn_id.to_string()))
}

fn non_empty_string(value: &Value) -> Option<&str> {
    let value = value.as_str()?.trim();
    (!value.is_empty()).then_some(value)
}

fn observed_turn_key(session_id: &str, turn_id: &str) -> String {
    format!("{session_id}\n{turn_id}")
}

fn file_len(path: &Path) -> io::Result<u64> {
    Ok(fs::metadata(path)?.len())
}

fn find_rollout_path(root: &Path, session_id: &str) -> Option<PathBuf> {
    let mut stack = vec![(root.to_path_buf(), 0_usize)];
    let mut best: Option<(SystemTime, PathBuf)> = None;

    while let Some((dir, depth)) = stack.pop() {
        let entries = fs::read_dir(&dir).ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };

            if file_type.is_dir() {
                if depth < ROLLOUT_SEARCH_MAX_DEPTH {
                    stack.push((path, depth + 1));
                }
                continue;
            }

            if !file_type.is_file() {
                continue;
            }

            let file_name = entry.file_name();
            let file_name = file_name.to_string_lossy();
            if !file_name.contains(session_id)
                || !(file_name.ends_with(".jsonl") || file_name.ends_with(".json"))
            {
                continue;
            }

            let modified = entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .unwrap_or(UNIX_EPOCH);
            if best
                .as_ref()
                .is_none_or(|(best_modified, _)| modified > *best_modified)
            {
                best = Some((modified, path));
            }
        }
    }

    best.map(|(_, path)| path)
}

fn drain_rollout_file(path: &Path, offset: u64, turn_id: &str) -> io::Result<(u64, bool)> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    file.seek(SeekFrom::Start(offset.min(len)))?;

    let mut reader = BufReader::new(file);
    let mut line = String::new();
    let mut is_complete = false;

    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        if line_marks_turn_complete(&line, turn_id) {
            is_complete = true;
        }
    }

    Ok((reader.stream_position()?, is_complete))
}

fn line_marks_turn_complete(line: &str, turn_id: &str) -> bool {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return false;
    };

    let payload = value.get("payload").unwrap_or(&value);
    let payload_type = payload.get("type").and_then(Value::as_str);
    let payload_turn_id = payload.get("turn_id").and_then(Value::as_str);
    if payload_type == Some("task_complete") && payload_turn_id == Some(turn_id) {
        return true;
    }

    let top_level_type = value.get("type").and_then(Value::as_str);
    let kind = value.get("kind").and_then(Value::as_str);
    top_level_type == Some("turn")
        && matches!(kind, Some("end" | "completed" | "complete"))
        && value.get("turn_id").and_then(Value::as_str) == Some(turn_id)
}

fn unmatched_chrome_response_trace(id: &str, message: &Value) -> String {
    format!("received unmatched Chrome response id={id} payload={message}")
}

fn log(host_name: &str, message: &str) {
    let _ = writeln!(io::stderr(), "[{host_name}] {message}");
}

fn trace(host_name: &str, message: &str) {
    if env::var_os(TRACE_ENV).is_some() {
        log(host_name, message);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_replacement_preserves_other_fields() {
        let message = json!({ "jsonrpc": "2.0", "id": 1, "method": "getTabs" });
        assert_eq!(
            with_id(message, Value::String("linux-1-1".to_string())),
            json!({ "jsonrpc": "2.0", "id": "linux-1-1", "method": "getTabs" })
        );
    }

    #[test]
    fn app_server_control_request_is_answered_without_a_browser_client() {
        let request = json!({
            "jsonrpc": "2.0",
            "id": "native-host:1",
            "method": "ensureCodexAppServer"
        });
        let response = app_server_control_response(&request, || {
            Ok(json!({ "localAppServerUrl": "http://127.0.0.1:46287" }))
        });

        assert_eq!(response["id"], json!("native-host:1"));
        assert_eq!(
            response["result"]["localAppServerUrl"],
            json!("http://127.0.0.1:46287")
        );
    }

    #[test]
    fn app_server_control_error_preserves_the_extension_error_type() {
        let request = json!({
            "jsonrpc": "2.0",
            "id": "native-host:2",
            "method": "codexRuntime/hello"
        });
        let response = app_server_control_response(&request, || {
            Err(AppServerControlError::typed(
                "version_mismatch",
                anyhow::anyhow!("runtime protocols are incompatible"),
            ))
        });

        assert_eq!(response["id"], json!("native-host:2"));
        assert_eq!(response["error"]["code"], json!(-32000));
        assert_eq!(response["error"]["data"]["type"], json!("version_mismatch"));
    }

    #[test]
    fn extracts_session_turn_from_browser_request() {
        let message = json!({
            "jsonrpc": "2.0",
            "id": "request-1",
            "method": "getTabs",
            "params": {
                "session_id": " session-1 ",
                "turn_id": "turn-1"
            }
        });

        assert_eq!(
            session_turn_from_message(&message),
            Some(("session-1".to_string(), "turn-1".to_string()))
        );
    }

    #[test]
    fn extracts_session_turn_from_cursor_move_request() {
        let message = json!({
            "jsonrpc": "2.0",
            "id": "request-1",
            "method": "moveMouse",
            "params": {
                "session_id": "session-1",
                "turn_id": "turn-1",
                "tabId": 42,
                "x": 240,
                "y": 160,
                "waitForArrival": true
            }
        });

        assert_eq!(
            session_turn_from_message(&message),
            Some(("session-1".to_string(), "turn-1".to_string()))
        );
    }

    #[test]
    fn id_replacement_preserves_cursor_move_params() {
        let message = json!({
            "jsonrpc": "2.0",
            "id": "request-1",
            "method": "moveMouse",
            "params": {
                "session_id": "session-1",
                "turn_id": "turn-1",
                "tabId": 42,
                "x": 240,
                "y": 160,
                "waitForArrival": true
            }
        });

        assert_eq!(
            with_id(message, Value::String("linux-1-1".to_string())),
            json!({
                "jsonrpc": "2.0",
                "id": "linux-1-1",
                "method": "moveMouse",
                "params": {
                    "session_id": "session-1",
                    "turn_id": "turn-1",
                    "tabId": 42,
                    "x": 240,
                    "y": 160,
                    "waitForArrival": true
                }
            })
        );
    }

    #[test]
    fn builds_turn_ended_message_for_chrome_extension() {
        assert_eq!(
            turn_ended_message("session-1", "turn-1"),
            json!({
                "jsonrpc": "2.0",
                "id": "native-turn-ended:session-1:turn-1",
                "method": "turnEnded",
                "params": {
                    "session_id": "session-1",
                    "turn_id": "turn-1"
                }
            })
        );
    }

    #[test]
    fn unmatched_chrome_response_trace_includes_payload() {
        let message = json!({
            "jsonrpc": "2.0",
            "id": "native-turn-ended:session-1:turn-1",
            "result": null,
        });

        assert_eq!(
            unmatched_chrome_response_trace("native-turn-ended:session-1:turn-1", &message),
            r#"received unmatched Chrome response id=native-turn-ended:session-1:turn-1 payload={"id":"native-turn-ended:session-1:turn-1","jsonrpc":"2.0","result":null}"#
        );
    }

    #[test]
    fn recognizes_rollout_completion_lines() {
        let payload_line = r#"{"timestamp":"2026-05-09T12:00:00Z","type":"event_msg","payload":{"type":"task_complete","turn_id":"turn-1"}}"#;
        let top_level_line = r#"{"type":"turn","kind":"completed","turn_id":"turn-1"}"#;

        assert!(line_marks_turn_complete(payload_line, "turn-1"));
        assert!(line_marks_turn_complete(top_level_line, "turn-1"));
        assert!(!line_marks_turn_complete(payload_line, "turn-2"));
        assert!(!line_marks_turn_complete("not json", "turn-1"));
    }

    #[test]
    fn finds_nested_rollout_path_by_session_id() {
        let root = unique_test_dir("sky-cua-rollout-path");
        let nested = root.join("2026").join("05").join("09");
        fs::create_dir_all(&nested).unwrap();
        let path = nested.join("rollout-2026-05-09T12-00-00-session-1.jsonl");
        fs::write(&path, "{}\n").unwrap();

        assert_eq!(find_rollout_path(&root, "session-1"), Some(path));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn drains_rollout_file_from_offset() {
        let root = unique_test_dir("sky-cua-rollout-drain");
        fs::create_dir_all(&root).unwrap();
        let path = root.join("rollout-session-1.jsonl");
        fs::write(
            &path,
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"other\"}}\n",
        )
        .unwrap();
        let offset = file_len(&path).unwrap();

        let complete =
            r#"{"type":"event_msg","payload":{"type":"task_complete","turn_id":"turn-1"}}"#;
        writeln!(
            fs::OpenOptions::new().append(true).open(&path).unwrap(),
            "ignored\n{complete}"
        )
        .unwrap();
        let (new_offset, is_complete) = drain_rollout_file(&path, offset, "turn-1").unwrap();

        assert!(new_offset >= offset);
        assert!(is_complete);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rollout_tracker_detects_late_discovered_completion_and_removes_observation() {
        let root = unique_test_dir("sky-cua-rollout-late");
        fs::create_dir_all(&root).unwrap();
        let tracker = test_rollout_tracker(Some(root.clone()));
        tracker.observe_turn("session-1".to_string(), "turn-1".to_string());

        assert!(tracker.process_rollouts().unwrap().is_empty());

        let nested = root.join("2026").join("05").join("09");
        fs::create_dir_all(&nested).unwrap();
        let path = nested.join("rollout-session-1.jsonl");
        let complete =
            r#"{"type":"event_msg","payload":{"type":"task_complete","turn_id":"turn-1"}}"#;
        writeln!(File::create(path).unwrap(), "{complete}").unwrap();

        assert_eq!(
            tracker.process_rollouts().unwrap(),
            vec![("session-1".to_string(), "turn-1".to_string())]
        );
        assert!(tracker.process_rollouts().unwrap().is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn chrome_request_routing_uses_primary_client_only() {
        let mut clients = HashMap::new();
        assert_eq!(
            select_primary_client_id(&clients),
            Err(ChromeClientRouteError::NoClients)
        );

        clients.insert(6, test_client_with_role(ClientRole::Unknown));
        assert_eq!(select_primary_client_id(&clients), Ok(6));

        clients.insert(8, test_client_with_role(ClientRole::Ephemeral));
        assert_eq!(select_primary_client_id(&clients), Ok(6));

        clients.insert(7, test_client_with_role(ClientRole::Primary));
        assert_eq!(select_primary_client_id(&clients), Ok(7));

        clients.insert(9, test_client_with_role(ClientRole::Primary));
        assert_eq!(
            select_primary_client_id(&clients),
            Err(ChromeClientRouteError::MultipleClients)
        );
    }

    #[test]
    fn chrome_request_routing_rejects_multiple_unknown_clients() {
        let mut clients = HashMap::new();
        clients.insert(7, test_client_with_role(ClientRole::Unknown));
        clients.insert(8, test_client_with_role(ClientRole::Unknown));

        assert_eq!(
            select_primary_client_id(&clients),
            Err(ChromeClientRouteError::MultipleClients)
        );
    }

    #[test]
    fn sky_cua_mcp_client_does_not_evict_primary_client_or_pending_requests() {
        let mut state = test_host_state();

        let primary_client_id = state.add_client(test_client().writer.clone());
        let evicted_clients = state.update_client_role_for_message(
            primary_client_id,
            &json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "getInfo",
                "params": { "session_id": "codex-session", "turn_id": "turn-1" }
            }),
        );
        assert!(evicted_clients.is_empty());
        assert_eq!(state.clients[&primary_client_id].role, ClientRole::Primary);

        state.pending_chrome_requests.insert(
            "chrome-request".to_string(),
            PendingChromeRequest {
                client_id: primary_client_id,
                client_request_id: json!("client-request-1"),
                created_at: Instant::now(),
                settlement: None,
                state: PendingRequestState::Active,
            },
        );
        state.pending_client_requests.insert(
            "client-request".to_string(),
            PendingClientRequest {
                client_id: primary_client_id,
                chrome_request_id: json!("chrome-request-1"),
                created_at: Instant::now(),
            },
        );

        let mcp_client_id = state.add_client(test_client().writer.clone());
        let evicted_clients = state.update_client_role_for_message(
            mcp_client_id,
            &json!({
                "jsonrpc": "2.0",
                "id": "sky-cua-browser-info",
                "method": "getInfo",
                "params": { "session_id": "sky-cua-mcp", "turn_id": "browser-list-tabs" }
            }),
        );

        assert!(evicted_clients.is_empty());
        assert!(state.clients.contains_key(&primary_client_id));
        assert!(state.clients.contains_key(&mcp_client_id));
        assert_eq!(state.clients[&primary_client_id].role, ClientRole::Primary);
        assert_eq!(state.clients[&mcp_client_id].role, ClientRole::Ephemeral);
        assert!(state.pending_chrome_requests.contains_key("chrome-request"));
        assert!(state.pending_client_requests.contains_key("client-request"));
        assert_eq!(
            select_primary_client_id(&state.clients),
            Ok(primary_client_id)
        );
    }

    #[test]
    fn propagated_codex_identity_stays_ephemeral_with_explicit_provenance() {
        let mut state = test_host_state();
        let primary_client_id = state.add_client(test_client().writer.clone());
        state.update_client_role_for_message(
            primary_client_id,
            &json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "getInfo",
                "params": { "session_id": "browser-use-session", "turn_id": "turn-1" }
            }),
        );

        let sky_cua_client_id = state.add_client(test_client().writer.clone());
        let evicted = state.update_client_role_for_message(
            sky_cua_client_id,
            &json!({
                "jsonrpc": "2.0",
                "id": "sky-cua-browser-info",
                "method": "getInfo",
                "params": {
                    "session_id": "browser-use-session",
                    "turn_id": "turn-1",
                    "_sky_cua_client_role": "ephemeral",
                    "_sky_cua_observe_turns": true
                }
            }),
        );

        assert!(evicted.is_empty());
        assert_eq!(state.clients[&primary_client_id].role, ClientRole::Primary);
        assert_eq!(
            state.clients[&sky_cua_client_id].role,
            ClientRole::Ephemeral
        );
        assert_eq!(
            select_primary_client_id(&state.clients),
            Ok(primary_client_id)
        );
        assert!(client_observes_rollout_turns(
            state.clients[&sky_cua_client_id].role,
            &json!({
                "params": {
                    "_sky_cua_observe_turns": true
                }
            })
        ));
    }

    #[test]
    fn heartbeat_is_fallback_without_evicting_or_overriding_primary() {
        let mut state = test_host_state();
        let heartbeat_client_id = state.add_client(test_client().writer.clone());
        state.update_client_role_for_message(
            heartbeat_client_id,
            &json!({
                "jsonrpc": "2.0",
                "id": "heartbeat",
                "method": "getInfo",
                "params": {
                    "session_id": "sky-cua-heartbeat-keepalive",
                    "turn_id": "heartbeat",
                    "_sky_cua_client_role": "heartbeat"
                }
            }),
        );
        assert_eq!(
            state.clients[&heartbeat_client_id].role,
            ClientRole::Heartbeat
        );
        assert_eq!(
            select_primary_client_id(&state.clients),
            Ok(heartbeat_client_id)
        );

        let primary_client_id = state.add_client(test_client().writer.clone());
        let evicted = state.update_client_role_for_message(
            primary_client_id,
            &json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "getInfo",
                "params": { "session_id": "browser-use-session", "turn_id": "turn-1" }
            }),
        );

        assert!(evicted.is_empty());
        assert!(state.clients.contains_key(&heartbeat_client_id));
        assert_eq!(
            select_primary_client_id(&state.clients),
            Ok(primary_client_id)
        );
    }

    #[test]
    fn newest_heartbeat_is_selected_during_daemon_handoff() {
        let mut state = test_host_state();
        let first_id = state.add_client(test_client().writer.clone());
        state.update_client_role_for_message(
            first_id,
            &json!({
                "jsonrpc": "2.0",
                "id": "first-heartbeat",
                "method": "getInfo",
                "params": { "_sky_cua_client_role": "heartbeat" }
            }),
        );
        let second_id = state.add_client(test_client().writer.clone());
        state.update_client_role_for_message(
            second_id,
            &json!({
                "jsonrpc": "2.0",
                "id": "second-heartbeat",
                "method": "getInfo",
                "params": { "_sky_cua_client_role": "heartbeat" }
            }),
        );

        assert!(state.clients.contains_key(&first_id));
        assert!(state.clients.contains_key(&second_id));
        assert_eq!(select_primary_client_id(&state.clients), Ok(second_id));
    }

    #[test]
    fn sky_cua_mcp_client_does_not_observe_rollout_turns() {
        let mut state = test_host_state();
        let mcp_client_id = state.add_client(test_client().writer.clone());
        state.update_client_role_for_message(
            mcp_client_id,
            &json!({
                "jsonrpc": "2.0",
                "id": "sky-cua-browser-info",
                "method": "getInfo",
                "params": { "session_id": "sky-cua-mcp", "turn_id": "browser-list-tabs" }
            }),
        );

        assert_eq!(state.clients[&mcp_client_id].role, ClientRole::Ephemeral);
        assert!(!client_observes_rollout_turns(
            state.clients[&mcp_client_id].role,
            &json!({
                "params": {
                    "session_id": "sky-cua-mcp",
                    "turn_id": "browser-list-tabs",
                    "_sky_cua_client_role": "ephemeral",
                    "_sky_cua_observe_turns": false
                }
            })
        ));
        assert!(client_observes_rollout_turns(
            ClientRole::Primary,
            &json!({})
        ));
    }

    #[test]
    fn sky_cua_mcp_client_stays_ephemeral_for_stale_session_maintenance() {
        let mut state = test_host_state();
        let primary_client_id = state.add_client(test_client().writer.clone());
        state.update_client_role_for_message(
            primary_client_id,
            &json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "getInfo",
                "params": { "session_id": "codex-session", "turn_id": "turn-1" }
            }),
        );
        let mcp_client_id = state.add_client(test_client().writer.clone());
        state.update_client_role_for_message(
            mcp_client_id,
            &json!({
                "jsonrpc": "2.0",
                "id": "sky-cua-browser-info",
                "method": "getInfo",
                "params": { "session_id": "sky-cua-mcp", "turn_id": "browser-list-tabs" }
            }),
        );

        let evicted_clients = state.update_client_role_for_message(
            mcp_client_id,
            &json!({
                "jsonrpc": "2.0",
                "id": "finalize-stale-session",
                "method": "finalizeTabs",
                "params": { "session_id": "sky-cua-cursor-proof", "turn_id": "browser-list-tabs" }
            }),
        );

        assert!(evicted_clients.is_empty());
        assert_eq!(state.clients[&primary_client_id].role, ClientRole::Primary);
        assert_eq!(state.clients[&mcp_client_id].role, ClientRole::Ephemeral);
        assert_eq!(
            select_primary_client_id(&state.clients),
            Ok(primary_client_id)
        );
    }

    #[test]
    fn accept_client_caps_non_primary_clients_without_evicting_primary() {
        let mut state = test_host_state();
        let primary_client_id = state.add_client(test_client().writer.clone());
        state.update_client_role_for_message(
            primary_client_id,
            &json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "getInfo",
                "params": { "session_id": "codex-session", "turn_id": "turn-1" }
            }),
        );

        let mut evicted_clients = Vec::new();
        for _ in 0..(MAX_NON_PRIMARY_CLIENTS + 2) {
            let (_client_id, evicted) = state.accept_client(test_client().writer.clone());
            evicted_clients.extend(evicted);
        }

        assert_eq!(state.clients[&primary_client_id].role, ClientRole::Primary);
        assert_eq!(
            state
                .clients
                .values()
                .filter(|client| client.role != ClientRole::Primary)
                .count(),
            MAX_NON_PRIMARY_CLIENTS
        );
        assert_eq!(evicted_clients.len(), 2);
        assert!(
            evicted_clients
                .iter()
                .all(|(_, client)| client.role != ClientRole::Primary)
        );
        assert_eq!(
            select_primary_client_id(&state.clients),
            Ok(primary_client_id)
        );
    }

    #[test]
    fn accept_client_churn_does_not_evict_heartbeat_fallback() {
        let mut state = test_host_state();
        let heartbeat_client_id = state.add_client(test_client().writer.clone());
        state.update_client_role_for_message(
            heartbeat_client_id,
            &json!({
                "jsonrpc": "2.0",
                "id": "heartbeat",
                "method": "getInfo",
                "params": {
                    "session_id": "sky-cua-heartbeat-keepalive",
                    "turn_id": "heartbeat",
                    "_sky_cua_client_role": "heartbeat"
                }
            }),
        );

        let mut evicted_clients = Vec::new();
        for _ in 0..(MAX_NON_PRIMARY_CLIENTS + 2) {
            let (_client_id, evicted) = state.accept_client(test_client().writer.clone());
            evicted_clients.extend(evicted);
        }

        assert_eq!(
            state.clients[&heartbeat_client_id].role,
            ClientRole::Heartbeat
        );
        assert_eq!(evicted_clients.len(), 2);
        assert!(
            evicted_clients
                .iter()
                .all(|(_, client)| client.role != ClientRole::Heartbeat)
        );
        assert_eq!(
            state
                .clients
                .values()
                .filter(|client| client_role_is_prunable(client.role))
                .count(),
            MAX_NON_PRIMARY_CLIENTS
        );
        assert_eq!(
            select_primary_client_id(&state.clients),
            Ok(heartbeat_client_id)
        );
    }

    #[test]
    fn new_primary_client_evicts_previous_primary_and_pending_requests() {
        let mut state = test_host_state();
        let first_client_id = state.add_client(test_client().writer.clone());
        state.update_client_role_for_message(
            first_client_id,
            &json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "getInfo",
                "params": { "session_id": "codex-session", "turn_id": "turn-1" }
            }),
        );
        state.pending_chrome_requests.insert(
            "chrome-request".to_string(),
            PendingChromeRequest {
                client_id: first_client_id,
                client_request_id: json!("client-request-1"),
                created_at: Instant::now(),
                settlement: None,
                state: PendingRequestState::Active,
            },
        );

        let second_client_id = state.add_client(test_client().writer.clone());
        let evicted_clients = state.update_client_role_for_message(
            second_client_id,
            &json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "getInfo",
                "params": { "session_id": "codex-session-2", "turn_id": "turn-2" }
            }),
        );

        assert_ne!(first_client_id, second_client_id);
        assert_eq!(evicted_clients.len(), 1);
        assert_eq!(evicted_clients[0].0, first_client_id);
        assert!(!state.clients.contains_key(&first_client_id));
        assert_eq!(state.clients[&second_client_id].role, ClientRole::Primary);
        assert!(state.pending_chrome_requests.is_empty());
    }

    #[test]
    fn evicted_client_requests_are_ignored() {
        let state = Arc::new(Mutex::new(test_host_state()));

        handle_client_message(
            &state,
            99,
            json!({ "jsonrpc": "2.0", "id": 1, "method": "getTabs" }),
        );

        let state = state.lock().unwrap();
        assert!(state.pending_chrome_requests.is_empty());
        assert_eq!(state.next_chrome_id, 1);
    }

    #[test]
    fn stale_client_requests_do_not_observe_rollout_turns() {
        let state = Arc::new(Mutex::new(test_host_state()));

        handle_client_message(
            &state,
            99,
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "getTabs",
                "params": {
                    "session_id": "session-1",
                    "turn_id": "turn-1"
                }
            }),
        );

        let tracker = state.lock().unwrap().rollout_tracker.clone();
        assert!(tracker.inner.lock().unwrap().observed.is_empty());
    }

    #[test]
    fn stale_cursor_move_requests_do_not_observe_rollout_turns() {
        let state = Arc::new(Mutex::new(test_host_state()));

        handle_client_message(
            &state,
            99,
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "moveMouse",
                "params": {
                    "session_id": "session-1",
                    "turn_id": "turn-1",
                    "tabId": 42,
                    "x": 240,
                    "y": 160,
                    "waitForArrival": true
                }
            }),
        );

        let tracker = state.lock().unwrap().rollout_tracker.clone();
        assert!(tracker.inner.lock().unwrap().observed.is_empty());
    }

    #[test]
    fn disconnect_cleanup_removes_pending_state_for_client() {
        let mut pending_chrome = HashMap::from([
            (
                "keep".to_string(),
                PendingChromeRequest {
                    client_id: 1,
                    client_request_id: json!("chrome-request-1"),
                    created_at: Instant::now(),
                    settlement: None,
                    state: PendingRequestState::Active,
                },
            ),
            (
                "drop".to_string(),
                PendingChromeRequest {
                    client_id: 2,
                    client_request_id: json!("chrome-request-2"),
                    created_at: Instant::now(),
                    settlement: None,
                    state: PendingRequestState::Active,
                },
            ),
        ]);
        let mut pending_client = HashMap::from([
            (
                "keep".to_string(),
                PendingClientRequest {
                    client_id: 1,
                    chrome_request_id: json!("client-request-1"),
                    created_at: Instant::now(),
                },
            ),
            (
                "drop".to_string(),
                PendingClientRequest {
                    client_id: 2,
                    chrome_request_id: json!("client-request-2"),
                    created_at: Instant::now(),
                },
            ),
        ]);

        remove_pending_requests_for_client(&mut pending_chrome, &mut pending_client, 2);

        assert!(pending_chrome.contains_key("keep"));
        assert!(!pending_chrome.contains_key("drop"));
        assert!(pending_client.contains_key("keep"));
        assert!(!pending_client.contains_key("drop"));
    }

    #[test]
    fn authorizes_same_uid_unix_socket_peer() {
        let (stream, _peer) = UnixStream::pair().unwrap();

        authorize_client_peer(&stream).unwrap();
    }

    #[test]
    fn chrome_response_is_forwarded_to_the_matching_client_after_unlock() {
        // Exercises the write-after-unlock routing: a Chrome response is matched
        // to its pending client request and the reply frame reaches the client's
        // socket. Regression guard for moving the socket write out from under the
        // host state lock.
        let (mut peer, writer_stream) = UnixStream::pair().unwrap();
        let state = Arc::new(Mutex::new(test_host_state()));
        {
            let mut state = state.lock().unwrap();
            let client_id = state.add_client(Arc::new(Mutex::new(writer_stream)));
            state.pending_chrome_requests.insert(
                "linux-1-1".to_string(),
                PendingChromeRequest {
                    client_id,
                    client_request_id: json!("client-req-1"),
                    created_at: Instant::now(),
                    settlement: None,
                    state: PendingRequestState::Active,
                },
            );
        }

        handle_chrome_message(
            &state,
            json!({ "jsonrpc": "2.0", "id": "linux-1-1", "result": {"ok": true} }),
        );

        let forwarded = read_frame(&mut peer).unwrap().unwrap();
        assert_eq!(forwarded["id"], json!("client-req-1"));
        assert_eq!(forwarded["result"]["ok"], json!(true));
        assert!(state.lock().unwrap().pending_chrome_requests.is_empty());
    }

    fn test_client() -> Client {
        test_client_with_role(ClientRole::Unknown)
    }

    fn test_client_with_role(role: ClientRole) -> Client {
        let (stream, _peer) = UnixStream::pair().unwrap();
        Client {
            writer: Arc::new(Mutex::new(stream)),
            role,
            daemon_generation: (role == ClientRole::ControlPlane).then_some("daemon-1".to_string()),
            capabilities: HashSet::new(),
            connected_at: Instant::now(),
        }
    }

    fn test_host_state() -> HostState {
        let stdout = Arc::new(Mutex::new(io::stdout()));
        HostState::new(
            "com.openai.codexextension",
            Arc::clone(&stdout),
            RolloutTracker::without_worker("com.openai.codexextension".to_string(), stdout, None),
        )
    }

    fn test_rollout_tracker(sessions_root: Option<PathBuf>) -> RolloutTracker {
        RolloutTracker::without_worker(
            "com.openai.codexextension".to_string(),
            Arc::new(Mutex::new(io::stdout())),
            sessions_root,
        )
    }

    fn unique_test_dir(prefix: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        env::temp_dir().join(format!("{prefix}-{}-{nonce}", process::id()))
    }
}
