//! Accessibility (AX) runtime.
//!
//! Every AX call is synchronous IPC into the target application. Nothing in
//! this module's public surface blocks a tokio runtime thread: work runs on
//! dedicated worker lanes, one per target process, so an unresponsive
//! application stalls only its own lane. Each lane sets an AX messaging
//! timeout on the application element and honours the caller's deadline,
//! returning the partial tree read so far with `timed_out` set.

mod actions;
mod chromium;
mod element;
mod lane;
mod refs;
mod tree;
mod vocab;
mod window;

#[cfg(test)]
mod live_tests;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};
use sky_cua_platform::model::{ActionOutcome, ActionRequest, ElementNode};

use crate::types::MacWindow;
use actions::Executed;
use element::AxElement;
use lane::{DEADLINE_GRACE, JobGuard, LaneContext, Lanes};
use refs::RefStore;
use tree::{Names, WalkOutput};

/// Limits for one tree read.
#[derive(Debug, Clone, Copy)]
pub struct TreeBudget {
    /// Wall-clock budget for the whole read.
    pub deadline: Duration,
    /// Hard cap on emitted elements.
    pub max_elements: usize,
    /// Hard cap on tree depth.
    pub max_depth: usize,
}

impl Default for TreeBudget {
    fn default() -> Self {
        Self {
            deadline: Duration::from_millis(2_500),
            max_elements: 1_500,
            max_depth: 64,
        }
    }
}

/// The result of reading one window's accessibility tree.
#[derive(Debug, Clone)]
pub struct AxTree {
    /// Flat pre-order element list. `element_index` equals the position in
    /// this vector, index 0 is the window, `parent_index` refers into it, and
    /// `backend_ref` is set on every element this runtime can act on later.
    /// Bounds are desktop points (`DesktopLogical`); the caller maps them
    /// into its capture, which it takes concurrently with this read.
    pub elements: Vec<ElementNode>,
    /// The element, depth, or per-node child cap dropped part of the tree.
    pub truncated: bool,
    /// The deadline or an AX messaging timeout stopped the walk.
    pub timed_out: bool,
}

/// How long an action may wait for its lane: it can queue behind a tree read
/// of the same process (deadline plus grace) and then needs a few messages.
const ACTION_WAIT: Duration = Duration::from_secs(6);
/// Window lookup plus three writes.
const RAISE_WAIT: Duration = Duration::from_secs(6);
/// Pause after activating a dormant Chromium tree before re-reading it;
/// Chromium builds the views tree within ~150 ms (the Linux backend measured
/// the same on AT-SPI).
const ACTIVATION_SETTLE: Duration = Duration::from_millis(200);
/// Attribute writes one activation sends; see `chromium::activate`.
const ACTIVATION_WRITES: u32 = 2;
/// Budget a post-activation re-read needs to be worth starting.
const MIN_REREAD: Duration = Duration::from_millis(250);

/// Handle to the AX worker lanes. Cheap to clone.
#[derive(Clone, Default)]
pub struct AxRuntime {
    inner: Arc<Inner>,
}

#[derive(Default)]
struct Inner {
    lanes: Lanes,
    refs: Mutex<RefStore<AxElement>>,
    generation: AtomicU64,
    /// Processes that accepted the Chromium activation write.
    activation_attempted: Mutex<chromium::ActivationAttempts>,
}

impl std::fmt::Debug for AxRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let retained = self.inner.refs.lock().map(|refs| refs.len()).unwrap_or(0);
        f.debug_struct("AxRuntime")
            .field("lanes", &self.inner.lanes)
            .field("retained_reads", &retained)
            .finish()
    }
}

impl AxRuntime {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Reads the accessibility tree of exactly `window`.
    ///
    /// `app_windows` is every CoreGraphics window of the same process in
    /// front-to-back order; it feeds `window_match::match_ax_window`. Fails
    /// closed with `AccessibilityCoverageLimited` when the AX window cannot be
    /// correlated exactly, and with the `permissions` missing-grant error when
    /// Accessibility is not granted.
    pub async fn read_window_tree(
        &self,
        window: &MacWindow,
        app_windows: &[MacWindow],
        budget: TreeBudget,
    ) -> Result<AxTree, BackendError> {
        let started = Instant::now();
        let deadline = started + budget.deadline;
        let generation = self.inner.generation.fetch_add(1, Ordering::Relaxed) + 1;
        let pid = window.pid;
        let request = TreeRequest {
            window: window.clone(),
            app_windows: app_windows.to_vec(),
            budget,
            deadline,
            generation,
        };
        let inner = Arc::clone(&self.inner);
        let output = self
            .inner
            .lanes
            .run(
                pid,
                budget.deadline + DEADLINE_GRACE,
                move |context, guard| read_on_lane(context, guard, &request, &inner),
            )
            .await?;
        tracing::debug!(
            pid,
            window_id = window.window_id,
            elements = output.tree.elements.len(),
            read = output.stats.read,
            round_trips = output.stats.round_trips,
            truncated = output.tree.truncated,
            timed_out = output.tree.timed_out,
            elapsed_ms = started.elapsed().as_millis() as u64,
            "macOS Accessibility tree read"
        );
        if let Ok(mut refs) = self.inner.refs.lock() {
            refs.insert(pid, generation, output.handles);
        }
        Ok(output.tree)
    }

    /// Performs `request` as an AX semantic action when its
    /// `resolved_element.backend_ref` names a live element from an earlier
    /// tree read and AX supports the action. Returns `Ok(None)` when the
    /// request must fall through to physical input.
    pub async fn try_semantic_action(
        &self,
        request: &ActionRequest,
    ) -> Result<Option<ActionOutcome>, BackendError> {
        let Some(call) = actions::action_for_request(request)? else {
            return Ok(None);
        };
        let Some(element) = self
            .inner
            .refs
            .lock()
            .ok()
            .and_then(|refs| refs.get(&call.target))
        else {
            // The reference was issued by this runtime but its read has been
            // evicted or reset: no live element to act on.
            return Ok(None);
        };
        let mutating = Arc::new(AtomicBool::new(false));
        let lane_mutating = Arc::clone(&mutating);
        let action = call.action.clone();
        let result = self
            .inner
            .lanes
            .run(call.target.pid, ACTION_WAIT, move |_, guard| {
                actions::execute(&element, &action, &lane_mutating, guard)
            })
            .await;
        match result {
            Ok(Executed::Done(message)) => Ok(Some(actions::success(message))),
            Ok(Executed::Unsupported) if call.action.requires_ax_success() => {
                Err(BackendError::new(
                    BackendErrorCode::ActionRequiresPhysicalInput,
                    format!(
                        "macOS Accessibility {} was unavailable for the selected element",
                        call.action.primitive_name()
                    ),
                ))
            }
            Ok(Executed::Unsupported) => Ok(None),
            Err(error)
                if error.code == BackendErrorCode::DesktopRequestDeadlineExceeded.as_str()
                    && mutating.load(Ordering::SeqCst) =>
            {
                Err(BackendError::new(
                    BackendErrorCode::CuaActionOutcomeUnknown,
                    format!(
                        "macOS Accessibility {} was sent but its lane did not answer: {}",
                        call.action.primitive_name(),
                        error.message
                    ),
                ))
            }
            Err(error) if call.action.requires_ax_success() || must_surface(&error) => Err(error),
            Err(error) => {
                tracing::debug!(
                    code = error.code,
                    message = %error.message,
                    "macOS Accessibility semantic action failed; falling back to physical input"
                );
                Ok(None)
            }
        }
    }

    /// Raises `window` within its application (`AXRaise`) and makes it main.
    pub async fn raise_window(
        &self,
        window: &MacWindow,
        app_windows: &[MacWindow],
    ) -> Result<(), BackendError> {
        let window = window.clone();
        let app_windows = app_windows.to_vec();
        self.inner
            .lanes
            .run(window.pid, RAISE_WAIT, move |context, guard| {
                window::raise(context, guard, &Names::new(), &window, &app_windows)
            })
            .await
    }

    /// Drops retained element handles and worker lanes after a request was
    /// abandoned for exceeding its deadline.
    pub async fn reset(&self) {
        if let Ok(mut refs) = self.inner.refs.lock() {
            refs.clear();
        }
        self.inner.lanes.clear();
    }
}

/// Errors a non-primitive action reports instead of falling back: a stale
/// snapshot, an action that may already have happened, or a missing grant.
fn must_surface(error: &BackendError) -> bool {
    [
        BackendErrorCode::SnapshotStale,
        BackendErrorCode::CuaActionOutcomeUnknown,
        BackendErrorCode::PermissionRequired,
        BackendErrorCode::InvalidRequest,
    ]
    .iter()
    .any(|code| error.code == code.as_str())
}

/// True when `backend_ref` was issued by this runtime.
#[must_use]
pub fn is_ax_backend_ref(backend_ref: &str) -> bool {
    refs::is_ax_ref(backend_ref)
}

struct TreeRequest {
    window: MacWindow,
    app_windows: Vec<MacWindow>,
    budget: TreeBudget,
    deadline: Instant,
    generation: u64,
}

fn read_on_lane(
    context: &LaneContext,
    guard: &JobGuard<'_>,
    request: &TreeRequest,
    inner: &Inner,
) -> Result<WalkOutput, BackendError> {
    let names = Names::new();
    let root = window::find_window(
        context,
        guard,
        &names,
        &request.window,
        &request.app_windows,
    )?;
    let walk = |deadline: Instant| {
        tree::walk(
            &names,
            &root,
            context.pid,
            request.generation,
            deadline,
            request.budget,
        )
    };
    let first = walk(request.deadline)?;
    if first.tree.timed_out || !chromium::looks_dormant(&first.stats) {
        return Ok(first);
    }
    let Some(engine) = request
        .window
        .executable
        .as_deref()
        .and_then(chromium::engine)
    else {
        return Ok(first);
    };
    let Some(process) = u32::try_from(context.pid)
        .ok()
        .and_then(sky_cua_platform::process::darwin::process_generation)
    else {
        return Ok(first);
    };
    if guard.is_abandoned() {
        // Waking the tree writes to the application; an expired read must not.
        return Ok(first);
    }
    if Instant::now() + ACTIVATION_WRITES * lane::MESSAGING_TIMEOUT
        > request.deadline + DEADLINE_GRACE
    {
        // Each activation write can wait out the messaging timeout. Without
        // room for both inside the caller's wait the lane would be abandoned
        // and this tree lost; the next read wakes the application instead.
        return Ok(first);
    }
    let first_attempt = inner
        .activation_attempted
        .lock()
        .is_ok_and(|mut attempted| attempted.claim(process));
    if !first_attempt {
        return Ok(first);
    }
    let accepted = chromium::activate(&context.app, guard);
    if let Ok(mut attempted) = inner.activation_attempted.lock() {
        attempted.settle(&process, &accepted);
    }
    if Instant::now() + ACTIVATION_SETTLE + MIN_REREAD > request.deadline {
        // No budget left to re-read; the next read sees the woken tree.
        return Ok(first);
    }
    std::thread::sleep(ACTIVATION_SETTLE);
    let Ok(second) = walk(request.deadline) else {
        return Ok(first);
    };
    tracing::debug!(
        pid = context.pid,
        ?engine,
        ?accepted,
        before = first.stats.read,
        after = second.stats.read,
        "activated a dormant Chromium accessibility tree"
    );
    Ok(if second.stats.read > first.stats.read {
        second
    } else {
        first
    })
}
