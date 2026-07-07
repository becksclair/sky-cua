use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use sky_cua_platform::model::{
    BrowserClaimTabResponse, BrowserListTabsResponse, BrowserMoveMouseResponse,
    BrowserOpenResponse, BrowserTab, BrowserTargetKind, DiagnosticEntry,
};
use tokio::net::UnixStream;
use tokio::time::Instant as TokioInstant;

use super::affinity;
use super::cdp::{self, BrowserCdpAction, BrowserCdpResult};
use super::diagnostics::browser_open_timeout_diagnostic;
use super::probe::{list_tabs_from_sockets, run_on_responsive_bridge_socket_until};
use super::session;
use super::sockets::{
    BrowserSocketSelection, browser_bridge_disconnected_for_selection,
    browser_socket_selection_from_env, find_bridge_sockets,
};
use super::tabs::tab_id_value;

pub(super) struct BrowserBridgeExecutor {
    sockets: Vec<PathBuf>,
    deadline: TokioInstant,
}

impl BrowserBridgeExecutor {
    pub(super) fn from_env(deadline: TokioInstant) -> Result<Self, DiagnosticEntry> {
        // Any browser-bridge use starts the heartbeat keepalive (once) so the
        // extension's 30s driver-liveness ping is answered and it stops
        // detaching chrome.debugger from our tabs mid-session.
        super::keepalive::ensure_spawned();
        let selection = browser_socket_selection_from_env()?;
        Self::from_selection(selection, deadline)
    }

    fn from_selection(
        selection: BrowserSocketSelection,
        deadline: TokioInstant,
    ) -> Result<Self, DiagnosticEntry> {
        let sockets = find_bridge_sockets(selection);
        if sockets.is_empty() {
            return Err(browser_bridge_disconnected_for_selection(selection));
        }

        Ok(Self { sockets, deadline })
    }

    pub(super) async fn list_tabs(
        &self,
        target: Option<BrowserTargetKind>,
    ) -> BrowserListTabsResponse {
        let results = list_tabs_from_sockets(self.sockets.clone(), target).await;
        record_listed_tab_affinities(&results);
        let mut tabs = Vec::new();
        let mut diagnostics = Vec::new();
        let mut connected_any = false;
        for (_, result) in results {
            match result {
                Ok(mut socket_tabs) => {
                    connected_any = true;
                    tabs.append(&mut socket_tabs);
                }
                Err(diagnostic) => diagnostics.push(diagnostic),
            }
        }
        if connected_any {
            diagnostics.clear();
        }

        BrowserListTabsResponse {
            target,
            tabs,
            diagnostics,
        }
    }

    pub(super) async fn open_tab(
        &self,
        target: BrowserTargetKind,
        url: Option<&str>,
    ) -> Result<BrowserOpenResponse, DiagnosticEntry> {
        self.run_on_responsive_socket(|socket, stream| async move {
            let response =
                session::open_tab_from_socket(&socket, target, url, self.deadline, stream).await?;
            if let Some(tab) = &response.tab {
                affinity::record_tab_socket(&tab.tab_id, &socket);
            }
            Ok(response)
        })
        .await
    }

    pub(super) async fn claim_tab(
        &self,
        target: BrowserTargetKind,
        tab_id: &str,
    ) -> Result<BrowserClaimTabResponse, DiagnosticEntry> {
        let sockets = self.sockets_for_tab(tab_id);
        let terminal = TerminalDiagnostic::default();
        run_on_responsive_bridge_socket_until(sockets, self.deadline, |socket, stream| {
            let terminal = &terminal;
            async move {
                if let Some(diagnostic) = terminal.get() {
                    return Err(diagnostic);
                }
                let result =
                    session::claim_tab_from_socket(&socket, target, tab_id, self.deadline, stream)
                        .await;
                match result {
                    Ok(response) => {
                        if response.tab.is_some() {
                            affinity::record_tab_socket(tab_id, &socket);
                        }
                        Ok(response)
                    }
                    Err(diagnostic) => {
                        // Claiming mutates session ownership, so it is never
                        // safe to retry on another socket.
                        Err(terminal.classify(tab_id, &socket, false, diagnostic))
                    }
                }
            }
        })
        .await
        .map_err(|diagnostic| terminal.resolve(diagnostic))
    }

    /// Route a tab-bound request to the socket known to own the tab; only an
    /// unknown tab falls back to probing every candidate socket.
    fn sockets_for_tab(&self, tab_id: &str) -> Vec<PathBuf> {
        match affinity::tab_socket_affinity(tab_id, &self.sockets) {
            Some(socket) => vec![socket],
            None => self.sockets.clone(),
        }
    }

    pub(super) fn bind_tab<'a>(
        &'a self,
        target: BrowserTargetKind,
        tab_id: &'a str,
    ) -> BrowserSessionBinding<'a> {
        BrowserSessionBinding {
            executor: self,
            target,
            tab_id,
        }
    }

    async fn run_on_responsive_socket<T, F, Fut>(&self, action: F) -> Result<T, DiagnosticEntry>
    where
        F: FnMut(PathBuf, UnixStream) -> Fut,
        Fut: std::future::Future<Output = Result<T, DiagnosticEntry>>,
    {
        run_on_responsive_bridge_socket_until(self.sockets.clone(), self.deadline, action).await
    }
}

pub(super) struct BrowserSessionBinding<'a> {
    executor: &'a BrowserBridgeExecutor,
    target: BrowserTargetKind,
    tab_id: &'a str,
}

impl BrowserSessionBinding<'_> {
    pub(super) async fn run_cdp(
        &self,
        action: BrowserCdpAction,
    ) -> Result<BrowserCdpResult, DiagnosticEntry> {
        match self
            .run_operation(BoundTabOperation::Cdp { action: &action })
            .await?
        {
            BoundTabResult::Cdp(result) => Ok(result),
            BoundTabResult::MoveMouse => unreachable!("CDP operation returns CDP result"),
        }
    }

    pub(super) async fn move_mouse(
        &self,
        x: f64,
        y: f64,
        wait_for_arrival: bool,
    ) -> Result<BrowserMoveMouseResponse, DiagnosticEntry> {
        match self
            .run_operation(BoundTabOperation::MoveMouse {
                x,
                y,
                wait_for_arrival,
            })
            .await?
        {
            BoundTabResult::MoveMouse => Ok(BrowserMoveMouseResponse {
                target: self.target,
                tab_id: self.tab_id.to_string(),
                x,
                y,
                wait_for_arrival,
                diagnostics: Vec::new(),
            }),
            BoundTabResult::Cdp(_) => {
                unreachable!("move-mouse operation returns move-mouse result")
            }
        }
    }

    async fn run_operation(
        &self,
        operation: BoundTabOperation<'_>,
    ) -> Result<BoundTabResult, DiagnosticEntry> {
        let sockets = self.executor.sockets_for_tab(self.tab_id);
        let terminal = TerminalDiagnostic::default();
        run_on_responsive_bridge_socket_until(sockets, self.executor.deadline, |socket, stream| {
            let terminal = &terminal;
            async move {
                if let Some(diagnostic) = terminal.get() {
                    return Err(diagnostic);
                }
                match self
                    .run_operation_on_socket(&socket, stream, operation)
                    .await
                {
                    Ok(result) => {
                        affinity::record_tab_socket(self.tab_id, &socket);
                        Ok(result)
                    }
                    Err(diagnostic) => Err(terminal.classify(
                        self.tab_id,
                        &socket,
                        operation.replay_safe(),
                        diagnostic,
                    )),
                }
            }
        })
        .await
        .map_err(|diagnostic| terminal.resolve(diagnostic))
    }

    async fn run_operation_on_socket(
        &self,
        socket: &Path,
        mut stream: UnixStream,
        operation: BoundTabOperation<'_>,
    ) -> Result<BoundTabResult, DiagnosticEntry> {
        let tab_id = tab_id_value(self.tab_id);
        let result = operation
            .run(&mut stream, socket, &tab_id, self.executor.deadline)
            .await;
        match result {
            Ok(result) => Ok(result),
            Err(diagnostic) if session::is_recoverable_cdp_session_diagnostic(&diagnostic) => {
                session::recover_cdp_session_until(
                    &mut stream,
                    socket,
                    &tab_id,
                    self.executor.deadline,
                    session::is_cdp_command_timeout_diagnostic(&diagnostic),
                )
                .await?;
                // A recoverable failure can arrive part-way through a
                // multi-command operation: a click dispatches mouseMoved ->
                // mousePressed -> mouseReleased on one stream, so a timeout,
                // "Detached while handling command", or unattached error on a
                // later sub-command can follow an earlier one that already
                // landed in the page. The session reset above heals the wedged
                // debugger session, but replaying the whole operation would
                // re-dispatch the sub-commands that already executed (double
                // click, double keystroke, double submit). Replay is therefore
                // only safe when the operation cannot mutate the page twice;
                // every mutating class is surfaced as reset-not-replayed no
                // matter which recoverable error tripped it — the earlier
                // timeout-only gate let a mid-sequence detach double-apply.
                if !operation.replay_safe() {
                    return Err(DiagnosticEntry {
                        details: Some(
                            "The tab's debugger session was reset, but the command was not \
                             replayed because an earlier step may have already taken effect. \
                             If browser actions keep failing this way the extension's debugger \
                             relay is likely wedged; use desktop-control tools for this tab."
                                .to_string(),
                        ),
                        ..diagnostic
                    });
                }
                operation
                    .run(&mut stream, socket, &tab_id, self.executor.deadline)
                    .await
            }
            Err(diagnostic) => Err(diagnostic),
        }
    }
}

/// Stops the responsive-socket loop from retrying a tab-bound request on
/// another bridge socket. Tab ids are per-browser integers, so a mutating
/// request that failed for any reason other than `No tab with id` must not
/// move to another socket: it may already have engaged the tab (or a
/// colliding one), and retrying elsewhere risks driving an unrelated tab
/// that happens to share the id. Read-only operations are exempt — retrying
/// them on another socket cannot double-apply anything.
#[derive(Default)]
struct TerminalDiagnostic(OnceLock<DiagnosticEntry>);

impl TerminalDiagnostic {
    fn get(&self) -> Option<DiagnosticEntry> {
        self.0.get().cloned()
    }

    /// The terminal diagnostic is authoritative for the whole call; an
    /// earlier non-owner's "No tab with id" pushed first by the socket loop
    /// must not shadow it in the surfaced error.
    fn resolve(&self, fallback: DiagnosticEntry) -> DiagnosticEntry {
        self.get().unwrap_or(fallback)
    }

    /// Record `diagnostic` as terminal unless it proves the tab is simply
    /// not on `socket` or the operation is `safe_elsewhere` (read-only, so
    /// a cross-socket retry cannot mutate twice). A tab-not-found answer
    /// from the recorded owner also drops the affinity entry so the next
    /// call rediscovers the owner; a not-found from any other socket leaves
    /// the mapping alone — it says nothing about the owner.
    fn classify(
        &self,
        tab_id: &str,
        socket: &Path,
        safe_elsewhere: bool,
        diagnostic: DiagnosticEntry,
    ) -> DiagnosticEntry {
        if session::is_tab_not_found_diagnostic(&diagnostic) {
            affinity::forget_tab_socket_if_owner(tab_id, socket);
        } else if !safe_elsewhere {
            let _ = self.0.set(diagnostic.clone());
        }
        diagnostic
    }
}

/// Record which socket owns each listed tab. A tab id reported by more than
/// one socket is genuinely ambiguous (per-browser ids can collide), so its
/// mapping is dropped rather than guessed.
fn record_listed_tab_affinities(results: &[(PathBuf, Result<Vec<BrowserTab>, DiagnosticEntry>)]) {
    let mut owners: HashMap<&str, Option<&Path>> = HashMap::new();
    for (socket, result) in results {
        let Ok(tabs) = result else { continue };
        // The listing is the authoritative live set for this socket: entries
        // it owns for tabs that no longer appear were closed and would
        // otherwise never be pruned while the browser stays connected.
        let live_tab_ids: std::collections::HashSet<&str> =
            tabs.iter().map(|tab| tab.tab_id.as_str()).collect();
        affinity::retain_socket_tabs(socket, &live_tab_ids);
        for tab in tabs {
            owners
                .entry(tab.tab_id.as_str())
                .and_modify(|owner| {
                    if *owner != Some(socket.as_path()) {
                        *owner = None;
                    }
                })
                .or_insert(Some(socket.as_path()));
        }
    }
    for (tab_id, owner) in owners {
        match owner {
            Some(socket) => affinity::record_tab_socket(tab_id, socket),
            None => affinity::forget_tab_socket(tab_id),
        }
    }
}

#[derive(Clone, Copy)]
enum BoundTabOperation<'a> {
    Cdp {
        action: &'a BrowserCdpAction,
    },
    MoveMouse {
        x: f64,
        y: f64,
        wait_for_arrival: bool,
    },
}

enum BoundTabResult {
    Cdp(BrowserCdpResult),
    MoveMouse,
}

impl BoundTabOperation<'_> {
    /// Whether the operation can run twice without mutating the page twice.
    /// Snapshot and screenshot only read; cursor moves are absolute, so a
    /// repeat lands on the same position. Input dispatch, navigation, eval,
    /// and scroll all compound when replayed.
    fn replay_safe(self) -> bool {
        match self {
            BoundTabOperation::Cdp { action } => matches!(
                action,
                BrowserCdpAction::Snapshot { .. } | BrowserCdpAction::Screenshot
            ),
            BoundTabOperation::MoveMouse { .. } => true,
        }
    }

    async fn run(
        self,
        stream: &mut UnixStream,
        socket: &Path,
        tab_id: &serde_json::Value,
        deadline: TokioInstant,
    ) -> Result<BoundTabResult, DiagnosticEntry> {
        if deadline <= TokioInstant::now() {
            return Err(browser_open_timeout_diagnostic());
        }

        match self {
            BoundTabOperation::Cdp { action } => {
                cdp::cdp_action_on_stream(stream, socket, tab_id, action, deadline)
                    .await
                    .map(BoundTabResult::Cdp)
            }
            BoundTabOperation::MoveMouse {
                x,
                y,
                wait_for_arrival,
            } => session::move_mouse_on_stream(
                stream,
                socket,
                tab_id,
                x,
                y,
                wait_for_arrival,
                deadline,
            )
            .await
            .map(|()| BoundTabResult::MoveMouse),
        }
    }
}
