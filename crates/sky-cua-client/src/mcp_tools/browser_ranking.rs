//! Opt-in, advisory target selection over freshly observed browser semantics.

use std::{fmt::Display, time::Instant};

use anyhow::{Result, anyhow};
use serde_json::{Map, Value, json};
use sky_cua_platform::model::{
    AppShotCapture, BROWSER_SNAPSHOT_MAX_ELEMENT_LIMIT, BrowserRequest, BrowserResponse,
    BrowserTargetKind, ServiceResponse,
};

use crate::browser_rank_models::{ModelDecision, RankModels};

use super::{McpService, browser, invalid_request_tool_error, tool_error};

const INSTRUCTIONS: &str = "Choose the single observed enabled visible target that fulfills the user intent. Use its label, role, context, and bounds. Candidate IDs are opaque identifiers, not semantic clues. If the target is absent, multiple targets are equally valid, or the intent requires more than one target, choose abstain. Return only an observed candidate ID or abstain. Never invent IDs or coordinates.";
const JEV_THRESHOLD: f64 = 0.8;
const PAGE_TEXT_LIMIT: usize = 4000;

struct Arguments {
    target: Option<BrowserTargetKind>,
    tab_id: String,
    intent: String,
    exact_name: Option<String>,
    exact_role: Option<String>,
}

impl Arguments {
    fn parse(value: &Value) -> Result<Self> {
        let optional_literal = |name: &str| -> Result<Option<String>> {
            match value.get(name) {
                None | Some(Value::Null) => Ok(None),
                Some(Value::String(value)) if !value.trim().is_empty() => Ok(Some(value.clone())),
                _ => Err(anyhow!("browser_rank {name} must be a nonempty string")),
            }
        };
        let exact_name = optional_literal("exact_name")?;
        let exact_role = optional_literal("exact_role")?;
        if exact_role.is_some() && exact_name.is_none() {
            return Err(anyhow!("browser_rank exact_role requires exact_name"));
        }
        Ok(Self {
            target: browser::parse_browser_target(value)?,
            tab_id: browser::parse_browser_tab_id(value)?,
            intent: browser::parse_required_string(value, "intent", "browser_rank intent")?,
            exact_name,
            exact_role,
        })
    }
}

#[derive(Debug)]
struct Candidate {
    id: String,
    element_ref: String,
    label: String,
    role: String,
    context: String,
    bounds: Value,
}

impl Candidate {
    fn public(&self) -> Value {
        json!({
            "id": self.id,
            "label": self.label,
            "role": self.role,
            "context": self.context,
            "bounds": self.bounds,
            "disabled": false,
            "visible": true,
        })
    }
}

struct Observation {
    appshot_id: String,
    document_generation: u64,
    page_text: String,
    candidates: Vec<Candidate>,
}

/// Observed page text, sent once per request in the model state.
fn page_text(snapshot: &Value) -> String {
    snapshot["text"]
        .as_str()
        .unwrap_or_default()
        .chars()
        .take(PAGE_TEXT_LIMIT)
        .collect()
}

/// Per-candidate context holds only candidate-specific facts (the link
/// destination); shared page text belongs to the payload state.
fn candidates(snapshot: &Value) -> Vec<Candidate> {
    let Some(elements) = snapshot.get("elements").and_then(Value::as_array) else {
        return Vec::new();
    };
    elements
        .iter()
        .enumerate()
        .filter_map(|(index, element)| {
            let bounds = element.get("bounds")?;
            let coordinate = |key: &str| bounds.get(key)?.as_f64().filter(|n| n.is_finite());
            coordinate("x")?;
            coordinate("y")?;
            if coordinate("width")? <= 0.0 || coordinate("height")? <= 0.0 {
                return None;
            }
            if element
                .get("disabled")
                .is_some_and(|value| value != &Value::Bool(false))
                || element.get("visible").and_then(Value::as_bool) != Some(true)
            {
                return None;
            }
            let element_ref = element.get("ref")?.as_str()?.to_owned();
            if element_ref.is_empty() {
                return None;
            }
            let label = element["name"].as_str().unwrap_or_default().to_owned();
            let role = element["role"]
                .as_str()
                .filter(|role| !role.is_empty())
                .or_else(|| element["tag"].as_str())
                .unwrap_or_default()
                .to_owned();
            let context = element["href"]
                .as_str()
                .filter(|href| !href.is_empty())
                .map(|href| format!("href: {href}"))
                .unwrap_or_default();
            Some(Candidate {
                id: format!("c{index}"),
                element_ref,
                label,
                role,
                context,
                bounds: bounds.clone(),
            })
        })
        .collect()
}

/// The inner `Err` is a ready tool-error result carrying the service's
/// structured code and message, matching the other browser handlers.
fn observe(
    service: &impl McpService,
    arguments: &Arguments,
    identity: Option<&sky_cua_platform::BrowserSessionIdentity>,
) -> Result<std::result::Result<Observation, Value>> {
    let request = browser::browser_service_request(
        BrowserRequest::ObserveAppShot {
            target: arguments.target,
            tab_id: arguments.tab_id.clone(),
            text_limit: Some(PAGE_TEXT_LIMIT),
            element_offset: None,
            element_limit: Some(BROWSER_SNAPSHOT_MAX_ELEMENT_LIMIT),
            element_query: None,
            capture_timeout_ms: None,
            include_image_data: false,
        },
        identity,
    );
    match service.call(&request)? {
        ServiceResponse::Browser {
            response: BrowserResponse::AppShot { response },
        } => match response.appshot.capture {
            AppShotCapture::Browser {
                tab_id,
                document_generation,
                semantic_snapshot,
                ..
            } if tab_id == arguments.tab_id => Ok(Ok(Observation {
                appshot_id: response.appshot.appshot_id,
                document_generation,
                page_text: page_text(&semantic_snapshot),
                candidates: candidates(&semantic_snapshot),
            })),
            _ => Err(anyhow!("browser_rank observation target mismatch")),
        },
        ServiceResponse::Error { code, message, .. } => Ok(Err(tool_error(code, message)?)),
        _ => Err(anyhow!("unexpected response for browser_rank observation")),
    }
}

fn exact_candidate(
    candidates: &[Candidate],
    name: Option<&str>,
    role: Option<&str>,
) -> Option<usize> {
    let name = name?;
    let mut matching = candidates.iter().enumerate().filter(|(_, candidate)| {
        candidate.label == name && role.is_none_or(|role| candidate.role == role)
    });
    let (index, _) = matching.next()?;
    matching.next().is_none().then_some(index)
}

fn payload(intent: &str, page_text: &str, candidates: &[Candidate]) -> Value {
    let public: Vec<Value> = candidates.iter().map(Candidate::public).collect();
    let mut criteria: Map<String, Value> = candidates
        .iter()
        .zip(&public)
        .map(|(candidate, public)| (candidate.id.clone(), public.clone()))
        .collect();
    criteria.insert(
        "abstain".into(),
        json!("No unique enabled visible target fulfills the intent."),
    );
    json!({
        "state": {"intent": intent, "page_text": page_text, "candidates": public},
        "questions": {"target": {"type": "choice", "instructions": INSTRUCTIONS, "criteria": criteria}},
    })
}

#[derive(Debug)]
struct Ranking {
    choice: Option<usize>,
    source: &'static str,
    probability: Option<f64>,
    fallback_reason: Option<String>,
    jev_ms: Option<u64>,
    terra_ms: Option<u64>,
}

impl Ranking {
    fn unresolved(source: &'static str, reason: impl Into<String>) -> Self {
        Self {
            choice: None,
            source,
            probability: None,
            fallback_reason: Some(reason.into()),
            jev_ms: None,
            terra_ms: None,
        }
    }
}

fn decision_index(candidates: &[Candidate], decision: &ModelDecision) -> Option<usize> {
    candidates
        .iter()
        .position(|candidate| candidate.id == decision.choice)
}

fn rank_candidates<E: Display>(
    candidates: &[Candidate],
    payload: &Value,
    jev: impl FnOnce(&Value) -> std::result::Result<ModelDecision, E>,
    terra: impl FnOnce(&Value) -> std::result::Result<ModelDecision, E>,
) -> Ranking {
    let mut ranking = Ranking::unresolved("jev", "jev_error");
    match jev(payload) {
        Ok(decision) => {
            ranking.jev_ms = Some(decision.elapsed_ms);
            let index = decision_index(candidates, &decision);
            if let Some(index) = index
                && decision
                    .probability
                    .is_some_and(|p| p.is_finite() && (JEV_THRESHOLD..=1.0).contains(&p))
            {
                ranking.choice = Some(index);
                ranking.probability = decision.probability;
                ranking.fallback_reason = None;
                return ranking;
            }
            ranking.fallback_reason = Some(
                if decision.choice == "abstain" {
                    "jev_abstained"
                } else if index.is_none() {
                    "jev_unknown_choice"
                } else {
                    "jev_low_or_invalid_probability"
                }
                .into(),
            );
        }
        Err(error) => ranking.fallback_reason = Some(format!("jev_error:{error}")),
    }
    ranking.source = "terra";
    match terra(payload) {
        Ok(decision) => {
            ranking.terra_ms = Some(decision.elapsed_ms);
            ranking.choice = decision_index(candidates, &decision);
            if ranking.choice.is_none() {
                let reason = if decision.choice == "abstain" {
                    "terra_abstained"
                } else {
                    "terra_unknown_choice"
                };
                ranking.fallback_reason = Some(format!(
                    "{};{reason}",
                    ranking.fallback_reason.as_deref().unwrap_or_default()
                ));
            }
        }
        Err(error) => {
            ranking.fallback_reason = Some(format!(
                "{};terra_error:{error}",
                ranking.fallback_reason.as_deref().unwrap_or_default()
            ))
        }
    }
    ranking
}

fn current_candidate<'a>(chosen: &Candidate, current: &'a Observation) -> Option<&'a Candidate> {
    let mut matching = current.candidates.iter().filter(|candidate| {
        candidate.element_ref == chosen.element_ref
            && candidate.label == chosen.label
            && candidate.role == chosen.role
    });
    let candidate = matching.next()?;
    matching.next().is_none().then_some(candidate)
}

pub(super) fn handle(
    service: &impl McpService,
    arguments: Value,
    browser_identity: Option<&sky_cua_platform::BrowserSessionIdentity>,
) -> Result<Value> {
    handle_with(
        service,
        arguments,
        browser_identity,
        |candidates, payload| {
            let models = RankModels::from_env();
            rank_candidates(
                candidates,
                payload,
                |payload| models.jev(payload),
                |payload| models.terra(payload),
            )
        },
    )
}

fn handle_with(
    service: &impl McpService,
    arguments: Value,
    browser_identity: Option<&sky_cua_platform::BrowserSessionIdentity>,
    rank: impl FnOnce(&[Candidate], &Value) -> Ranking,
) -> Result<Value> {
    let started = Instant::now();
    let arguments = match Arguments::parse(&arguments) {
        Ok(arguments) => arguments,
        Err(error) => return invalid_request_tool_error(error.to_string()),
    };
    let observed = match observe(service, &arguments, browser_identity)? {
        Ok(observed) => observed,
        Err(error) => return Ok(error),
    };
    let mut ranking = if let Some(index) = exact_candidate(
        &observed.candidates,
        arguments.exact_name.as_deref(),
        arguments.exact_role.as_deref(),
    ) {
        Ranking {
            choice: Some(index),
            source: "deterministic",
            probability: None,
            fallback_reason: None,
            jev_ms: None,
            terra_ms: None,
        }
    } else if observed.candidates.is_empty() {
        Ranking::unresolved("deterministic", "no_actionable_candidates")
    } else {
        rank(
            &observed.candidates,
            &payload(&arguments.intent, &observed.page_text, &observed.candidates),
        )
    };
    let mut appshot_id = observed.appshot_id.clone();
    let mut element_ref = None;
    if let Some(index) = ranking.choice {
        let current = match observe(service, &arguments, browser_identity)? {
            Ok(current) => current,
            Err(error) => return Ok(error),
        };
        appshot_id = current.appshot_id.clone();
        if observed.document_generation == current.document_generation {
            element_ref = current_candidate(&observed.candidates[index], &current)
                .map(|candidate| candidate.element_ref.clone());
        }
        if element_ref.is_none() {
            ranking.choice = None;
            ranking.fallback_reason = Some("target_changed".into());
        }
    }
    let status = if element_ref.is_some() {
        "selected"
    } else {
        "unresolved"
    };
    let mut structured = json!({
        "status": status,
        "source": ranking.source,
        "tab_id": arguments.tab_id,
        "appshot_id": appshot_id,
        "fallback_reason": ranking.fallback_reason,
        "timing_ms": {"total": started.elapsed().as_millis(), "jev": ranking.jev_ms, "terra": ranking.terra_ms},
    });
    if let Some(element_ref) = element_ref {
        structured["ref"] = json!(element_ref);
    }
    if let Some(probability) = ranking.probability {
        structured["probability"] = json!(probability);
    }
    let text = if status == "selected" {
        format!(
            "Selected one observed browser target via {}. No action was performed; use the returned ref and appshot_id for the action.",
            ranking.source
        )
    } else {
        format!(
            "Browser target unresolved: {}. No action was performed.",
            ranking.fallback_reason.as_deref().unwrap_or("no_selection")
        )
    };
    Ok(
        json!({"content": [{"type": "text", "text": text}], "structuredContent": structured, "isError": false}),
    )
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::VecDeque;

    use super::*;
    use sky_cua_platform::model::{BrowserAppShotResponse, ServiceRequest};

    fn node(name: &str, element_ref: &str) -> Value {
        json!({"name": name, "ref": element_ref, "tag": "button", "role": null,
            "disabled": false, "visible": true,
            "bounds": {"x": 10, "y": 20, "width": 100, "height": 30}})
    }

    fn fixture(nodes: Vec<Value>) -> Vec<Candidate> {
        candidates(&json!({"text": "Observed page text", "elements": nodes}))
    }

    fn decision(choice: &str, probability: Option<f64>) -> ModelDecision {
        ModelDecision {
            choice: choice.into(),
            probability,
            elapsed_ms: 4,
        }
    }

    #[test]
    fn projection_uses_actual_semantics_and_excludes_unusable_controls() {
        let mut disabled = node("Disabled", "disabled-ref");
        disabled["disabled"] = json!(true);
        let mut hidden = node("Hidden", "hidden-ref");
        hidden["bounds"]["width"] = json!(0);
        let mut explicitly_hidden = node("Hidden flag", "hidden-flag-ref");
        explicitly_hidden["visible"] = json!(false);
        let mut malformed = node("Malformed", "malformed-ref");
        malformed["bounds"]["x"] = json!("10");
        let mut missing_visibility = node("Missing visibility", "missing-visibility-ref");
        missing_visibility
            .as_object_mut()
            .unwrap()
            .remove("visible");
        let mut good = node("Save", "private-ref");
        good["href"] = json!("https://example.test/save");
        let found = fixture(vec![
            disabled,
            hidden,
            explicitly_hidden,
            malformed,
            missing_visibility,
            good,
        ]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "c5");
        assert_eq!(found[0].role, "button");
        assert_eq!(found[0].element_ref, "private-ref");
        let payload = payload("Save changes", "Observed page text", &found);
        assert_eq!(payload["state"]["candidates"][0]["role"], "button");
        assert_eq!(
            payload["state"]["candidates"][0]["context"],
            "href: https://example.test/save"
        );
        assert_eq!(payload["state"]["page_text"], "Observed page text");
        assert!(!payload.to_string().contains("private-ref"));
        assert!(payload["questions"]["target"]["criteria"]["abstain"].is_string());
    }

    #[test]
    fn page_text_is_sent_once_regardless_of_candidate_count() {
        let marker = "unique-page-text-marker";
        let snapshot = json!({
            "text": format!("{marker}{}", "x".repeat(2 * PAGE_TEXT_LIMIT)),
            "elements": (0..50).map(|i| node(&format!("Link {i}"), &format!("ref-{i}"))).collect::<Vec<_>>(),
        });
        let found = candidates(&snapshot);
        assert_eq!(found.len(), 50);
        assert!(found.iter().all(|candidate| candidate.context.is_empty()));
        let text = page_text(&snapshot);
        assert_eq!(text.chars().count(), PAGE_TEXT_LIMIT);
        let serialized = payload("Open a link", &text, &found).to_string();
        assert_eq!(serialized.matches(marker).count(), 1);
        assert!(serialized.len() < PAGE_TEXT_LIMIT + 50 * 600);
    }

    #[test]
    fn exact_name_requires_literal_unique_match_and_optional_role() {
        let mut link = node("Save", "second");
        link["role"] = json!("link");
        let found = fixture(vec![node("Save", "first"), link]);
        assert_eq!(exact_candidate(&found, None, None), None);
        assert_eq!(exact_candidate(&found, Some("Save"), None), None);
        assert_eq!(
            exact_candidate(&found, Some("Save"), Some("button")),
            Some(0)
        );
        assert_eq!(exact_candidate(&found, Some("save"), None), None);
        assert_eq!(exact_candidate(&found, Some(" Save "), None), None);
        assert!(
            Arguments::parse(&json!({"tab_id":"1","intent":"Save","exact_role":"button"})).is_err()
        );
    }

    #[test]
    fn confident_jev_choice_avoids_terra() {
        let found = fixture(vec![node("Save", "first")]);
        let result = rank_candidates(
            &found,
            &payload("Save", "Observed page text", &found),
            |_| Ok::<_, &str>(decision("c0", Some(0.8))),
            |_| panic!("Terra must not run after confident Jev choice"),
        );
        assert_eq!(result.choice, Some(0));
        assert_eq!(result.source, "jev");
        assert_eq!(result.probability, Some(0.8));
        assert!(result.fallback_reason.is_none());
    }

    #[test]
    fn invalid_jev_decisions_all_fall_back_to_terra() {
        let found = fixture(vec![node("Save", "first")]);
        for rejected in [
            decision("c0", Some(0.79)),
            decision("c0", None),
            decision("c0", Some(f64::NAN)),
            decision("c0", Some(f64::INFINITY)),
            decision("c0", Some(1.1)),
            decision("unknown", Some(0.99)),
            decision("abstain", Some(0.99)),
        ] {
            let result = rank_candidates(
                &found,
                &payload("Save", "Observed page text", &found),
                |_| Ok::<_, &str>(rejected),
                |_| Ok(decision("c0", None)),
            );
            assert_eq!(result.choice, Some(0));
            assert_eq!(result.source, "terra");
            assert!(result.fallback_reason.is_some());
            assert!(result.probability.is_none());
        }
    }

    #[test]
    fn provider_failures_and_terra_abstention_never_return_a_choice() {
        let found = fixture(vec![node("Save", "first")]);
        for terra in [
            Ok(decision("abstain", None)),
            Ok(decision("unknown", None)),
            Err("provider_timeout"),
        ] {
            let result = rank_candidates(
                &found,
                &payload("Save", "Observed page text", &found),
                |_| Err("provider_timeout"),
                |_| terra,
            );
            assert_eq!(result.choice, None);
            assert_eq!(result.source, "terra");
            assert!(
                result
                    .fallback_reason
                    .unwrap()
                    .starts_with("jev_error:provider_timeout;")
            );
        }
    }

    #[test]
    fn revalidation_requires_the_same_ref_name_role_and_usable_state() {
        let found = fixture(vec![node("Save", "opaque-first")]);
        let chosen = &found[0];
        let current = |nodes| Observation {
            appshot_id: "current".into(),
            document_generation: 1,
            page_text: String::new(),
            candidates: fixture(nodes),
        };
        assert!(current_candidate(chosen, &current(vec![node("Save", "opaque-first")])).is_some());
        assert!(
            current_candidate(chosen, &current(vec![node("Save", "opaque-rebound")])).is_none()
        );
        assert!(
            current_candidate(chosen, &current(vec![node("Delete", "opaque-first")])).is_none()
        );
        let mut changed = node("Save", "opaque-first");
        changed["role"] = json!("link");
        assert!(current_candidate(chosen, &current(vec![changed])).is_none());
        let mut changed = node("Save", "opaque-first");
        changed["disabled"] = json!(true);
        assert!(current_candidate(chosen, &current(vec![changed])).is_none());
    }

    struct MockService {
        responses: RefCell<VecDeque<ServiceResponse>>,
        requests: RefCell<Vec<ServiceRequest>>,
    }

    impl McpService for MockService {
        fn call(&self, request: &ServiceRequest) -> Result<ServiceResponse> {
            self.requests.borrow_mut().push(request.clone());
            self.responses
                .borrow_mut()
                .pop_front()
                .ok_or_else(|| anyhow!("unexpected extra observation"))
        }
    }

    fn response(appshot_id: &str, nodes: Vec<Value>) -> ServiceResponse {
        response_with_generation(appshot_id, 1, nodes)
    }

    fn response_with_generation(
        appshot_id: &str,
        document_generation: u64,
        nodes: Vec<Value>,
    ) -> ServiceResponse {
        let appshot = serde_json::from_value(json!({
            "appshot_id": appshot_id, "trigger":"observe", "captured_at":"2026-11-05T00:00:00Z",
            "consistency":"stable", "surface":"browser", "tab_id":"tab-1",
            "url":"https://example.test/", "viewport":{"width":800,"height":600},
            "document_generation":document_generation, "semantic_snapshot":{"text":"Save changes", "elements":nodes},
            "image":{"content_id":"image-1","mime_type":"image/png","size_bytes":0,
                "sha256":"unused","source":"screenshot","persistence":"temporary"},
            "action_snapshot":{"snapshot_id":"snapshot-1"},
            "coverage":{"pixels_complete":true,"semantics_complete":true,
                "secure_regions_redacted":false,"projection_truncated":false},
            "capability_profile_id":"profile-1"
        }))
        .unwrap();
        ServiceResponse::Browser {
            response: BrowserResponse::AppShot {
                response: BrowserAppShotResponse {
                    appshot,
                    image_data_base64: String::new(),
                    image_mime_type: String::new(),
                },
            },
        }
    }

    fn mock(second: Value) -> MockService {
        MockService {
            responses: RefCell::new(VecDeque::from([
                response("shot-before", vec![node("Save", "opaque-ref")]),
                response("shot-after", vec![second]),
            ])),
            requests: RefCell::new(Vec::new()),
        }
    }

    #[test]
    fn deterministic_selection_reobserves_and_returns_current_appshot_without_models() {
        let service = mock(node("Save", "opaque-ref"));
        let result = handle_with(
            &service,
            json!({"tab_id":"tab-1", "intent":"Save changes", "exact_name":"Save"}),
            None,
            |_, _| panic!("Explicit unique exact match must avoid model setup"),
        )
        .unwrap();
        assert_eq!(result["structuredContent"]["status"], "selected");
        assert_eq!(result["structuredContent"]["source"], "deterministic");
        assert_eq!(result["structuredContent"]["appshot_id"], "shot-after");
        assert_eq!(result["structuredContent"]["ref"], "opaque-ref");
        assert_eq!(service.requests.borrow().len(), 2);
        for request in service.requests.borrow().iter() {
            assert!(matches!(
                request,
                ServiceRequest::Browser {
                    request: BrowserRequest::ObserveAppShot {
                        include_image_data: false,
                        ..
                    },
                    ..
                }
            ));
        }
    }

    #[test]
    fn no_actionable_candidates_skips_model_setup_and_reobservation() {
        let mut disabled = node("Save", "disabled-ref");
        disabled["disabled"] = json!(true);
        let service = MockService {
            responses: RefCell::new(VecDeque::from([response("shot-empty", vec![disabled])])),
            requests: RefCell::new(Vec::new()),
        };
        let result = handle_with(
            &service,
            json!({"tab_id":"tab-1", "intent":"Save changes"}),
            None,
            |_, _| panic!("No actionable candidates must skip model setup"),
        )
        .unwrap();
        assert_eq!(result["structuredContent"]["status"], "unresolved");
        assert_eq!(result["structuredContent"]["source"], "deterministic");
        assert_eq!(
            result["structuredContent"]["fallback_reason"],
            "no_actionable_candidates"
        );
        assert_eq!(result["structuredContent"]["appshot_id"], "shot-empty");
        assert!(result["structuredContent"].get("ref").is_none());
        assert_eq!(service.requests.borrow().len(), 1);
    }

    #[test]
    fn observation_service_errors_keep_their_structured_code_and_message() {
        let service_error = || ServiceResponse::Error {
            ok: false,
            code: "BrowserTabNotClaimed".into(),
            message: "tab-1 is not claimed".into(),
            session_id: None,
            turn_id: None,
            retry: None,
        };
        for responses in [
            vec![service_error()],
            vec![
                response("shot-before", vec![node("Save", "opaque-ref")]),
                service_error(),
            ],
        ] {
            let service = MockService {
                responses: RefCell::new(VecDeque::from(responses)),
                requests: RefCell::new(Vec::new()),
            };
            let result = handle_with(
                &service,
                json!({"tab_id":"tab-1", "intent":"Save changes", "exact_name":"Save"}),
                None,
                |_, _| panic!("Explicit unique exact match must avoid model setup"),
            )
            .unwrap();
            assert_eq!(result["isError"], true);
            assert_eq!(result["structuredContent"]["code"], "BrowserTabNotClaimed");
            assert_eq!(
                result["structuredContent"]["message"],
                "tab-1 is not claimed"
            );
        }
    }

    #[test]
    fn model_choice_that_changed_during_inference_has_no_actionable_ref() {
        let service = mock(node("Save", "replacement-ref"));
        let result = handle_with(
            &service,
            json!({"tab_id":"tab-1", "intent":"Save"}),
            None,
            |found, payload| {
                rank_candidates(
                    found,
                    payload,
                    |_| Ok::<_, &str>(decision("c0", Some(0.9))),
                    |_| panic!("unexpected fallback"),
                )
            },
        )
        .unwrap();
        assert_eq!(result["structuredContent"]["status"], "unresolved");
        assert_eq!(
            result["structuredContent"]["fallback_reason"],
            "target_changed"
        );
        assert_eq!(result["structuredContent"]["appshot_id"], "shot-after");
        assert!(result["structuredContent"].get("ref").is_none());
    }

    #[test]
    fn identical_target_in_a_new_document_has_no_actionable_ref() {
        let service = MockService {
            responses: RefCell::new(VecDeque::from([
                response_with_generation("shot-before", 1, vec![node("Save", "opaque-ref")]),
                response_with_generation("shot-after", 2, vec![node("Save", "opaque-ref")]),
            ])),
            requests: RefCell::new(Vec::new()),
        };
        let result = handle_with(
            &service,
            json!({"tab_id":"tab-1", "intent":"Save changes", "exact_name":"Save"}),
            None,
            |_, _| panic!("Explicit unique exact match must avoid model setup"),
        )
        .unwrap();
        assert_eq!(result["structuredContent"]["status"], "unresolved");
        assert_eq!(
            result["structuredContent"]["fallback_reason"],
            "target_changed"
        );
        assert_eq!(result["structuredContent"]["appshot_id"], "shot-after");
        assert!(result["structuredContent"].get("ref").is_none());
    }
}
