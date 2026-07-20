use chrono::{DateTime, Utc};

use super::*;

const MAX_AUTH_WINDOW_SECONDS: i64 = 5 * 60;
const MAX_REASON_CHARS: usize = 512;
const MAX_LABEL_CHARS: usize = 128;
const MAX_SELECTOR_BYTES: usize = 16 * 1024;
const MAX_SELECTOR_DEPTH: usize = 12;

pub(super) async fn execute(
    shared: &Shared,
    actor: &BridgeActor,
    operation: &super::super::DispatchOperation,
    method: &str,
    params: &Value,
    timeout_ms: u64,
    identity: &BrowserSessionIdentity,
) -> Option<ExecutorOutcome> {
    match method {
        "reportBotDetection" => Some(report_bot_detection(shared, operation, params)),
        "browserAuthHandoff" => {
            Some(browser_auth_handoff(shared, actor, operation, params, timeout_ms, identity).await)
        }
        _ => None,
    }
}

fn report_bot_detection(
    shared: &Shared,
    operation: &super::super::DispatchOperation,
    params: &Value,
) -> ExecutorOutcome {
    let result = (|| {
        let object = required_object(params, "reportBotDetection params")?;
        validate_tab_id(object.get("tabId"), operation)?;
        let reason = required_string(object.get("reason"), "reason", MAX_LABEL_CHARS)?;
        if !matches!(
            reason,
            "captcha_failed" | "access_denied" | "challenge_loop" | "unexpected_bot_error"
        ) {
            return Err("reportBotDetection reason is not recognized".to_owned());
        }
        let hostname = object
            .get("hostname")
            .and_then(Value::as_str)
            .ok_or_else(|| "reportBotDetection requires hostname".to_owned())?;
        let hostname = validate_hostname(hostname)?;
        Ok((reason, hostname))
    })();
    let (reason, hostname) = match result {
        Ok(result) => result,
        Err(error) => return ExecutorOutcome::DefinitiveFailure(error),
    };

    record_host_operation(
        shared,
        operation,
        format!(
            "bot_detection_reported:{reason}:{}",
            hostname.as_deref().unwrap_or("redacted")
        ),
    );
    ExecutorOutcome::DefinitiveSuccess(
        json!({ "status": "reported", "hostname": hostname }).to_string(),
    )
}

async fn browser_auth_handoff(
    shared: &Shared,
    actor: &BridgeActor,
    operation: &super::super::DispatchOperation,
    params: &Value,
    timeout_ms: u64,
    identity: &BrowserSessionIdentity,
) -> ExecutorOutcome {
    let request = match validate_auth_request(params, operation) {
        Ok(request) => request,
        Err(error) => return ExecutorOutcome::DefinitiveFailure(error),
    };
    if request.expires_at <= Utc::now() {
        record_host_operation(shared, operation, "browser_auth_expired".to_owned());
        return auth_status("expired");
    }

    let mut preflight = BridgeActorRequest::new(
        "executeCdp",
        json!({
            "target": { "tabId": tab_id_value(operation) },
            "method": "Runtime.evaluate",
            "commandParams": {
                "expression": auth_preflight_expression(&request.string_selectors),
                "awaitPromise": false,
                "returnByValue": true,
            },
        }),
        format!(
            "{}:browser-auth-preflight",
            operation.identity.operation_id.0
        ),
        OperationClass::ReadOnly,
    );
    preflight.timeout = Duration::from_millis(timeout_ms.clamp(1, 5_000));
    preflight.target_lifetime_key = operation_target(&operation.scope);
    let response = match actor.request(preflight).await {
        Ok(response) => response,
        Err(BridgeActorError::UpstreamError(error)) => {
            return ExecutorOutcome::DefinitiveFailure(format!(
                "__SKY_CUA_UPSTREAM_ERROR__{error}"
            ));
        }
        Err(error) => {
            return ExecutorOutcome::DefinitiveFailure(format!(
                "browserAuthHandoff preflight failed: {error:?}"
            ));
        }
    };
    let observed = match parse_auth_preflight(&response) {
        Ok(observed) => observed,
        Err(status) => {
            record_host_operation(shared, operation, format!("browser_auth_{status}"));
            return auth_status(status);
        }
    };
    if observed.origin != request.origin {
        record_host_operation(shared, operation, "browser_auth_origin_changed".to_owned());
        return auth_status("origin_changed");
    }
    if !observed.selectors_valid {
        record_host_operation(shared, operation, "browser_auth_locator_invalid".to_owned());
        return auth_status("locator_invalid");
    }
    if request.expires_at <= Utc::now() {
        record_host_operation(shared, operation, "browser_auth_expired".to_owned());
        return auth_status("expired");
    }

    // Linux v1 deliberately ships no credential handoff UI. The request is
    // still admitted as a real tab mutation and validated against live page
    // state, but no credential values are collected or sent over the bridge.
    let _ = identity;
    record_host_operation(shared, operation, "browser_auth_unavailable".to_owned());
    auth_status("unavailable")
}

#[derive(Debug)]
struct ValidatedAuthRequest {
    origin: String,
    expires_at: DateTime<Utc>,
    string_selectors: Vec<String>,
}

fn validate_auth_request(
    params: &Value,
    operation: &super::super::DispatchOperation,
) -> Result<ValidatedAuthRequest, String> {
    let object = required_object(params, "browserAuthHandoff params")?;
    validate_tab_id(object.get("tabId"), operation)?;
    reject_credential_value_keys(object)?;

    let origin = required_string(object.get("origin"), "origin", 2_048)?;
    if !valid_origin(origin) {
        return Err("browserAuthHandoff origin must be an absolute HTTP(S) origin".to_owned());
    }
    required_string(object.get("reason"), "reason", MAX_REASON_CHARS)?;

    let expires_at = required_string(object.get("expires_at"), "expires_at", 64)?;
    let expires_at = DateTime::parse_from_rfc3339(expires_at)
        .map_err(|_| "browserAuthHandoff expires_at must be RFC 3339".to_owned())?
        .with_timezone(&Utc);
    let now = Utc::now();
    if expires_at.signed_duration_since(now).num_seconds() > MAX_AUTH_WINDOW_SECONDS {
        return Err("browserAuthHandoff expires_at exceeds five minutes".to_owned());
    }

    let fields = object
        .get("fields")
        .and_then(Value::as_array)
        .ok_or_else(|| "browserAuthHandoff fields must be an array".to_owned())?;
    if !(1..=6).contains(&fields.len()) {
        return Err("browserAuthHandoff requires between one and six fields".to_owned());
    }
    let mut ids = HashSet::new();
    let mut string_selectors = Vec::new();
    for field in fields {
        let field = required_object(field, "browserAuthHandoff field")?;
        require_exact_keys(
            field,
            &[
                "id",
                "label",
                "type",
                "autocomplete",
                "required",
                "selector",
            ],
            "browserAuthHandoff field",
        )?;
        let id = required_string(field.get("id"), "field id", 64)?;
        if !valid_field_id(id) || !ids.insert(id) {
            return Err("browserAuthHandoff field ids must be unique identifiers".to_owned());
        }
        required_string(field.get("label"), "field label", MAX_LABEL_CHARS)?;
        let input_type = required_string(field.get("type"), "field type", 64)?;
        if !input_type
            .chars()
            .all(|character| character.is_ascii_lowercase() || character == '-')
        {
            return Err("browserAuthHandoff field type is invalid".to_owned());
        }
        required_string(field.get("autocomplete"), "field autocomplete", 128)?;
        if !field.get("required").is_some_and(Value::is_boolean) {
            return Err("browserAuthHandoff field required must be boolean".to_owned());
        }
        validate_selector(field.get("selector"), &mut string_selectors)?;
    }

    if let Some(submit) = object.get("submit") {
        let submit = required_object(submit, "browserAuthHandoff submit")?;
        require_exact_keys(submit, &["selector", "action"], "browserAuthHandoff submit")?;
        validate_selector(submit.get("selector"), &mut string_selectors)?;
        let action = required_string(submit.get("action"), "submit action", 32)?;
        if !matches!(action, "click" | "press_enter") {
            return Err("browserAuthHandoff submit action is invalid".to_owned());
        }
    }

    Ok(ValidatedAuthRequest {
        origin: origin.to_owned(),
        expires_at,
        string_selectors,
    })
}

fn validate_selector(
    value: Option<&Value>,
    string_selectors: &mut Vec<String>,
) -> Result<(), String> {
    let value = value.ok_or_else(|| "browserAuthHandoff selector is required".to_owned())?;
    if let Some(selector) = value.as_str() {
        if selector.trim().is_empty()
            || selector.len() > MAX_SELECTOR_BYTES
            || selector.chars().any(char::is_control)
        {
            return Err("browserAuthHandoff selector is invalid".to_owned());
        }
        string_selectors.push(selector.to_owned());
        return Ok(());
    }
    let encoded = serde_json::to_vec(value)
        .map_err(|_| "browserAuthHandoff selector is invalid".to_owned())?;
    if encoded.len() > MAX_SELECTOR_BYTES {
        return Err("browserAuthHandoff selector is invalid".to_owned());
    }
    let descriptor = value.get("descriptor").unwrap_or(value);
    validate_locator_descriptor(descriptor, 0)
}

fn validate_locator_descriptor(value: &Value, depth: usize) -> Result<(), String> {
    if depth >= MAX_SELECTOR_DEPTH {
        return Err("browserAuthHandoff locator is too deeply nested".to_owned());
    }
    let object = required_object(value, "browserAuthHandoff locator")?;
    require_exact_keys(
        object,
        &["kind", "args", "parent"],
        "browserAuthHandoff locator",
    )?;
    let kind = required_string(object.get("kind"), "locator kind", 64)?;
    if !matches!(
        kind,
        "locator"
            | "frameLocator"
            | "getByLabel"
            | "getByPlaceholder"
            | "getByRole"
            | "getByTestId"
            | "getByText"
            | "first"
            | "last"
            | "nth"
            | "filter"
            | "and"
            | "or"
    ) {
        return Err("browserAuthHandoff locator kind is invalid".to_owned());
    }
    if let Some(args) = object.get("args")
        && args.as_array().is_none_or(|args| args.len() > 8)
    {
        return Err("browserAuthHandoff locator args are invalid".to_owned());
    }
    if let Some(parent) = object.get("parent") {
        validate_locator_descriptor(parent, depth + 1)?;
    }
    Ok(())
}

fn reject_credential_value_keys(object: &serde_json::Map<String, Value>) -> Result<(), String> {
    const FORBIDDEN: &[&str] = &[
        "credential",
        "credentials",
        "password",
        "secret",
        "token",
        "value",
        "values",
    ];
    if object.keys().any(|key| FORBIDDEN.contains(&key.as_str())) {
        return Err("browserAuthHandoff must not contain credential values".to_owned());
    }
    Ok(())
}

fn require_exact_keys(
    object: &serde_json::Map<String, Value>,
    allowed: &[&str],
    label: &str,
) -> Result<(), String> {
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(format!("{label} contains an unsupported property"));
    }
    Ok(())
}

fn valid_field_id(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && chars.all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.')
        })
}

fn valid_origin(value: &str) -> bool {
    if value.trim() != value || value.chars().any(char::is_control) {
        return false;
    }
    let Some((scheme, authority)) = value.split_once("://") else {
        return false;
    };
    matches!(scheme, "http" | "https")
        && !authority.is_empty()
        && !authority.contains(['/', '?', '#', '@'])
        && authority != "."
}

fn validate_hostname(value: &str) -> Result<Option<String>, String> {
    if value.is_empty() {
        return Ok(None);
    }
    if value.len() > 253
        || value.trim() != value
        || value.chars().any(char::is_control)
        || value.contains(['/', '?', '#', '@'])
    {
        return Err("reportBotDetection hostname is invalid".to_owned());
    }
    Ok(Some(value.to_owned()))
}

fn required_object<'a>(
    value: &'a Value,
    label: &str,
) -> Result<&'a serde_json::Map<String, Value>, String> {
    value
        .as_object()
        .ok_or_else(|| format!("{label} must be an object"))
}

fn required_string<'a>(
    value: Option<&'a Value>,
    label: &str,
    max_chars: usize,
) -> Result<&'a str, String> {
    let value = value
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("browserAuthHandoff {label} must be a non-empty string"))?;
    if value.chars().count() > max_chars || value.chars().any(char::is_control) {
        return Err(format!("browserAuthHandoff {label} is invalid"));
    }
    Ok(value)
}

fn validate_tab_id(
    value: Option<&Value>,
    operation: &super::super::DispatchOperation,
) -> Result<(), String> {
    let requested = value
        .and_then(|value| match value {
            Value::String(value) => Some(value.clone()),
            Value::Number(value) if value.is_u64() => Some(value.to_string()),
            _ => None,
        })
        .ok_or_else(|| "raw host command requires tabId".to_owned())?;
    let OperationScope::Tab(tab) = &operation.scope else {
        return Err("raw host command must be tab scoped".to_owned());
    };
    if requested != tab.tab_id {
        return Err("raw host command tabId does not match its owned tab scope".to_owned());
    }
    Ok(())
}

fn tab_id_value(operation: &super::super::DispatchOperation) -> Value {
    let OperationScope::Tab(tab) = &operation.scope else {
        unreachable!("validated browser auth operation is tab scoped")
    };
    tab.tab_id
        .parse::<u64>()
        .map(Value::from)
        .unwrap_or_else(|_| Value::String(tab.tab_id.clone()))
}

fn auth_preflight_expression(selectors: &[String]) -> String {
    let selectors = serde_json::to_string(selectors).expect("selectors serialize");
    format!(
        "(()=>{{const selectors={selectors};let selectorsValid=true;for(const selector of selectors){{try{{if(document.querySelectorAll(selector).length!==1)selectorsValid=false;}}catch{{selectorsValid=false;}}}}return{{origin:location.origin,selectorsValid}};}})()"
    )
}

#[derive(Debug, Eq, PartialEq)]
struct AuthPreflight {
    origin: String,
    selectors_valid: bool,
}

fn parse_auth_preflight(response: &Value) -> Result<AuthPreflight, &'static str> {
    let cdp = response
        .get("result")
        .and_then(|result| result.get("result"))
        .or_else(|| response.get("result"))
        .unwrap_or(response);
    if cdp.get("exceptionDetails").is_some() {
        return Err("locator_invalid");
    }
    let value = cdp.get("value").unwrap_or(cdp);
    let origin = value
        .get("origin")
        .and_then(Value::as_str)
        .ok_or("page_changed")?;
    let selectors_valid = value
        .get("selectorsValid")
        .and_then(Value::as_bool)
        .ok_or("page_changed")?;
    Ok(AuthPreflight {
        origin: origin.to_owned(),
        selectors_valid,
    })
}

fn auth_status(status: &str) -> ExecutorOutcome {
    ExecutorOutcome::DefinitiveSuccess(json!({ "status": status }).to_string())
}

fn record_host_operation(
    shared: &Shared,
    operation: &super::super::DispatchOperation,
    state: String,
) {
    let tab_key = match &operation.scope {
        OperationScope::Tab(tab) => Some(BrowserTabKey {
            browser_instance_id: tab.browser_instance_id.0.clone(),
            extension_tab_id: tab.tab_id.clone(),
        }),
        OperationScope::BridgeGlobal(_) | OperationScope::DaemonGlobal => None,
    };
    shared
        .control
        .get()
        .expect("integration control initialized")
        .events
        .record(
            BrowserControlEventKind::OperationState { state },
            super::super::introspection::EventContext {
                principal_id: Some(operation.principal.id.clone()),
                group_id: operation.group_id.as_ref().map(|group| group.0.clone()),
                tab_key,
                operation_id: Some(operation.identity.operation_id.0.clone()),
            },
        );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::control_plane::operation::OperationIdentity;
    use crate::browser::control_plane::{ClientId, OperationId, Principal, UpstreamCorrelation};

    fn operation() -> super::super::super::DispatchOperation {
        super::super::super::DispatchOperation {
            identity: OperationIdentity {
                operation_id: OperationId::from("auth-operation"),
                daemon_generation: "generation".to_owned(),
                canonical_fingerprint: "fingerprint".to_owned(),
                upstream: UpstreamCorrelation {
                    ingress: "test".to_owned(),
                    request_id: None,
                },
            },
            client_id: ClientId::from("client"),
            principal: Principal::new("principal", 1000),
            group_id: None,
            scope: OperationScope::Tab(TabKey::new("browser", "44")),
            class: OperationClass::Mutation,
            payload: String::new(),
        }
    }

    fn valid_auth() -> Value {
        json!({
            "tabId": 44,
            "origin": "https://example.test",
            "reason": "Sign in",
            "expires_at": (Utc::now() + chrono::Duration::minutes(2)).to_rfc3339(),
            "fields": [{
                "id": "username",
                "label": "Email",
                "type": "email",
                "autocomplete": "username",
                "required": true,
                "selector": "input[name=email]"
            }],
            "submit": {"selector":"button[type=submit]","action":"click"}
        })
    }

    #[test]
    fn auth_schema_accepts_exact_non_secret_fields_and_locator_descriptors() {
        let mut request = valid_auth();
        request["fields"][0]["selector"] = json!({
            "kind":"locator",
            "args":["input[name=email]"],
            "parent":{"kind":"frameLocator","args":["iframe#auth"]}
        });
        let validated = validate_auth_request(&request, &operation()).unwrap();
        assert_eq!(validated.origin, "https://example.test");
        assert_eq!(validated.string_selectors, ["button[type=submit]"]);
    }

    #[test]
    fn auth_schema_rejects_credentials_bad_expiry_and_bad_selectors_without_echoing_values() {
        let mut secret = valid_auth();
        secret["fields"][0]["value"] = json!("never-log-this");
        let error = validate_auth_request(&secret, &operation()).unwrap_err();
        assert!(error.contains("unsupported property"));
        assert!(!error.contains("never-log-this"));

        let mut late = valid_auth();
        late["expires_at"] = json!((Utc::now() + chrono::Duration::minutes(6)).to_rfc3339());
        assert!(validate_auth_request(&late, &operation()).is_err());

        let mut selector = valid_auth();
        selector["fields"][0]["selector"] = json!({"kind":"evaluate","args":["secret"]});
        assert!(validate_auth_request(&selector, &operation()).is_err());
    }

    #[test]
    fn preflight_parser_distinguishes_origin_and_selector_state() {
        let parsed = parse_auth_preflight(&json!({
            "result":{"result":{"value":{
                "origin":"https://example.test",
                "selectorsValid":true
            }}}
        }))
        .unwrap();
        assert_eq!(
            parsed,
            AuthPreflight {
                origin: "https://example.test".to_owned(),
                selectors_valid: true
            }
        );
        assert_eq!(
            parse_auth_preflight(&json!({"result":{"exceptionDetails":{}}})),
            Err("locator_invalid")
        );
    }
}
