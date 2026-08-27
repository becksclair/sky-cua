pub(super) fn parse_key_sequence(arguments: &serde_json::Value) -> Option<Vec<String>> {
    if let Some(keys) = arguments.get("keys").and_then(serde_json::Value::as_array)
        && let Some(parsed) = parse_segments(keys.iter().filter_map(serde_json::Value::as_str))
    {
        return Some(parsed);
    }

    arguments
        .get("key")
        .and_then(serde_json::Value::as_str)
        .and_then(|key| parse_segments(key.split('+')))
}

fn parse_segments<'a>(segments: impl Iterator<Item = &'a str>) -> Option<Vec<String>> {
    let parsed = segments
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();

    (!parsed.is_empty()).then(|| normalize_shortcut_key_sequence(parsed))
}

fn normalize_shortcut_key_sequence(mut keys: Vec<String>) -> Vec<String> {
    if keys.len() <= 1 || !keys.iter().any(|key| is_shortcut_modifier(key)) {
        return keys;
    }

    for key in &mut keys {
        if is_single_ascii_letter(key) {
            key.make_ascii_lowercase();
        }
    }
    keys
}

fn is_single_ascii_letter(key: &str) -> bool {
    key.len() == 1 && key.as_bytes()[0].is_ascii_alphabetic()
}

fn is_shortcut_modifier(key: &str) -> bool {
    normalized_key_matches(
        key,
        &[
            "ctrl",
            "control",
            "ctrll",
            "ctrlr",
            "controlr",
            "rightctrl",
            "rightcontrol",
            "alt",
            "altl",
            "altr",
            "rightalt",
            "altgr",
            "level3",
            "isolevel3shift",
            "modeswitch",
            "shift",
            "shiftl",
            "shiftr",
            "rightshift",
            "meta",
            "super",
            "superl",
            "metal",
        ],
    )
}

fn normalized_key_matches(key: &str, candidates: &[&str]) -> bool {
    candidates
        .iter()
        .any(|candidate| normalized_key_eq(key, candidate))
}

fn normalized_key_eq(key: &str, candidate: &str) -> bool {
    key.chars()
        .filter(|&character| character != '_')
        .flat_map(char::to_lowercase)
        .eq(candidate.chars())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::parse_key_sequence;

    #[test]
    fn parses_key_chord_string() {
        assert_eq!(
            parse_key_sequence(&json!({"key": "Ctrl+L"})),
            Some(vec!["Ctrl".to_string(), "l".to_string()])
        );
        assert_eq!(
            parse_key_sequence(&json!({"keys": ["Control", "A"]})),
            Some(vec!["Control".to_string(), "a".to_string()])
        );
        assert_eq!(
            parse_key_sequence(&json!({"key": "A"})),
            Some(vec!["A".to_string()])
        );
        assert_eq!(
            parse_key_sequence(&json!({"keys": [], "key": "Ctrl+L"})),
            Some(vec!["Ctrl".to_string(), "l".to_string()])
        );
    }
}
