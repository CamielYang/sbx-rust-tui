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

## Create-sandbox wizard

Select **Create new sandbox (wizard)** (first menu item) and press `Enter`. It
walks through these settings one at a time:

1. **Name** — 2–63 ASCII letters, numbers, `-` or `.`; each dot-separated part
   must start and end with an alphanumeric character; `default` is reserved.
2. **Agent** — `claude`, `codex`, `copilot`, `cursor`, `devin`, `docker-agent`,
   `droid`, `kiro`, `opencode` (default), `shell`.
3. **Root size** — optional, e.g. `40g` → `DOCKER_SANDBOXES_ROOT_SIZE=40g`.
4. **Docker data size** — optional, e.g. `20g` → `DOCKER_SANDBOXES_DOCKER_SIZE=20g`.
5. **CPUs** — positive integer; `0` means auto (default: `2`).
6. **Memory** — binary units such as `512m` or `8g`; minimum 512 MiB, maximum
   host memory. Defaults to half of host memory (clamped to 512 MiB–32 GiB)
   and warns above 75% of host memory.
7. **Review** — shows the exact command, then `y`/`Enter` creates the sandbox.

In wizard text fields, `Ctrl+U` clears the line. `Esc` goes back one step (or
cancels on the first step); in the review step `n`/`Esc` returns to memory.

Example generated command:

```sh
DOCKER_SANDBOXES_ROOT_SIZE=40g DOCKER_SANDBOXES_DOCKER_SIZE=20g \
  sbx create --name demo-tui opencode --cpus 2 --memory 3584m
```

The two size variables are passed only to the child process via
`Command::env`, so the TUI's own environment is unchanged.

## Add your own commands

Edit the `MENU` constant in [`src/app.rs`](src/app.rs):

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
- [`src/app.rs`](src/app.rs) — menu, state, wizard, validation and execution
- [`src/ui.rs`](src/ui.rs) — ratatui rendering

## Notes / possible next steps

- Commands currently run synchronously; the UI is blocked while a command runs.
  Moving execution to a worker thread with a channel would fix that.
- Output is captured with `Command::output()`, so interactive commands that need
  a TTY are not supported yet.
- Command list is static. A config file (TOML) would make it editable without
  recompiling.
