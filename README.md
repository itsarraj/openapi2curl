# openapi2curl

Reads an OpenAPI 3.x spec and prints a ready-to-run `curl` command for
every operation, with real example values filled in for path/query
parameters and JSON request bodies. The usual alternative is opening
Swagger UI and clicking "Try it out" through a browser, or hand-writing
the `curl` yourself from the spec — this does it for the whole spec in
one pass, from a terminal, in CI-scriptable form.

## Usage

```bash
openapi2curl spec.json
openapi2curl spec.yaml --base-url https://staging.example.com
```

```
$ openapi2curl users-api.json
# GET /users
curl -X GET 'https://api.example.com/v1/users?limit=20&status=active' \
  -H 'Accept: application/json'

# POST /users
curl -X POST 'https://api.example.com/v1/users' \
  -H 'Accept: application/json' \
  -H 'Content-Type: application/json' \
  -d '{"age":18,"email":"user@example.com","name":"string"}'

# GET /users/{id}
curl -X GET 'https://api.example.com/v1/users/1?include=profile' \
  -H 'Accept: application/json'
```

Both JSON and YAML specs are accepted — the file extension isn't
consulted, the content is (JSON is tried first, YAML as a fallback).
The base URL defaults to the spec's first `servers[].url` entry, is
overridable with `--base-url` (useful for pointing the exact same spec
at a local/staging server instead of whatever's baked into the spec),
and falls back to `http://localhost:8080` if the spec declares no
servers at all.

## Where the example values come from

For each parameter and request-body field, in priority order:

1. The parameter's own top-level `example` (or the request body
   media type's `example`/first `examples` entry).
2. The schema's `example`.
3. The schema's `default`.
4. The first value in the schema's `enum`.
5. A type/format-based placeholder: `1` for `integer`, `1.5` for
   `number`, `true` for `boolean`, `user@example.com` for a `string`
   with `format: email`, an all-zeros UUID for `format: uuid`, a
   fixed date/date-time for `format: date`/`date-time`, and a generic
   `"string"` otherwise.

Request bodies are built the same way, recursively, from the schema's
`properties` (or `items`, for an array body) when no `example` is
present anywhere on the schema.

## Status: built and verified against a realistic 3-operation spec, including actually running the generated commands against a live local server

- **34 unit tests** (`cargo test --lib`): the example-value resolution
  priority chain (`example` beats `default` beats `enum` beats a
  type-based placeholder, tested both directions so a lower-priority
  source never wins by accident), format-specific placeholders (email,
  uuid, date, date-time), recursive request-body construction from
  nested `properties`/`items` including a depth-limited termination
  test for a structurally self-referential schema; spec parsing (path/
  method/parameter/request-body extraction, `servers[0].url` picked up
  as the default base URL, a YAML document parsing to the identical
  operation list as the equivalent JSON, non-method sibling keys like
  a shared `parameters` block correctly ignored); and command
  rendering (exact string assertions on the full multi-line output for
  a GET with path+query params and a POST with a JSON body, query
  values containing a space percent-encoded while an `@` in an email
  address is deliberately left unencoded, a header parameter rendered
  as its own `-H` line without duplicating a spec-declared `Accept`
  header, single-quote shell-escaping for a value that contains one).
- **CLI run against a realistic 3-operation `users-api` spec** (`GET
  /users` with two query parameters, `POST /users` with a JSON request
  body, `GET /users/{id}` with a path parameter and an example-valued
  query parameter), both as JSON and as the line-for-line equivalent
  YAML: the two produced byte-identical output (`diff` confirmed zero
  differences), and `--base-url` correctly overrode the spec's own
  `servers[0].url` in the output.
- **Went further than string assertions**: all 3 generated commands
  were extracted from real tool output and actually executed with
  `bash`/`curl` against a real local Python `http.server` listening on
  `127.0.0.1:8931`, logging exactly what it received. All 3 hit the
  correct method and path (`GET /users?limit=20&status=active`, `POST
  /users`, `GET /users/1?include=profile`), the POST's `Content-Type`
  and JSON body arrived intact and byte-identical to what the tool
  printed, and every request carried the expected `Accept:
  application/json` header — confirming the generated commands aren't
  just plausible-looking strings, they're real, working HTTP requests.

**Not done / deliberately deferred**: `$ref` resolution isn't
implemented — a parameter or schema defined via `{"$ref":
"#/components/..."}` isn't followed, the same documented scope limit
`apidiff` has for parameters. Only `application/json` request bodies
get a rendered `-d` body; a `multipart/form-data` or
`application/x-www-form-urlencoded` operation still gets a full `curl`
command (method, path, headers) but no body content — real multipart
form generation is a meaningfully bigger feature, not attempted here.
Query-parameter array serialization (`style`/`explode`) isn't modeled;
an array-typed query parameter always renders as a single placeholder
value rather than the comma-joined, repeated-key, or `deepObject`
forms OpenAPI allows. `oneOf`/`anyOf`/`allOf` schema composition isn't
resolved when building a request-body example — a property using one
of these renders as `null` rather than picking a branch. No
authentication scheme handling (`securitySchemes`) — an operation that
requires a bearer token or API key doesn't get an `Authorization`
header added automatically; that's left for the operator to add by
hand, the same way `apidiff` doesn't attempt request/response body
schema diffing in its own v1.
