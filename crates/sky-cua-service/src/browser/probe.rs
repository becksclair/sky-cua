use std::path::{Path, PathBuf};
use std::sync::Arc;

use sky_cua_platform::model::{
    BrowserSessionIdentity, BrowserTab, BrowserTargetKind, DiagnosticEntry,
};
use tokio::net::UnixStream;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tokio::time::Instant as TokioInstant;

use super::diagnostics::{browser_bridge_disconnected_diagnostic, browser_open_timeout_diagnostic};
use super::protocol::{BRIDGE_INFO_REQUEST_ID, LIST_TABS_REQUEST_ID};
use super::sockets::record_bridge_socket_result;
use super::tabs::parse_list_tabs_response;
use super::transport::{
    browser_session_params, connect_bridge_socket, list_tabs_method, send_bridge_request,
};

const MAX_CONCURRENT_BRIDGE_SOCKET_PROBES: usize = 8;
type BridgeProbeResult = Result<UnixStream, DiagnosticEntry>;
type BridgeProbeTaskResult = (usize, PathBuf, BridgeProbeResult);

pub(super) async fn list_tabs_from_sockets(
    sockets: Vec<PathBuf>,
    target: Option<BrowserTargetKind>,
    identity: &BrowserSessionIdentity,
) -> Vec<(PathBuf, Result<Vec<BrowserTab>, DiagnosticEntry>)> {
    let mut tasks = JoinSet::new();
    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_BRIDGE_SOCKET_PROBES));
    let proxy = super::control_plane::capture_persistent_proxy();
    for (index, socket) in sockets.into_iter().enumerate() {
        let semaphore = Arc::clone(&semaphore);
        let identity = identity.clone();
        let proxy = proxy.clone();
        tasks.spawn(async move {
            super::control_plane::scope_persistent_proxy(proxy, async move {
                let _permit = semaphore.acquire_owned().await.expect("semaphore is open");
                let result = list_tabs_from_socket(&socket, target, &identity).await;
                (index, socket, result)
            })
            .await
        });
    }

    let mut results = Vec::new();
    while let Some(joined) = tasks.join_next().await {
        if let Ok(result) = joined {
            results.push(result);
        }
    }
    results.sort_by_key(|(index, _, _)| *index);
    for (_, socket, result) in &results {
        record_bridge_socket_result(socket, result.as_ref());
    }
    results
        .into_iter()
        .map(|(_, socket, result)| (socket, result))
        .collect()
}

pub(super) async fn first_responsive_bridge_socket(
    sockets: Vec<PathBuf>,
) -> Result<PathBuf, DiagnosticEntry> {
    let mut probes = BridgeProbeCollector::new(sockets);
    let mut diagnostics = Vec::new();

    while let Some((socket, result)) = probes.next_completed().await {
        match result {
            Ok(_stream) => return Ok(socket),
            Err(diagnostic) => diagnostics.push(diagnostic),
        }
    }

    Err(diagnostics
        .into_iter()
        .next()
        .unwrap_or_else(browser_bridge_disconnected_diagnostic))
}

pub(super) async fn run_on_responsive_bridge_socket_until<T, F, Fut>(
    sockets: Vec<PathBuf>,
    deadline: TokioInstant,
    mut action: F,
) -> Result<T, DiagnosticEntry>
where
    F: FnMut(PathBuf, UnixStream) -> Fut,
    Fut: std::future::Future<Output = Result<T, DiagnosticEntry>>,
{
    let mut probes = BridgeProbeCollector::new(sockets);
    let mut probe_diagnostics = Vec::new();
    let mut action_diagnostics = Vec::new();
    let mut responsive_any = false;

    while let Some((socket, result)) = probes.next_completed_until(deadline).await? {
        match result {
            Ok(stream) => {
                responsive_any = true;
                match action(socket, stream).await {
                    Ok(response) => return Ok(response),
                    Err(diagnostic) => action_diagnostics.push(diagnostic),
                }
            }
            Err(diagnostic) => probe_diagnostics.push(diagnostic),
        }
    }

    if responsive_any {
        Err(action_diagnostics
            .into_iter()
            .next()
            .unwrap_or_else(browser_bridge_disconnected_diagnostic))
    } else {
        Err(probe_diagnostics
            .into_iter()
            .next()
            .unwrap_or_else(browser_bridge_disconnected_diagnostic))
    }
}

struct BridgeProbeCollector {
    tasks: JoinSet<BridgeProbeTaskResult>,
}

impl BridgeProbeCollector {
    fn new(sockets: Vec<PathBuf>) -> Self {
        Self {
            tasks: spawn_bridge_probe_tasks(sockets),
        }
    }

    async fn next_completed(&mut self) -> Option<(PathBuf, BridgeProbeResult)> {
        while let Some(joined) = self.tasks.join_next().await {
            if let Ok((_, socket, result)) = joined {
                record_bridge_socket_result(&socket, result.as_ref());
                return Some((socket, result));
            }
        }
        None
    }

    async fn next_completed_until(
        &mut self,
        deadline: TokioInstant,
    ) -> Result<Option<(PathBuf, BridgeProbeResult)>, DiagnosticEntry> {
        let remaining = deadline
            .checked_duration_since(TokioInstant::now())
            .ok_or_else(browser_open_timeout_diagnostic)?;
        tokio::time::timeout(remaining, self.next_completed())
            .await
            .map_err(|_| browser_open_timeout_diagnostic())
    }
}

fn spawn_bridge_probe_tasks(sockets: Vec<PathBuf>) -> JoinSet<BridgeProbeTaskResult> {
    let mut tasks = JoinSet::new();
    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_BRIDGE_SOCKET_PROBES));
    let proxy = super::control_plane::capture_persistent_proxy();
    for (index, socket) in sockets.into_iter().enumerate() {
        let semaphore = Arc::clone(&semaphore);
        let proxy = proxy.clone();
        tasks.spawn(async move {
            super::control_plane::scope_persistent_proxy(proxy, async move {
                let _permit = semaphore.acquire_owned().await.expect("semaphore is open");
                let result = probe_bridge_socket(&socket).await;
                (index, socket, result)
            })
            .await
        });
    }
    tasks
}

async fn probe_bridge_socket(socket: &Path) -> Result<UnixStream, DiagnosticEntry> {
    let mut stream = connect_bridge_socket(socket).await?;
    let identity = BrowserSessionIdentity {
        session_id: "sky-cua-mcp".to_string(),
        turn_id: "browser-probe".to_string(),
        thread_id: None,
    };
    send_bridge_request(
        &mut stream,
        socket,
        BRIDGE_INFO_REQUEST_ID,
        "getInfo",
        browser_session_params(&identity),
    )
    .await?;
    Ok(stream)
}

async fn list_tabs_from_socket(
    socket: &Path,
    target: Option<BrowserTargetKind>,
    identity: &BrowserSessionIdentity,
) -> Result<Vec<BrowserTab>, DiagnosticEntry> {
    let mut stream = connect_bridge_socket(socket).await?;
    let response = send_bridge_request(
        &mut stream,
        socket,
        LIST_TABS_REQUEST_ID,
        list_tabs_method(),
        browser_session_params(identity),
    )
    .await?;
    parse_list_tabs_response(response.get("result"), target)
}
