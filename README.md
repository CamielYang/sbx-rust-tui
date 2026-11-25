# sbx-rust-tui

A themed terminal dashboard for Docker Sandboxes (`sbx`), built with
[ratcn](https://github.com/kristoferlund/ratcn) on top of Ratatui. Browse and
manage sandboxes, create new ones, and inspect live resource usage without
leaving the terminal.

## Run

```sh
cargo run
```

## Dashboard

The screen uses ratcn components and its Tokyo Night theme:

- a header with the sandbox count,
- a toolbar with **+ New sandbox**, **Refresh**, and **Quit** buttons,
- a scrollable sandbox list with running/stopped indicators,
- an **Actions** pane whose colored buttons follow the sandbox you click or
  commit with Enter: **Start** (green) and **Remove** (red) for a stopped
  sandbox, **Stop** (orange), **Exec shell** (gray), **Publish ports** (cyan),
  **Info** (blue), and **Remove** (red) for a running one. Every button changes
  shade on focus and hover, and actions that do not apply are hidden rather
  than disabled,
- a confirmation dialog for **Start**, **Stop**, and **Remove**, so none of
  those commands runs until you explicitly confirm it,
- a **Published ports** dialog for running sandboxes, with a table of published
  mappings and an **Add port** button at the top,
- a footer with the latest message and keyboard shortcuts,
- transient toast notifications in the top-right corner.

Focus, hover, hit-testing, and modal layering are handled by ratcn's runtime;
`App` owns all state and applies the messages components emit.

Sandbox names and statuses come from `sbx ls --format json` at startup and are
refreshed after create/start/stop/remove, when `R` is pressed, and every five
seconds. Older `sbx` versions without structured output fall back to `sbx ls -q`
and show status `unknown`. If `sbx` is missing or fails, the last known list is
kept and the error appears in the footer and as a toast. While the Info dialog
is open, its live metrics are re-collected every ten seconds on a worker thread.
The ports dialog reads `sbx port ls <name> --format json` when it opens and
again after a successful publish; a read failure is shown inside the dialog.

### Actions

| Button | Command | Behaviour |
| --- | --- | --- |
| Start / Stop | `sbx run -d --name {name}` / `sbx stop {name}` | Opens a confirmation dialog first. `sbx` has no standalone `start` verb, so a stopped sandbox is started by re-attaching with `run --name`; `-d` leaves it running detached. |
| Exec shell | `sbx exec -it {name} bash` | Suspends the TUI, opens an external terminal, and restores the TUI when it closes. On Linux it tries common terminal emulators in order; on macOS it uses Terminal.app; on Windows a new `cmd` window. If no terminal opens, the command is shown in the footer. |
| Publish ports | `sbx port ls {name} --format json`, then `sbx port publish {name} {host}:{sandbox}` | Opens the published-ports dialog for a running sandbox; **Add port** validates the two fields and publishes the mapping, then refreshes the table. |
| Info | `sbx inspect {name} --format json` then `sbx exec {name} -- sh -c …` | Opens a themed dialog with status, CPU as `{cores} ({usage}%)`, and memory/root/Docker usage as `used / total`. Guest metrics are collected when Info opens and every ten seconds while it stays open; `sbx inspect` supplies fallback values. Missing values render as `—`; errors appear as a note. |
| Remove | `sbx remove {name} --force` | Opens a confirmation dialog first; `--force` avoids `sbx`'s own prompt, which the TUI cannot answer. |

Start, Stop, and Remove all ask **Confirm start / stop / remove** before doing
anything. **Cancel** or `Esc` dismisses the dialog and leaves the sandbox
untouched; confirming while the sandbox's status has changed in the meantime is
also cancelled. Info is only available for running sandboxes. It closes with
`Esc` or the **Close** button. Clicking a sandbox row (or pressing
Enter on it) focuses that sandbox for the Actions pane; hovering merely moves
the list cursor.

## Keys

| Key | Action |
| --- | --- |
| `Tab` / `Shift+Tab` | Move focus through toolbar buttons, the sandbox list, and action buttons |
| `↑` / `↓` / `j` / `k` | Move the sandbox list cursor |
| `PgUp` / `PgDn` / wheel | Scroll the sandbox list |
| `Enter` / `Space` | Activate the focused button, or focus the cursor row for the Actions pane |
| `N` | Open the create dialog |
| `R` | Refresh the sandbox list |
| `S` | Ask to start a stopped sandbox, or stop a running one |
| `X` / `P` / `I` / `D` | Exec / publish ports / Inspect / ask to remove the selected sandbox (`X`, `P`, and `I` need a running sandbox) |
| `q` / `Esc` / `Ctrl+C` | Quit (terminal is always restored) |

Global shortcuts apply only on the dashboard; while a dialog is open, keys are
handled by its controls. In the confirmation dialog, `Esc` cancels the pending
action. `Ctrl+C` remains available everywhere, including while a sandbox is
being created.

## Create dialog

Open it with **+ New sandbox** or `N`. All settings are visible and editable at
once using ratcn inputs and an agent select:

1. **Name** — required, 2–63 ASCII letters, numbers, `-` or `.`; each
   dot-separated part must start and end with an alphanumeric character;
   `default` is reserved.
2. **Agent** — `claude`, `codex`, `copilot`, `cursor`, `devin`, `docker-agent`,
   `droid`, `kiro`, `opencode` (default), `shell`. Click the field or press
   `Enter` to open the option popup.
3. **Root size (GB)** — optional number, e.g. `40` →
   `DOCKER_SANDBOXES_ROOT_SIZE=40g`.
4. **Docker data (GB)** — optional number, e.g. `20` →
   `DOCKER_SANDBOXES_DOCKER_SIZE=20g`.
5. **CPUs** — positive integer; `0` means auto (default: `2`).
6. **Memory** — binary units such as `512m` or `8g`; minimum 512 MiB, maximum
   host memory. Defaults to half of host memory (clamped to 512 MiB–32 GiB)
   and warns above 75% of host memory.

**Create sandbox** validates every field. The generated command is:

```sh
DOCKER_SANDBOXES_ROOT_SIZE=40g DOCKER_SANDBOXES_DOCKER_SIZE=20g \
  sbx create --name demo-tui opencode --cpus 2 --memory 3584m
```

Size variables are passed only to the child process via `Command::env`, so the
TUI's own environment is unchanged.

Creation runs on a worker thread. While it is in flight, a persistent loading
toast and themed progress bar animate, all inputs and buttons are disabled, and
repeated keys cannot start a second sandbox. `Ctrl+C` still quits. On success
the dialog closes, the list refreshes, and a success toast appears; on failure
the dialog stays open with the invalid field marked and the error in the footer.

## Published ports dialog

Open it with **Publish ports** or `P` on a running sandbox. The table lists the
mappings returned by `sbx port ls {name} --format json`, with columns for the
host port, sandbox port, and protocol. An **Add port** button sits at the top of
the dialog, next to two inputs:

1. **Host port** — the port on the host, default `8080`.
2. **Sandbox port** — the port inside the sandbox, default `3000`.

Pressing **Add port** validates both as numbers from 1 to 65535 and runs
`sbx port publish {name} {host}:{sandbox}`. With the defaults unchanged, that is
`sbx port publish {name} 8080:3000`. On success the table is re-read and a toast
confirms the mapping; on failure the dialog stays open and shows the error in
its status line and in the footer. The dialog closes with `Esc` or **Close**.
More ports than fit are counted in the status line.

## Mouse

Mouse input is enabled on startup and disabled on exit (including on panic).
Click toolbar or action buttons, click a sandbox row to focus it for the Actions
pane, use the wheel to scroll, and click dialog fields/buttons. Hovering a row
only moves the cursor, so the actions keep targeting the last clicked sandbox.
The agent select opens a popup that ratcn keeps anchored to its field. Exec
temporarily suspends the TUI so the external terminal behaves normally.

## Windows build (cross-compile from Linux)

```sh
rustup target add x86_64-pc-windows-gnu
# Debian/Ubuntu: linker for the windows-gnu target
sudo apt-get install gcc-mingw-w64-x86-64
cargo build --release --target x86_64-pc-windows-gnu
```

The executable is written to
`target/x86_64-pc-windows-gnu/release/sbx-rust-tui.exe`. `sbx` must be on
`PATH`. Host memory detection reads `/proc/meminfo`, so on Windows the memory
default is `4g` and host-memory warnings are skipped.

## Code layout

- [`src/main.rs`](src/main.rs) — terminal lifecycle, ratcn runtime, event loop
- [`src/app.rs`](src/app.rs) — sandbox data, form/validation, `sbx` commands, state
- [`src/ui.rs`](src/ui.rs) — ratcn declarations, message mapping, and rendering

## Notes

- Start, Stop, and Remove run only after confirmation, and those `sbx` commands
  still run synchronously, so the UI is briefly blocked while they execute.
  `sbx port ls` and `sbx port publish` are also synchronous but short.
- Live resource values come from `nproc`, two `/proc/stat` samples one second
  apart, `/proc/meminfo`, and `df -k` inside the microVM via
  `sbx exec {name} -- sh -c …`. CPU usage is the percentage of all cores busy
  over that one-second sample. Used space is `total - available`
  (`MemAvailable` when present).
- The five-second ticker only re-reads `sbx ls`. The ten-second Info ticker
  re-collects guest metrics on a worker thread; results for a dialog that was
  closed in the meantime are discarded.
