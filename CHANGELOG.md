# Changelog

Releases are tagged `vX.Y.Z` on the `opensdk` binary (`cli/Cargo.toml`); the
library crates are not versioned independently. Earlier releases are described
on their GitHub release pages.

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
