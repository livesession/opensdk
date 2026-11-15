# clap_opencli

Describe a [clap](https://docs.rs/clap) CLI as an
[OpenCLI](https://github.com/spectreconsole/open-cli) document: the commands,
positionals and options come from the clap tree, and what clap cannot know
(exit codes, examples, environment variables, an interactive session) is
declared next to it. Builder and derive CLIs work alike; there is no
proc-macro, and the crate leaves your clap features as they are.

```toml
[dev-dependencies]
clap_opencli = { git = "https://github.com/livesession/opensdk", tag = "v0.4.0" }
```

A test keeps a committed `opencli.json` current:

```rust
use clap::CommandFactory;
use clap_opencli::{EnvVar, Hidden, KeyBinding, Options};

#[test]
fn the_reference_document_is_current() {
    let doc = clap_opencli::from_command(&acme::Cli::command())
        .options(Options { hidden: Hidden::Omit }) // default: Hidden::Mark
        .exit_code(0, "Success")
        .exit_code(2, "Usage error")
        .example("acme ask 'hello'")
        .env(EnvVar::new("ACME_COLOR", "When to color output").values(["always", "never"]))
        .interactive(true)
        .interactive_commands(clap_opencli::interactive_from_command(&acme::Repl::command()).unwrap())
        .keys([KeyBinding::new(["ctrl+c"], "Interrupt, then quit")])
        .at(&["resume"], |c| c.example("acme resume --last")) // an unknown path is an error
        .extension("x-acme-channel", "stable") // x- keys only
        .build()
        .unwrap();
    clap_opencli::snapshot::assert_file("opencli.json", &doc); // OPENCLI_UPDATE=1 rewrites
}
```

`build()` returns an [`opencli_model::Document`](../opencli_model): the typed
model, with `x-*` keys kept, the generic extensions (`x-interactive`,
`x-environment`, `x-passthrough`) typed, and their JSON Schema exported for
your own validator.

## The mapping

Read after `Command::build`, compared with the unbuilt tree to find what clap
added. clap's `help` subcommand and every help or version argument (by action,
not by id) are skipped.

| clap | OpenCLI |
|---|---|
| bin name, else name; `version`; `about`; `long_about` when it differs | `info.title`, `info.version`, `info.summary`, `info.description` (each can be overridden) |
| a subcommand: name, visible aliases, `long_about` else `about`, `hide` | a command: `name`, `aliases`, `description`, `hidden` |
| positionals, in index order | `arguments`: the first value name lowercased (else the id), `required`, `arity` unless exactly one, the non-hidden possible values as `acceptedValues`, the help heading as `group`, `hidden` |
| options | `options`: the long name; `aliases` = the short plus the visible aliases; one argument per value name; flags (`SetTrue`, `SetFalse`, `Count`) take none and get no default. An option with only a short is an error and a one-character long alias is left out: an OpenCLI option is named by its long name and a one-character alias reads as a short |
| `global(true)` | `recursive: true` where declared; the copies clap spreads to subcommands are not written |
| `Append`, `Count` | metadata `repeatable` |
| `default_value(s)` unless hidden | metadata `default` |
| `env` (the `env` feature) | metadata `env`, and a root `x-environment` entry |
| `conflicts_with` (groups unrolled), `exclusive`, `last` | metadata `conflictsWith` (spellings), `exclusive`, `last` |
| `hide` | `hidden: true` (`Hidden::Mark`), or left out (`Hidden::Omit`) |

Not mapped: `requires` (clap has no getter for it), whether an argument group
is required, one-character long aliases, value hints, external subcommands.

## Versions

clap 4.5 and every later 4.x; CI holds a job at 4.5. The crate writes the
flat OpenCLI shape (the root command's fields on the document itself), with
`opencli: "1.0.0"`. `opencli_model` also reads the upstream root-`command`
shape.

## Features

- `env`: map `Arg::env`. It enables clap's own `env` feature, so it is off by
  default.
