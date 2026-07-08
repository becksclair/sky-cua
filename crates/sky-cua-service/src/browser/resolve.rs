//! Stateless re-resolution of a snapshot-referenced page element to its live
//! CSS-pixel center, so browser input can aim by element identity instead of
//! coordinates the agent computed and that may have gone stale.
//!
//! ## Contract (stable across Option 1 / Option 2 implementations)
//!
//! The element handle is an opaque token the browser snapshot emits per element
//! and the agent passes back verbatim. It is self-contained: this module needs
//! no server-held per-tab state to re-find the element (sky-cua drives the
//! browser over ephemeral per-operation bridge connections, so there is no
//! shared element cache). [`resolve_element_center`] decodes the token, runs a
//! resolution over the bridge, and returns the element's current center or a
//! structured diagnostic.
//!
//! Diagnostic codes on failure (both are terminal caller errors — re-observe
//! and retry): `BrowserElementUnresolved` when the token matches no element on
//! the page (reasons like not-found / ambiguous), and
//! `BrowserElementNotActionable` when the element was found but cannot be
//! clicked (zero-size, off-screen after a scroll attempt, or covered by another
//! element).
//!
//! Milestone 0 ships this module as the shared signature only; the real
//! resolver (Stream 1A) replaces [`resolve_element_center`]'s body. The
//! signature is fixed here so the service input path (Stream 1B) compiles and
//! is testable against it independently.

use std::path::Path;

use sky_cua_platform::model::DiagnosticEntry;
use tokio::net::UnixStream;
use tokio::time::Instant as TokioInstant;

/// The resolved live position of a snapshot-referenced element, in CSS pixels —
/// the same coordinate space as browser click coordinates and screenshot
/// pixels, so the center feeds the existing `Input.dispatchMouseEvent` path
/// with no scaling conversion.
// Milestone 0 fixes these signatures so Streams 1A/1B compile against them;
// they are consumed when Stream 1B wires the element dispatch. The allow is
// removed then.
#[allow(dead_code)]
pub(super) struct ResolvedElementCenter {
    pub x: f64,
    pub y: f64,
}

/// Re-locate the element named by the opaque snapshot `element_ref` in the live
/// page and return its current center. Stateless; see the module docs for the
/// token contract and the failure diagnostic codes.
#[allow(dead_code)]
pub(super) async fn resolve_element_center(
    stream: &mut UnixStream,
    socket: &Path,
    element_ref: &str,
    deadline: TokioInstant,
) -> Result<ResolvedElementCenter, DiagnosticEntry> {
    // Milestone 0 stub: report unresolved until Stream 1A lands the resolver.
    let _ = (stream, socket, element_ref, deadline);
    Err(element_unresolved_diagnostic(
        "browser element resolution is not yet implemented",
    ))
}

/// Build the `BrowserElementUnresolved` diagnostic used when a token matches no
/// element on the current page.
pub(super) fn element_unresolved_diagnostic(message: impl Into<String>) -> DiagnosticEntry {
    DiagnosticEntry {
        code: "BrowserElementUnresolved".to_string(),
        message: message.into(),
        details: Some(
            "The referenced element is no longer on the page. Re-run \
             observe(surface=\"browser\") to get fresh element references, then retry."
                .to_string(),
        ),
    }
}

/// Build the `BrowserElementNotActionable` diagnostic used when the element was
/// found but cannot receive a click (zero-size, off-screen, or covered).
#[allow(dead_code)]
pub(super) fn element_not_actionable_diagnostic(message: impl Into<String>) -> DiagnosticEntry {
    DiagnosticEntry {
        code: "BrowserElementNotActionable".to_string(),
        message: message.into(),
        details: Some(
            "The referenced element is present but cannot be clicked right now (hidden, \
             off-screen, or covered by another element). Re-observe and retry, or use \
             coordinates if the target is a canvas/map region."
                .to_string(),
        ),
    }
}
