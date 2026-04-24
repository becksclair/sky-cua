use std::path::PathBuf;

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};
use sky_cua_platform::model::PixelSize;

use crate::portal::screenshot;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct X11Capture {
    pub path: PathBuf,
    pub pixel_size: Option<PixelSize>,
}

pub fn x11_capture_available() -> bool {
    crate::x11::windowing::x11_server_running()
}

pub async fn capture_still(snapshot_id: &str) -> Result<X11Capture, BackendError> {
    let output_path = screenshot::capture_output_path(snapshot_id);
    if let Some(parent) = output_path.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|error| {
            BackendError::new(
                BackendErrorCode::Internal,
                format!(
                    "failed to create X11 capture directory {}: {error}",
                    parent.display()
                ),
            )
        })?;
    }
    if tokio::fs::try_exists(&output_path).await.unwrap_or(false) {
        let _ = tokio::fs::remove_file(&output_path).await;
    }

    let blocking_output_path = output_path.clone();
    tokio::task::spawn_blocking(move || capture_still_blocking(blocking_output_path))
        .await
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::Internal,
                format!("X11 capture task failed to join cleanly: {error}"),
            )
        })??;

    let pixel_size = screenshot::pixel_size_from_path(&output_path);
    Ok(X11Capture {
        path: output_path,
        pixel_size,
    })
}

fn capture_still_blocking(output_path: PathBuf) -> Result<(), BackendError> {
    gst::init().map_err(|error| {
        BackendError::new(
            BackendErrorCode::Internal,
            format!("failed to initialize GStreamer for X11 capture: {error}"),
        )
    })?;

    let source = gst::ElementFactory::make("ximagesrc")
        .property("use-damage", false)
        .property("show-pointer", true)
        .property("num-buffers", 1i32)
        .build()
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::Internal,
                format!("failed to construct the ximagesrc element: {error}"),
            )
        })?;
    let convert = gst::ElementFactory::make("videoconvert")
        .build()
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::Internal,
                format!("failed to construct the videoconvert element for X11 capture: {error}"),
            )
        })?;
    let encoder = gst::ElementFactory::make("pngenc")
        .property("snapshot", true)
        .build()
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::Internal,
                format!("failed to construct the pngenc element for X11 capture: {error}"),
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
                BackendErrorCode::Internal,
                format!("failed to construct the appsink element for X11 capture: {error}"),
            )
        })?;
    let sink = sink.dynamic_cast::<gst_app::AppSink>().map_err(|_| {
        BackendError::new(
            BackendErrorCode::Internal,
            "constructed X11 appsink element could not be cast to gstreamer_app::AppSink",
        )
    })?;

    let pipeline = gst::Pipeline::new();
    pipeline
        .add_many([&source, &convert, &encoder, sink.upcast_ref()])
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::Internal,
                format!("failed to assemble the X11 capture pipeline: {error}"),
            )
        })?;
    gst::Element::link_many([&source, &convert, &encoder, sink.upcast_ref()]).map_err(|error| {
        BackendError::new(
            BackendErrorCode::Internal,
            format!("failed to link the X11 capture pipeline: {error}"),
        )
    })?;

    let result = (|| -> Result<(), BackendError> {
        pipeline.set_state(gst::State::Playing).map_err(|error| {
            BackendError::new(
                BackendErrorCode::Internal,
                format!("failed to start the X11 capture pipeline: {error}"),
            )
        })?;

        let sample = sink
            .try_pull_sample(gst::ClockTime::from_seconds(8))
            .ok_or_else(|| x11_capture_timeout_error(&pipeline, &sink))?;
        let buffer = sample.buffer().ok_or_else(|| {
            BackendError::new(
                BackendErrorCode::Internal,
                "X11 capture pipeline produced a sample without a buffer",
            )
        })?;
        let data = buffer.map_readable().map_err(|error| {
            BackendError::new(
                BackendErrorCode::Internal,
                format!("failed to map X11 capture bytes from the appsink buffer: {error}"),
            )
        })?;
        std::fs::write(&output_path, data.as_ref()).map_err(|error| {
            BackendError::new(
                BackendErrorCode::Internal,
                format!(
                    "failed to write the X11 PNG capture to {}: {error}",
                    output_path.display()
                ),
            )
        })?;
        Ok(())
    })();

    let _ = pipeline.set_state(gst::State::Null);
    result
}

fn x11_capture_timeout_error(pipeline: &gst::Pipeline, sink: &gst_app::AppSink) -> BackendError {
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
                    BackendErrorCode::Internal,
                    match detail {
                        Some(detail) => {
                            format!(
                                "in-process X11 capture failed: {} ({detail})",
                                error.error()
                            )
                        }
                        None => format!("in-process X11 capture failed: {}", error.error()),
                    },
                );
            }
            gst::MessageView::Eos(_) => {
                return BackendError::new(
                    BackendErrorCode::Internal,
                    "in-process X11 capture reached EOS before a sample was pulled from appsink",
                );
            }
            _ => {}
        }
    }

    if sink.is_eos() {
        BackendError::new(
            BackendErrorCode::Internal,
            "in-process X11 capture appsink reached EOS before yielding a sample",
        )
    } else {
        BackendError::new(
            BackendErrorCode::Internal,
            "timed out waiting for an in-process X11 sample from appsink",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::X11Capture;

    #[test]
    fn capture_struct_holds_a_path() {
        let capture = X11Capture {
            path: "/tmp/x11-demo.png".into(),
            pixel_size: None,
        };
        assert_eq!(capture.path.to_string_lossy(), "/tmp/x11-demo.png");
    }
}
