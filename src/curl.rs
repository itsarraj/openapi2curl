use crate::spec::{Operation, ParamLocation};

/// Wraps a value in single quotes for safe use as a shell argument,
/// escaping any embedded single quote with the standard
/// `'\''`-outside-in trick — the generated commands are meant to be
/// pasted and run as-is, including when an example value happens to
/// contain a quote.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Percent-encodes a query-string component. Keeps unreserved
/// characters plus `@` and `:` unescaped (both are legal, unencoded,
/// inside an RFC 3986 query — the same choice real API tooling like
/// Postman's curl exporter makes, so an email address in a query value
/// doesn't come out mangled).
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'@' | b':' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn build_url(op: &Operation, base_url: &str) -> String {
    let mut path = op.path.clone();
    for p in op
        .parameters
        .iter()
        .filter(|p| p.location == ParamLocation::Path)
    {
        path = path.replace(&format!("{{{}}}", p.name), &p.value);
    }

    let mut url = format!("{}{}", base_url.trim_end_matches('/'), path);

    let query: Vec<String> = op
        .parameters
        .iter()
        .filter(|p| p.location == ParamLocation::Query)
        .map(|p| format!("{}={}", percent_encode(&p.name), percent_encode(&p.value)))
        .collect();
    if !query.is_empty() {
        url.push('?');
        url.push_str(&query.join("&"));
    }

    url
}

/// Renders one [`Operation`] as a multi-line, copy-pasteable `curl`
/// command: `-X METHOD 'url'`, then a `-H` per header parameter, then a
/// default `Accept: application/json` (unless the spec already defines
/// its own `Accept` header parameter), then — for a JSON request body —
/// a `Content-Type` header and a `-d` with the resolved example body.
pub fn render_curl(op: &Operation, base_url: &str) -> String {
    let mut lines = vec![format!(
        "curl -X {} {}",
        op.method,
        shell_quote(&build_url(op, base_url))
    )];

    for p in op
        .parameters
        .iter()
        .filter(|p| p.location == ParamLocation::Header)
    {
        lines.push(format!(
            "-H {}",
            shell_quote(&format!("{}: {}", p.name, p.value))
        ));
    }

    let has_accept_header = op
        .parameters
        .iter()
        .any(|p| p.location == ParamLocation::Header && p.name.eq_ignore_ascii_case("accept"));
    if !has_accept_header {
        lines.push(format!("-H {}", shell_quote("Accept: application/json")));
    }

    if let Some(body) = &op.request_body {
        if let Some(json) = &body.json_body {
            lines.push(format!(
                "-H {}",
                shell_quote(&format!("Content-Type: {}", body.content_type))
            ));
            let body_str = serde_json::to_string(json).unwrap_or_default();
            lines.push(format!("-d {}", shell_quote(&body_str)));
        }
    }

    let mut out = lines[0].clone();
    for line in &lines[1..] {
        out.push_str(" \\\n  ");
        out.push_str(line);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{Parameter, RequestBody};

    fn get_op(path: &str, params: Vec<Parameter>) -> Operation {
        Operation {
            method: "GET".to_string(),
            path: path.to_string(),
            parameters: params,
            request_body: None,
        }
    }

    fn param(name: &str, location: ParamLocation, value: &str) -> Parameter {
        Parameter {
            name: name.to_string(),
            location,
            required: false,
            value: value.to_string(),
        }
    }

    #[test]
    fn get_with_path_and_query_params_renders_exact_structure() {
        let op = get_op(
            "/users/{id}",
            vec![
                param("id", ParamLocation::Path, "1"),
                param("include", ParamLocation::Query, "profile"),
            ],
        );
        let out = render_curl(&op, "https://api.example.com/v1");
        assert_eq!(
            out,
            "curl -X GET 'https://api.example.com/v1/users/1?include=profile' \\\n  -H 'Accept: application/json'"
        );
    }

    #[test]
    fn multiple_query_parameters_preserve_declaration_order_and_are_joined_with_ampersand() {
        let op = get_op(
            "/users",
            vec![
                param("limit", ParamLocation::Query, "20"),
                param("status", ParamLocation::Query, "active"),
            ],
        );
        let out = render_curl(&op, "https://api.example.com");
        assert!(
            out.starts_with("curl -X GET 'https://api.example.com/users?limit=20&status=active'")
        );
    }

    #[test]
    fn query_value_with_a_space_is_percent_encoded() {
        let op = get_op(
            "/search",
            vec![param("q", ParamLocation::Query, "hello world")],
        );
        let out = render_curl(&op, "https://api.example.com");
        assert!(out.contains("q=hello%20world"));
    }

    #[test]
    fn email_address_in_a_query_value_keeps_the_at_sign_unencoded() {
        let op = get_op(
            "/lookup",
            vec![param("email", ParamLocation::Query, "user@example.com")],
        );
        let out = render_curl(&op, "https://api.example.com");
        assert!(out.contains("email=user@example.com"));
    }

    #[test]
    fn header_parameter_renders_as_its_own_dash_h_line() {
        let op = get_op(
            "/private",
            vec![param("X-Api-Key", ParamLocation::Header, "secret123")],
        );
        let out = render_curl(&op, "https://api.example.com");
        assert!(out.contains("-H 'X-Api-Key: secret123'"));
        // still gets the default Accept header since this isn't named Accept
        assert!(out.contains("-H 'Accept: application/json'"));
    }

    #[test]
    fn explicit_accept_header_is_not_duplicated() {
        let op = get_op(
            "/negotiated",
            vec![param("Accept", ParamLocation::Header, "application/xml")],
        );
        let out = render_curl(&op, "https://api.example.com");
        assert_eq!(out.matches("Accept").count(), 1);
        assert!(out.contains("-H 'Accept: application/xml'"));
    }

    #[test]
    fn post_with_json_body_renders_content_type_and_dash_d() {
        let op = Operation {
            method: "POST".to_string(),
            path: "/users".to_string(),
            parameters: vec![],
            request_body: Some(RequestBody {
                content_type: "application/json".to_string(),
                json_body: Some(serde_json::json!({"name": "string", "email": "user@example.com"})),
            }),
        };
        let out = render_curl(&op, "https://api.example.com/v1");
        assert_eq!(
            out,
            "curl -X POST 'https://api.example.com/v1/users' \\\n  -H 'Accept: application/json' \\\n  -H 'Content-Type: application/json' \\\n  -d '{\"email\":\"user@example.com\",\"name\":\"string\"}'"
        );
    }

    #[test]
    fn non_json_request_body_omits_dash_d_but_still_renders_the_command() {
        let op = Operation {
            method: "POST".to_string(),
            path: "/upload".to_string(),
            parameters: vec![],
            request_body: Some(RequestBody {
                content_type: "multipart/form-data".to_string(),
                json_body: None,
            }),
        };
        let out = render_curl(&op, "https://api.example.com");
        assert!(!out.contains("-d "));
        assert!(out.starts_with("curl -X POST 'https://api.example.com/upload'"));
    }

    #[test]
    fn base_url_trailing_slash_does_not_produce_a_double_slash() {
        let op = get_op("/users", vec![]);
        let out = render_curl(&op, "https://api.example.com/");
        assert!(out.contains("'https://api.example.com/users'"));
        assert!(!out.contains("//users"));
    }

    #[test]
    fn value_containing_a_single_quote_is_shell_escaped() {
        let op = get_op(
            "/search",
            vec![param("q", ParamLocation::Query, "o%27brien")],
        );
        // percent-encoded already at the URL level, but exercise the
        // shell_quote escaping path directly via a header instead:
        let op_with_header = get_op(
            "/x",
            vec![param("X-Name", ParamLocation::Header, "O'Brien")],
        );
        let out = render_curl(&op_with_header, "https://api.example.com");
        assert!(out.contains(r"O'\''Brien"));
        let _ = render_curl(&op, "https://api.example.com"); // sanity: doesn't panic
    }
}
