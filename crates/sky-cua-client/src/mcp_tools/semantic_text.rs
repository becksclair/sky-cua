use serde_json::Value;
use sky_cua_platform::model::{AppShotCapture, AppShotEnvelope};

/// Pi and other MCP hosts send `content` to the model but may keep
/// `structuredContent` only as UI/session metadata. Keep the model-facing
/// semantic fallback below Pi's 50 KiB output guard while retaining enough
/// room for the ordinary result summary and truncation notice.
const MODEL_SEMANTIC_TEXT_MAX_BYTES: usize = 32 * 1024;

pub(super) fn append_appshot_semantics(text: &mut String, appshot: &AppShotEnvelope) {
    let (label, projection) = match &appshot.capture {
        AppShotCapture::Desktop {
            semantic_projection,
            ..
        } => ("desktop accessibility projection", semantic_projection),
        AppShotCapture::Browser {
            semantic_snapshot, ..
        } => ("browser semantic snapshot", semantic_snapshot),
        AppShotCapture::Phone {
            semantic_projection,
            ..
        } => ("phone accessibility projection", semantic_projection),
    };
    append_semantic_value(text, label, projection);
}

pub(super) fn append_semantic_value(text: &mut String, label: &str, projection: &Value) {
    if semantic_value_is_empty(projection) {
        return;
    }
    let Ok((serialized, truncated)) = bounded_semantic_json(projection) else {
        return;
    };
    text.push_str("\nModel-facing ");
    text.push_str(label);
    text.push_str(" (bounded JSON):\n");
    text.push_str(&serialized);
    if truncated {
        text.push_str("\n[Semantic projection truncated at 32768 bytes; structuredContent and the AppShot artifact retain the complete bounded projection.]");
    }
}

fn bounded_semantic_json(projection: &Value) -> serde_json::Result<(String, bool)> {
    let serialized = serde_json::to_string(projection)?;
    if serialized.len() <= MODEL_SEMANTIC_TEXT_MAX_BYTES {
        return Ok((serialized, false));
    }

    let mut bounded = projection.clone();
    while shrink_largest_array(&mut bounded, true) || shrink_largest_array(&mut bounded, false) {
        let candidate = serde_json::to_string(&bounded)?;
        if candidate.len() <= MODEL_SEMANTIC_TEXT_MAX_BYTES {
            return Ok((candidate, true));
        }
    }

    Ok((bounded_prefix_envelope(&serialized)?, true))
}

fn shrink_largest_array(value: &mut Value, semantic_arrays_only: bool) -> bool {
    let largest = largest_array_len(value, semantic_arrays_only);
    if largest <= 1 {
        return false;
    }
    truncate_first_array(value, semantic_arrays_only, largest, largest.div_ceil(2))
}

fn largest_array_len(value: &Value, semantic_arrays_only: bool) -> usize {
    match value {
        Value::Array(values) => values
            .iter()
            .map(|value| largest_array_len(value, semantic_arrays_only))
            .max()
            .unwrap_or(0)
            .max(if semantic_arrays_only {
                0
            } else {
                values.len()
            }),
        Value::Object(values) => values
            .iter()
            .map(|(key, value)| {
                let own = (matches!(key.as_str(), "elements" | "nodes"))
                    .then(|| value.as_array().map(Vec::len))
                    .flatten()
                    .unwrap_or(0);
                own.max(largest_array_len(value, semantic_arrays_only))
            })
            .max()
            .unwrap_or(0),
        _ => 0,
    }
}

fn truncate_first_array(
    value: &mut Value,
    semantic_arrays_only: bool,
    target_len: usize,
    retained_len: usize,
) -> bool {
    match value {
        Value::Array(values) => {
            if !semantic_arrays_only && values.len() == target_len {
                values.truncate(retained_len);
                return true;
            }
            values.iter_mut().any(|value| {
                truncate_first_array(value, semantic_arrays_only, target_len, retained_len)
            })
        }
        Value::Object(values) => {
            for (key, value) in values {
                if matches!(key.as_str(), "elements" | "nodes")
                    && value
                        .as_array()
                        .is_some_and(|values| values.len() == target_len)
                {
                    value
                        .as_array_mut()
                        .expect("array shape checked above")
                        .truncate(retained_len);
                    return true;
                }
                if truncate_first_array(value, semantic_arrays_only, target_len, retained_len) {
                    return true;
                }
            }
            false
        }
        _ => false,
    }
}

fn bounded_prefix_envelope(serialized: &str) -> serde_json::Result<String> {
    let mut low = 0usize;
    let mut high = serialized.len().min(MODEL_SEMANTIC_TEXT_MAX_BYTES);
    while !serialized.is_char_boundary(high) {
        high -= 1;
    }
    let mut best = serde_json::to_string(&serde_json::json!({
        "projection_prefix": "",
        "truncated": true,
    }))?;
    while low <= high {
        let mut middle = low + (high - low) / 2;
        while !serialized.is_char_boundary(middle) {
            middle -= 1;
        }
        let candidate = serde_json::to_string(&serde_json::json!({
            "projection_prefix": &serialized[..middle],
            "truncated": true,
        }))?;
        if candidate.len() <= MODEL_SEMANTIC_TEXT_MAX_BYTES {
            best = candidate;
            if middle == serialized.len() {
                break;
            }
            low = middle
                + serialized[middle..]
                    .chars()
                    .next()
                    .expect("middle precedes the end of serialized input")
                    .len_utf8();
        } else if middle == 0 {
            break;
        } else {
            high = middle - 1;
            while !serialized.is_char_boundary(high) {
                high -= 1;
            }
        }
    }
    Ok(best)
}

fn semantic_value_is_empty(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Array(values) => values.is_empty(),
        Value::Object(values) => values.is_empty(),
        Value::String(value) => value.is_empty(),
        Value::Bool(_) | Value::Number(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semantic_text_keeps_small_projection_model_facing() {
        let mut text = String::from("summary");
        append_semantic_value(
            &mut text,
            "phone accessibility projection",
            &serde_json::json!({"nodes": [{"text": "Settings", "clickable": true}]}),
        );
        assert!(text.contains("Model-facing phone accessibility projection"));
        assert!(text.contains("\"text\":\"Settings\""));
        assert!(text.contains("\"clickable\":true"));
    }

    #[test]
    fn semantic_text_truncates_as_valid_json() {
        let mut text = String::from("summary");
        append_semantic_value(
            &mut text,
            "projection",
            &serde_json::json!({
                "nodes": (0..500).map(|index| serde_json::json!({
                    "text": format!("Settings é {index} {}", "x".repeat(128)),
                    "clickable": true,
                })).collect::<Vec<_>>()
            }),
        );
        assert!(text.contains("Semantic projection truncated at 32768 bytes"));
        assert!(text.is_char_boundary(text.len()));
        assert!(text.len() < MODEL_SEMANTIC_TEXT_MAX_BYTES + 512);
        let json = text
            .split_once("Model-facing projection (bounded JSON):\n")
            .unwrap()
            .1
            .split_once("\n[Semantic projection truncated")
            .unwrap()
            .0;
        let parsed: Value = serde_json::from_str(json).unwrap();
        let nodes = parsed["nodes"].as_array().unwrap();
        assert!(!nodes.is_empty());
        assert!(nodes.len() < 500);
    }

    #[test]
    fn scalar_fallback_is_valid_json_without_input_sized_boundary_storage() {
        let projection = Value::String("é\\\"".repeat(100_000));
        let (serialized, truncated) = bounded_semantic_json(&projection).unwrap();

        assert!(truncated);
        assert!(serialized.len() <= MODEL_SEMANTIC_TEXT_MAX_BYTES);
        let parsed: Value = serde_json::from_str(&serialized).unwrap();
        assert_eq!(parsed["truncated"], true);
        assert!(!parsed["projection_prefix"].as_str().unwrap().is_empty());
    }

    #[test]
    fn semantic_text_omits_empty_projection() {
        let mut text = String::from("summary");
        append_semantic_value(&mut text, "projection", &serde_json::json!({}));
        assert_eq!(text, "summary");
    }
}
