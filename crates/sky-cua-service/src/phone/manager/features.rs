use std::time::Duration;

use serde::{Serialize, de::DeserializeOwned};
use sky_cua_platform::model::{
    ContentPersistence, ContentRef, ContentSource, PhoneCameraRequest, PhoneCameraResponse,
    PhoneClipboardRequest, PhoneClipboardResponse, PhoneContentRequest, PhoneContentResponse,
    PhoneEditorRequest, PhoneEditorResponse, PhoneFeatureCall, PhoneFeatureError,
    PhoneSessionSelector, PhoneStorageRequest, PhoneStorageResponse,
};

use super::PhoneManager;

impl PhoneManager {
    async fn direct_feature<T: DeserializeOwned, R: Serialize>(
        &self,
        session: &PhoneSessionSelector,
        method: &str,
        request: &R,
        idempotent: bool,
    ) -> Result<T, PhoneFeatureError> {
        let session_id = self
            .resolve_session_id(session)
            .ok_or_else(|| feature_error("session_not_found", "phone session was not found"))?;
        let (device_id, epoch) = self.direct_identity(&session_id).ok_or_else(|| {
            feature_error(
                "provider_unavailable",
                "phone feature requires an authenticated direct Companion session",
            )
        })?;
        let provider = self.direct_provider.as_ref().ok_or_else(|| {
            feature_error(
                "provider_unavailable",
                "direct Companion provider is unavailable",
            )
        })?;
        let params = serde_json::to_value(request)
            .map_err(|error| feature_error("invalid_request", error.to_string()))?;
        let result = provider
            .dispatch(
                &device_id,
                epoch,
                method,
                params,
                idempotent,
                Duration::from_secs(30),
            )
            .await
            .map_err(direct_feature_error)?;
        serde_json::from_value(result)
            .map_err(|error| feature_error("invalid_response", error.to_string()))
    }

    pub(super) async fn phone_content(
        &self,
        call: PhoneFeatureCall<PhoneContentRequest>,
    ) -> Result<PhoneContentResponse, PhoneFeatureError> {
        let Some(session_id) = self.resolve_session_id(&call.session) else {
            return Err(feature_error(
                "session_not_found",
                "phone session was not found",
            ));
        };
        let Some((device_id, epoch)) = self.direct_identity(&session_id) else {
            return Err(feature_error(
                "provider_unavailable",
                "phone content requires an authenticated direct Companion session",
            ));
        };
        let Some(provider) = &self.direct_provider else {
            return Err(feature_error(
                "provider_unavailable",
                "direct Companion provider is unavailable",
            ));
        };
        let runtime = provider.runtime();
        match call.request {
            PhoneContentRequest::Describe { content_id } => {
                let content = runtime
                    .describe_content_artifact(&device_id, epoch, &content_id)
                    .ok()
                    .map(
                        |(sha256, size_bytes, mime_type, expires_at_ms)| ContentRef {
                            content_id: content_id.clone(),
                            device_id: Some(device_id),
                            link_epoch: Some(epoch),
                            mime_type,
                            filename: None,
                            size_bytes,
                            sha256,
                            source: ContentSource::CompanionBlob,
                            expires_at_ms: Some(expires_at_ms),
                            persistence: ContentPersistence::Temporary,
                        },
                    );
                if let Some(content) = content {
                    Ok(PhoneContentResponse {
                        content: Some(content),
                        path: None,
                        released: false,
                    })
                } else {
                    self.direct_feature(
                        &call.session,
                        "content.describe",
                        &PhoneContentRequest::Describe { content_id },
                        true,
                    )
                    .await
                }
            }
            PhoneContentRequest::Release { content_id } => {
                if runtime
                    .release_content_artifact(&device_id, epoch, &content_id)
                    .is_ok()
                {
                    Ok(PhoneContentResponse {
                        content: None,
                        path: None,
                        released: true,
                    })
                } else {
                    self.direct_feature(
                        &call.session,
                        "content.release",
                        &PhoneContentRequest::Release { content_id },
                        false,
                    )
                    .await
                }
            }
            PhoneContentRequest::ExportHostFile { content_id, path } => {
                if runtime
                    .describe_content_artifact(&device_id, epoch, &content_id)
                    .is_err()
                {
                    let _: PhoneContentResponse = self
                        .direct_feature(
                            &call.session,
                            "content.export",
                            &PhoneContentRequest::Describe {
                                content_id: content_id.clone(),
                            },
                            true,
                        )
                        .await?;
                }
                let content = runtime
                    .describe_content_artifact(&device_id, epoch, &content_id)
                    .ok()
                    .map(
                        |(sha256, size_bytes, mime_type, expires_at_ms)| ContentRef {
                            content_id,
                            device_id: Some(device_id.clone()),
                            link_epoch: Some(epoch),
                            mime_type,
                            filename: None,
                            size_bytes,
                            sha256,
                            source: ContentSource::CompanionBlob,
                            expires_at_ms: Some(expires_at_ms),
                            persistence: ContentPersistence::Temporary,
                        },
                    );
                let written = content
                    .as_ref()
                    .and_then(|reference| {
                        runtime
                            .read_content_artifact(&device_id, epoch, reference)
                            .ok()
                    })
                    .and_then(|bytes| {
                        let target = std::path::PathBuf::from(&path);
                        let parent = target.parent()?;
                        std::fs::create_dir_all(parent).ok()?;
                        let temporary = target.with_extension("sky-cua-part");
                        std::fs::write(&temporary, bytes).ok()?;
                        std::fs::rename(temporary, &target).ok()?;
                        Some(path.clone())
                    });
                match (content, written) {
                    (Some(content), Some(path)) => Ok(PhoneContentResponse {
                        content: Some(content),
                        path: Some(path),
                        released: false,
                    }),
                    _ => Err(feature_error(
                        "content_export_failed",
                        "content could not be verified and exported to the requested host path",
                    )),
                }
            }
            PhoneContentRequest::ImportHostFile { path, mime_type } => {
                let bytes = std::fs::read(&path)
                    .map_err(|error| feature_error("host_file_read_failed", error.to_string()))?;
                let filename = std::path::Path::new(&path)
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned());
                let content = provider
                    .send_content(&device_id, epoch, &bytes, &mime_type, filename)
                    .await
                    .map_err(direct_feature_error)?;
                Ok(PhoneContentResponse {
                    content: Some(content),
                    path: None,
                    released: false,
                })
            }
        }
    }

    pub(super) async fn phone_clipboard(
        &self,
        call: PhoneFeatureCall<PhoneClipboardRequest>,
    ) -> Result<PhoneClipboardResponse, PhoneFeatureError> {
        let idempotent = matches!(
            call.request,
            PhoneClipboardRequest::Get | PhoneClipboardRequest::Changes { .. }
        );
        self.direct_feature(&call.session, "clipboard", &call.request, idempotent)
            .await
    }

    pub(super) async fn phone_editor(
        &self,
        call: PhoneFeatureCall<PhoneEditorRequest>,
    ) -> Result<PhoneEditorResponse, PhoneFeatureError> {
        let idempotent = matches!(call.request, PhoneEditorRequest::Context);
        self.direct_feature(&call.session, "editor", &call.request, idempotent)
            .await
    }

    pub(super) async fn phone_camera(
        &self,
        call: PhoneFeatureCall<PhoneCameraRequest>,
    ) -> Result<PhoneCameraResponse, PhoneFeatureError> {
        let idempotent = matches!(
            call.request,
            PhoneCameraRequest::Enumerate
                | PhoneCameraRequest::Capabilities { .. }
                | PhoneCameraRequest::PreviewFrame { .. }
        );
        self.direct_feature(&call.session, "camera", &call.request, idempotent)
            .await
    }

    pub(super) async fn phone_storage(
        &self,
        call: PhoneFeatureCall<PhoneStorageRequest>,
    ) -> Result<PhoneStorageResponse, PhoneFeatureError> {
        let idempotent = matches!(
            call.request,
            PhoneStorageRequest::Roots
                | PhoneStorageRequest::List { .. }
                | PhoneStorageRequest::Stat { .. }
                | PhoneStorageRequest::Read { .. }
                | PhoneStorageRequest::Hash { .. }
                | PhoneStorageRequest::Search { .. }
                | PhoneStorageRequest::Thumbnail { .. }
                | PhoneStorageRequest::Metadata { .. }
                | PhoneStorageRequest::ListSafRoots
        );
        self.direct_feature(&call.session, "storage", &call.request, idempotent)
            .await
    }
}

fn feature_error(code: impl Into<String>, message: impl Into<String>) -> PhoneFeatureError {
    PhoneFeatureError {
        code: code.into(),
        message: message.into(),
    }
}

fn direct_feature_error(error: super::super::direct::DirectRuntimeError) -> PhoneFeatureError {
    use super::super::direct::DirectRuntimeError;
    match error {
        DirectRuntimeError::Remote { code, message } => feature_error(code, message),
        DirectRuntimeError::NotConnected | DirectRuntimeError::Disconnected => {
            feature_error("not_connected", "direct Companion link is not connected")
        }
        DirectRuntimeError::LinkEpochChanged { .. } => {
            feature_error("epoch_mismatch", "direct Companion link epoch changed")
        }
        DirectRuntimeError::DeadlineExceeded => feature_error(
            "deadline_exceeded",
            "direct Companion request deadline elapsed",
        ),
        DirectRuntimeError::Protocol(message) => feature_error("protocol_error", message),
    }
}
