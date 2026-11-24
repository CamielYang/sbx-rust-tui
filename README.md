# sbx-rust-tui

A small ratatui skeleton for launching frequently used Docker Sandboxes (`sbx`)
and Docker commands from a TUI instead of typing them each time.

## Run

```sh
cargo run
```

## Keys

| Key | Action |
| --- | --- |
| `↑` / `↓` or `k` / `j` | Move through the command list |
| `Enter` | Preview the selected command and ask for confirmation |
| `y` / `Enter` | Run the confirmed command |
| `n` / `Esc` | Cancel confirmation |
| `PgUp` / `PgDn` | Scroll the captured output |
| `c` | Clear the output pane |
| `q` / `Esc` / `Ctrl+C` | Quit (terminal is always restored) |

Commands are shown before they run, and stdout/stderr are captured and displayed
in the details pane. Any `{sandbox}` placeholder in a command's arguments is
replaced with the `SANDBOX_NAME` environment variable at launch time.

## Add your own commands

Edit the `COMMANDS` constant in [`src/app.rs`](src/app.rs):

```rust
CommandSpec {
    name: "My command",
    description: "What this command does.",
    program: "sbx",
    args: &["port", "publish", "{sandbox}", "8080:8080/tcp"],
},
```

## Layout

- [`src/main.rs`](src/main.rs) — terminal setup/restore, panic hook, event loop
- [`src/app.rs`](src/app.rs) — state, key handling, command execution
- [`src/ui.rs`](src/ui.rs) — ratatui rendering

## Notes / possible next steps

- Commands currently run synchronously; the UI is blocked while a command runs.
  Moving execution to a worker thread with a channel would fix that.
- Output is captured with `Command::output()`, so interactive commands that need
  a TTY are not supported yet.
- Command list is static. A config file (TOML) would make it editable without
  recompiling.
