//! Shared schema helpers used across the desktop/browser/phone/status tool
//! families: exact-branch discriminated-union builders, optional-field
//! wrappers, and the runtime schema-acceptance validator.

use serde_json::{Map, Value, json};

pub(super) const WINDOW_SELECTOR_KEYS: [&str; 9] = [
    "window_id",
    "pid",
    "tty",
    "terminal_pid",
    "terminal_command",
    "terminal_cwd",
    "app_id",
    "wm_class",
    "title",
];

pub(super) const DISPLAY_SELECTOR_KEYS: [&str; 3] = ["display_id", "display_name", "display_index"];

pub(super) fn normalize_required_property_schemas(
    input_schema: &mut serde_json::Map<String, Value>,
) {
    let mut normalized = Value::Object(input_schema.clone());
    normalize_required_property_schemas_in_value(&mut normalized);
    *input_schema = normalized
        .as_object()
        .expect("normalized input schema must remain an object")
        .clone();
}

pub(super) fn normalize_required_property_schemas_in_value(schema: &mut Value) {
    match schema {
        Value::Object(object) => {
            let missing_required = object
                .get("required")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .filter(|field| {
                    !object
                        .get("properties")
                        .and_then(Value::as_object)
                        .is_some_and(|properties| properties.contains_key(*field))
                })
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>();

            if !missing_required.is_empty() {
                let properties = object
                    .entry("properties".to_string())
                    .or_insert_with(|| Value::Object(Map::new()))
                    .as_object_mut()
                    .expect("schema properties must be an object");
                for field in missing_required {
                    properties.entry(field).or_insert_with(|| json!({}));
                }
            }

            for value in object.values_mut() {
                normalize_required_property_schemas_in_value(value);
            }
        }
        Value::Array(items) => {
            for value in items {
                normalize_required_property_schemas_in_value(value);
            }
        }
        _ => {}
    }
}

pub(super) fn normalize_root_composition_schema(input_schema: &mut serde_json::Map<String, Value>) {
    if input_schema.get("type") != Some(&Value::String("object".into())) {
        return;
    }
    for key in ["anyOf", "oneOf"] {
        let Some(mut branches) = input_schema.remove(key).and_then(|value| match value {
            Value::Array(branches) => Some(branches),
            _ => None,
        }) else {
            continue;
        };
        for branch in &mut branches {
            if let Some(branch) = branch.as_object_mut() {
                branch
                    .entry("type".to_string())
                    .or_insert_with(|| Value::String("object".to_string()));
            }
        }
        let constraint = json!({key: branches});
        match input_schema.get_mut("allOf").and_then(Value::as_array_mut) {
            Some(all_of) => all_of.push(constraint),
            None => {
                input_schema.insert("allOf".to_string(), json!([constraint]));
            }
        }
    }
}

pub(super) fn merge_properties(left: Value, right: Value) -> Value {
    let mut merged = left
        .as_object()
        .unwrap_or_else(|| panic!("merge_properties left must be object: {left:?}"))
        .clone();
    let right = right
        .as_object()
        .unwrap_or_else(|| panic!("merge_properties right must be object: {right:?}"));
    merged.extend(right.clone());
    Value::Object(merged)
}

pub(super) fn exact_branch_constraints(
    properties: &Value,
    discriminator: &str,
    branches: &[(&str, &[&str], &[&str])],
) -> Value {
    json!({
        "allOf": [exact_branch_one_of(
            properties,
            &branches
                .iter()
                .map(|(value, required, allowed)| {
                    (vec![(discriminator, *value)], *required, *allowed, None)
                })
                .collect::<Vec<_>>(),
        )]
    })
}

pub(super) fn exact_branch_one_of(
    properties: &Value,
    branches: &[(Vec<(&str, &str)>, &[&str], &[&str], Option<Value>)],
) -> Value {
    json!({
        "oneOf": branches
            .iter()
            .map(|(discriminators, required, allowed, extra)| {
                let mut schema = exact_branch_schema(properties, discriminators, required, allowed);
                if let Some(extra) = extra {
                    merge_schema_constraints(&mut schema, extra);
                }
                schema
            })
            .collect::<Vec<_>>()
    })
}

pub(super) fn exact_branch_schema_with_constraints(
    properties: &Value,
    discriminators: &[(&str, &str)],
    required: &[&str],
    allowed: &[&str],
    extra_constraints: Value,
) -> Value {
    let mut schema = exact_branch_schema(properties, discriminators, required, allowed);
    merge_schema_constraints(&mut schema, &extra_constraints);
    schema
}

pub(super) fn merge_schema_constraints(schema: &mut Value, extra_constraints: &Value) {
    let schema = schema
        .as_object_mut()
        .expect("branch schema constraints target must be object");
    let extra = extra_constraints.as_object().unwrap_or_else(|| {
        panic!("extra branch constraints must be object: {extra_constraints:?}")
    });
    schema.extend(extra.clone());
}

pub(super) fn exact_branch_schema(
    properties: &Value,
    discriminators: &[(&str, &str)],
    required: &[&str],
    allowed: &[&str],
) -> Value {
    let root_properties = properties
        .as_object()
        .unwrap_or_else(|| panic!("exact branch properties must be object: {properties:?}"));
    let mut branch_properties = Map::new();
    for name in allowed {
        let schema = root_properties
            .get(*name)
            .unwrap_or_else(|| panic!("exact branch references unknown property {name}"));
        branch_properties.insert((*name).to_string(), schema.clone());
    }
    for (name, value) in discriminators {
        branch_properties.insert((*name).to_string(), json!({"const": value}));
    }

    let mut required_fields = Vec::new();
    for (name, _) in discriminators {
        required_fields.push(*name);
    }
    for name in required {
        if !required_fields.contains(name) {
            required_fields.push(*name);
        }
    }

    json!({
        "type": "object",
        "properties": branch_properties,
        "required": required_fields,
        "additionalProperties": false
    })
}

pub(super) fn non_empty_string_schema() -> Value {
    json!({
        "type": "string",
        "minLength": 1
    })
}

pub(super) fn non_blank_string_schema() -> Value {
    json!({
        "type": "string",
        "minLength": 1,
        "pattern": ".*\\S.*"
    })
}

pub(super) fn optional_absent_string_schema(schema: Value) -> Value {
    json!({
        "anyOf": [
            schema,
            {"type": "string", "const": ""},
            {"type": "null"}
        ]
    })
}

pub(super) fn optional_null_schema(schema: Value) -> Value {
    json!({
        "anyOf": [
            schema,
            {"type": "null"}
        ]
    })
}

pub(super) fn optional_bool_schema(schema: Value) -> Value {
    optional_null_schema(schema)
}

pub(super) fn optional_zero_integer_schema(schema: Value) -> Value {
    json!({
        "anyOf": [
            schema,
            {"type": "integer", "const": 0},
            {"type": "null"}
        ]
    })
}

pub(super) fn limit_schema() -> Value {
    json!({"type": "integer", "minimum": 0})
}

pub(super) fn optional_limit_schema() -> Value {
    optional_null_schema(limit_schema())
}

pub(super) fn any_active_selector_constraint(selectors: &[&str]) -> Value {
    json!({
        "anyOf": selectors.iter().map(|selector| active_selector_constraint(selector)).collect::<Vec<_>>()
    })
}

pub(super) fn one_active_selector_constraint(selectors: &[&str]) -> Value {
    json!({
        "oneOf": selectors.iter().map(|selector| active_selector_constraint(selector)).collect::<Vec<_>>()
    })
}

pub(super) fn same_group_pair_constraints(keys: &[&str]) -> Vec<Value> {
    let mut constraints = Vec::new();
    for (index, left) in keys.iter().enumerate() {
        for right in keys.iter().skip(index + 1) {
            constraints.push(json!({
                "allOf": [
                    active_selector_constraint(left),
                    active_selector_constraint(right)
                ]
            }));
        }
    }
    constraints
}

pub(super) fn active_selector_constraint(selector: &str) -> Value {
    let schema = match selector {
        "pid" | "terminal_pid" => json!({"type": "integer", "minimum": 1}),
        "display_index" => json!({"type": "integer", "minimum": 0}),
        _ => json!({"type": "string", "minLength": 1, "pattern": ".*\\S.*"}),
    };
    json!({
        "required": [selector],
        "properties": {
            selector: schema
        }
    })
}

pub(super) fn window_target_schema() -> Value {
    json!({
        "window_id": optional_absent_string_schema(json!({
            "type": "string",
            "minLength": 1,
            "description": "Exact window_id from list_resources(surface=\"desktop\", resource=\"windows\")."
        })),
        "pid": optional_zero_integer_schema(json!({
            "type": "integer",
            "minimum": 1,
            "description": "Process ID from list_resources(surface=\"desktop\", resource=\"windows\"). 0 is ignored."
        })),
        "tty": optional_absent_string_schema(json!({
            "type": "string",
            "minLength": 1,
            "description": "Terminal tty such as /dev/pts/7 or pts/7."
        })),
        "terminal_pid": optional_zero_integer_schema(json!({
            "type": "integer",
            "minimum": 1,
            "description": "Terminal process ID from desktop window terminal metadata. 0 is ignored."
        })),
        "terminal_command": optional_absent_string_schema(non_empty_string_schema()),
        "terminal_cwd": optional_absent_string_schema(non_empty_string_schema()),
        "app_id": optional_absent_string_schema(non_empty_string_schema()),
        "wm_class": optional_absent_string_schema(non_empty_string_schema()),
        "title": optional_absent_string_schema(non_empty_string_schema())
    })
}

pub(super) fn window_target_constraint() -> Value {
    any_active_selector_constraint(&WINDOW_SELECTOR_KEYS)
}

pub(crate) fn schema_accepts(schema: &Value, instance: &Value) -> bool {
    let Some(schema) = schema.as_object() else {
        return true;
    };
    if !schema
        .keys()
        .all(|keyword| schema_keyword_is_supported(keyword))
    {
        return false;
    }
    if let Some(expected_type) = schema.get("type")
        && !schema_type_accepts(expected_type, instance)
    {
        return false;
    }
    if let Some(required) = schema.get("required").and_then(Value::as_array) {
        let Some(instance_object) = instance.as_object() else {
            return false;
        };
        if !required.iter().all(|field| {
            field
                .as_str()
                .is_some_and(|field| instance_object.contains_key(field))
        }) {
            return false;
        }
    }
    if let Some(expected) = schema.get("const")
        && instance != expected
    {
        return false;
    }
    if let Some(allowed) = schema.get("enum").and_then(Value::as_array)
        && !allowed.iter().any(|allowed| allowed == instance)
    {
        return false;
    }
    if let Some(minimum) = schema.get("minimum").and_then(Value::as_f64) {
        let Some(value) = instance.as_f64() else {
            return false;
        };
        if value < minimum {
            return false;
        }
    }
    if let Some(maximum) = schema.get("maximum").and_then(Value::as_f64) {
        let Some(value) = instance.as_f64() else {
            return false;
        };
        if value > maximum {
            return false;
        }
    }
    if let Some(minimum) = schema.get("exclusiveMinimum").and_then(Value::as_f64) {
        let Some(value) = instance.as_f64() else {
            return false;
        };
        if value <= minimum {
            return false;
        }
    }
    if let Some(maximum) = schema.get("exclusiveMaximum").and_then(Value::as_f64) {
        let Some(value) = instance.as_f64() else {
            return false;
        };
        if value >= maximum {
            return false;
        }
    }
    if let Some(minimum) = schema.get("minLength").and_then(Value::as_u64) {
        let Some(value) = instance.as_str() else {
            return false;
        };
        if value.chars().count() < minimum as usize {
            return false;
        }
    }
    if let Some(maximum) = schema.get("maxLength").and_then(Value::as_u64) {
        let Some(value) = instance.as_str() else {
            return false;
        };
        if value.chars().count() > maximum as usize {
            return false;
        }
    }
    if let Some(minimum) = schema.get("minItems").and_then(Value::as_u64) {
        let Some(value) = instance.as_array() else {
            return false;
        };
        if value.len() < minimum as usize {
            return false;
        }
    }
    if let Some(maximum) = schema.get("maxItems").and_then(Value::as_u64) {
        let Some(value) = instance.as_array() else {
            return false;
        };
        if value.len() > maximum as usize {
            return false;
        }
    }
    if let Some(minimum) = schema.get("minProperties").and_then(Value::as_u64) {
        let Some(value) = instance.as_object() else {
            return false;
        };
        if value.len() < minimum as usize {
            return false;
        }
    }
    if let Some(maximum) = schema.get("maxProperties").and_then(Value::as_u64) {
        let Some(value) = instance.as_object() else {
            return false;
        };
        if value.len() > maximum as usize {
            return false;
        }
    }
    if let Some(pattern) = schema.get("pattern").and_then(Value::as_str) {
        let Some(value) = instance.as_str() else {
            return false;
        };
        if !schema_pattern_accepts(pattern, value) {
            return false;
        }
    }
    if let Some(rejected) = schema.get("not")
        && schema_accepts(rejected, instance)
    {
        return false;
    }
    if let Some(properties) = schema.get("properties").and_then(Value::as_object)
        && let Some(instance_object) = instance.as_object()
    {
        for (name, property_schema) in properties {
            if let Some(property_value) = instance_object.get(name)
                && !schema_accepts(property_schema, property_value)
            {
                return false;
            }
        }
        if schema.get("additionalProperties") == Some(&Value::Bool(false))
            && !instance_object
                .keys()
                .all(|name| properties.contains_key(name))
        {
            return false;
        }
    }
    if let Some(item_schema) = schema.get("items")
        && let Some(instance_array) = instance.as_array()
        && !instance_array
            .iter()
            .all(|item| schema_accepts(item_schema, item))
    {
        return false;
    }
    if let Some(all_of) = schema.get("allOf").and_then(Value::as_array)
        && !all_of.iter().all(|schema| schema_accepts(schema, instance))
    {
        return false;
    }
    if let Some(any_of) = schema.get("anyOf").and_then(Value::as_array)
        && !any_of.iter().any(|schema| schema_accepts(schema, instance))
    {
        return false;
    }
    if let Some(one_of) = schema.get("oneOf").and_then(Value::as_array)
        && one_of
            .iter()
            .filter(|schema| schema_accepts(schema, instance))
            .count()
            != 1
    {
        return false;
    }
    if let Some(if_schema) = schema.get("if")
        && schema_accepts(if_schema, instance)
        && let Some(then_schema) = schema.get("then")
        && !schema_accepts(then_schema, instance)
    {
        return false;
    }
    true
}

pub(super) fn schema_pattern_accepts(pattern: &str, value: &str) -> bool {
    match pattern {
        "^(https?://[^\\s]+|about:blank)$" => {
            value == "about:blank"
                || url_with_scheme_and_non_empty_rest(value, "http://")
                || url_with_scheme_and_non_empty_rest(value, "https://")
        }
        ".*\\S.*" => value.chars().any(|character| !character.is_whitespace()),
        _ => false,
    }
}

pub(super) fn schema_keyword_is_supported(keyword: &str) -> bool {
    matches!(
        keyword,
        "additionalProperties"
            | "allOf"
            | "anyOf"
            | "const"
            | "description"
            | "enum"
            | "exclusiveMaximum"
            | "exclusiveMinimum"
            | "if"
            | "items"
            | "maxItems"
            | "maxLength"
            | "maxProperties"
            | "maximum"
            | "minItems"
            | "minLength"
            | "minProperties"
            | "minimum"
            | "not"
            | "oneOf"
            | "pattern"
            | "properties"
            | "required"
            | "then"
            | "type"
    )
}

pub(super) fn url_with_scheme_and_non_empty_rest(value: &str, scheme: &str) -> bool {
    value
        .strip_prefix(scheme)
        .is_some_and(|rest| !rest.is_empty() && !rest.chars().any(char::is_whitespace))
}

pub(super) fn schema_type_accepts(expected_type: &Value, instance: &Value) -> bool {
    match expected_type {
        Value::String(expected_type) => schema_single_type_accepts(expected_type, instance),
        Value::Array(expected_types) => expected_types.iter().any(|expected_type| {
            expected_type
                .as_str()
                .is_some_and(|expected_type| schema_single_type_accepts(expected_type, instance))
        }),
        _ => false,
    }
}

pub(super) fn schema_single_type_accepts(expected_type: &str, instance: &Value) -> bool {
    match expected_type {
        "array" => instance.is_array(),
        "boolean" => instance.is_boolean(),
        "integer" => instance
            .as_i64()
            .or_else(|| {
                instance
                    .as_u64()
                    .and_then(|value| i64::try_from(value).ok())
            })
            .is_some(),
        "null" => instance.is_null(),
        "number" => instance.is_number(),
        "object" => instance.is_object(),
        "string" => instance.is_string(),
        _ => false,
    }
}
