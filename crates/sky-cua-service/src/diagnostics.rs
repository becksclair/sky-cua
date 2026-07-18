use sky_cua_platform::model::ServiceResponse;

pub fn error_response(code: impl Into<String>, message: impl Into<String>) -> ServiceResponse {
    ServiceResponse::Error {
        ok: false,
        code: code.into(),
        message: message.into(),
        session_id: None,
        turn_id: None,
        retry: None,
    }
}
