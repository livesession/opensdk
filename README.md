# opensdk

The SDK toolchain behind [xyd](https://github.com/livesession/xyd): it turns an
OpenAPI 3.x document into typed, functional client SDKs for seven languages, and
into command-line interfaces.

```
OpenAPI 3.x ──► OpenSDK IR ──► go · node · python · ruby · java · dotnet · rust
            └─► OpenCLI    ──► go-cli · rust-cli
```

Rust throughout. The `opensdk` binary is the user-facing entry point; xyd
consumes the same crates through a napi addon.

## Layout

| path | what |
|---|---|
| `crates/xyd_openapi2opensdk` | OpenAPI → the OpenSDK IR |
| `crates/xyd_openapi2opencli` | OpenAPI → an OpenCLI doc (+ the `x-openapi` request binding) |
| `crates/xyd_opencli2{go,rust}` | OpenCLI → a buildable CLI project |
| `crates/xyd_opencli2opensdk` | OpenCLI → the OpenSDK IR |
| `crates/xyd_opensdk_{go,node,python,ruby,java,dotnet,rust}` | the seven emitters |
| `crates/xyd_opensdk_framework` | the emitter contract, orchestrator, and the regen-safe `write_project` lifecycle |
| `crates/xyd_opensdk_{core,config,diff,chain}` | IR + behavior, config shapes, the breaking-change classifier, the chain pipeline |
| `crates/xyd_opensdk_cli` | the `opensdk` binary |
| `crates/xyd_oas_doc` | `DocCtx` — the shared spec loader/dereferencer (xyd depends on this one too) |
| `crates/xyd_{opensdk_cli_common,opensdk_e2e,parity_kit}` | test harnesses |

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

## Relationship to xyd

xyd pins this repo as a submodule at `xyd/opensdk` and path-deps these crates
from its napi addon. The crates keep their `xyd_*` names deliberately — the
extraction preserved every git tree hash, and renaming would have thrown that
proof away.

MIT licensed.
