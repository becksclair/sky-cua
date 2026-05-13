use crate::frame::{read_frame, write_frame};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    fs,
    io::{self, Read, Write},
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
    time::Instant,
};

const SKY_CUA_SOCKET_DIR_ENV: &str = "SKY_CUA_BROWSER_USE_SOCKET_DIR";
const CODEX_SOCKET_DIR_ENV: &str = "CODEX_BROWSER_USE_SOCKET_DIR";
const DEFAULT_SOCKET_DIR: &str = "/tmp/codex-browser-use";

type SharedState = Arc<Mutex<HostState>>;
type SharedClientWriter = Arc<Mutex<UnixStream>>;

#[derive(Clone)]
struct Client {
    writer: SharedClientWriter,
}

struct PendingChromeRequest {
    client_id: usize,
    client_request_id: Value,
    created_at: Instant,
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
            Self::NoClients => "No Codex browser client is connected",
            Self::MultipleClients => {
                "Multiple Codex browser clients are connected; Chrome requests require exactly one"
            }
        }
    }
}

struct HostState {
    host_name: String,
    stdout: Arc<Mutex<io::Stdout>>,
    clients: HashMap<usize, Client>,
    pending_chrome_requests: HashMap<String, PendingChromeRequest>,
    pending_client_requests: HashMap<String, PendingClientRequest>,
    next_client_id: usize,
    next_chrome_id: u64,
    next_client_request_id: u64,
}

impl HostState {
    fn new(host_name: impl Into<String>, stdout: Arc<Mutex<io::Stdout>>) -> Self {
        Self {
            host_name: host_name.into(),
            stdout,
            clients: HashMap::new(),
            pending_chrome_requests: HashMap::new(),
            pending_client_requests: HashMap::new(),
            next_client_id: 1,
            next_chrome_id: 1,
            next_client_request_id: 1,
        }
    }

    fn replace_with_client(&mut self, writer: SharedClientWriter) -> (usize, Vec<(usize, Client)>) {
        let evicted_clients = self.clients.drain().collect::<Vec<_>>();
        if !evicted_clients.is_empty() {
            self.pending_chrome_requests.clear();
            self.pending_client_requests.clear();
        }

        let id = self.next_client_id;
        self.next_client_id += 1;
        self.clients.insert(id, Client { writer });
        (id, evicted_clients)
    }

    fn remove_client(&mut self, client_id: usize) {
        self.clients.remove(&client_id);
        remove_pending_requests_for_client(
            &mut self.pending_chrome_requests,
            &mut self.pending_client_requests,
            client_id,
        );
    }

    fn cleanup_old_requests(&mut self) {
        const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
        const MAX_PENDING: usize = 100;
        let now = Instant::now();
        self.pending_chrome_requests
            .retain(|_, req| now.duration_since(req.created_at) < TIMEOUT);
        self.pending_client_requests
            .retain(|_, req| now.duration_since(req.created_at) < TIMEOUT);
        while self.pending_chrome_requests.len() > MAX_PENDING {
            if let Some(oldest) = self
                .pending_chrome_requests
                .iter()
                .min_by_key(|(_, req)| req.created_at)
                .map(|(id, _)| id.clone())
            {
                self.pending_chrome_requests.remove(&oldest);
            }
        }
        while self.pending_client_requests.len() > MAX_PENDING {
            if let Some(oldest) = self
                .pending_client_requests
                .iter()
                .min_by_key(|(_, req)| req.created_at)
                .map(|(id, _)| id.clone())
            {
                self.pending_client_requests.remove(&oldest);
            }
        }
    }

    fn send_chrome(&self, message: &Value) {
        let mut stdout = self.stdout.lock().expect("stdout mutex poisoned");
        if let Err(error) = write_frame(&mut *stdout, message) {
            log(&self.host_name, &format!("native stdout error: {error}"));
            process::exit(1);
        }
    }

    fn send_client(&self, client_id: usize, message: &Value) {
        let Some(client) = self.clients.get(&client_id) else {
            return;
        };

        let mut writer = client.writer.lock().expect("client writer mutex poisoned");
        if let Err(error) = write_frame(&mut *writer, message) {
            log(
                &self.host_name,
                &format!("client socket write error: {error}"),
            );
        }
    }

    fn broadcast_clients(&self, message: &Value) {
        for client_id in self.clients.keys().copied().collect::<Vec<_>>() {
            self.send_client(client_id, message);
        }
    }
}

pub fn serve(host_name: String) -> Result<()> {
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
    let state = Arc::new(Mutex::new(HostState::new(host_name, stdout)));
    {
        let state = Arc::clone(&state);
        thread::spawn(move || accept_clients(listener, state));
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
        fs::metadata(path).with_context(|| format!("failed to stat {}", path.display()))?;
    if !metadata.is_dir() {
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
            state.replace_with_client(writer)
        };
        for (evicted_id, evicted_client) in evicted_clients {
            log(
                &get_host_name(&state),
                &format!(
                    "evicting stale browser client {evicted_id} after a newer client connected"
                ),
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
    if is_response(&message) {
        let Some(id) = message_id_as_string(&message) else {
            return;
        };

        let mut state = state.lock().expect("host state mutex poisoned");
        let Some(pending) = state.pending_client_requests.get(&id).cloned() else {
            return;
        };
        if pending.client_id != client_id {
            return;
        }
        state.pending_client_requests.remove(&id);

        state.send_chrome(&with_id(message, pending.chrome_request_id));
        return;
    }

    if !is_request(&message) {
        let state = state.lock().expect("host state mutex poisoned");
        if state.clients.contains_key(&client_id) {
            state.send_chrome(&message);
        }
        return;
    }

    if message.get("method").and_then(Value::as_str) == Some("ping") {
        let Some(id) = message.get("id").cloned() else {
            return;
        };
        let state = state.lock().expect("host state mutex poisoned");
        state.send_client(
            client_id,
            &json!({ "jsonrpc": "2.0", "id": id, "result": "pong" }),
        );
        return;
    }

    let Some(client_request_id) = message.get("id").cloned() else {
        return;
    };

    let mut state = state.lock().expect("host state mutex poisoned");
    if !state.clients.contains_key(&client_id) {
        return;
    }
    let chrome_id = format!("linux-{}-{}", process::id(), state.next_chrome_id);
    state.next_chrome_id += 1;
    state.cleanup_old_requests();
    state.pending_chrome_requests.insert(
        chrome_id.clone(),
        PendingChromeRequest {
            client_id,
            client_request_id,
            created_at: Instant::now(),
        },
    );
    state.send_chrome(&with_id(message, Value::String(chrome_id)));
}

fn handle_chrome_message(state: &SharedState, message: Value) {
    if is_response(&message) {
        let Some(id) = message_id_as_string(&message) else {
            return;
        };

        let mut state = state.lock().expect("host state mutex poisoned");
        let Some(pending) = state.pending_chrome_requests.remove(&id) else {
            return;
        };

        state.send_client(
            pending.client_id,
            &with_id(message, pending.client_request_id),
        );
        return;
    }

    if !is_request(&message) {
        let state = state.lock().expect("host state mutex poisoned");
        state.broadcast_clients(&message);
        return;
    }

    let chrome_request_id = message.get("id").cloned().unwrap_or(Value::Null);
    let mut state = state.lock().expect("host state mutex poisoned");
    let client_id = match select_single_client_id(&state.clients) {
        Ok(client_id) => client_id,
        Err(error) => {
            state.send_chrome(&json!({
                "jsonrpc": "2.0",
                "id": chrome_request_id,
                "error": {
                    "code": -32000,
                    "message": error.message()
                }
            }));
            return;
        }
    };

    let client_request_id = format!("chrome-{}-{}", process::id(), state.next_client_request_id);
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
    state.send_client(
        client_id,
        &with_id(message, Value::String(client_request_id)),
    );
}

fn select_single_client_id(
    clients: &HashMap<usize, Client>,
) -> std::result::Result<usize, ChromeClientRouteError> {
    match clients.len() {
        0 => Err(ChromeClientRouteError::NoClients),
        1 => Ok(*clients.keys().next().expect("one client id")),
        _ => Err(ChromeClientRouteError::MultipleClients),
    }
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

fn log(host_name: &str, message: &str) {
    let _ = writeln!(io::stderr(), "[{host_name}] {message}");
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
    fn rejects_chrome_request_routing_without_exactly_one_client() {
        let clients = HashMap::new();
        assert_eq!(
            select_single_client_id(&clients),
            Err(ChromeClientRouteError::NoClients)
        );

        let mut clients = HashMap::new();
        clients.insert(7, test_client());
        assert_eq!(select_single_client_id(&clients), Ok(7));

        clients.insert(8, test_client());
        assert_eq!(
            select_single_client_id(&clients),
            Err(ChromeClientRouteError::MultipleClients)
        );
    }

    #[test]
    fn replacing_browser_client_evicts_stale_clients_and_pending_requests() {
        let mut state = test_host_state();

        let (first_client_id, evicted_clients) =
            state.replace_with_client(test_client().writer.clone());
        assert!(evicted_clients.is_empty());
        assert!(state.clients.contains_key(&first_client_id));

        state.pending_chrome_requests.insert(
            "chrome-request".to_string(),
            PendingChromeRequest {
                client_id: first_client_id,
                client_request_id: json!("client-request-1"),
                created_at: Instant::now(),
            },
        );
        state.pending_client_requests.insert(
            "client-request".to_string(),
            PendingClientRequest {
                client_id: first_client_id,
                chrome_request_id: json!("chrome-request-1"),
                created_at: Instant::now(),
            },
        );

        let (second_client_id, evicted_clients) =
            state.replace_with_client(test_client().writer.clone());

        assert_ne!(first_client_id, second_client_id);
        assert_eq!(evicted_clients.len(), 1);
        assert_eq!(evicted_clients[0].0, first_client_id);
        assert!(!state.clients.contains_key(&first_client_id));
        assert!(state.clients.contains_key(&second_client_id));
        assert!(state.pending_chrome_requests.is_empty());
        assert!(state.pending_client_requests.is_empty());
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
    fn disconnect_cleanup_removes_pending_state_for_client() {
        let mut pending_chrome = HashMap::from([
            (
                "keep".to_string(),
                PendingChromeRequest {
                    client_id: 1,
                    client_request_id: json!("chrome-request-1"),
                    created_at: Instant::now(),
                },
            ),
            (
                "drop".to_string(),
                PendingChromeRequest {
                    client_id: 2,
                    client_request_id: json!("chrome-request-2"),
                    created_at: Instant::now(),
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

    fn test_client() -> Client {
        let (stream, _peer) = UnixStream::pair().unwrap();
        Client {
            writer: Arc::new(Mutex::new(stream)),
        }
    }

    fn test_host_state() -> HostState {
        HostState::new(
            "com.openai.codexextension",
            Arc::new(Mutex::new(io::stdout())),
        )
    }
}
