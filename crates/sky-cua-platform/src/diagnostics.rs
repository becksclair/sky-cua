use thiserror::Error;

use crate::model::DiagnosticEntry;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendErrorCode {
    UnsupportedEnvironment,
    PortalUnavailable,
    PortalCapabilityMissing,
    PortalApprovalPending,
    PortalRequestDenied,
    CaptureBackendDowngraded,
    CaptureSourceGeometryMissing,
    CaptureFrameBlank,
    PipeWireUnavailable,
    PipeWireStreamFailed,
    AccessibilityUnavailable,
    AccessibilityCoverageLimited,
    SnapshotStale,
    ActionRequiresPhysicalInput,
    ActionUnsupportedForEnvironment,
    ServiceUnavailable,
    InvalidRequest,
    CuaActionOutcomeUnknown,
    NotImplemented,
    Internal,
    /// A desktop request (observe/doctor/list/screenshot) exceeded the
    /// server-side deadline and was abandoned so a single hung AT-SPI or
    /// portal call cannot wedge the shared desktop request lane forever.
    DesktopRequestDeadlineExceeded,
}

impl BackendErrorCode {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UnsupportedEnvironment => "UnsupportedEnvironment",
            Self::PortalUnavailable => "PortalUnavailable",
            Self::PortalCapabilityMissing => "PortalCapabilityMissing",
            Self::PortalApprovalPending => "PortalApprovalPending",
            Self::PortalRequestDenied => "PortalRequestDenied",
            Self::CaptureBackendDowngraded => "CaptureBackendDowngraded",
            Self::CaptureSourceGeometryMissing => "CaptureSourceGeometryMissing",
            Self::CaptureFrameBlank => "CaptureFrameBlank",
            Self::PipeWireUnavailable => "PipeWireUnavailable",
            Self::PipeWireStreamFailed => "PipeWireStreamFailed",
            Self::AccessibilityUnavailable => "AccessibilityUnavailable",
            Self::AccessibilityCoverageLimited => "AccessibilityCoverageLimited",
            Self::SnapshotStale => "SnapshotStale",
            Self::ActionRequiresPhysicalInput => "ActionRequiresPhysicalInput",
            Self::ActionUnsupportedForEnvironment => "ActionUnsupportedForEnvironment",
            Self::ServiceUnavailable => "ServiceUnavailable",
            Self::InvalidRequest => "InvalidRequest",
            Self::CuaActionOutcomeUnknown => "CuaActionOutcomeUnknown",
            Self::NotImplemented => "NotImplemented",
            Self::Internal => "Internal",
            Self::DesktopRequestDeadlineExceeded => "DesktopRequestDeadlineExceeded",
        }
    }
}

#[derive(Debug, Error)]
#[error("{code}: {message}")]
pub struct BackendError {
    pub code: &'static str,
    pub message: String,
}

impl BackendError {
    #[must_use]
    pub fn new(code: BackendErrorCode, message: impl Into<String>) -> Self {
        Self {
            code: code.as_str(),
            message: message.into(),
        }
    }

    #[must_use]
    pub fn diagnostic(&self) -> DiagnosticEntry {
        DiagnosticEntry {
            code: self.code.to_string(),
            message: self.message.clone(),
            details: None,
        }
    }
}

#[derive(Debug, Default)]
pub struct DiagnosticBuilder {
    entries: Vec<DiagnosticEntry>,
}

impl DiagnosticBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(
        &mut self,
        code: BackendErrorCode,
        message: impl Into<String>,
        details: Option<String>,
    ) {
        self.push_code(code.as_str(), message, details);
    }

    pub fn push_code(
        &mut self,
        code: impl Into<String>,
        message: impl Into<String>,
        details: Option<String>,
    ) {
        self.entries.push(DiagnosticEntry {
            code: code.into(),
            message: message.into(),
            details,
        });
    }

    #[must_use]
    pub fn finish(self) -> Vec<DiagnosticEntry> {
        self.entries
    }
}
