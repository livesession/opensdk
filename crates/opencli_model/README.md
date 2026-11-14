# opencli_model

A typed model of [OpenCLI](https://github.com/spectreconsole/open-cli)
documents, with no CLI library behind it.

- **The model.** `Document` and the types under it model the flat OpenCLI
  draft shape: the root command's arguments, options, subcommands, exit
  codes, examples, `interactive` and metadata sit on the document itself.
  Every object keeps the keys it does not type in `extensions`, so `x-*` keys
  round-trip. Reading also accepts the upstream root-`command` shape and lifts
  it to flat (`lift_root_command`); writing is always flat, in one canonical
  key order. The `opencli` string written is `"1.0.0"`.
- **Three generic extensions**, typed:
  - `x-interactive: { commands: [Command], keys: [{ keys, description, context? }] }`:
    what a running interactive session accepts. The commands are typed inside
    the session, written without their prefix, and never argv commands, so a
    docs site makes no page for them.
  - `x-environment: [{ name, description, values? }]`: the environment
    variables a command reads.
  - `x-passthrough: { program? }`, on a command: the CLI hands every argument
    after the command, unparsed, to another program. The command's children,
    if any, document that program; code generators never read them.
- **Schemas.** `SCHEMA_JSON` is the base OpenCLI JSON Schema and
  `EXTENSIONS_SCHEMA_JSON` (`schema/opencli-extensions.schema.json`) the
  contract of the three extensions. Validate a document against both, with the
  validator of your choice.
- **`merge_fragment`** merges a hand-written OpenCLI fragment into a
  document: root `options`, `exitCodes`, `examples`, `interactive`,
  `info.description` and `x-*` keys replace; `commands` append (a command
  named like an existing top-level one updates its description, aliases and
  hidden flag). `x-openapi` stays the converter's.
- **`mount`** places a component CLI's document at one command of a host
  CLI's document: a passthrough stub (`x-passthrough` with no arguments,
  options or subcommands) is filled, or a new command is appended. The host's
  recursive options in scope must not collide with any option of the
  component (drop the component's with `MountOptions::drop_options`); the
  component's `info.version` is recorded as the metadata `componentVersion`.
  The rules are on the function's docs.

```rust
use opencli_model::{mount, Command, Document, MountOptions, XPassthrough};

let mut host = Document::new("acme", "2.0.0");
let mut agent = Command::new("agent");
agent.x_passthrough = Some(XPassthrough::program("acme-agent"));
host.commands.push(agent);

let mut component = Document::new("acme-agent", "0.4.0");
component.commands.push(Command::new("resume"));

let report = mount(&mut host, "agent", &component, &MountOptions::default()).unwrap();
assert_eq!(report.to_string(), "mounted agent 0.4.0");
assert_eq!(host.command(&["agent", "resume"]).unwrap().name, "resume");
```

[`clap_opencli`](../clap_opencli) builds a `Document` from a clap CLI.
