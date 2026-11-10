# Changelog

Releases are tagged `vX.Y.Z` on the `opensdk` binary (`cli/Cargo.toml`); the
library crates are not versioned independently. Earlier releases are described
on their GitHub release pages.

## 0.3.0

- Converter options (`openapi2opencli`, and the `go-cli`/`rust-cli` sdk.json
  sections):
  - `namesFrom` names each command from an operation extension (a dotted path
    such as `x-docs.path` whose value `<group>/.../<verb>` places the command;
    a `retrieve` verb keeps its `get` alias) and records the value as the
    command's `docsPath` metadata; a group it creates is described by the
    matching tag's description, else as `<name> commands`. An operation
    without the value falls back to the derived name unless
    `namesFromRequired` is set (which requires `namesFrom`).
  - `pathPrefixStrip` takes a leading path prefix (`/v3`; a `{param}` segment
    matches any parameter) off before the default derivation; the full path
    still decides the command's shape and stays the request path.
  - `defaultHeaders` and `headersFromExtension` (header → a dotted path under
    `info`, then the document root) resolve into the root
    `x-openapi.defaultHeaders` (an extension property).
  - `includeSecurity` records each command's security requirements as
    `x-openapi.security`: OpenCLI `XOpenApiSecurity` entries — the scheme's
    descriptor as its root entry has it, plus `ref` (the scheme's key) and
    `scopes`.
  All additive: a document converted without them is unchanged. Converter
  fixtures `13.names-from`, `14.headers-and-security`, `15.path-prefix-params`.
- The converter rejects two commands answering to one spelling at one level —
  names AND aliases (a stated `get` beside `retrieve`, whose alias is `get`) —
  which clap rejected at runtime.
- The OpenCLI JSON Schema is vendored under `crates/openapi2opencli/tests`, and
  every committed OpenCLI document (converter outputs, backend fixture inputs,
  the `opensdk opencli` goldens) is validated against it.
- `opensdk opencli [--spec <openapi>] [--config sdk.json] [--lang
  rust-cli|go-cli] [--declare <fragment.json>]... [--output <file>]
  [--sdk-name <name>]` writes the OpenCLI document a CLI target generates from
  and merges hand-written OpenCLI fragments into it (root
  `options`/`exitCodes`/`info.description` replace, `commands` append, a
  command named like an existing top-level one updates its description,
  aliases and hidden flag; a name or alias another top-level command answers
  to is an error). `generate --spec <opencli.json>` drives a CLI target from
  such a document, skipping the converter; an SDK language — or a config
  declaring one beside the CLI targets — refuses it before anything is
  written. Fixtures under `cli/__fixtures__/opencli`.
- `opencli2rust`: a document with root `options`, root `exitCodes` or
  `x-openapi.defaultHeaders` gets the root seam:
  - the root options as clap arguments; `recursive` → global, listed under a
    "Global options" help heading; a recursive option cannot also be required
    in clap, so a required one is emitted optional with a warning;
  - `CliOverrides::before_dispatch` (read them into the `Context` before any
    command runs) and `CliOverrides::print_usage_error` (choose a usage
    error's exit code; default clap's);
  - `Context::headers` seeded from `config::DEFAULT_HEADERS`, sent when a
    request lacks them, with at most one `Authorization` (an explicit one
    replaces the environment credential).
  A root option sharing an id, long or short with clap's `-h/--help` or
  `-V/--version`, another root option, or (when global) any command's option
  or positional is a generation error naming both: `try_opencli2rust` returns
  it, `opencli2rust` panics on it. Other documents generate byte-identically.
  Fixtures `11.root-options` and `12.default-headers`, and
  `tests/runtime_smoke.rs` (`XYD_SMOKE_RUST=1`, failing on any build error),
  which builds both and checks the requests and the help on the wire.
- `opencli2rust`: a `json`-style `--body` reads `@file`, `@-` (stdin) and
  `@@...` (a literal value starting with `@`) as well as an inline value, and
  sends the declared media type as the `Content-Type` when it is not
  `application/json` (a `flatten` or `multipart` body is JSON-assembled and
  stays `application/json`); the `--body` value is no longer also sent as a
  `?body=` query parameter; a resource with no API leaf no longer emits a
  one-arm `match` (`clippy::match_single_binding` in the generated crate).
- `baseURL` in a CLI section reaches the generated config in both backends (it
  passed the option split and was then dropped; `baseUrl` is accepted too), and
  the `regen` binary reads `version_from` and `base_url` from `regen.toml`.
- An `oauth2` security scheme maps to the `bearer` kind (RFC 6750: the access
  token is sent as `Authorization: Bearer <token>`) in both converters, instead
  of `other`. Every generated SDK and CLI now reads the token from the same
  variable and sends the same header as for an `http`/`bearer` scheme; the Go
  and Java SDKs and both CLIs previously sent no credential for an `oauth2`
  scheme. The IR entry carries `scheme: "bearer"`, `flows` (the flow types) and
  `scopes` (the scope catalogue merged across the flows), both declared on
  `SdkSecurity` in the IR schema, so `opensdk diff` does not report a document
  moving from `http`/`bearer` to `oauth2` as `security-changed`.

## 0.2.2

- Nested type groups on the docs type reference: `typeReference.nested`
  (`x-sdk.<lang>.types.nested`) renders every IR type reachable from an
  operation's request (the body and the query/header parameter types) and
  response — those roots included, inline/synthesized types too — as a lookup
  table keyed by IR type name, each entry `{ typeName, kind, schemaRef?, description?, fields | values
  | variants | of }` with struct fields rendered the way the language's models
  declare them. The walk is breadth-first with a cycle guard; `nestedDepth` in
  a language's emitter options caps the hops from the operation (default 8,
  `0` leaves `nested` out).
- The source schema on every rendered type and field: `schemaRef`
  (`#/components/schemas/<Name>`) wherever the IR type came from a component
  schema, absent for a type hoisted from an inline schema whatever its
  synthesized name; `refTypeName` (the IR type name, the `nested` key) on the
  request and response groups beside the language `typeName`. The converter
  records the origin on the IR as `types[].schemaRef`.
- `refTypeName` names the IR type behind an array or map field too
  (`data: Widget[]` → `Widget`), not only a direct ref.
- Fixtures: converter `12.nested-envelope` and the emitters' `13.nested-envelope`
  (a list envelope kept as the declared type whose paging block is an inline
  object beside an array of a component that names further components, one of
  them through a `oneOf` union of two components and an array alias; a query
  param that refs an enum). The docs suites render a fixture with its
  `docs.options.json` bag, else `{ "nestedDepth": 0 }`: the captured corpus
  goldens were re-minted at depth 0 (`XYD_BLESS=1`) and differ from the capture
  only in the additive keys above; `13.nested-envelope` is the full `nested`
  golden per language.

## 0.2.1

- `xsdk --config <sdk.json>` (plus `--sdk-name` / `--grouping`): the docs pass
  receives the same identity and options as `generate` — `sdkName`, each
  language's section (`packageName`, `modulePath`, `envVar`, ...), `grouping`
  and the per-language `behavior` layers — so the embedded `x-sdk` samples
  import the configured package, read the `<SDKNAME>_API_KEY` variable and call
  the hinted method names. `EmbedXSdkOptions` carries the same for library
  callers of `docs_map`.
- `grouping.operationHints["METHOD /path"].pagination: false | true` and
  `behavior.pagination: "none" | "auto"`: keep an operation's (or every
  operation's) declared response type instead of the `Page<T>` wrap the list
  heuristic produces. Honoured by the converter, all seven emitters and the
  docs samples; the heuristic itself is now documented on `detect_pagination`
  and in the README.
- `behavior.defaultHeaders` (static) and `behavior.headersFromExtension` (a
  header whose value is an `x-*` extension of the document) are emitted into
  every language's runtime and sent on every request, under the per-call
  headers; each client gains an override knob (`defaultHeaders` /
  `default_headers` / `defaultHeader(..)` / `default_header(..)`). The
  multi-target `generate` re-resolves a per-language layer's extension headers
  against the document.
- A per-language `behavior` layer is applied by running the converter again for
  that language (multi-target `generate` and `xsdk` alike), so the keys the
  converter decides on — `pagination`, `idempotency`, `headersFromExtension` —
  take effect per language exactly as on `generate --lang`; the `pagination`
  shorthand is expanded in every layer before the layers merge. A `null` under
  `defaultHeaders` cancels a header whether it is static or extension-derived.
- `generate` from a pre-parsed IR (`--spec ir.json`) applies the config's
  `behavior` (global and per-language) over the IR's `sdk` block on both the
  single- and the multi-target path; `headersFromExtension` entries the IR
  carries no value for are reported, there being no document to resolve them
  against.
- The Go usage sample reads the credential from the environment
  (`os.Getenv("<SDKNAME>_API_KEY")`) like the other five languages; no sample
  pastes a literal key.
- Converter fixtures `10.pagination-hint` / `11.pagination-none` (option-driven
  cases with an `options.json`), emitter fixture `12.default-headers` for all
  seven languages, and `cargo run -p opensdk --example bless` to (re-)mint an
  emitter fixture's goldens.

## 0.2.0

See the GitHub release.
