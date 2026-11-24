# sbx-rust-tui

A ratatui dashboard for Docker Sandboxes (`sbx`): browse active sandboxes and
create new ones without leaving the terminal.

## Run

```sh
cargo run
```

## Home screen

The home screen has a centered **Docker Sandbox Builder** title, a grid of blocks
(three columns per row) for the active sandboxes, and a trailing
**+ New Sandbox** button. Each sandbox block shows a left-aligned name with a
green dot when the sandbox is running, or a red dot when it is stopped/unknown.
Below the name are four buttons in one row: **Stop**, **Exec**, **Remove**, and
**Info**. Clicking the rest of a block selects it; clicking **+ New Sandbox**
opens the create wizard as a modal over the dashboard.

Sandbox names and statuses come from `sbx ls --format json` at startup and are
refreshed after a sandbox is created, stopped, or removed (or when `r` is
pressed). Older `sbx` versions without structured output fall back to
`sbx ls -q` and show status `unknown`. If `sbx` is missing or fails, the grid is
empty and the error is shown in the footer.

### Sandbox buttons

| Button | Command | Behaviour |
| --- | --- | --- |
| Stop | `sbx stop {name}` | Stops the sandbox, then refreshes the grid. |
| Exec | `sbx exec -it {name} bash` | Leaves the TUI, opens an external terminal, runs the shell there, and restores the TUI when that terminal closes. On Linux it tries common terminal emulators in order (`x-terminal-emulator`, `gnome-terminal`, `konsole`, `xfce4-terminal`, `mate-terminal`, `kitty`, `alacritty`, `xterm`); on macOS it uses Terminal.app; on Windows a new `cmd` window. If no terminal can be opened, the footer shows the command to run manually. |
| Remove | `sbx remove {name} --force` | Removes the sandbox, then refreshes the grid. `--force` is added because the TUI has no TTY for `sbx`'s confirmation prompt; removal is immediate and cannot be undone. |
| Info | `sbx inspect {name} --format json` | Opens a modal with name, status, and resource details (CPU, memory, root size, Docker size). Values not reported by `sbx` appear as `n/a`; inspection errors appear as a note in the modal. |

The Info modal closes with `Esc`, `q`, `i`, `Enter`, or its **[X]** button.

## Windows build (cross-compile from Linux)

```sh
rustup target add x86_64-pc-windows-gnu
# Debian/Ubuntu: linker for the windows-gnu target
sudo apt-get install gcc-mingw-w64-x86-64
cargo build --release --target x86_64-pc-windows-gnu
```

The executable is written to
`target/x86_64-pc-windows-gnu/release/sbx-rust-tui.exe`. It is a 64-bit Windows
console program; copy it to a Windows machine and run it from a terminal.

Two platform notes:

- Creating a sandbox calls `sbx`, so `sbx` must be available on the Windows
  machine in `PATH`.
- The wizard reads `/proc/meminfo` for its default memory limit and warnings.
  That file does not exist on Windows, so it falls back to `4g` and skips the
  host-memory checks there.

## Keys

| Key | Action |
| --- | --- |
| `←` / `→` / `↑` / `↓` or `h` / `l` / `k` / `j` | Move between blocks |
| `Home` / `End` | First / last block |
| `PgUp` / `PgDn` | Scroll the grid |
| `Enter` (on **+ New Sandbox**) | Open the create wizard modal |
| `s` / `x` / `d` / `i` | Stop / Exec / Remove / Inspect the selected sandbox |
| `r` | Refresh the sandbox list |
| `q` / `Esc` / `Ctrl+C` | Quit (terminal is always restored) |

Inside the sandbox info modal:

| Key | Action |
| --- | --- |
| `Esc` / `q` / `i` / `Enter` | Close the modal (or click **[X]**) |

Inside the wizard modal:

| Key | Action |
| --- | --- |
| Type characters | Edit the current field |
| `↑` / `↓` or `k` / `j` | Choose an agent |
| `Ctrl+U` | Clear the current field |
| `Enter` | Next step |
| `Esc` | Previous step (or cancel on the first) |
| `y` / `Enter` | Create the sandbox on the review step |
| `n` / `Esc` | Return to the memory step from review |

## Mouse

Mouse capture is enabled on startup and disabled when the app exits (including on
panic). Click a sandbox block to select it, click one of its **Stop**, **Exec**,
**Remove**, or **Info** buttons to act on it, click **+ New Sandbox** to open the
wizard, and use the scroll wheel to move between grid rows. Exec temporarily
disables mouse capture so the external terminal behaves normally.

The wizard modal has an **[X]** close button in its top-right corner and a
**[ Create Sandbox ]** button at the bottom. The Create button is disabled
(dimmed, not clickable) until the final **Review** step, so all fields have been
reviewed before a sandbox can be created.

## New sandbox wizard modal

Open the modal by selecting **+ New Sandbox** and pressing `Enter`. It walks
through these settings one at a time:

1. **Name** — required, 2–63 ASCII letters, numbers, `-` or `.`; each
   dot-separated part must start and end with an alphanumeric character;
   `default` is reserved.
2. **Agent** — all options are shown in a two-column list with the current one
   marked: `claude`, `codex`, `copilot`, `cursor`, `devin`, `docker-agent`,
   `droid`, `kiro`, `opencode` (default), `shell`. Move with `↑`/`↓` or `j`/`k`.
3. **Root size** — optional, in GB: type just a number, e.g. `40` →
   `DOCKER_SANDBOXES_ROOT_SIZE=40g`.
4. **Docker data size** — optional, in GB: type just a number, e.g. `20` →
   `DOCKER_SANDBOXES_DOCKER_SIZE=20g`.
5. **CPUs** — positive integer; `0` means auto (default: `2`).
6. **Memory** — binary units such as `512m` or `8g`; minimum 512 MiB, maximum
   host memory. Defaults to half of host memory (clamped to 512 MiB–32 GiB)
   and warns above 75% of host memory.
7. **Review** — enable the **Create Sandbox** button; `y`/`Enter` or a click
   creates the sandbox. No command is printed on this step.

Example generated command:

```sh
DOCKER_SANDBOXES_ROOT_SIZE=40g DOCKER_SANDBOXES_DOCKER_SIZE=20g \
  sbx create --name demo-tui opencode --cpus 2 --memory 3584m
```

The two size variables are passed only to the child process via
`Command::env`, so the TUI's own environment is unchanged.

## Code layout

- [`src/main.rs`](src/main.rs) — terminal setup/restore, panic hook, event loop
- [`src/app.rs`](src/app.rs) — sandbox list, state, wizard, validation and execution
- [`src/ui.rs`](src/ui.rs) — dashboard grid and modal rendering

## Notes / possible next steps

- Sandbox creation, stopping, removal, and inspection run synchronously; the UI
  is blocked while those `sbx` commands run. Moving execution to a worker thread
  with a channel would fix that.
- Resource values in the info modal are read from `sbx inspect --format json`.
  Which fields are present depends on the sandbox type and `sbx` version, so
  missing values are shown as `n/a`.
