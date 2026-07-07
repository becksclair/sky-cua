use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::PathBuf;
use std::time::Duration;

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};
use sky_cua_platform::model::PixelSize;

use crate::portal::screenshot;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipeWireFrameCapture {
    pub path: PathBuf,
    pub pixel_size: Option<PixelSize>,
}

const FORCE_FAILURE_ENV: &str = "SKY_CUA_FORCE_PIPEWIRE_CAPTURE_FAILURE";

/// Deadline for joining the blocking GStreamer capture task. Overridable via
/// `SKY_CUA_PIPEWIRE_CAPTURE_JOIN_TIMEOUT_MS` so tests can exercise the
/// timeout path without waiting out the production default.
fn capture_join_timeout() -> Duration {
    static TIMEOUT: std::sync::OnceLock<Duration> = std::sync::OnceLock::new();
    *TIMEOUT.get_or_init(|| {
        std::env::var("SKY_CUA_PIPEWIRE_CAPTURE_JOIN_TIMEOUT_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .map(Duration::from_millis)
            .unwrap_or(Duration::from_secs(15))
    })
}

pub async fn capture_png_frame(
    snapshot_id: &str,
    node_id: u32,
    remote_fd: OwnedFd,
) -> Result<PipeWireFrameCapture, BackendError> {
    if should_force_capture_failure() {
        return Err(BackendError::new(
            BackendErrorCode::PipeWireStreamFailed,
            format!("forced PipeWire capture failure via {FORCE_FAILURE_ENV} for downgrade smoke"),
        ));
    }
    let output_path = screenshot::capture_output_path(snapshot_id);
    if let Some(parent) = output_path.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|error| {
            BackendError::new(
                BackendErrorCode::Internal,
                format!(
                    "failed to create PipeWire capture directory {}: {error}",
                    parent.display()
                ),
            )
        })?;
    }
    if tokio::fs::try_exists(&output_path).await.unwrap_or(false) {
        let _ = tokio::fs::remove_file(&output_path).await;
    }

    let blocking_output_path = output_path.clone();
    let handle = tokio::task::spawn_blocking(move || {
        capture_png_frame_blocking(blocking_output_path, node_id, remote_fd)
    });
    join_capture_task(handle).await?;

    let pixel_size = screenshot::pixel_size_from_path(&output_path);
    Ok(PipeWireFrameCapture {
        path: output_path,
        pixel_size,
    })
}

/// Bound the join of a `spawn_blocking` capture task to [`capture_join_timeout`].
///
/// A `pipeline.set_state(Null)` teardown deep inside the blocking task can
/// deadlock; without a bound, awaiting the `JoinHandle` hangs the async
/// caller forever, which (before this fix) wedged the daemon's shared
/// desktop request lane. On elapse the `timeout` future is dropped — this
/// only stops *waiting* for the blocking OS thread, it does not cancel it
/// (`spawn_blocking` tasks are not abortable), so the orphaned thread runs
/// to completion independently and cannot corrupt caller-side state.
async fn join_capture_task<T: Send + 'static>(
    handle: tokio::task::JoinHandle<Result<T, BackendError>>,
) -> Result<T, BackendError> {
    match tokio::time::timeout(capture_join_timeout(), handle).await {
        Ok(join_result) => join_result.map_err(|error| {
            BackendError::new(
                BackendErrorCode::PipeWireStreamFailed,
                format!("PipeWire frame capture task failed to join cleanly: {error}"),
            )
        })?,
        Err(_) => Err(BackendError::new(
            BackendErrorCode::PipeWireStreamFailed,
            format!(
                "PipeWire frame capture task exceeded the {:?} join timeout and was abandoned",
                capture_join_timeout()
            ),
        )),
    }
}

fn should_force_capture_failure() -> bool {
    std::env::var_os(FORCE_FAILURE_ENV).is_some_and(|value| {
        !value.is_empty() && value != "0" && value != "false" && value != "FALSE"
    })
}

fn capture_png_frame_blocking(
    output_path: PathBuf,
    node_id: u32,
    remote_fd: OwnedFd,
) -> Result<(), BackendError> {
    gst::init().map_err(|error| {
        BackendError::new(
            BackendErrorCode::PipeWireUnavailable,
            format!("failed to initialize GStreamer for PipeWire frame capture: {error}"),
        )
    })?;

    let source = gst::ElementFactory::make("pipewiresrc")
        .property("fd", remote_fd.as_raw_fd())
        .property("path", node_id.to_string())
        .property("num-buffers", 1i32)
        .property("keepalive-time", 1000i32)
        .build()
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::PipeWireUnavailable,
                format!("failed to construct the pipewiresrc element: {error}"),
            )
        })?;
    let convert = gst::ElementFactory::make("videoconvert")
        .build()
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::PipeWireUnavailable,
                format!("failed to construct the videoconvert element: {error}"),
            )
        })?;
    let encoder = gst::ElementFactory::make("pngenc")
        .property("snapshot", true)
        .build()
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::PipeWireUnavailable,
                format!("failed to construct the pngenc element: {error}"),
            )
        })?;
    let sink = gst::ElementFactory::make("appsink")
        .property("emit-signals", false)
        .property("sync", false)
        .property("drop", true)
        .property("max-buffers", 1u32)
        .build()
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::PipeWireUnavailable,
                format!("failed to construct the appsink element: {error}"),
            )
        })?;
    let sink = sink.dynamic_cast::<gst_app::AppSink>().map_err(|_| {
        BackendError::new(
            BackendErrorCode::PipeWireUnavailable,
            "constructed appsink element could not be cast to gstreamer_app::AppSink",
        )
    })?;

    let pipeline = gst::Pipeline::new();
    pipeline
        .add_many([&source, &convert, &encoder, sink.upcast_ref()])
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::PipeWireUnavailable,
                format!("failed to assemble the in-process PipeWire capture pipeline: {error}"),
            )
        })?;
    gst::Element::link_many([&source, &convert, &encoder, sink.upcast_ref()]).map_err(|error| {
        BackendError::new(
            BackendErrorCode::PipeWireUnavailable,
            format!("failed to link the in-process PipeWire capture pipeline: {error}"),
        )
    })?;

    let result = (|| -> Result<(), BackendError> {
        pipeline.set_state(gst::State::Playing).map_err(|error| {
            BackendError::new(
                BackendErrorCode::PipeWireStreamFailed,
                format!("failed to start the PipeWire capture pipeline: {error}"),
            )
        })?;

        let sample = sink
            .try_pull_sample(gst::ClockTime::from_seconds(8))
            .ok_or_else(|| pipewire_sample_timeout_error(&pipeline, &sink))?;
        let buffer = sample.buffer().ok_or_else(|| {
            BackendError::new(
                BackendErrorCode::PipeWireStreamFailed,
                "PipeWire capture pipeline produced a sample without a buffer",
            )
        })?;
        let data = buffer.map_readable().map_err(|error| {
            BackendError::new(
                BackendErrorCode::PipeWireStreamFailed,
                format!("failed to map PipeWire capture bytes from the appsink buffer: {error}"),
            )
        })?;
        std::fs::write(&output_path, data.as_ref()).map_err(|error| {
            BackendError::new(
                BackendErrorCode::PipeWireStreamFailed,
                format!(
                    "failed to write the in-process PipeWire PNG capture to {}: {error}",
                    output_path.display()
                ),
            )
        })?;
        Ok(())
    })();

    let _ = pipeline.set_state(gst::State::Null);
    result
}

fn pipewire_sample_timeout_error(
    pipeline: &gst::Pipeline,
    sink: &gst_app::AppSink,
) -> BackendError {
    let message = pipeline.bus().and_then(|bus| {
        bus.timed_pop_filtered(
            gst::ClockTime::ZERO,
            &[gst::MessageType::Error, gst::MessageType::Eos],
        )
    });

    if let Some(message) = message {
        match message.view() {
            gst::MessageView::Error(error) => {
                let detail = error.debug().map(|debug| debug.to_string());
                return BackendError::new(
                    BackendErrorCode::PipeWireStreamFailed,
                    match detail {
                        Some(detail) => format!(
                            "in-process PipeWire capture failed: {} ({detail})",
                            error.error()
                        ),
                        None => format!("in-process PipeWire capture failed: {}", error.error()),
                    },
                );
            }
            gst::MessageView::Eos(_) => {
                return BackendError::new(
                    BackendErrorCode::PipeWireStreamFailed,
                    "in-process PipeWire capture reached EOS before a sample was pulled from appsink",
                );
            }
            _ => {}
        }
    }

    if sink.is_eos() {
        BackendError::new(
            BackendErrorCode::PipeWireStreamFailed,
            "in-process PipeWire capture appsink reached EOS before yielding a sample",
        )
    } else {
        BackendError::new(
            BackendErrorCode::PipeWireStreamFailed,
            "timed out waiting for an in-process PipeWire sample from appsink",
        )
    }
}

pub(crate) fn duplicate_remote_fd(remote_fd: &OwnedFd) -> Result<OwnedFd, BackendError> {
    let duplicated = unsafe { libc::dup(remote_fd.as_raw_fd()) };
    if duplicated < 0 {
        return Err(BackendError::new(
            BackendErrorCode::PipeWireUnavailable,
            format!(
                "failed to duplicate the cached PipeWire remote fd: {}",
                std::io::Error::last_os_error()
            ),
        ));
    }

    let duplicated = unsafe { OwnedFd::from_raw_fd(duplicated) };
    Ok(duplicated)
}

#[cfg(test)]
mod tests {
    use super::{
        FORCE_FAILURE_ENV, PipeWireFrameCapture, join_capture_task, should_force_capture_failure,
    };
    use serial_test::serial;
    use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};

    #[test]
    fn capture_struct_holds_a_path() {
        let capture = PipeWireFrameCapture {
            path: "/tmp/demo.png".into(),
            pixel_size: None,
        };
        assert_eq!(capture.path.to_string_lossy(), "/tmp/demo.png");
    }

    #[test]
    #[serial]
    fn forced_failure_flag_obeys_common_falsey_values() {
        let original = std::env::var_os(FORCE_FAILURE_ENV);
        unsafe { std::env::remove_var(FORCE_FAILURE_ENV) };
        assert!(!should_force_capture_failure());
        unsafe { std::env::set_var(FORCE_FAILURE_ENV, "0") };
        assert!(!should_force_capture_failure());
        unsafe { std::env::set_var(FORCE_FAILURE_ENV, "false") };
        assert!(!should_force_capture_failure());
        unsafe { std::env::set_var(FORCE_FAILURE_ENV, "1") };
        assert!(should_force_capture_failure());
        if let Some(value) = original {
            unsafe { std::env::set_var(FORCE_FAILURE_ENV, value) };
        } else {
            unsafe { std::env::remove_var(FORCE_FAILURE_ENV) };
        }
    }

    // Each test runs in its own nextest process, so setting this env var
    // before the first call to `capture_join_timeout()` (which caches it in
    // a `OnceLock`) is race-free within that process.
    #[tokio::test]
    async fn capture_join_task_exceeding_the_timeout_returns_pipewire_stream_failed() {
        unsafe { std::env::set_var("SKY_CUA_PIPEWIRE_CAPTURE_JOIN_TIMEOUT_MS", "50") };
        let handle = tokio::task::spawn_blocking(|| -> Result<(), BackendError> {
            // Long enough to comfortably outlast the 50ms test deadline;
            // `spawn_blocking` tasks are not abortable, so the tokio runtime
            // teardown still joins this thread after the test body returns.
            std::thread::sleep(std::time::Duration::from_millis(300));
            Ok(())
        });
        let started = std::time::Instant::now();
        let result = join_capture_task(handle).await;
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "join_capture_task should return promptly once the timeout elapses, took {:?}",
            started.elapsed()
        );
        let error =
            result.expect_err("a task exceeding the join timeout must be reported as an error");
        assert_eq!(error.code, BackendErrorCode::PipeWireStreamFailed.as_str());
        assert!(error.message.contains("join timeout"));
    }

    #[tokio::test]
    async fn capture_join_task_within_the_timeout_returns_the_task_result() {
        unsafe { std::env::set_var("SKY_CUA_PIPEWIRE_CAPTURE_JOIN_TIMEOUT_MS", "5000") };
        let handle = tokio::task::spawn_blocking(|| -> Result<u32, BackendError> { Ok(42) });
        let result = join_capture_task(handle)
            .await
            .expect("task within the timeout should succeed");
        assert_eq!(result, 42);
    }
}
