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
operation (default 8, set in a language's section of the options bag; `0`
leaves `nested` out). The IR records the origin as `types[].schemaRef`.

MIT licensed.
