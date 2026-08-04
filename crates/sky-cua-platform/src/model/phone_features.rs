//! Grouped direct-Companion feature request contracts.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::{ContentRef, PhoneSessionSelector};

/// V1 camera capture is deliberately bounded at the phone. Captured media
/// remains phone-local until a separate content-export request is made.
pub const PHONE_CAMERA_V1_MAX_WIDTH: u32 = 1920;
pub const PHONE_CAMERA_V1_MAX_HEIGHT: u32 = 1080;
pub const PHONE_CAMERA_V1_MAX_VIDEO_DURATION_MS: u64 = 60_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneFeatureError {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PhoneFeatureCall<T> {
    #[serde(flatten)]
    pub session: PhoneSessionSelector,
    #[serde(flatten)]
    pub request: T,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum PhoneContentRequest {
    Describe { content_id: String },
    ImportHostFile { path: String, mime_type: String },
    ExportHostFile { content_id: String, path: String },
    Release { content_id: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PhoneContentResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<ContentRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub released: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PhoneClipboardItem {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub html: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<ContentRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent_uri: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mime_types: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PhoneClipboardPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<PhoneClipboardItem>,
    pub sensitive: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changed_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum PhoneClipboardRequest {
    Get,
    Set { payload: PhoneClipboardPayload },
    Clear,
    Changes { since_sequence: u64, limit: u32 },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PhoneClipboardResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<PhoneClipboardPayload>,
    pub sequence: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changes: Vec<PhoneClipboardPayload>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum PhoneEditorRequest {
    Context,
    SetText { text: String },
    InsertText { text: String },
    SetSelection { start: i32, end: i32 },
    SelectAll,
    Copy,
    Cut,
    Paste,
    InsertContent { content: ContentRef },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PhoneEditorResponse {
    pub outcome: PhoneEditorOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surrounding_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection_start: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection_end: Option<i32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accepted_mime_types: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PhoneEditorOutcome {
    Applied,
    InsertedDirectly,
    PastedThroughClipboard,
    AttachedThroughUi,
    UnsupportedMimeType,
    NoEditableTarget,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PhoneCameraFacing {
    Front,
    Back,
    External,
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneMediaSize {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneFpsRange {
    pub min: u32,
    pub max: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PhoneFlashMode {
    Off,
    On,
    Auto,
    Screen,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneVideoProfile {
    pub size: PhoneMediaSize,
    pub fps: u32,
    pub video_mime_type: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub audio_mime_types: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PhoneCameraDescriptor {
    pub camera_id: String,
    pub facing: PhoneCameraFacing,
    pub logical: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub physical_camera_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub photo_sizes: Vec<PhoneMediaSize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub video_profiles: Vec<PhoneVideoProfile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fps_ranges: Vec<PhoneFpsRange>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flash_modes: Vec<PhoneFlashMode>,
    pub hardware_torch: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_torch_strength: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_zoom: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_zoom: Option<f32>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub vendor_extensions: BTreeMap<String, serde_json::Value>,
    pub max_capture_size: PhoneMediaSize,
    pub max_video_duration_ms: u64,
    pub automatic_media_transfer: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct PhoneCameraOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<PhoneMediaSize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fps: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flash: Option<PhoneFlashMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_audio: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct PhoneCameraControls {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zoom: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus_x: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus_y: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exposure_compensation: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub torch_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub torch_strength: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stabilization_mode: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneCameraMediaMetadata {
    pub camera_id: String,
    pub size: PhoneMediaSize,
    pub mime_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation_degrees: Option<i32>,
    pub audio_included: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum PhoneCameraRequest {
    Enumerate,
    Capabilities {
        camera_id: String,
    },
    Photo {
        camera_id: String,
        options: PhoneCameraOptions,
    },
    VideoStart {
        camera_id: String,
        options: PhoneCameraOptions,
    },
    VideoPause {
        camera_session_id: String,
    },
    VideoResume {
        camera_session_id: String,
    },
    VideoStop {
        camera_session_id: String,
    },
    PreviewStart {
        camera_id: String,
        options: PhoneCameraOptions,
    },
    PreviewFrame {
        camera_session_id: String,
    },
    PreviewStop {
        camera_session_id: String,
    },
    Controls {
        camera_session_id: String,
        controls: PhoneCameraControls,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PhoneCameraResponse {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cameras: Vec<PhoneCameraDescriptor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<ContentRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<PhoneCameraMediaMetadata>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PhoneStorageEntryKind {
    File,
    Directory,
    Collection,
    Content,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PhoneStorageRootKind {
    CompanionPrivate,
    Shared,
    MediaStore,
    Saf,
    ContentUri,
    Temporary,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneStorageFeatures {
    pub read: bool,
    pub write: bool,
    pub random_access: bool,
    pub rename: bool,
    pub delete: bool,
    pub trash: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneStorageEntry {
    pub uri: String,
    pub name: String,
    pub kind: PhoneStorageEntryKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified_at_ms: Option<u64>,
    pub features: PhoneStorageFeatures,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhoneStorageRoot {
    pub root_id: String,
    pub uri: String,
    pub display_name: String,
    pub kind: PhoneStorageRootKind,
    pub features: PhoneStorageFeatures,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PhoneStorageMetadata {
    pub entry: PhoneStorageEntry,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub vendor_extensions: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum PhoneStorageRequest {
    Roots,
    List {
        uri: String,
    },
    Stat {
        uri: String,
    },
    Read {
        uri: String,
    },
    Write {
        uri: String,
        content: ContentRef,
    },
    Mkdir {
        uri: String,
    },
    Copy {
        source: String,
        destination: String,
    },
    Move {
        source: String,
        destination: String,
    },
    Rename {
        uri: String,
        name: String,
    },
    Delete {
        uri: String,
    },
    Trash {
        uri: String,
    },
    Hash {
        uri: String,
        algorithm: String,
    },
    Search {
        root: String,
        query: String,
        limit: u32,
    },
    Thumbnail {
        uri: String,
        max_width: u32,
        max_height: u32,
    },
    Metadata {
        uri: String,
    },
    AddSafRoot,
    RemoveSafRoot {
        root_id: String,
    },
    ListSafRoots,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PhoneStorageResponse {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<PhoneStorageEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roots: Vec<PhoneStorageRoot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<ContentRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<PhoneStorageMetadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_uri: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grouped_operations_keep_exact_family_specific_tags() {
        let request = PhoneStorageRequest::Copy {
            source: "shared://primary/a".into(),
            destination: "shared://primary/b".into(),
        };
        let value = serde_json::to_value(request).expect("serializes");
        assert_eq!(value["operation"], "copy");
        assert_eq!(value["source"], "shared://primary/a");
    }

    #[test]
    fn camera_descriptor_exposes_bounded_local_capture_contract() {
        let descriptor: PhoneCameraDescriptor = serde_json::from_value(serde_json::json!({
            "camera_id": "0",
            "facing": "back",
            "logical": true,
            "hardware_torch": true,
            "vendor_extensions": {},
            "max_capture_size": {
                "width": PHONE_CAMERA_V1_MAX_WIDTH,
                "height": PHONE_CAMERA_V1_MAX_HEIGHT
            },
            "max_video_duration_ms": PHONE_CAMERA_V1_MAX_VIDEO_DURATION_MS,
            "automatic_media_transfer": false
        }))
        .expect("bounded camera descriptor");
        assert_eq!(
            descriptor.max_capture_size,
            PhoneMediaSize {
                width: 1920,
                height: 1080
            }
        );
        assert_eq!(descriptor.max_video_duration_ms, 60_000);
        assert!(!descriptor.automatic_media_transfer);
    }
}
