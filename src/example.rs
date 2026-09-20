use serde_json::Value;

/// A resolved JSON scalar rendered as plain text for interpolation into
/// a URL or header value. Arrays/objects fall back to their compact JSON
/// text — an edge case a path/query/header value shouldn't realistically
/// hit, but rendering *something* correct beats panicking.
pub fn scalar_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Resolves a single placeholder value for a parameter's JSON Schema,
/// in priority order: the schema's own `example`, then its `default`,
/// then the first `enum` value, then a sensible type/format-based
/// placeholder. Called after checking the *parameter's own* top-level
/// `example` first (OpenAPI allows an `example` directly on the
/// parameter object, outside `schema` — that one wins if present).
pub fn placeholder_for_schema(schema: &Value) -> String {
    if let Some(v) = schema.get("example") {
        return scalar_to_string(v);
    }
    if let Some(v) = schema.get("default") {
        return scalar_to_string(v);
    }
    if let Some(first) = schema
        .get("enum")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
    {
        return scalar_to_string(first);
    }

    match schema.get("type").and_then(Value::as_str) {
        Some("integer") => "1".to_string(),
        Some("number") => "1.5".to_string(),
        Some("boolean") => "true".to_string(),
        Some("array") => "value".to_string(),
        Some("string") | None => match schema.get("format").and_then(Value::as_str) {
            Some("uuid") => "00000000-0000-0000-0000-000000000000".to_string(),
            Some("date") => "2024-01-01".to_string(),
            Some("date-time") => "2024-01-01T00:00:00Z".to_string(),
            Some("email") => "user@example.com".to_string(),
            _ => "string".to_string(),
        },
        _ => "value".to_string(),
    }
}

const MAX_DEPTH: usize = 8;

/// Builds a full example JSON value for a request-body schema, walking
/// `properties`/`items` recursively. Depth-limited (real schemas don't
/// nest this deep; a self-referential schema — `Category` containing a
/// `children: [Category]` field — would otherwise recurse forever).
pub fn build_json_example(schema: &Value) -> Value {
    build_json_example_at(schema, 0)
}

fn build_json_example_at(schema: &Value, depth: usize) -> Value {
    if let Some(example) = schema.get("example") {
        return example.clone();
    }
    if let Some(default) = schema.get("default") {
        return default.clone();
    }
    if let Some(first) = schema
        .get("enum")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
    {
        return first.clone();
    }
    if depth >= MAX_DEPTH {
        return Value::Null;
    }

    let is_object = schema.get("type").and_then(Value::as_str) == Some("object")
        || (schema.get("type").is_none() && schema.get("properties").is_some());

    if is_object {
        let mut map = serde_json::Map::new();
        if let Some(props) = schema.get("properties").and_then(Value::as_object) {
            for (name, prop_schema) in props {
                map.insert(name.clone(), build_json_example_at(prop_schema, depth + 1));
            }
        }
        return Value::Object(map);
    }

    match schema.get("type").and_then(Value::as_str) {
        Some("array") => {
            let items = schema.get("items").cloned().unwrap_or(Value::Null);
            Value::Array(vec![build_json_example_at(&items, depth + 1)])
        }
        Some("integer") => serde_json::json!(1),
        Some("number") => serde_json::json!(1.5),
        Some("boolean") => Value::Bool(true),
        Some("string") | None => match schema.get("format").and_then(Value::as_str) {
            Some("uuid") => Value::String("00000000-0000-0000-0000-000000000000".to_string()),
            Some("date") => Value::String("2024-01-01".to_string()),
            Some("date-time") => Value::String("2024-01-01T00:00:00Z".to_string()),
            Some("email") => Value::String("user@example.com".to_string()),
            _ => Value::String("string".to_string()),
        },
        _ => Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_example_wins_over_default_and_type() {
        let schema = serde_json::json!({"type": "integer", "example": 42, "default": 1});
        assert_eq!(placeholder_for_schema(&schema), "42");
    }

    #[test]
    fn schema_default_wins_over_enum_and_type() {
        let schema = serde_json::json!({"type": "string", "default": "pending", "enum": ["active", "inactive"]});
        assert_eq!(placeholder_for_schema(&schema), "pending");
    }

    #[test]
    fn enum_first_value_wins_over_bare_type_placeholder() {
        let schema = serde_json::json!({"type": "string", "enum": ["active", "inactive"]});
        assert_eq!(placeholder_for_schema(&schema), "active");
    }

    #[test]
    fn integer_type_falls_back_to_a_sensible_placeholder() {
        let schema = serde_json::json!({"type": "integer"});
        assert_eq!(placeholder_for_schema(&schema), "1");
    }

    #[test]
    fn boolean_type_falls_back_to_true() {
        let schema = serde_json::json!({"type": "boolean"});
        assert_eq!(placeholder_for_schema(&schema), "true");
    }

    #[test]
    fn string_format_email_gets_a_realistic_placeholder() {
        let schema = serde_json::json!({"type": "string", "format": "email"});
        assert_eq!(placeholder_for_schema(&schema), "user@example.com");
    }

    #[test]
    fn string_format_uuid_gets_a_realistic_placeholder() {
        let schema = serde_json::json!({"type": "string", "format": "uuid"});
        assert_eq!(
            placeholder_for_schema(&schema),
            "00000000-0000-0000-0000-000000000000"
        );
    }

    #[test]
    fn bare_string_type_with_no_hints_gets_a_generic_placeholder() {
        let schema = serde_json::json!({"type": "string"});
        assert_eq!(placeholder_for_schema(&schema), "string");
    }

    #[test]
    fn empty_schema_defaults_to_a_generic_string_placeholder() {
        assert_eq!(placeholder_for_schema(&serde_json::json!({})), "string");
    }

    #[test]
    fn builds_a_full_object_example_from_properties() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "age": {"type": "integer", "default": 18},
                "verified": {"type": "boolean"}
            }
        });
        let example = build_json_example(&schema);
        assert_eq!(
            example,
            serde_json::json!({"name": "string", "age": 18, "verified": true})
        );
    }

    #[test]
    fn builds_an_array_example_from_items() {
        let schema = serde_json::json!({"type": "array", "items": {"type": "string"}});
        assert_eq!(build_json_example(&schema), serde_json::json!(["string"]));
    }

    #[test]
    fn top_level_schema_example_is_used_verbatim_over_generated_fields() {
        let schema = serde_json::json!({
            "type": "object",
            "example": {"name": "Ada Lovelace"},
            "properties": {"name": {"type": "string"}}
        });
        assert_eq!(
            build_json_example(&schema),
            serde_json::json!({"name": "Ada Lovelace"})
        );
    }

    #[test]
    fn deeply_self_referential_schema_terminates_instead_of_recursing_forever() {
        // A schema that (structurally) refers to itself via `items`
        // without ever hitting a scalar leaf must still terminate.
        fn nested(depth: usize) -> Value {
            if depth == 0 {
                serde_json::json!({"type": "array", "items": Value::Null})
            } else {
                serde_json::json!({"type": "array", "items": nested(depth - 1)})
            }
        }
        let schema = nested(20);
        // Must return without stack overflow or hanging; exact shape
        // beyond the depth cutoff isn't the point, termination is.
        let _ = build_json_example(&schema);
    }
}
