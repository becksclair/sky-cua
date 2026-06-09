use std::path::{Path, PathBuf};

use sky_cua_platform::model::{
    BrowserClaimTabResponse, BrowserListTabsResponse, BrowserMoveMouseResponse,
    BrowserOpenResponse, BrowserTargetKind, DiagnosticEntry,
};
use tokio::net::UnixStream;
use tokio::time::Instant as TokioInstant;

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
            session::open_tab_from_socket(&socket, target, url, self.deadline, stream).await
        })
        .await
    }

    pub(super) async fn claim_tab(
        &self,
        target: BrowserTargetKind,
        tab_id: &str,
    ) -> Result<BrowserClaimTabResponse, DiagnosticEntry> {
        self.run_on_responsive_socket(|socket, stream| async move {
            session::claim_tab_from_socket(&socket, target, tab_id, self.deadline, stream).await
        })
        .await
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
        self.executor
            .run_on_responsive_socket(|socket, stream| async move {
                self.run_operation_on_socket(&socket, stream, operation)
                    .await
            })
            .await
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
                )
                .await?;
                operation
                    .run(&mut stream, socket, &tab_id, self.executor.deadline)
                    .await
            }
            Err(diagnostic) => Err(diagnostic),
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
