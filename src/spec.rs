use anyhow::{Context, Result};
use serde_json::Value;

use crate::example::{build_json_example, placeholder_for_schema, scalar_to_string};

const HTTP_METHODS: &[&str] = &[
    "get", "post", "put", "patch", "delete", "options", "head", "trace",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamLocation {
    Path,
    Query,
    Header,
    Cookie,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Parameter {
    pub name: String,
    pub location: ParamLocation,
    pub required: bool,
    /// The resolved example/default/type-based placeholder, already a
    /// plain string ready to interpolate into a URL or header.
    pub value: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RequestBody {
    pub content_type: String,
    /// `None` when the body's content type isn't `application/json` —
    /// this v1 only renders JSON bodies (see README scope notes).
    pub json_body: Option<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Operation {
    pub method: String,
    pub path: String,
    pub parameters: Vec<Parameter>,
    pub request_body: Option<RequestBody>,
}

pub struct ParsedSpec {
    pub base_url: Option<String>,
    pub operations: Vec<Operation>,
}

/// Parses an OpenAPI 3.x document, JSON or YAML. JSON is tried first
/// (a strict, faster parse); a document that isn't valid JSON falls
/// back to YAML — this also transparently covers pure JSON-as-YAML,
/// but trying JSON directly first avoids YAML's looser scalar coercion
/// rules (e.g. an unquoted `on`/`off` string) surprising a JSON author.
pub fn parse_spec(content: &str) -> Result<ParsedSpec> {
    let root: Value = serde_json::from_str(content)
        .or_else(|_| serde_yaml::from_str(content))
        .context("not valid JSON or YAML")?;

    let base_url = root
        .get("servers")
        .and_then(Value::as_array)
        .and_then(|servers| servers.first())
        .and_then(|s| s.get("url"))
        .and_then(Value::as_str)
        .map(str::to_string);

    let paths = root
        .get("paths")
        .and_then(Value::as_object)
        .context("no top-level 'paths' object — not an OpenAPI spec?")?;

    let mut operations = Vec::new();
    for (path, path_item) in paths {
        let Some(path_item_obj) = path_item.as_object() else {
            continue;
        };
        for method in HTTP_METHODS {
            let Some(operation) = path_item_obj.get(*method) else {
                continue;
            };
            operations.push(Operation {
                method: method.to_uppercase(),
                path: path.clone(),
                parameters: extract_parameters(operation),
                request_body: extract_request_body(operation),
            });
        }
    }

    Ok(ParsedSpec {
        base_url,
        operations,
    })
}

fn extract_parameters(operation: &Value) -> Vec<Parameter> {
    let Some(params) = operation.get("parameters").and_then(Value::as_array) else {
        return Vec::new();
    };
    params
        .iter()
        .filter_map(|p| {
            let name = p.get("name")?.as_str()?.to_string();
            let location = match p.get("in")?.as_str()? {
                "path" => ParamLocation::Path,
                "query" => ParamLocation::Query,
                "header" => ParamLocation::Header,
                "cookie" => ParamLocation::Cookie,
                _ => return None,
            };
            let required = p.get("required").and_then(Value::as_bool).unwrap_or(false);
            let empty = Value::Object(serde_json::Map::new());
            let schema = p.get("schema").unwrap_or(&empty);
            // A parameter's own top-level `example` (a sibling of
            // `schema`, not inside it) takes priority over anything
            // found while resolving the schema itself.
            let value = match p.get("example") {
                Some(example) => scalar_to_string(example),
                None => placeholder_for_schema(schema),
            };
            Some(Parameter {
                name,
                location,
                required,
                value,
            })
        })
        .collect()
}

fn extract_request_body(operation: &Value) -> Option<RequestBody> {
    let content = operation.get("requestBody")?.get("content")?.as_object()?;

    let (content_type, media) = match content.get("application/json") {
        Some(media) => ("application/json".to_string(), media),
        None => {
            let (k, v) = content.iter().next()?;
            (k.clone(), v)
        }
    };

    if content_type != "application/json" {
        // A form-encoded or multipart body isn't rendered as `-d` JSON
        // in this v1 — the command is still emitted, just without a
        // body (see README scope notes).
        return Some(RequestBody {
            content_type,
            json_body: None,
        });
    }

    let example = media.get("example").cloned().or_else(|| {
        media
            .get("examples")
            .and_then(Value::as_object)
            .and_then(|exs| exs.values().next())
            .and_then(|ex| ex.get("value"))
            .cloned()
    });

    let json_body = Some(example.unwrap_or_else(|| {
        media
            .get("schema")
            .map(build_json_example)
            .unwrap_or(Value::Null)
    }));

    Some(RequestBody {
        content_type,
        json_body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "openapi": "3.0.3",
        "servers": [{"url": "https://api.example.com/v1"}],
        "paths": {
            "/users": {
                "get": {
                    "parameters": [
                        {"name": "limit", "in": "query", "schema": {"type": "integer", "default": 20}}
                    ]
                },
                "post": {
                    "requestBody": {
                        "content": {
                            "application/json": {
                                "schema": {
                                    "type": "object",
                                    "properties": {"name": {"type": "string"}}
                                }
                            }
                        }
                    }
                }
            },
            "/users/{id}": {
                "get": {
                    "parameters": [
                        {"name": "id", "in": "path", "required": true, "schema": {"type": "integer"}},
                        {"name": "include", "in": "query", "example": "profile"}
                    ]
                }
            }
        }
    }"#;

    #[test]
    fn extracts_base_url_from_servers() {
        let parsed = parse_spec(SAMPLE).unwrap();
        assert_eq!(
            parsed.base_url.as_deref(),
            Some("https://api.example.com/v1")
        );
    }

    #[test]
    fn extracts_one_operation_per_path_and_method() {
        let parsed = parse_spec(SAMPLE).unwrap();
        assert_eq!(parsed.operations.len(), 3);
        assert!(parsed
            .operations
            .iter()
            .any(|o| o.path == "/users" && o.method == "GET"));
        assert!(parsed
            .operations
            .iter()
            .any(|o| o.path == "/users" && o.method == "POST"));
        assert!(parsed
            .operations
            .iter()
            .any(|o| o.path == "/users/{id}" && o.method == "GET"));
    }

    #[test]
    fn path_parameter_with_no_example_gets_a_type_based_placeholder() {
        let parsed = parse_spec(SAMPLE).unwrap();
        let op = parsed
            .operations
            .iter()
            .find(|o| o.path == "/users/{id}")
            .unwrap();
        let id = op.parameters.iter().find(|p| p.name == "id").unwrap();
        assert_eq!(id.location, ParamLocation::Path);
        assert!(id.required);
        assert_eq!(id.value, "1");
    }

    #[test]
    fn parameter_top_level_example_is_used_verbatim() {
        let parsed = parse_spec(SAMPLE).unwrap();
        let op = parsed
            .operations
            .iter()
            .find(|o| o.path == "/users/{id}")
            .unwrap();
        let include = op.parameters.iter().find(|p| p.name == "include").unwrap();
        assert_eq!(include.value, "profile");
    }

    #[test]
    fn query_parameter_schema_default_is_used_when_no_example() {
        let parsed = parse_spec(SAMPLE).unwrap();
        let op = parsed
            .operations
            .iter()
            .find(|o| o.path == "/users" && o.method == "GET")
            .unwrap();
        let limit = op.parameters.iter().find(|p| p.name == "limit").unwrap();
        assert_eq!(limit.value, "20");
    }

    #[test]
    fn request_body_is_built_from_schema_properties_when_no_example() {
        let parsed = parse_spec(SAMPLE).unwrap();
        let op = parsed
            .operations
            .iter()
            .find(|o| o.path == "/users" && o.method == "POST")
            .unwrap();
        let body = op.request_body.as_ref().unwrap();
        assert_eq!(body.content_type, "application/json");
        assert_eq!(
            body.json_body.as_ref().unwrap(),
            &serde_json::json!({"name": "string"})
        );
    }

    #[test]
    fn operation_with_no_request_body_key_is_none() {
        let parsed = parse_spec(SAMPLE).unwrap();
        let op = parsed
            .operations
            .iter()
            .find(|o| o.path == "/users" && o.method == "GET")
            .unwrap();
        assert!(op.request_body.is_none());
    }

    #[test]
    fn yaml_input_parses_identically_to_the_equivalent_json() {
        let yaml = r#"
openapi: "3.0.3"
paths:
  /ping:
    get: {}
"#;
        let parsed = parse_spec(yaml).unwrap();
        assert_eq!(parsed.operations.len(), 1);
        assert_eq!(parsed.operations[0].path, "/ping");
        assert_eq!(parsed.operations[0].method, "GET");
    }

    #[test]
    fn non_method_keys_at_the_path_item_level_are_ignored() {
        let spec = r#"{"paths": {"/x": {"parameters": [], "get": {}}}}"#;
        let parsed = parse_spec(spec).unwrap();
        assert_eq!(parsed.operations.len(), 1);
    }

    #[test]
    fn missing_paths_object_is_a_clean_error() {
        assert!(parse_spec(r#"{"openapi": "3.0.0"}"#).is_err());
    }

    #[test]
    fn garbage_input_is_a_clean_error_not_a_panic() {
        assert!(parse_spec("{{{not json or yaml:::").is_err());
    }
}
