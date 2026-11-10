# opensdk

An SDK toolchain: it turns an OpenAPI 3.x document into typed, functional client
SDKs for seven languages, and into command-line interfaces.

```
OpenAPI 3.x ──► OpenSDK IR ──► go · node · python · ruby · java · dotnet · rust
            └─► OpenCLI    ──► go-cli · rust-cli
```

Rust throughout. The `opensdk` binary is the user-facing entry point; the
crates are also consumable directly as libraries.

## Layout

| path | what |
|---|---|
| `crates/openapi2opensdk` | OpenAPI → the OpenSDK IR |
| `crates/openapi2opencli` | OpenAPI → an OpenCLI doc (+ the `x-openapi` request binding) |
| `crates/opencli2{go,rust}` | OpenCLI → a buildable CLI project |
| `crates/opencli2opensdk` | OpenCLI → the OpenSDK IR |
| `crates/opensdk_{go,node,python,ruby,java,dotnet,rust}` | the seven emitters |
| `crates/opensdk_framework` | the emitter contract, orchestrator, and the regen-safe `write_project` lifecycle |
| `crates/opensdk_{core,config,diff,chain}` | IR + behavior, config shapes, the breaking-change classifier, the chain pipeline |
| `cli` | the `opensdk` binary |
| `crates/oas_doc` | `DocCtx` — the shared spec loader/dereferencer |
| `crates/{opensdk_cli_common,opensdk_e2e,parity_kit}` | test harnesses |

## Developing

```bash
cargo test --workspace     # the goldens
npm ci                     # tsc + @types/node, needed by the node emitter's smokes
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
```

Two things about the tests are worth knowing before you change anything:

**The goldens are byte comparisons.** Generated trees, `docs.json`, recorded
requests — all compared byte for byte against committed output. That is why
`Cargo.lock` and `package-lock.json` are committed, why `.gitattributes` pins
`eol=lf`, and why `serde_json`'s `preserve_order` feature is load-bearing
(without it every emitted JSON key sorts and the goldens re-order wholesale).

**The toolchain tiers skip silently by default.** `compile_smoke` ("does the
generated SDK actually compile?") and `cli_smoke` ("does it send the right
argv?") return early when a language toolchain is absent, and `cargo test`
still prints `ok`. CI sets `XYD_SMOKE_<LANG>=1` and `XYD_CLI_SMOKE_<LANG>=1`,
which turn a missing toolchain into a failure — so the tiers cannot go dark.
Set them locally too if you want the real thing. Never set `XYD_BLESS` in CI:
it regenerates goldens instead of checking them.

## CLI generation (`go-cli`, `rust-cli`)

A CLI target runs two steps: the converter (`openapi2opencli`) turns the OpenAPI
document into an OpenCLI document — the command tree plus an `x-openapi`
request binding per command — and the backend (`opencli2go`, `opencli2rust`)
turns that into a buildable project. The target's sdk.json section is one flat
bag holding both steps' options; the two key sets are disjoint.

**Converter options.** Also accepted in a root `x-cli` block of the OpenAPI
document (an explicit option wins over the block):

| option | what |
|---|---|
| `cliName` | the CLI's name (`info.title`); default: the SDK name, else the document title |
| `version` | `info.version`; default: the publish version, else the document's |
| `rootCommand` | wrap every generated command under one parent (`<cli> api ...`) |
| `grammar` | `"noun-verb"` (default, `widgets list`) or `"verb-noun"` (`get widgets`) |
| `namesFrom` | name each command from an OPERATION extension instead of its method and path: a dotted path such as `x-docs.path`, whose value `<group>/.../<verb>` gives the group path (every segment but the last) and the verb; kebab-cased, a `retrieve` verb keeps its `get` alias, recorded as the command's `docsPath` metadata. A group it creates is described by the tag whose kebab-cased name matches (its `description`), else as `<name> commands`. An operation without the extension falls back to the derived name |
| `namesFromRequired` | a missing `namesFrom` value is an error (every offending operation is listed); requires `namesFrom` |
| `pathPrefixStrip` | strip a leading path prefix (`/v3`, `/orgs/{org}` — a `{param}` segment matches any parameter) before deriving a name, segment-aligned. The stripped path gives the name; the FULL path decides the shape (an item path still promotes a trailing custom verb), and the request keeps the full path |
| `defaultHeaders` | literal headers every request carries unless it sets them, emitted as the root `x-openapi.defaultHeaders` — an extension property the OpenCLI schema does not declare (its objects are open); sent by the `rust-cli` runtime, not yet by `go-cli` |
| `headersFromExtension` | header → a dotted path under `info` (then the document root) whose value is the header's, resolved at conversion time; a present value wins over a `defaultHeaders` entry of the same name |
| `includeSecurity` | record each command's security requirements as `x-openapi.security`, from the operation's `security` (else the document's): one OpenCLI `XOpenApiSecurity` entry per scheme — the scheme's descriptor as the root entry has it (`type`, `kind`, `envVar`, `scheme`/`in`/`name`/`bearerFormat`) plus `ref` (its `components.securitySchemes` key) and `scopes`, as written |
| `includePaths` | only paths starting with one of these prefixes |
| `includeMethods` | the HTTP methods to convert (default `get put patch post delete`) |
| `includeHeaders` | header and cookie parameters become flags (default: skipped) |
| `flagCase` | `"kebab"` (default) or `"camel"` flag names |
| `bodyStrategy` | `"json"`: always one `--body` flag instead of a flag per top-level field |
| `actionAliases` | `false` drops the automatic `get` alias of `retrieve` |
| `verbMap` | rename the derived verbs (`listCollection`, `getItem`, `createCollection`, `updateItem`, `deleteItem`) |
| `customActionVerbs` | trailing path segments that read as a verb (`/jobs/{id}/cancel` → `jobs cancel`) |
| `singularOverrides` | irregular singulars for `verb-noun` placement |
| `authEnvVar` | the credential's environment variable (default `<CLINAME>_API_KEY`) |
| `grouping`, `maxBodyDepth` | accepted for compatibility; no effect |

Per operation, an `x-cli` block (`group`, `verb`, `aliases`, `hidden`,
`description`, `grammar`, `ignore`) overrides whatever placed the command.
Names and aliases share one namespace per level: two commands answering to one
spelling — two operations named alike, or a stated `get` beside a `retrieve`
whose alias is `get` — is a conversion error naming both.

Every OpenCLI document the repo commits (the converter's fixture outputs, the
backends' fixture inputs, the `opensdk opencli` goldens) is validated against
the OpenCLI JSON Schema vendored in `crates/openapi2opencli/tests`.

**Backend options.** `rust-cli`: `crateName`, `binName`, `edition`, `baseURL`,
`versionFrom` (`"spec"` or `"crate"`); `go-cli`: `modulePath`, `binName`,
`goVersion`, `baseURL`.

**The OpenCLI document as an artifact.** `opensdk opencli` stops after the
converter and writes the OpenCLI JSON, with the converter options of a CLI
section (`--lang`, default: the first CLI section in the config):

```bash
# opensdk opencli [--spec <openapi>] [--config sdk.json] [--lang rust-cli|go-cli]
#                 [--declare <fragment>]... [--output <file>] [--sdk-name <name>]
opensdk opencli --config sdk.json --lang rust-cli \
  --declare cli/root.json --declare cli/auth.json --output opencli.json
opensdk generate --config sdk.json --lang rust-cli --spec opencli.json
```

`--spec` defaults to the config's `api`, and `--sdk-name` to its `sdkName`
(the CLI name when the section sets no `cliName`).

Each `--declare` fragment is an OpenCLI-shaped object merged in order: root
`options`, `exitCodes` and `info.description` replace; `commands` append — a
fragment command named like an existing top-level command (the `rootCommand`
wrapper, say) instead updates that command's `description`, `aliases` and
`hidden` and leaves its children alone. A name or alias another top-level
command already answers to is an error, and so is an unknown root key; keys
the merge does not know (a `$schema`) keep their place.
`generate` recognizes an OpenCLI document (a root `opencli` key) and hands it
straight to the backend, so a CLI that is part generated and part hand-written
is described by one document. Only the CLI targets generate from it: a config
that also declares SDK languages is refused before anything is written.

**The Rust root seam.** A document with root `options`, root `exitCodes` or
`x-openapi.defaultHeaders` gets a runtime with a seam for the hand-written
half of a CLI (any other document generates exactly as before; `go-cli` does
not read these yet):

- root `options` become clap arguments on the root command; a `recursive` one
  is `.global(true)` (accepted after any subcommand, listed under a "Global
  options" heading in every `--help`). clap cannot require a global argument,
  so a recursive option that is also `required` is emitted optional, with a
  warning. A root option sharing an id, long or short with clap's `-h/--help`
  or `-V/--version`, another root option, or — when global — any command's
  option or positional is a generation error naming both
  (`opencli2rust::try_opencli2rust`; `opencli2rust` panics on it);
- `CliOverrides::before_dispatch(&self, ctx: &mut Context, root: &ArgMatches)`
  runs after parsing and before any command, to read root options into the
  context;
- `CliOverrides::print_usage_error(&self, err: &clap::Error) -> ExitCode`
  prints a usage error (and `--help`/`--version`) and chooses the exit code;
  the default is clap's (2, or 0 for help and version);
- `Context::headers`, seeded from `config::DEFAULT_HEADERS`, are sent on every
  request that does not set them itself, and an `Authorization` header — on the
  request or in the context — replaces the environment credential instead of
  being sent beside it.

In every Rust CLI a `json`-style `--body` takes `@file` (read the file), `@-`
(read stdin) and `@@...` (the literal value with one `@` dropped, for a body
that starts with `@`) as well as an inline value, and sends the binding's
declared media type as the `Content-Type` when it is not `application/json`.
A `flatten` or `multipart` body is assembled from flags into JSON by this
backend, so it is always sent as `application/json`.

## Configuration (`sdk.json`)

One file drives `generate`, `publish` and `xsdk`. The top-level keys are
`version`, `api` (the OpenAPI document, relative to the file), `sdkName`,
`behavior`, `grouping` and `publish`; every other top-level object is a
language section (`typescript`, `python`, `go`, `ruby`, `java`, `csharp`,
`rust`, `go-cli`, `rust-cli`) carrying that emitter's options plus an optional
`output`, `behavior` (a per-language override) and `publish`. A per-language
`behavior` is layered over the top-level one and applied by running the
converter again for that language, so it may set any key — the ones the
converter decides on (`pagination`, `idempotency`, `headersFromExtension`)
included.

```jsonc
{
  "version": 1,
  "api": "openapi.yaml",
  "sdkName": "Widgetco",                       // -> the WIDGETCO_API_KEY variable every client reads
  "behavior": {
    "retry": { "maxRetries": 2 },
    "pagination": "auto",                      // "auto" (default) | "none": the list heuristic, see below
    "defaultHeaders": { "X-Some-Version": "1.0" },   // sent on every request, by every language
    "headersFromExtension": { "X-Release": "x-release" } // a header whose value is info.x-release (or a root x-*)
  },
  "grouping": {
    "mountRules": { "assistants": "beta/assistants" },
    "operationHints": {
      "GET /gadgets": { "pagination": false, "action": "search" }
    }
  },
  "typescript": { "packageName": "@acme/widgets", "output": "./sdk/ts" },
  "python":     { "packageName": "acme_widgets" },                // the IMPORT name, not the distribution name
  "go":         { "packageName": "widgets", "modulePath": "github.com/acme/widgets-go" },
  "ruby":       { "packageName": "acme-widgets", "moduleName": "AcmeWidgets" },
  "java":       { "packageName": "widgets", "basePackage": "com.acme" },
  "csharp":     { "namespace": "Acme.Widgets" }
}
```

**Pagination.** By default the converter treats a `GET … list` operation as
paginated when its 2xx object response has an array field named `data`,
`items` or `results`, and either a top-level marker (`has_more`, `next`,
`next_cursor`, `has_next` → cursor style, with the first `after`/`cursor`/
`starting_after`/`page` query param as the cursor), an integer
`offset`/`skip`/`start` query param (offset style), or — for `data`/`items` —
nothing at all (a single page). Every emitter then returns `Page<Item>` instead
of the declared response type, so sibling fields of the envelope (a nested
`list: { has_more, next_cursor, total }` block, counts, metadata) are not
reachable from the typed return. `operationHints["GET /x"].pagination: false`
keeps that one operation's declared type; `behavior.pagination: "none"` turns
the heuristic off for every operation (a hint of `true` re-enables it for
one), at the top level or in one language's `behavior`. The hint is honoured by
every emitter and by the docs samples.

**Default headers.** `behavior.defaultHeaders` is a static map; each entry of
`behavior.headersFromExtension` names a header and the `x-*` extension (read
from `info`, then the document root) that supplies its value — a present
extension wins over a static entry of the same name, an absent one changes
nothing. Every generated client sends them on every request, layered under the
per-call headers, and every client exposes an override (`defaultHeaders` on the
TypeScript/Python/C# client, `default_headers:` in Ruby, `defaultHeader(name,
value)` on the Java builder, `default_header(name, value)` on the Rust
builder; Go's `option.WithHeader` already wins over an SDK default). A
per-language `behavior` layer may add or cancel entries: `null` drops one,
whether it is static or comes from an extension. Generating from a pre-parsed
IR (`--spec ir.json`) still applies the config's `behavior`, but there is no
document to read `headersFromExtension` from — a header the IR does not already
carry is not sent, and `generate` says so.

**Docs samples (`xsdk`).** `opensdk xsdk --config sdk.json --spec openapi.yaml
--output openapi.sdk.yaml` embeds an `x-sdk` block per operation. The docs
pass receives the same identity and options as `generate` — `sdkName`, the
language sections, `grouping` — so a sample imports the configured package,
reads the configured environment variable (never a literal key) and calls the
hinted method name; a `pagination: false` operation shows its declared type.
`--sdk-name` and `--grouping` override the config the way they do for
`generate`. Programmatic callers of the docs capabilities (`docs_map`) follow
the same contract: convert with the converter bag (`sdkName`, `mountRules`,
`operationHints`, `sdkBehavior`), then call a language's docs with that
language's section as the options bag.

**Type references.** Each `x-sdk.<lang>.types` (and `typeReference` in the
`docs_map` form) has three parts. `request` and `response` are one level deep:
the field rows of the request params type and of the response type, each row
carrying `name`, `langType`, `required`, `description`, `deprecated` and — when
the field's type is, or wraps through arrays and maps, a named IR type —
`refTypeName` (the IR type name) and `schemaRef` (`#/components/schemas/<Name>`
when that IR type came from a component schema; absent for a type the converter
hoisted from an inline schema, such as a `list` block named `<Model>ListList`).
The two groups carry `typeName` (the language's name for the type),
`refTypeName` and `schemaRef` the same way. `nested` is the lookup table behind
both: every IR type reachable from the request — the body type and the
query/header parameter types — or from the response type (those roots
included), keyed by `refTypeName`, each entry `{ typeName, kind, schemaRef?,
description?, fields | values | variants | of }` with struct fields rendered as
the language's models declare them — so a page can expand `list` into
`has_more`, `next_cursor`, `total`, follow `data[]` to the item type and on to
its own nested types, and link a type to its schema page only when `schemaRef`
says one exists. The walk is breadth-first with a visited set (a
self-referencing type terminates) and stops `nestedDepth` hops from the
operation (default 8, set in a language's section of the options bag as a
non-negative integer or a string holding one; `0` leaves `nested` out; any
other value falls back to the default). The IR records the origin as
`types[].schemaRef`, which the IR schema declares on `NamedType`.

## Authentication

Every `components.securitySchemes` entry becomes an IR `security` entry whose
`kind` is what the generators branch on:

| OpenAPI scheme | `kind` |
|---|---|
| `http` with `scheme: bearer` | `bearer` |
| `oauth2` (any flow) | `bearer` |
| `http` with `scheme: basic` | `basic` |
| `apiKey` with `in: header` / `query` / `cookie` | `apiKey-header` / `apiKey-query` / `apiKey-cookie` |
| anything else | `other` |

A `bearer` client sends `Authorization: Bearer <token>`; an `apiKey-*` client
sends the key under the scheme's `name`. The credential is read from one
environment variable, `<SDKNAME>_API_KEY` by default. An SDK applies its
document's first scheme; a generated CLI applies each one it recognizes.

An `oauth2` scheme is a bearer scheme at runtime (RFC 6750: an OAuth 2.0 access
token is sent as `Authorization: Bearer <token>`). The generated client does
not run the flow; it sends the access token it is given, from the same variable
an `http`/`bearer` scheme uses, so the generated code is the same for both. The
IR entry carries `scheme: "bearer"` plus what the document declares about the
flows, for docs and tooling: `flows` (the flow types, in document order) and
`scopes` (the scope catalogue merged across the flows, the first description
of a scope winning). `opensdk diff` identifies the document's schemes by their
`kind`, `name`, `in` and `scheme`, so a document that moves from an
`http`/`bearer` scheme to an `oauth2` one is not a `security-changed` (an
operation-level `security` override is still compared as a whole).

MIT licensed.
