//! Bin entry (`opensdk`) — the port of `src/cli.ts`.

fn main() {
    std::process::exit(opensdk_cli::command::main());
}
