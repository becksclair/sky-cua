//! Per-process AX worker lanes.
//!
//! Each target process gets one dedicated OS thread, created on first use and
//! exiting after `IDLE_EXIT` without work. Jobs arrive over a channel and
//! answer through a tokio oneshot, so the async side never blocks. A lane
//! stuck in a hung application is abandoned by removing it from the map: the
//! next request for that process gets a fresh lane, and the abandoned thread
//! exits once its in-flight call returns (bounded by the AX messaging
//! timeout).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, TryRecvError};
use std::sync::{Arc, Mutex, Once};
use std::time::Duration;

use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};
use tokio::sync::oneshot;

use super::element::AxElement;

/// Per-message AX timeout, applied process-wide (system-wide element) and on
/// every application element.
///
/// A responsive application answers a batched attribute read in well under
/// 10 ms; one that needs a full second for a single message is busy on its
/// main thread and effectively hung for interactive use. One second is also
/// well inside the default 2.5 s tree budget, so a hung application costs
/// the walk at most one message before it returns the partial tree with
/// `timed_out`, and it bounds how long an abandoned lane lingers. The macOS
/// default (6 s) would let a single stuck message outlive the whole budget.
pub(crate) const MESSAGING_TIMEOUT: Duration = Duration::from_secs(1);

/// How long past the caller's deadline the async side waits for a lane: one
/// in-flight message may start just before the deadline and run for the full
/// messaging timeout, plus scheduling slack.
pub(crate) const DEADLINE_GRACE: Duration = Duration::from_millis(1_250);

/// An idle lane's thread exits after this long; recreating one is a thread
/// spawn (tens of microseconds).
#[cfg(not(test))]
const IDLE_EXIT: Duration = Duration::from_secs(30);
#[cfg(test)]
const IDLE_EXIT: Duration = Duration::from_millis(100);

/// State a lane job runs against.
pub(crate) struct LaneContext {
    pub pid: i32,
    /// The application element, with `MESSAGING_TIMEOUT` set.
    pub app: AxElement,
}

/// Lets a running job see that its caller has stopped waiting for it.
///
/// Abandoning a lane only detaches it; a job already running keeps going
/// until its current AX message returns. A job consults its guard before
/// every write to the application, so an expired request cannot raise a
/// window, press a control, or set a value after its caller has moved on.
pub(crate) struct JobGuard<'a> {
    abandoned: &'a (dyn Fn() -> bool + 'a),
}

impl JobGuard<'_> {
    #[must_use]
    pub(crate) fn is_abandoned(&self) -> bool {
        (self.abandoned)()
    }

    /// Fails when the caller has stopped waiting. Call before each AX write.
    pub(crate) fn ensure_wanted(&self) -> Result<(), BackendError> {
        if self.is_abandoned() {
            return Err(BackendError::new(
                BackendErrorCode::DesktopRequestDeadlineExceeded,
                "the macOS Accessibility request expired before this write; nothing further was sent",
            ));
        }
        Ok(())
    }
}

type Job = Box<dyn FnOnce(&LaneContext) + Send + 'static>;

struct LaneEntry {
    id: u64,
    sender: mpsc::Sender<Job>,
}

#[derive(Default)]
struct LaneMap {
    lanes: Mutex<HashMap<i32, LaneEntry>>,
    next_id: AtomicU64,
}

/// The set of live lanes. Cheap to clone.
#[derive(Clone, Default)]
pub(crate) struct Lanes {
    map: Arc<LaneMap>,
}

impl std::fmt::Debug for Lanes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let count = self.map.lanes.lock().map(|lanes| lanes.len()).unwrap_or(0);
        f.debug_struct("Lanes").field("live", &count).finish()
    }
}

impl Lanes {
    /// Runs `job` on the lane for `pid` and waits up to `wait` for its answer
    /// without blocking the calling task's thread. On expiry the lane is
    /// abandoned and the request fails with `DesktopRequestDeadlineExceeded`.
    pub(crate) async fn run<T, F>(
        &self,
        pid: i32,
        wait: Duration,
        job: F,
    ) -> Result<T, BackendError>
    where
        T: Send + 'static,
        F: FnOnce(&LaneContext, &JobGuard<'_>) -> Result<T, BackendError> + Send + 'static,
    {
        let (sender, mut receiver) = oneshot::channel();
        let lane_id = self.submit(
            pid,
            Box::new(move |context| {
                // The caller gave up (deadline or cancellation): skip the IPC.
                if sender.is_closed() {
                    return;
                }
                let result = {
                    let abandoned = || sender.is_closed();
                    job(
                        context,
                        &JobGuard {
                            abandoned: &abandoned,
                        },
                    )
                };
                let _ = sender.send(result);
            }),
        )?;
        match tokio::time::timeout(wait, &mut receiver).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => {
                // The job was dropped without answering: the lane thread
                // panicked. Make sure the next request gets a fresh lane.
                self.abandon(pid, lane_id);
                Err(BackendError::new(
                    BackendErrorCode::Internal,
                    format!("the macOS Accessibility lane for pid {pid} exited without answering"),
                ))
            }
            Err(_) => {
                // Close before returning so a job that is still running sees
                // the expiry at its next guard check, and before the caller
                // inspects any state the job publishes.
                receiver.close();
                self.abandon(pid, lane_id);
                Err(BackendError::new(
                    BackendErrorCode::DesktopRequestDeadlineExceeded,
                    format!(
                        "pid {pid} did not answer macOS Accessibility within {} ms; its lane was abandoned and the next request starts a fresh one",
                        wait.as_millis()
                    ),
                ))
            }
        }
    }

    fn submit(&self, pid: i32, job: Job) -> Result<u64, BackendError> {
        let mut lanes = self.map.lanes.lock().map_err(|_| poisoned())?;
        if let Some(entry) = lanes.get(&pid) {
            match entry.sender.send(job) {
                Ok(()) => return Ok(entry.id),
                // The lane thread is gone (it panicked); replace it.
                Err(mpsc::SendError(job)) => {
                    lanes.remove(&pid);
                    return self.spawn(&mut lanes, pid, job);
                }
            }
        }
        self.spawn(&mut lanes, pid, job)
    }

    fn spawn(
        &self,
        lanes: &mut HashMap<i32, LaneEntry>,
        pid: i32,
        job: Job,
    ) -> Result<u64, BackendError> {
        let id = self.map.next_id.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = mpsc::channel::<Job>();
        sender.send(job).map_err(|_| poisoned())?;
        let map = Arc::clone(&self.map);
        std::thread::Builder::new()
            .name(format!("sky-cua-ax-{pid}"))
            .spawn(move || lane_main(&map, pid, id, &receiver))
            .map_err(|error| {
                BackendError::new(
                    BackendErrorCode::Internal,
                    format!("failed to start a macOS Accessibility lane for pid {pid}: {error}"),
                )
            })?;
        lanes.insert(pid, LaneEntry { id, sender });
        Ok(id)
    }

    /// Detaches lane `id` of `pid` if it is still the current one.
    pub(crate) fn abandon(&self, pid: i32, id: u64) {
        if let Ok(mut lanes) = self.map.lanes.lock()
            && lanes.get(&pid).is_some_and(|entry| entry.id == id)
        {
            lanes.remove(&pid);
        }
    }

    /// Detaches every lane; each thread exits after its in-flight job.
    pub(crate) fn clear(&self) {
        if let Ok(mut lanes) = self.map.lanes.lock() {
            lanes.clear();
        }
    }

    #[cfg(test)]
    pub(crate) fn live(&self) -> usize {
        self.map.lanes.lock().map(|lanes| lanes.len()).unwrap_or(0)
    }
}

fn poisoned() -> BackendError {
    BackendError::new(
        BackendErrorCode::Internal,
        "the macOS Accessibility lane map is poisoned",
    )
}

static GLOBAL_TIMEOUT: Once = Once::new();

fn lane_main(map: &LaneMap, pid: i32, id: u64, receiver: &mpsc::Receiver<Job>) {
    // Per the AX headers, a timeout set on one element reference applies to
    // that reference only; element references copied out of the tree carry
    // none and fall back to the process-wide value set on the system-wide
    // element. Setting both bounds every message this crate sends.
    GLOBAL_TIMEOUT.call_once(|| {
        let _ = AxElement::system_wide().set_messaging_timeout(MESSAGING_TIMEOUT.as_secs_f32());
    });
    let app = AxElement::application(pid);
    let _ = app.set_messaging_timeout(MESSAGING_TIMEOUT.as_secs_f32());
    let context = LaneContext { pid, app };
    loop {
        match receiver.recv_timeout(IDLE_EXIT) {
            Ok(job) => job(&context),
            Err(RecvTimeoutError::Disconnected) => return,
            Err(RecvTimeoutError::Timeout) => {
                // Senders only send while holding the map lock, so checking
                // the queue and removing ourselves under the same lock cannot
                // lose a job.
                let Ok(mut lanes) = map.lanes.lock() else {
                    return;
                };
                match receiver.try_recv() {
                    Ok(job) => {
                        drop(lanes);
                        job(&context);
                    }
                    Err(TryRecvError::Empty | TryRecvError::Disconnected) => {
                        if lanes.get(&pid).is_some_and(|entry| entry.id == id) {
                            lanes.remove(&pid);
                        }
                        return;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[tokio::test(flavor = "current_thread")]
    async fn a_stuck_lane_is_abandoned_and_other_processes_are_unaffected() {
        let lanes = Lanes::default();
        let stuck = lanes.clone();
        let slow = tokio::spawn(async move {
            stuck
                .run(-101, Duration::from_millis(50), |_, _| {
                    std::thread::sleep(Duration::from_millis(400));
                    Ok(1)
                })
                .await
        });
        // Another process answers promptly while -101 is stuck.
        let started = Instant::now();
        let other = lanes
            .run(-102, Duration::from_secs(1), |context, _| Ok(context.pid))
            .await
            .unwrap();
        assert_eq!(other, -102);
        assert!(started.elapsed() < Duration::from_millis(300));

        let error = slow.await.unwrap().unwrap_err();
        assert_eq!(error.code, "DesktopRequestDeadlineExceeded");
        // The next request for the stuck pid gets a fresh lane and answers.
        let fresh = lanes
            .run(-101, Duration::from_secs(1), |_, _| Ok(2))
            .await
            .unwrap();
        assert_eq!(fresh, 2);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_running_job_sees_that_its_caller_stopped_waiting() {
        let lanes = Lanes::default();
        let (observed, observation) = mpsc::channel();
        let error = lanes
            .run::<(), _>(-106, Duration::from_millis(50), move |_, guard| {
                let before = guard.is_abandoned();
                // Outlive the caller's wait, as a slow window lookup would.
                std::thread::sleep(Duration::from_millis(300));
                let _ = observed.send((before, guard.ensure_wanted().map_err(|error| error.code)));
                Ok(())
            })
            .await
            .unwrap_err();
        assert_eq!(error.code, "DesktopRequestDeadlineExceeded");

        let (before, after) = observation
            .recv_timeout(Duration::from_secs(2))
            .expect("the abandoned job still runs to its guard check");
        assert!(!before, "a job whose caller is waiting is wanted");
        assert_eq!(after, Err("DesktopRequestDeadlineExceeded"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_panicking_job_reports_internal_and_the_lane_is_replaced() {
        let lanes = Lanes::default();
        let error = lanes
            .run::<(), _>(-103, Duration::from_secs(1), |_, _| panic!("boom"))
            .await
            .unwrap_err();
        assert_eq!(error.code, "Internal");
        let value = lanes
            .run(-103, Duration::from_secs(1), |_, _| Ok(3))
            .await
            .unwrap();
        assert_eq!(value, 3);
        lanes.clear();
        assert_eq!(lanes.live(), 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn idle_lanes_exit_and_are_recreated_on_demand() {
        let lanes = Lanes::default();
        lanes
            .run(-105, Duration::from_secs(1), |_, _| Ok(()))
            .await
            .unwrap();
        assert_eq!(lanes.live(), 1);
        tokio::time::sleep(IDLE_EXIT * 4).await;
        assert_eq!(lanes.live(), 0);
        let value = lanes
            .run(-105, Duration::from_secs(1), |_, _| Ok(5))
            .await
            .unwrap();
        assert_eq!(value, 5);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn jobs_for_one_process_run_in_order_on_one_lane() {
        let lanes = Lanes::default();
        let first = lanes.run(-104, Duration::from_secs(1), |_, _| {
            Ok(std::thread::current().id())
        });
        let second = lanes.run(-104, Duration::from_secs(1), |_, _| {
            Ok(std::thread::current().id())
        });
        let (first, second) = tokio::join!(first, second);
        assert_eq!(first.unwrap(), second.unwrap());
        assert_eq!(lanes.live(), 1);
    }
}
