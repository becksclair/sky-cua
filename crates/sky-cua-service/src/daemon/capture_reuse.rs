use super::*;

pub(super) fn reuse_unchanged_capture(
    snapshot: &mut AppStateSnapshot,
    previous: Option<&AppStateSnapshot>,
) -> bool {
    let Some(previous) = previous else {
        return false;
    };
    let (Some(current_capture), Some(previous_capture)) =
        (snapshot.capture.as_mut(), previous.capture.as_ref())
    else {
        return false;
    };
    if !capture_metadata_compatible_for_reuse(current_capture, previous_capture) {
        return false;
    }
    let Some(current_path) = current_capture.screenshot_path.as_deref() else {
        return false;
    };
    let Some(previous_path) = comparable_previous_screenshot_path(previous_capture) else {
        return false;
    };
    let Ok(current_bytes) = fs::read(current_path) else {
        return false;
    };
    let Ok(previous_bytes) = fs::read(previous_path) else {
        return false;
    };
    if current_bytes != previous_bytes {
        return false;
    }

    current_capture.screenshot_path = previous_capture.screenshot_path.clone();
    current_capture.model_image_bytes = previous_capture.model_image_bytes;
    current_capture.model_image_encode_ms = previous_capture.model_image_encode_ms;
    true
}

fn comparable_previous_screenshot_path(capture: &CaptureInfo) -> Option<String> {
    let screenshot_path = capture.screenshot_path.as_deref()?;
    let path = Path::new(screenshot_path);
    if let Some(raw_path) = decomposited_screenshot_path(path)
        && raw_path.is_file()
    {
        return Some(raw_path.display().to_string());
    }
    Some(screenshot_path.to_string())
}

fn decomposited_screenshot_path(path: &Path) -> Option<PathBuf> {
    let stem = path.file_stem()?.to_str()?;
    let raw_stem = stem.strip_suffix(".agent-cursor")?;
    let extension = path.extension()?;
    Some(path.with_file_name(Path::new(raw_stem).with_extension(extension)))
}

fn capture_metadata_compatible_for_reuse(current: &CaptureInfo, previous: &CaptureInfo) -> bool {
    current.backend == previous.backend
        && current.image_backend == previous.image_backend
        && current.coordinate_space == previous.coordinate_space
        && current.pixel_size == previous.pixel_size
        && current.original_pixel_size == previous.original_pixel_size
        && current.logical_to_pixel_scale == previous.logical_to_pixel_scale
        && current.logical_rect == previous.logical_rect
        && current.model_image_format == previous.model_image_format
        && current.model_image_quality == previous.model_image_quality
}
