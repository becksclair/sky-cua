use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use super::{AppShotEnvelope, AppShotRequired};
use super::{BrowserIntegrationReport, DiagnosticEntry};

/// Identity shared with Codex Browser Use for browser-tab ownership.
///
/// Codex supplies this per MCP tool call. Non-Codex clients omit it and the
/// service uses its legacy standalone identity instead.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserSessionIdentity {
    pub session_id: String,
    pub turn_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
}

/// Normalized caller lane for a browser request entering through MCP.
///
/// The installer declaration is advisory same-user provenance, not an
/// authorization boundary. Unknown declarations deliberately collapse to
/// [`BrowserCallerKind::LegacyUnknown`] instead of becoming arbitrary
/// identities.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BrowserCallerKind {
    CodexDesktop,
    CodexCli,
    OpenClaw,
    OpenCode,
    Pi,
    DirectMcp,
    LegacyUnknown,
}

impl BrowserCallerKind {
    /// Normalize a declared MCP/native-pipe caller label to a bounded set of
    /// ownership lanes. Punctuation and ASCII case are insignificant; unknown
    /// labels return `None` rather than becoming arbitrary principal IDs.
    #[must_use]
    pub fn from_provenance_label(value: &str) -> Option<Self> {
        let normalized: String = value
            .trim()
            .chars()
            .filter(|character| character.is_ascii_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect();
        match normalized.as_str() {
            "codexdesktop" | "chatgptdesktop" => Some(Self::CodexDesktop),
            "codex" | "codexcli" => Some(Self::CodexCli),
            "openclaw" => Some(Self::OpenClaw),
            "opencode" => Some(Self::OpenCode),
            "pi" | "piagent" | "pimcpadapter" => Some(Self::Pi),
            "generic" | "genericmcp" | "direct" | "directmcp" => Some(Self::DirectMcp),
            _ => None,
        }
    }
}

/// How the normalized caller lane was obtained.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BrowserProvenanceSource {
    InstallerDeclaration,
    RequestMetadataDeclaration,
    ClientInfoInference,
    TrustedCodexMetadata,
    HostProvidedIab,
    LegacyFallback,
}

impl BrowserProvenanceSource {
    /// Return the protocol-v1 value that older readers can decode.
    ///
    /// The exact value is emitted separately as a forward-compatible detail
    /// when this fallback differs. That keeps rollback readers operational
    /// without discarding the truthful source for current readers.
    pub(super) const fn v1_wire_fallback(self) -> Self {
        match self {
            Self::RequestMetadataDeclaration => Self::InstallerDeclaration,
            Self::HostProvidedIab => Self::TrustedCodexMetadata,
            source => source,
        }
    }

    pub(super) const fn v1_wire_detail(self) -> Option<Self> {
        match self {
            Self::RequestMetadataDeclaration | Self::HostProvidedIab => Some(self),
            _ => None,
        }
    }
}

/// MCP `initialize.clientInfo`, retained independently from caller
/// classification so a host's self-reported name never changes provenance.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserMcpClientInfo {
    pub name: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

/// Stable provenance for the lifetime of one MCP connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserCallerProvenance {
    pub caller: BrowserCallerKind,
    pub source: BrowserProvenanceSource,
    pub connection_id: String,
    pub declared_caller: Option<String>,
    pub client_info: Option<BrowserMcpClientInfo>,
}

#[derive(Serialize)]
struct BrowserCallerProvenanceRef<'a> {
    caller: BrowserCallerKind,
    source: BrowserProvenanceSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_detail: Option<BrowserProvenanceSource>,
    connection_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    declared_caller: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    client_info: Option<&'a BrowserMcpClientInfo>,
}

#[derive(Deserialize)]
struct BrowserCallerProvenanceOwned {
    caller: BrowserCallerKind,
    source: BrowserProvenanceSource,
    #[serde(default)]
    source_detail: Option<BrowserProvenanceSource>,
    connection_id: String,
    #[serde(default)]
    declared_caller: Option<String>,
    #[serde(default)]
    client_info: Option<BrowserMcpClientInfo>,
}

impl Serialize for BrowserCallerProvenance {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        BrowserCallerProvenanceRef {
            caller: self.caller,
            source: self.source.v1_wire_fallback(),
            source_detail: self.source.v1_wire_detail(),
            connection_id: &self.connection_id,
            declared_caller: self.declared_caller.as_deref(),
            client_info: self.client_info.as_ref(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for BrowserCallerProvenance {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = BrowserCallerProvenanceOwned::deserialize(deserializer)?;
        Ok(Self {
            caller: wire.caller,
            source: wire.source_detail.unwrap_or(wire.source),
            connection_id: wire.connection_id,
            declared_caller: wire.declared_caller,
            client_info: wire.client_info,
        })
    }
}

/// Logical agent attribution, independent from transport and operation IDs.
///
/// Codex supplies session/thread/turn values. Other and malformed legacy MCP
/// callers receive a stable connection-scoped session and omit turn fields.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserLogicalIdentity {
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
}

/// Retry-stable identity for one MCP `tools/call` operation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserOperationIdentity {
    pub operation_id: String,
    pub request_id_fingerprint: String,
}

/// Browser caller context propagated alongside the legacy session identity.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserRequestContext {
    pub provenance: BrowserCallerProvenance,
    pub logical_identity: BrowserLogicalIdentity,
    pub operation_identity: BrowserOperationIdentity,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BrowserRequest {
    Status,
    ListTabs {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<BrowserTargetKind>,
    },
    Open {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<BrowserTargetKind>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        url: Option<String>,
    },
    ClaimTab {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<BrowserTargetKind>,
        tab_id: String,
    },
    MoveMouse {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<BrowserTargetKind>,
        tab_id: String,
        x: f64,
        y: f64,
        #[serde(default = "default_wait_for_arrival")]
        wait_for_arrival: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        appshot_id: Option<String>,
    },
    Navigate {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<BrowserTargetKind>,
        tab_id: String,
        url: String,
    },
    Snapshot {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<BrowserTargetKind>,
        tab_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text_limit: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        element_offset: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        element_limit: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        element_query: Option<String>,
    },
    ObserveAppShot {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<BrowserTargetKind>,
        tab_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text_limit: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        element_offset: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        element_limit: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        element_query: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        capture_timeout_ms: Option<u64>,
        #[serde(
            default = "default_include_image_data",
            skip_serializing_if = "is_true"
        )]
        include_image_data: bool,
    },
    Screenshot {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<BrowserTargetKind>,
        tab_id: String,
        #[serde(
            default = "default_include_image_data",
            skip_serializing_if = "is_true"
        )]
        include_image_data: bool,
    },
    Click {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<BrowserTargetKind>,
        tab_id: String,
        x: f64,
        y: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        appshot_id: Option<String>,
    },
    /// Click an element by an opaque reference obtained from a browser
    /// snapshot, rather than by CSS-pixel coordinates. The service re-resolves
    /// the element's live position at click time; see the browser `resolve`
    /// module. `element_ref` is the opaque token the snapshot emitted; the
    /// client never parses it.
    ClickElement {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<BrowserTargetKind>,
        tab_id: String,
        element_ref: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        appshot_id: Option<String>,
    },
    TypeText {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<BrowserTargetKind>,
        tab_id: String,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        appshot_id: Option<String>,
    },
    /// Focus an element by an opaque snapshot reference and type into it in one
    /// step (see [`BrowserRequest::ClickElement`] for the reference contract).
    TypeTextElement {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<BrowserTargetKind>,
        tab_id: String,
        element_ref: String,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        appshot_id: Option<String>,
    },
    PressKey {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<BrowserTargetKind>,
        tab_id: String,
        key: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        appshot_id: Option<String>,
    },
    Scroll {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<BrowserTargetKind>,
        tab_id: String,
        delta_x: f64,
        delta_y: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        x: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        y: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        appshot_id: Option<String>,
    },
    Eval {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<BrowserTargetKind>,
        tab_id: String,
        expression: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        appshot_id: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
#[expect(
    clippy::large_enum_variant,
    reason = "serialized IPC response variants keep their public payload shapes inline"
)]
pub enum BrowserResponse {
    Status { report: BrowserStatusReport },
    ListTabs { response: BrowserListTabsResponse },
    Open { response: BrowserOpenResponse },
    ClaimTab { response: BrowserClaimTabResponse },
    MoveMouse { response: BrowserMoveMouseResponse },
    Navigate { response: BrowserNavigateResponse },
    Snapshot { response: BrowserSnapshotResponse },
    AppShot { response: BrowserAppShotResponse },
    AppShotRequired { rejection: AppShotRequired },
    Screenshot { response: BrowserScreenshotResponse },
    Click { response: BrowserActionResponse },
    TypeText { response: BrowserActionResponse },
    PressKey { response: BrowserActionResponse },
    Scroll { response: BrowserActionResponse },
    Eval { response: BrowserEvalResponse },
}

impl BrowserRequest {
    /// Whether this request converges to the same state on repetition and is
    /// therefore safe for the client to retry after an ambiguous failure.
    ///
    /// Reads (`Status`, `ListTabs`, `Snapshot`, `Screenshot`) and `MoveMouse`
    /// (an absolute cursor-position set, analogous to `ActivateWindow`'s
    /// focus-set convergence) are idempotent. Tab creation/claiming,
    /// navigation, and every input action (click, type, key, scroll) compound
    /// on repetition, as does arbitrary `Eval` script execution, whose side
    /// effects cannot be assumed safe to repeat.
    #[must_use]
    pub fn is_idempotent(&self) -> bool {
        match self {
            Self::Status
            | Self::ListTabs { .. }
            | Self::Snapshot { .. }
            | Self::Screenshot { .. }
            | Self::ObserveAppShot { .. }
            | Self::MoveMouse { .. } => true,
            Self::Open { .. }
            | Self::ClaimTab { .. }
            | Self::Navigate { .. }
            | Self::Click { .. }
            | Self::ClickElement { .. }
            | Self::TypeText { .. }
            | Self::TypeTextElement { .. }
            | Self::PressKey { .. }
            | Self::Scroll { .. }
            | Self::Eval { .. } => false,
        }
    }
}

fn default_wait_for_arrival() -> bool {
    true
}

fn default_include_image_data() -> bool {
    true
}

fn is_true(value: &bool) -> bool {
    *value
}

/// Browser automation targets. `user_chrome` (the user's real, logged-in
/// Chrome-family browser) is the only target: managed/isolated browser
/// lifecycle was retired on 2026-06-11 because an isolated profile defeats
/// the purpose of real-browser control.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BrowserTargetKind {
    UserChrome,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserTargetAvailability {
    pub target: BrowserTargetKind,
    pub available: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserStatusReport {
    pub enabled: bool,
    pub available_targets: Vec<BrowserTargetAvailability>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tabs_known: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser_integration: Option<BrowserIntegrationReport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_plane: Option<Box<super::browser_control::BrowserControlPlaneSnapshot>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<DiagnosticEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserTab {
    pub tab_id: String,
    pub target: BrowserTargetKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default)]
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserListTabsResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<BrowserTargetKind>,
    pub tabs: Vec<BrowserTab>,
    /// Number of tabs in the logical result set before an explicit limit.
    /// The service reports all discovered tabs; the MCP boundary rewrites this
    /// after text filters. Older responses omit it and deserialize as zero.
    #[serde(default)]
    pub total: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<DiagnosticEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BrowserOpenResponse {
    pub target: BrowserTargetKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab: Option<BrowserTab>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destination_appshot: Option<Box<AppShotEnvelope>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<DiagnosticEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserClaimTabResponse {
    pub target: BrowserTargetKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab: Option<BrowserTab>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<DiagnosticEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BrowserMoveMouseResponse {
    pub target: BrowserTargetKind,
    pub tab_id: String,
    pub x: f64,
    pub y: f64,
    pub wait_for_arrival: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<DiagnosticEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BrowserNavigateResponse {
    pub target: BrowserTargetKind,
    pub tab_id: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destination_appshot: Option<Box<AppShotEnvelope>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<DiagnosticEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BrowserSnapshotResponse {
    pub target: BrowserTargetKind,
    pub tab_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<DiagnosticEntry>,
}

/// Browser AppShot transport response. The canonical envelope is structured
/// content; image bytes are carried only in the MCP content attachment path.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BrowserAppShotResponse {
    pub appshot: AppShotEnvelope,
    #[serde(default)]
    pub image_data_base64: String,
    #[serde(default)]
    pub image_mime_type: String,
}

/// Environment variable gating arbitrary page-JavaScript execution via
/// `browser_eval`. This is a security boundary the MCP client and the service
/// must agree on, so the opt-in check lives here and is shared by both rather
/// than duplicated per crate.
pub const BROWSER_EVAL_ENV: &str = "SKY_CUA_BROWSER_EVAL";

/// Default visible-text budget for MCP `browser_snapshot` calls. Direct
/// service callers may omit the field to request the service maximum.
pub const BROWSER_SNAPSHOT_DEFAULT_TEXT_LIMIT: usize = 4_000;

/// Default actionable-element budget for MCP `browser_snapshot` calls.
pub const BROWSER_SNAPSHOT_DEFAULT_ELEMENT_LIMIT: usize = 200;

/// Maximum actionable-element budget for browser snapshots. This preserves
/// the original service-side capture ceiling.
pub const BROWSER_SNAPSHOT_MAX_ELEMENT_LIMIT: usize = 5_000;

/// Maximum visible-text budget for browser snapshots across MCP and service
/// boundaries.
pub const BROWSER_SNAPSHOT_MAX_TEXT_LIMIT: usize = 20_000;

/// Minimum browser AppShot capture deadline accepted at MCP and service boundaries.
pub const BROWSER_APPSHOT_MIN_CAPTURE_TIMEOUT_MS: u64 = 1_000;

/// Maximum browser AppShot capture deadline accepted at MCP and service boundaries.
pub const BROWSER_APPSHOT_MAX_CAPTURE_TIMEOUT_MS: u64 = 30_000;

#[must_use]
pub const fn is_valid_browser_appshot_capture_timeout_ms(value: u64) -> bool {
    value >= BROWSER_APPSHOT_MIN_CAPTURE_TIMEOUT_MS
        && value <= BROWSER_APPSHOT_MAX_CAPTURE_TIMEOUT_MS
}

/// Whether `browser_eval` page-JavaScript execution is enabled. Enabled by
/// default; the operator turns it off with `SKY_CUA_BROWSER_EVAL` set to
/// `off`, `0`, or `false`. This is a security boundary the MCP client and the
/// service must agree on, so the check lives here and is shared by both.
pub fn browser_eval_enabled() -> bool {
    !matches!(
        std::env::var(BROWSER_EVAL_ENV)
            .map(|value| value.trim().to_ascii_lowercase())
            .as_deref(),
        Ok("off" | "0" | "false")
    )
}

pub fn browser_diagnostic_is_error_code(code: &str) -> bool {
    matches!(
        code,
        "BrowserBridgeDisconnected"
            | "BrowserBridgeUnsupported"
            | "BrowserBridgeRequestFailed"
            | "BrowserBridgeRequestTimedOut"
            | "BrowserSelectionInvalid"
            | "BrowserTabIdInvalid"
            | "BrowserMouseCoordinateInvalid"
            | "BrowserTextInvalid"
            | "BrowserKeyInvalid"
            | "BrowserScrollInvalid"
            | "BrowserOpenUrlUnsupported"
            | "BrowserNavigationFailed"
            | "BrowserOpenPartial"
            | "BrowserClaimPartial"
            | "BrowserElementUnresolved"
            | "BrowserElementNotActionable"
            | "BrowserEvalException"
            | "BrowserEvalDisabled"
    )
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BrowserScreenshotResponse {
    pub target: BrowserTargetKind,
    pub tab_id: String,
    pub mime_type: String,
    pub data_base64: String,
    /// Filesystem path of the persisted capture, when the service wrote one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screenshot_path: Option<String>,
    /// Image width in pixels. Matches CSS viewport width so image pixels,
    /// snapshot element bounds, and pointer coordinates share one space.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    /// Image height in pixels. See `width`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<DiagnosticEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BrowserActionResponse {
    pub target: BrowserTargetKind,
    pub tab_id: String,
    pub action: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<DiagnosticEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BrowserEvalResponse {
    pub target: BrowserTargetKind,
    pub tab_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<DiagnosticEntry>,
}

#[must_use]
pub fn normalize_browser_open_url(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()
        && (value == "about:blank"
            || value.starts_with("https://")
            || value.starts_with("http://")))
    .then(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::normalize_browser_open_url;
    use super::{
        BROWSER_APPSHOT_MAX_CAPTURE_TIMEOUT_MS, BROWSER_APPSHOT_MIN_CAPTURE_TIMEOUT_MS,
        BrowserRequest, BrowserStatusReport, BrowserTargetAvailability, BrowserTargetKind,
        is_valid_browser_appshot_capture_timeout_ms,
    };

    #[test]
    fn browser_request_idempotency_matches_the_classification_table() {
        let idempotent = [
            BrowserRequest::Status,
            BrowserRequest::ListTabs { target: None },
            BrowserRequest::Snapshot {
                target: Some(BrowserTargetKind::UserChrome),
                tab_id: "123".to_string(),
                text_limit: None,
                element_offset: None,
                element_limit: None,
                element_query: None,
            },
            BrowserRequest::Screenshot {
                target: Some(BrowserTargetKind::UserChrome),
                tab_id: "123".to_string(),
                include_image_data: true,
            },
            BrowserRequest::MoveMouse {
                target: Some(BrowserTargetKind::UserChrome),
                tab_id: "123".to_string(),
                x: 10.0,
                y: 10.0,
                wait_for_arrival: true,
                appshot_id: Some("shot-1".into()),
            },
        ];
        for request in idempotent {
            assert!(request.is_idempotent(), "expected idempotent: {request:?}");
        }

        let non_idempotent = [
            BrowserRequest::Open {
                target: Some(BrowserTargetKind::UserChrome),
                url: Some("https://example.test/".to_string()),
            },
            BrowserRequest::ClaimTab {
                target: Some(BrowserTargetKind::UserChrome),
                tab_id: "123".to_string(),
            },
            BrowserRequest::Navigate {
                target: Some(BrowserTargetKind::UserChrome),
                tab_id: "123".to_string(),
                url: "https://example.test/".to_string(),
            },
            BrowserRequest::Click {
                target: Some(BrowserTargetKind::UserChrome),
                tab_id: "123".to_string(),
                x: 10.0,
                y: 10.0,
                appshot_id: Some("shot-1".into()),
            },
            BrowserRequest::TypeText {
                target: Some(BrowserTargetKind::UserChrome),
                tab_id: "123".to_string(),
                text: "hello".to_string(),
                appshot_id: Some("shot-1".into()),
            },
            BrowserRequest::PressKey {
                target: Some(BrowserTargetKind::UserChrome),
                tab_id: "123".to_string(),
                key: "Enter".to_string(),
                appshot_id: Some("shot-1".into()),
            },
            BrowserRequest::Scroll {
                target: Some(BrowserTargetKind::UserChrome),
                tab_id: "123".to_string(),
                delta_x: 0.0,
                delta_y: 100.0,
                x: None,
                y: None,
                appshot_id: Some("shot-1".into()),
            },
            BrowserRequest::Eval {
                target: Some(BrowserTargetKind::UserChrome),
                tab_id: "123".to_string(),
                expression: "1 + 1".to_string(),
                appshot_id: Some("shot-1".into()),
            },
        ];
        for request in non_idempotent {
            assert!(
                !request.is_idempotent(),
                "expected non-idempotent: {request:?}"
            );
        }
    }

    #[test]
    fn browser_open_url_allows_only_http_https_and_about_blank() {
        assert_eq!(
            normalize_browser_open_url(" https://example.test/ "),
            Some("https://example.test/".to_string())
        );
        assert_eq!(
            normalize_browser_open_url("http://127.0.0.1:8080/page"),
            Some("http://127.0.0.1:8080/page".to_string())
        );
        assert_eq!(
            normalize_browser_open_url("about:blank"),
            Some("about:blank".to_string())
        );
        assert_eq!(normalize_browser_open_url(""), None);
        assert_eq!(normalize_browser_open_url("file:///etc/passwd"), None);
        assert_eq!(normalize_browser_open_url("javascript:alert(1)"), None);
    }

    #[test]
    fn legacy_browser_status_omits_control_plane_exactly() {
        let report = BrowserStatusReport {
            enabled: true,
            available_targets: vec![BrowserTargetAvailability {
                target: BrowserTargetKind::UserChrome,
                available: false,
                detail: "legacy".to_owned(),
            }],
            tabs_known: None,
            browser_integration: None,
            control_plane: None,
            diagnostics: Vec::new(),
        };

        let encoded = serde_json::to_value(report).expect("serialize browser status");
        assert!(encoded.get("control_plane").is_none());
    }

    #[test]
    fn legacy_list_tabs_payload_defaults_total() {
        let response: super::BrowserListTabsResponse = serde_json::from_value(serde_json::json!({
            "target": "user_chrome",
            "tabs": [],
            "diagnostics": []
        }))
        .expect("legacy list tabs response");
        assert_eq!(response.total, 0);
    }

    #[test]
    fn appshot_timeout_bounds_are_shared() {
        assert!(is_valid_browser_appshot_capture_timeout_ms(
            BROWSER_APPSHOT_MIN_CAPTURE_TIMEOUT_MS
        ));
        assert!(is_valid_browser_appshot_capture_timeout_ms(
            BROWSER_APPSHOT_MAX_CAPTURE_TIMEOUT_MS
        ));
        assert!(!is_valid_browser_appshot_capture_timeout_ms(
            BROWSER_APPSHOT_MIN_CAPTURE_TIMEOUT_MS - 1
        ));
        assert!(!is_valid_browser_appshot_capture_timeout_ms(
            BROWSER_APPSHOT_MAX_CAPTURE_TIMEOUT_MS + 1
        ));
    }

    #[test]
    fn click_element_request_round_trips_without_coordinates() {
        let request = BrowserRequest::ClickElement {
            target: Some(BrowserTargetKind::UserChrome),
            tab_id: "42".to_string(),
            element_ref: "opaque-token".to_string(),
            appshot_id: Some("shot-1".into()),
        };
        let json = serde_json::to_string(&request).expect("serialize");
        assert!(
            json.contains("click_element"),
            "tag should be snake_case: {json}"
        );
        assert!(json.contains("opaque-token"));
        assert!(
            !json.contains("\"x\""),
            "must not carry coordinates: {json}"
        );
        let back: BrowserRequest = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, request);
    }

    #[test]
    fn type_text_element_request_round_trips() {
        let request = BrowserRequest::TypeTextElement {
            target: None,
            tab_id: "7".to_string(),
            element_ref: "tok".to_string(),
            text: "hello".to_string(),
            appshot_id: Some("shot-1".into()),
        };
        let json = serde_json::to_string(&request).expect("serialize");
        let back: BrowserRequest = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, request);
    }

    #[test]
    fn element_diagnostic_codes_are_terminal_errors() {
        assert!(super::browser_diagnostic_is_error_code(
            "BrowserElementUnresolved"
        ));
        assert!(super::browser_diagnostic_is_error_code(
            "BrowserElementNotActionable"
        ));
    }
}
