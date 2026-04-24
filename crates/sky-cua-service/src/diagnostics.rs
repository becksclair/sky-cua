use sky_cua_platform::model::ServiceResponse;

pub fn error_response(code: impl Into<String>, message: impl Into<String>) -> ServiceResponse {
    ServiceResponse::Error {
        code: code.into(),
        message: message.into(),
    }
}
