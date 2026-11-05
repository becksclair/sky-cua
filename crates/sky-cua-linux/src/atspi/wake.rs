//! Activation of dormant Chromium/Electron accessibility trees.
//!
//! Chromium on Linux registers its AT-SPI application root whenever session
//! accessibility is enabled at startup, but builds the views and web-content
//! tree only once an assistive technology is detected. Until then every
//! top-level frame reports a single null child. Chromium treats an ATK
//! `GetAttributes` or `GetRelationSet` call as the screen-reader signal
//! (`ui/accessibility/platform/ax_platform_node_auralinux.cc`, `AtkGetAttributes`
//! → `OnExtendedPropertiesUsedInWebContent`), which enables basic web
//! accessibility plus extended properties for the whole process. One such
//! call on a dormant tree is therefore enough to populate it, with no
//! relaunch or `--force-renderer-accessibility` flag.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use atspi::proxy::accessible::ObjectRefExt;
use atspi::proxy::proxy_ext::ProxyExt;
use atspi_connection::AccessibilityConnection;
use sky_cua_platform::model::{DiagnosticEntry, ElementNode};
use tracing::debug;

use crate::atspi::tree::{TreeWalkStats, walk_accessible_tree};

pub(crate) const TREE_ACTIVATED_CODE: &str = "AccessibilityTreeActivated";
pub(crate) const TREE_DORMANT_CODE: &str = "AccessibilityTreeDormant";

/// Delays before each re-walk after an activation request. Chromium builds
/// the views tree within ~150 ms; web documents can take a little longer.
#[cfg(not(test))]
const SETTLE_DELAYS: [Duration; 3] = [
    Duration::from_millis(150),
    Duration::from_millis(250),
    Duration::from_millis(400),
];
#[cfg(test)]
const SETTLE_DELAYS: [Duration; 3] = [Duration::ZERO; 3];

/// What sky-cua learned about one application connection. AT-SPI unique bus
/// names are never reused within one bus lifetime, so the name identifies one
/// application process until the bus restarts; [`forget_activations`] clears
/// the record whenever a fresh accessibility connection is made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActivationOutcome {
    /// Not a Chromium toolkit; its null-only nodes are not a dormant tree.
    NotChromium,
    Activated,
    /// An activation request was delivered and the tree did not grow.
    Dormant,
}

fn attempted_activations() -> &'static Mutex<HashMap<String, ActivationOutcome>> {
    static ATTEMPTS: OnceLock<Mutex<HashMap<String, ActivationOutcome>>> = OnceLock::new();
    ATTEMPTS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cached_outcome(bus_name: &str) -> Option<ActivationOutcome> {
    attempted_activations()
        .lock()
        .ok()
        .and_then(|attempts| attempts.get(bus_name).copied())
}

fn remember_outcome(bus_name: String, outcome: ActivationOutcome) {
    if let Ok(mut attempts) = attempted_activations().lock() {
        attempts.insert(bus_name, outcome);
    }
}

/// Drop every recorded outcome. Called on each fresh accessibility connection,
/// since a new connection may follow an a11y bus restart that reuses names.
pub(crate) fn forget_activations() {
    if let Ok(mut attempts) = attempted_activations().lock() {
        attempts.clear();
    }
}

const CHROMIUM_TOOLKIT: &str = "Chromium";

fn is_chromium_toolkit(toolkit_name: &str) -> bool {
    toolkit_name.trim().eq_ignore_ascii_case(CHROMIUM_TOOLKIT)
}

/// Flatten `root`, activating a dormant Chromium tree first when the walk
/// shows one. Returns the elements plus any activation diagnostic.
pub(crate) async fn walk_with_activation(
    connection: &AccessibilityConnection,
    root: &atspi::ObjectRefOwned,
    max_nodes: usize,
) -> (Vec<ElementNode>, Vec<DiagnosticEntry>) {
    let (elements, stats) = walk_accessible_tree(connection, root, max_nodes).await;
    if stats.null_only_parents == 0 {
        return (elements, Vec::new());
    }
    let Some(bus_name) = root.name_as_str().map(str::to_string) else {
        return (elements, Vec::new());
    };

    match cached_outcome(&bus_name) {
        // Already woken (remaining null-only nodes are subtrees Chromium has
        // not exposed, e.g. unloaded frames), or not Chromium at all.
        Some(ActivationOutcome::Activated | ActivationOutcome::NotChromium) => {
            return (elements, Vec::new());
        }
        Some(ActivationOutcome::Dormant) => {
            // Only a delivered request is ever recorded as Dormant.
            let report = ActivationReport {
                toolkit: CHROMIUM_TOOLKIT.to_string(),
                activation_sent: true,
                nodes_before: elements.len(),
                null_only_parents_before: stats.null_only_parents,
                nodes_after: elements.len(),
                null_only_parents_after: stats.null_only_parents,
                settle_attempts: 0,
                repeated: true,
            };
            return (elements, vec![report.diagnostic()]);
        }
        None => {}
    }

    let Some(toolkit) = toolkit_name(connection, root).await else {
        return (elements, Vec::new());
    };
    if !is_chromium_toolkit(&toolkit) {
        remember_outcome(bus_name, ActivationOutcome::NotChromium);
        return (elements, Vec::new());
    }

    let activation_sent = request_activation(connection, root).await;
    let (elements_after, stats_after, settle_attempts) =
        settle_after_activation(&SETTLE_DELAYS, || {
            walk_accessible_tree(connection, root, max_nodes)
        })
        .await;
    let report = ActivationReport {
        toolkit,
        activation_sent,
        nodes_before: elements.len(),
        null_only_parents_before: stats.null_only_parents,
        nodes_after: elements_after.len(),
        null_only_parents_after: stats_after.null_only_parents,
        settle_attempts,
        repeated: false,
    };
    // A failed request (e.g. a busy app timing out) is retried next observe.
    if activation_sent {
        remember_outcome(bus_name, report.outcome());
    }
    debug!(?report, "dormant Chromium accessibility tree activation");
    (elements_after, vec![report.diagnostic()])
}

async fn toolkit_name(
    connection: &AccessibilityConnection,
    root: &atspi::ObjectRefOwned,
) -> Option<String> {
    let accessible = root
        .as_accessible_proxy(connection.connection())
        .await
        .ok()?;
    let application_ref = accessible.get_application().await.ok()?;
    let application = application_ref
        .as_accessible_proxy(connection.connection())
        .await
        .ok()?;
    application
        .proxies()
        .await
        .ok()?
        .application()
        .await
        .ok()?
        .toolkit_name()
        .await
        .ok()
}

/// Send the ATK attribute query Chromium treats as a screen-reader signal.
async fn request_activation(
    connection: &AccessibilityConnection,
    root: &atspi::ObjectRefOwned,
) -> bool {
    let Ok(accessible) = root.as_accessible_proxy(connection.connection()).await else {
        return false;
    };
    match accessible.get_attributes().await {
        Ok(_) => true,
        Err(error) => {
            debug!(%error, "GetAttributes activation request failed");
            false
        }
    }
}

/// Re-walk after each delay until no null-only node remains or the delays
/// run out. Returns the last walk and how many walks ran.
async fn settle_after_activation<W, Fut>(
    delays: &[Duration],
    mut walk: W,
) -> (Vec<ElementNode>, TreeWalkStats, usize)
where
    W: FnMut() -> Fut,
    Fut: Future<Output = (Vec<ElementNode>, TreeWalkStats)>,
{
    let mut last = (Vec::new(), TreeWalkStats::default());
    for (attempt, delay) in delays.iter().enumerate() {
        tokio::time::sleep(*delay).await;
        last = walk().await;
        if last.1.null_only_parents == 0 {
            return (last.0, last.1, attempt + 1);
        }
    }
    (last.0, last.1, delays.len())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ActivationReport {
    toolkit: String,
    activation_sent: bool,
    nodes_before: usize,
    null_only_parents_before: usize,
    nodes_after: usize,
    null_only_parents_after: usize,
    settle_attempts: usize,
    /// The outcome comes from an earlier attempt on the same process.
    repeated: bool,
}

impl ActivationReport {
    /// Activated when the tree grew or its null placeholders were replaced.
    fn outcome(&self) -> ActivationOutcome {
        if self.activation_sent
            && (self.nodes_after > self.nodes_before
                || self.null_only_parents_after < self.null_only_parents_before)
        {
            ActivationOutcome::Activated
        } else {
            ActivationOutcome::Dormant
        }
    }

    fn diagnostic(&self) -> DiagnosticEntry {
        let details = format!(
            "toolkit={}; activation_sent={}; nodes_before={}; null_only_parents_before={}; nodes_after={}; null_only_parents_after={}; settle_attempts={}; repeated={}",
            self.toolkit,
            self.activation_sent,
            self.nodes_before,
            self.null_only_parents_before,
            self.nodes_after,
            self.null_only_parents_after,
            self.settle_attempts,
            self.repeated,
        );
        if self.outcome() == ActivationOutcome::Activated {
            DiagnosticEntry {
                code: TREE_ACTIVATED_CODE.to_string(),
                message: "The Chromium-family application's AT-SPI tree was dormant; sky-cua activated it with an ATK attribute query and re-read it.".to_string(),
                details: Some(details),
            }
        } else {
            DiagnosticEntry {
                code: TREE_DORMANT_CODE.to_string(),
                message: "The Chromium-family application kept its AT-SPI tree dormant after an activation request, so only its window frame is exposed. Relaunch it with --force-renderer-accessibility.".to_string(),
                details: Some(format!(
                    "{details}; remedy=relaunch_with_force_renderer_accessibility"
                )),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::{
        ActivationOutcome, ActivationReport, SETTLE_DELAYS, TREE_ACTIVATED_CODE, TREE_DORMANT_CODE,
        cached_outcome, forget_activations, is_chromium_toolkit, remember_outcome,
        settle_after_activation,
    };
    use crate::atspi::tree::TreeWalkStats;
    use sky_cua_platform::model::ElementNode;

    fn node(index: usize) -> ElementNode {
        ElementNode {
            element_index: index,
            parent_index: index.checked_sub(1),
            role: "panel".to_string(),
            name: None,
            description: None,
            value: None,
            text: None,
            numeric_value: None,
            supports_editable_text: false,
            state_flags: Vec::new(),
            semantic_actions: Vec::new(),
            bounds: None,
            backend_ref: None,
        }
    }

    fn dormant() -> TreeWalkStats {
        TreeWalkStats {
            null_only_parents: 1,
        }
    }

    fn report(nodes_after: usize, null_only_parents_after: usize, sent: bool) -> ActivationReport {
        ActivationReport {
            toolkit: "Chromium".to_string(),
            activation_sent: sent,
            nodes_before: 1,
            null_only_parents_before: 1,
            nodes_after,
            null_only_parents_after,
            settle_attempts: 1,
            repeated: false,
        }
    }

    #[test]
    fn fresh_connection_forgets_recorded_outcomes() {
        remember_outcome(":1.34".to_string(), ActivationOutcome::Activated);
        remember_outcome(":1.35".to_string(), ActivationOutcome::NotChromium);
        assert_eq!(cached_outcome(":1.34"), Some(ActivationOutcome::Activated));
        forget_activations();
        assert_eq!(cached_outcome(":1.34"), None);
        assert_eq!(cached_outcome(":1.35"), None);
    }

    #[test]
    fn chromium_toolkit_match_is_exact_and_case_insensitive() {
        assert!(is_chromium_toolkit("Chromium"));
        assert!(is_chromium_toolkit(" chromium "));
        assert!(!is_chromium_toolkit("Qt"));
        assert!(!is_chromium_toolkit("GTK"));
        assert!(!is_chromium_toolkit(""));
    }

    #[test]
    fn grown_tree_reports_activated() {
        let report = report(256, 0, true);
        assert_eq!(report.outcome(), ActivationOutcome::Activated);
        let diagnostic = report.diagnostic();
        assert_eq!(diagnostic.code, TREE_ACTIVATED_CODE);
        let details = diagnostic.details.expect("details");
        assert!(details.contains("nodes_before=1; null_only_parents_before=1; nodes_after=256"));
        assert!(!details.contains("remedy="));
    }

    #[test]
    fn unchanged_tree_reports_dormant_with_remedy() {
        let diagnostic = report(1, 1, true).diagnostic();
        assert_eq!(diagnostic.code, TREE_DORMANT_CODE);
        assert!(
            diagnostic
                .details
                .expect("details")
                .ends_with("remedy=relaunch_with_force_renderer_accessibility")
        );
    }

    #[test]
    fn failed_request_is_dormant_even_if_tree_changed() {
        assert_eq!(report(10, 0, false).outcome(), ActivationOutcome::Dormant);
    }

    #[tokio::test]
    async fn settle_stops_at_first_walk_without_null_only_nodes() {
        let calls = Cell::new(0usize);
        let (elements, stats, attempts) = settle_after_activation(&SETTLE_DELAYS, || {
            calls.set(calls.get() + 1);
            let call = calls.get();
            async move {
                if call < 2 {
                    (vec![node(0)], dormant())
                } else {
                    ((0..5).map(node).collect(), TreeWalkStats::default())
                }
            }
        })
        .await;
        assert_eq!(attempts, 2);
        assert_eq!(calls.get(), 2);
        assert_eq!(elements.len(), 5);
        assert_eq!(stats.null_only_parents, 0);
    }

    #[tokio::test]
    async fn settle_gives_up_after_every_delay() {
        let calls = Cell::new(0usize);
        let (elements, stats, attempts) = settle_after_activation(&SETTLE_DELAYS, || {
            calls.set(calls.get() + 1);
            async { (vec![node(0)], dormant()) }
        })
        .await;
        assert_eq!(attempts, SETTLE_DELAYS.len());
        assert_eq!(calls.get(), SETTLE_DELAYS.len());
        assert_eq!(elements.len(), 1);
        assert_eq!(stats.null_only_parents, 1);
    }
}
