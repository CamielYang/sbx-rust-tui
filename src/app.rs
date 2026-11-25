use std::io;
use std::process::Command as ProcessCommand;
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use serde_json::Value;

pub const AGENTS: &[&str] = &[
    "claude",
    "codex",
    "copilot",
    "cursor",
    "devin",
    "docker-agent",
    "droid",
    "kiro",
    "opencode",
    "shell",
];

pub const GRID_COLUMNS: usize = 3;

/// How often the dashboard re-reads `sbx ls` to keep running-state dots fresh.
/// VM metrics are deliberately not part of this tick; they are only collected
/// when the user opens the Info modal.
pub const STATUS_REFRESH_INTERVAL: Duration = Duration::from_secs(5);

/// Collected with `sbx exec <name> -- sh -c <script>`. Plain text keeps the
/// command portable across the minimal shells used inside the microVM.
const VM_METRICS_SCRIPT: &str = r#"
printf 'cpu=%s\n' "$(nproc 2>/dev/null || printf 0)"
awk '
  /^MemTotal:/     { total = $2 }
  /^MemFree:/      { free = $2 }
  /^MemAvailable:/ { avail = $2 }
  END {
    if (total != "") print "mem_total_kb=" total
    if (avail != "") print "mem_free_kb=" avail
    else if (free != "") print "mem_free_kb=" free
  }
' /proc/meminfo
df -k
"#;

/// An action bound to a rectangle on screen by the renderer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClickAction {
    Select(usize),
    OpenWizard,
    CloseWizard,
    CreateSandbox,
    Start(usize),
    Stop(usize),
    Exec(usize),
    Remove(usize),
    Info(usize),
    CloseInfo,
}

/// A clickable rectangle. The renderer refreshes these on every frame so the
/// coordinates always match what the user currently sees.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClickTarget {
    pub area: Rect,
    pub action: ClickAction,
}

impl ClickTarget {
    pub fn new(area: Rect, action: ClickAction) -> Self {
        Self { area, action }
    }

    fn contains(&self, x: u16, y: u16) -> bool {
        self.area.contains((x, y).into())
    }
}

/// A ready-to-run command, including any environment overrides for the child.
#[derive(Clone)]
pub struct PendingCommand {
    program: String,
    args: Vec<String>,
    env: Vec<(String, String)>,
}

impl PendingCommand {
    pub fn new(program: &str, args: &[&str]) -> Self {
        Self {
            program: program.to_owned(),
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
            env: Vec::new(),
        }
    }

    /// The exact shell spelling of this command, used for the external
    /// terminal and for tests.
    pub fn preview(&self) -> String {
        let mut parts = self
            .env
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>();
        parts.push(self.program.clone());
        parts.extend(self.args.iter().cloned());
        parts
            .into_iter()
            .map(|part| quote_if_needed(&part))
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn run(&self) -> io::Result<std::process::Output> {
        let mut command = ProcessCommand::new(&self.program);
        command.args(&self.args);
        for (key, value) in &self.env {
            command.env(key, value);
        }
        command.output()
    }

    /// Open a new terminal window and run this command there. The TUI must be
    /// suspended (raw mode / alternate screen disabled) by the caller first.
    pub fn run_in_external_terminal(&self) -> Result<(), String> {
        let command = self.preview();

        #[cfg(windows)]
        {
            ProcessCommand::new("cmd")
                .args(["/C", "start", "", "cmd", "/K", &command])
                .status()
                .map(|_| ())
                .map_err(|error| error.to_string())
        }

        #[cfg(target_os = "macos")]
        {
            let script = format!(
                "tell application \"Terminal\" to do script \"{}\"",
                command.replace('\\', "\\\\").replace('"', "\\\"")
            );
            ProcessCommand::new("osascript")
                .args([
                    "-e",
                    &script,
                    "-e",
                    "tell application \"Terminal\" to activate",
                ])
                .status()
                .map(|_| ())
                .map_err(|error| error.to_string())
        }

        #[cfg(not(any(windows, target_os = "macos")))]
        {
            // Keep the shell open after the command exits so its output stays
            // readable in the new window.
            let inner = format!("{command}; exec bash");
            let emulators: &[&[&str]] = &[
                &["x-terminal-emulator", "-e"],
                &["gnome-terminal", "--"],
                &["konsole", "-e"],
                &["xfce4-terminal", "-e"],
                &["mate-terminal", "-e"],
                &["kitty"],
                &["alacritty", "-e"],
                &["xterm", "-e"],
            ];
            for prefix in emulators {
                let mut process = ProcessCommand::new(prefix[0]);
                process.args(&prefix[1..]).args(["sh", "-c", &inner]);
                match process.status() {
                    Ok(_) => return Ok(()),
                    Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                    Err(error) => return Err(error.to_string()),
                }
            }
            Err(format!(
                "no terminal emulator found; run '{command}' in a terminal yourself"
            ))
        }
    }
}

fn quote_if_needed(part: &str) -> String {
    if part.contains(char::is_whitespace) {
        format!("'{part}'")
    } else {
        part.to_owned()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Wizard,
    Info,
}

// --- wizard -----------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WizardStep {
    Name,
    Agent,
    RootSize,
    DockerSize,
    Cpus,
    Memory,
    Confirm,
}

pub const WIZARD_STEPS: &[WizardStep] = &[
    WizardStep::Name,
    WizardStep::Agent,
    WizardStep::RootSize,
    WizardStep::DockerSize,
    WizardStep::Cpus,
    WizardStep::Memory,
    WizardStep::Confirm,
];

impl WizardStep {
    pub fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Agent => "Agent",
            Self::RootSize => "Root size",
            Self::DockerSize => "Docker data size",
            Self::Cpus => "CPUs",
            Self::Memory => "Memory",
            Self::Confirm => "Review",
        }
    }

    pub fn prompt(self) -> &'static str {
        match self {
            Self::Name => "Sandbox name (required, 2-63 ASCII letters, numbers, '-' or '.')",
            Self::Agent => "Pick an agent",
            Self::RootSize => "Root filesystem size in GB (number only), e.g. 40 (empty skips)",
            Self::DockerSize => "Docker data size in GB (number only), e.g. 20 (empty skips)",
            Self::Cpus => "Number of CPUs (positive integer, 0 = auto)",
            Self::Memory => "Memory limit in binary units, e.g. 512m or 8g",
            Self::Confirm => "Review your settings, then create the sandbox",
        }
    }

    pub fn is_text(self) -> bool {
        !matches!(self, Self::Agent | Self::Confirm)
    }

    fn next(self) -> Option<Self> {
        let index = WIZARD_STEPS.iter().position(|step| *step == self)?;
        WIZARD_STEPS.get(index + 1).copied()
    }

    fn previous(self) -> Option<Self> {
        let index = WIZARD_STEPS.iter().position(|step| *step == self)?;
        index
            .checked_sub(1)
            .and_then(|i| WIZARD_STEPS.get(i))
            .copied()
    }
}

enum WizardOutcome {
    Continue,
    Cancelled,
    Create(PendingCommand),
}

pub struct Wizard {
    step: WizardStep,
    name: String,
    agent_index: usize,
    root_size: String,
    docker_size: String,
    cpus: String,
    memory: String,
}

impl Wizard {
    fn new() -> Self {
        Self {
            step: WizardStep::Name,
            name: String::new(),
            agent_index: AGENTS
                .iter()
                .position(|agent| *agent == "opencode")
                .unwrap_or(0),
            root_size: String::new(),
            docker_size: String::new(),
            cpus: "2".to_owned(),
            memory: default_memory(),
        }
    }

    pub fn step(&self) -> WizardStep {
        self.step
    }

    /// True once every field has been reviewed and the sandbox can be created.
    pub fn is_ready(&self) -> bool {
        self.step == WizardStep::Confirm
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn agent_index(&self) -> usize {
        self.agent_index
    }

    pub fn field_error(&self) -> Option<String> {
        // Don't show an error on the untouched first field.
        if self.step == WizardStep::Name && self.name.is_empty() {
            return None;
        }
        self.error_for(self.step)
    }

    fn error_for(&self, step: WizardStep) -> Option<String> {
        match step {
            WizardStep::Name => validate_name(&self.name).err(),
            WizardStep::RootSize => validate_gb_size(&self.root_size).err(),
            WizardStep::DockerSize => validate_gb_size(&self.docker_size).err(),
            WizardStep::Cpus => validate_cpus(&self.cpus).err(),
            WizardStep::Memory => validate_memory(&self.memory).err(),
            _ => None,
        }
    }

    /// Validate every field, not just the current one. This powers the
    /// clickable Create button, which can be used from any wizard step.
    pub fn first_error(&self) -> Option<(WizardStep, String)> {
        WIZARD_STEPS
            .iter()
            .find_map(|step| self.error_for(*step).map(|error| (*step, error)))
    }

    pub fn go_to_step(&mut self, step: WizardStep) {
        self.step = step;
    }

    pub fn finish(&self) -> Result<PendingCommand, (WizardStep, String)> {
        match self.first_error() {
            Some(error) => Err(error),
            None => Ok(self.pending_command()),
        }
    }

    pub fn value_for(&self, step: WizardStep) -> String {
        match step {
            WizardStep::Name => self.name.clone(),
            WizardStep::Agent => AGENTS[self.agent_index].to_owned(),
            WizardStep::RootSize => self.root_size.clone(),
            WizardStep::DockerSize => self.docker_size.clone(),
            WizardStep::Cpus => self.cpus.clone(),
            WizardStep::Memory => self.memory.clone(),
            WizardStep::Confirm => "ready".to_owned(),
        }
    }

    pub fn warning(&self) -> Option<String> {
        let mib = parse_memory_mib(&self.memory)?;
        let total = host_memory_mib()?;
        if mib > total * 3 / 4 {
            Some(format!(
                "Warning: {mib} MiB is above 75% of host memory ({total} MiB)."
            ))
        } else {
            None
        }
    }

    #[cfg(test)]
    pub fn preview(&self) -> String {
        self.pending_command().preview()
    }

    fn pending_command(&self) -> PendingCommand {
        let mut args = vec![
            "create".to_owned(),
            "--name".to_owned(),
            self.name.trim().to_owned(),
            AGENTS[self.agent_index].to_owned(),
        ];
        if !self.cpus.trim().is_empty() {
            args.push("--cpus".to_owned());
            args.push(self.cpus.trim().to_owned());
        }
        if !self.memory.trim().is_empty() {
            args.push("--memory".to_owned());
            args.push(self.memory.trim().to_owned());
        }

        let mut env = Vec::new();
        if !self.root_size.trim().is_empty() {
            env.push((
                "DOCKER_SANDBOXES_ROOT_SIZE".to_owned(),
                format!("{}g", self.root_size.trim()),
            ));
        }
        if !self.docker_size.trim().is_empty() {
            env.push((
                "DOCKER_SANDBOXES_DOCKER_SIZE".to_owned(),
                format!("{}g", self.docker_size.trim()),
            ));
        }

        PendingCommand {
            program: "sbx".to_owned(),
            args,
            env,
        }
    }

    fn current_field_mut(&mut self) -> Option<&mut String> {
        match self.step {
            WizardStep::Name => Some(&mut self.name),
            WizardStep::RootSize => Some(&mut self.root_size),
            WizardStep::DockerSize => Some(&mut self.docker_size),
            WizardStep::Cpus => Some(&mut self.cpus),
            WizardStep::Memory => Some(&mut self.memory),
            WizardStep::Agent | WizardStep::Confirm => None,
        }
    }

    fn handle_key(&mut self, key: KeyEvent, message: &mut String) -> WizardOutcome {
        match self.step {
            WizardStep::Confirm => match key.code {
                KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
                    WizardOutcome::Create(self.pending_command())
                }
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                    self.step = WizardStep::Memory;
                    WizardOutcome::Continue
                }
                _ => WizardOutcome::Continue,
            },
            WizardStep::Agent => match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    if self.agent_index > 0 {
                        self.agent_index -= 1;
                    }
                    WizardOutcome::Continue
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if self.agent_index + 1 < AGENTS.len() {
                        self.agent_index += 1;
                    }
                    WizardOutcome::Continue
                }
                KeyCode::Enter => self.advance(message),
                KeyCode::Esc => self.go_back(),
                _ => WizardOutcome::Continue,
            },
            step if step.is_text() => {
                if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('u') {
                    if let Some(field) = self.current_field_mut() {
                        field.clear();
                    }
                    return WizardOutcome::Continue;
                }
                match key.code {
                    KeyCode::Char(character) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                        if let Some(field) = self.current_field_mut() {
                            field.push(character);
                        }
                        WizardOutcome::Continue
                    }
                    KeyCode::Backspace => {
                        if let Some(field) = self.current_field_mut() {
                            field.pop();
                        }
                        WizardOutcome::Continue
                    }
                    KeyCode::Enter => self.advance(message),
                    KeyCode::Esc => self.go_back(),
                    _ => WizardOutcome::Continue,
                }
            }
            _ => WizardOutcome::Continue,
        }
    }

    fn advance(&mut self, message: &mut String) -> WizardOutcome {
        if let Some(error) = self.field_error() {
            *message = error;
            return WizardOutcome::Continue;
        }
        message.clear();
        match self.step.next() {
            Some(next) => {
                self.step = next;
                WizardOutcome::Continue
            }
            None => WizardOutcome::Create(self.pending_command()),
        }
    }

    fn go_back(&mut self) -> WizardOutcome {
        match self.step.previous() {
            Some(previous) => {
                self.step = previous;
                WizardOutcome::Continue
            }
            None => WizardOutcome::Cancelled,
        }
    }
}

fn validate_name(name: &str) -> Result<(), String> {
    if !(2..=63).contains(&name.len()) {
        return Err("Name must be 2-63 characters.".to_owned());
    }
    if name == "default" {
        return Err("'default' is a reserved name.".to_owned());
    }
    if !name
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'.')
    {
        return Err("Use ASCII letters, numbers, '-' or '.'.".to_owned());
    }
    for part in name.split('.') {
        if !part
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
            || !part
                .bytes()
                .last()
                .is_some_and(|byte| byte.is_ascii_alphanumeric())
        {
            return Err(
                "Each dot-separated part must start and end with a letter or number.".to_owned(),
            );
        }
    }
    Ok(())
}

/// Sizes are always gigabytes, so the user only types the number.
fn validate_gb_size(value: &str) -> Result<(), String> {
    if value.is_empty() || value.bytes().all(|byte| byte.is_ascii_digit()) {
        Ok(())
    } else {
        Err("Enter a whole number of GB, e.g. 40.".to_owned())
    }
}

fn validate_cpus(value: &str) -> Result<(), String> {
    value
        .parse::<u32>()
        .map(|_| ())
        .map_err(|_| "CPUs must be a whole number (0 = auto).".to_owned())
}

fn parse_memory_mib(value: &str) -> Option<u64> {
    let value = value.trim().to_ascii_lowercase();
    let split = value.find(|character: char| !character.is_ascii_digit())?;
    let (number, unit) = value.split_at(split);
    let number = number.parse::<u64>().ok()?;
    let multiplier = match unit {
        "m" | "mb" | "mi" | "mib" => 1,
        "g" | "gb" | "gi" | "gib" => 1024,
        "t" | "tb" | "ti" | "tib" => 1024 * 1024,
        _ => return None,
    };
    number.checked_mul(multiplier)
}

fn validate_memory(value: &str) -> Result<(), String> {
    let Some(mib) = parse_memory_mib(value) else {
        return Err("Use binary units like 512m or 8g.".to_owned());
    };
    if mib < 512 {
        return Err("Minimum memory is 512 MiB.".to_owned());
    }
    if let Some(total) = host_memory_mib()
        && mib > total
    {
        return Err(format!("Maximum memory is host memory ({total} MiB)."));
    }
    Ok(())
}

fn host_memory_mib() -> Option<u64> {
    let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
    let line = meminfo.lines().find(|line| line.starts_with("MemTotal:"))?;
    let kib = line.split_whitespace().nth(1)?.parse::<u64>().ok()?;
    Some(kib / 1024)
}

fn default_memory() -> String {
    match host_memory_mib() {
        Some(total) => {
            let half = total / 2;
            let rounded = (half / 512) * 512;
            format!("{}m", rounded.clamp(512, 32 * 1024))
        }
        None => "4g".to_owned(),
    }
}

// --- sandbox data -----------------------------------------------------------

/// A sandbox as shown in a dashboard block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sandbox {
    pub name: String,
    pub status: String,
}

impl Sandbox {
    pub fn new(name: String, status: String) -> Self {
        Self { name, status }
    }

    pub fn is_running(&self) -> bool {
        self.status.trim().to_ascii_lowercase().contains("running")
    }
}

/// Resource details shown in the info modal. Values come from live commands
/// run inside the microVM; the structured `sbx inspect` output is only a
/// fallback for fields the guest cannot report.
#[derive(Clone, Debug, Default)]
pub struct SandboxDetails {
    pub name: String,
    pub status: String,
    pub cpus: Option<String>,
    pub mem_used: Option<String>,
    pub mem_total: Option<String>,
    pub root_used: Option<String>,
    pub root_total: Option<String>,
    pub docker_used: Option<String>,
    pub docker_total: Option<String>,
    pub error: Option<String>,
}

/// Raw KiB values read from the guest. Used space is derived as
/// total - available so every resource fits on one `used / total` row.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct VmMetrics {
    cpus: Option<String>,
    mem_total_kib: Option<u64>,
    mem_free_kib: Option<u64>,
    root_total_kib: Option<u64>,
    root_free_kib: Option<u64>,
    docker_total_kib: Option<u64>,
    docker_free_kib: Option<u64>,
}

impl VmMetrics {
    /// Format a resource as (used, total). Used is only known when the guest
    /// reported both numbers; a configured total alone is still useful.
    fn usage(total: Option<u64>, free: Option<u64>) -> (Option<String>, Option<String>) {
        let used = total
            .zip(free)
            .map(|(total, free)| format_size(total.saturating_sub(free)));
        (used, total.map(format_size))
    }
}

/// Convert the KiB values reported by `/proc/meminfo` and `df -k` into a
/// compact human-readable size.
fn format_size(kib: u64) -> String {
    const MIB: u64 = 1024;
    const GIB: u64 = 1024 * MIB;
    const TIB: u64 = 1024 * GIB;

    if kib >= TIB {
        format!("{:.1} TiB", kib as f64 / TIB as f64)
    } else if kib >= GIB {
        format!("{:.1} GiB", kib as f64 / GIB as f64)
    } else if kib >= MIB {
        format!("{:.1} MiB", kib as f64 / MIB as f64)
    } else {
        format!("{kib} KiB")
    }
}

/// Parse the key/value report plus `df -k` table emitted by
/// [`VM_METRICS_SCRIPT`]. The docker mount is preferred at its standard
/// location, with any other mount containing "docker" as a fallback.
fn parse_vm_metrics(output: &str) -> VmMetrics {
    let mut metrics = VmMetrics::default();
    // (total_kib, free_kib, is_standard_location)
    let mut docker_mount: Option<(u64, u64, bool)> = None;

    for line in output.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("cpu=") {
            if value.parse::<u64>().is_ok() {
                metrics.cpus = Some(value.to_owned());
            }
            continue;
        }
        if let Some(value) = line.strip_prefix("mem_total_kb=") {
            metrics.mem_total_kib = value.parse::<u64>().ok();
            continue;
        }
        if let Some(value) = line.strip_prefix("mem_free_kb=") {
            metrics.mem_free_kib = value.parse::<u64>().ok();
            continue;
        }

        // df rows: Filesystem 1K-blocks Used Available Use% Mounted on
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 6 || fields[0] == "Filesystem" {
            continue;
        }
        let (Ok(total), Ok(free)) = (fields[1].parse::<u64>(), fields[3].parse::<u64>()) else {
            continue;
        };
        let mount = fields[5..].join(" ");
        if mount == "/" {
            metrics.root_total_kib = Some(total);
            metrics.root_free_kib = Some(free);
        } else if mount == "/var/lib/docker" {
            docker_mount = Some((total, free, true));
        } else if mount.contains("docker")
            && docker_mount.as_ref().is_none_or(|(_, _, exact)| !*exact)
        {
            docker_mount = Some((total, free, false));
        }
    }

    if let Some((total, free, _)) = docker_mount {
        metrics.docker_total_kib = Some(total);
        metrics.docker_free_kib = Some(free);
    }
    metrics
}

/// Fetch sandboxes with their status from `sbx ls --format json`. Falls back
/// to `sbx ls -q` when structured output is unavailable.
pub fn fetch_sandboxes() -> Result<Vec<Sandbox>, String> {
    let structured = ProcessCommand::new("sbx")
        .args(["ls", "--format", "json"])
        .output()
        .map_err(|error| error.to_string())?;

    if structured.status.success()
        && let Ok(value) = serde_json::from_slice::<Value>(&structured.stdout)
    {
        let rows = value
            .as_array()
            .or_else(|| value.get("sandboxes").and_then(Value::as_array));
        if let Some(rows) = rows {
            return Ok(rows
                .iter()
                .filter_map(|row| {
                    let name = json_text(row, &["name", "sandbox_name"])?;
                    let status = json_text(row, &["status", "state"])
                        .unwrap_or_else(|| "unknown".to_owned());
                    Some(Sandbox::new(name, status))
                })
                .collect());
        }
    }

    let output = ProcessCommand::new("sbx")
        .args(["ls", "-q"])
        .output()
        .map_err(|error| error.to_string())?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(if stderr.is_empty() {
            format!("sbx ls exited with {}.", output.status)
        } else {
            stderr
        });
    }

    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|name| Sandbox::new(name.to_owned(), "unknown".to_owned()))
        .collect())
}

/// Read details for one sandbox. `sbx inspect` supplies the configured
/// values; a command inside the microVM supplies the live numbers. This is
/// called only when the user opens Info, so the values are always fresh.
pub fn inspect_sandbox(sandbox: &Sandbox) -> SandboxDetails {
    let mut details = SandboxDetails {
        name: sandbox.name.clone(),
        status: sandbox.status.clone(),
        ..SandboxDetails::default()
    };

    load_inspect_details(sandbox, &mut details);

    let output = ProcessCommand::new("sbx")
        .args(["exec", &sandbox.name, "--", "sh", "-c", VM_METRICS_SCRIPT])
        .output();

    match output {
        Ok(output) if output.status.success() => {
            let metrics = parse_vm_metrics(&String::from_utf8_lossy(&output.stdout));
            override_if_present(&mut details.cpus, metrics.cpus);

            let (mem_used, mem_total) =
                VmMetrics::usage(metrics.mem_total_kib, metrics.mem_free_kib);
            override_if_present(&mut details.mem_used, mem_used);
            override_if_present(&mut details.mem_total, mem_total);

            let (root_used, root_total) =
                VmMetrics::usage(metrics.root_total_kib, metrics.root_free_kib);
            override_if_present(&mut details.root_used, root_used);
            override_if_present(&mut details.root_total, root_total);

            let (docker_used, docker_total) =
                VmMetrics::usage(metrics.docker_total_kib, metrics.docker_free_kib);
            override_if_present(&mut details.docker_used, docker_used);
            override_if_present(&mut details.docker_total, docker_total);
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            push_error(
                &mut details,
                if stderr.is_empty() {
                    format!(
                        "Could not read live metrics (sbx exec exited with {}).",
                        output.status
                    )
                } else {
                    format!("Could not read live metrics: {stderr}")
                },
            );
        }
        Err(error) => {
            push_error(
                &mut details,
                format!("Could not read live metrics: {error}"),
            );
        }
    }

    details
}

/// Fill in configured values from `sbx inspect --format json`. These are used
/// as fallbacks when the guest cannot report a field.
fn load_inspect_details(sandbox: &Sandbox, details: &mut SandboxDetails) {
    let output = match ProcessCommand::new("sbx")
        .args(["inspect", &sandbox.name, "--format", "json"])
        .output()
    {
        Ok(output) => output,
        Err(error) => {
            push_error(details, format!("Could not inspect sandbox: {error}"));
            return;
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        push_error(
            details,
            if stderr.is_empty() {
                format!("sbx inspect exited with {}.", output.status)
            } else {
                stderr
            },
        );
        return;
    }

    let value: Value = match serde_json::from_slice(&output.stdout) {
        Ok(value) => value,
        Err(error) => {
            push_error(
                details,
                format!("Could not parse sbx inspect output: {error}"),
            );
            return;
        }
    };

    details.cpus = json_text(&value, &["cpus", "cpu", "cpu_count", "vcpus"]);
    details.mem_total = json_text(
        &value,
        &["memory", "memoryMiB", "memory_mib", "memory_mb", "mem"],
    );
    details.root_total = json_text(
        &value,
        &[
            "root_size",
            "rootSize",
            "rootfs",
            "root_filesystem_size",
            "rootFilesystemSize",
        ],
    );
    details.docker_total = json_text(
        &value,
        &[
            "docker_size",
            "dockerSize",
            "docker",
            "dind_volume_size",
            "dindVolumeSize",
        ],
    );
}

fn override_if_present(target: &mut Option<String>, value: Option<String>) {
    if value.is_some() {
        *target = value;
    }
}

fn push_error(details: &mut SandboxDetails, error: String) {
    match &mut details.error {
        Some(existing) => {
            existing.push_str("; ");
            existing.push_str(&error);
        }
        None => details.error = Some(error),
    }
}

/// Find a key near the root of a JSON document. `sbx inspect` nests the same
/// fields at different depths depending on the sandbox, so a bounded search
/// is more robust than a fixed path.
fn find_key<'a>(value: &'a Value, key: &str, depth: usize) -> Option<&'a Value> {
    if depth > 5 {
        return None;
    }
    if let Some(map) = value.as_object() {
        if let Some(found) = map.get(key) {
            return Some(found);
        }
        for child in map.values() {
            if let Some(found) = find_key(child, key, depth + 1) {
                return Some(found);
            }
        }
    } else if let Some(items) = value.as_array() {
        for child in items {
            if let Some(found) = find_key(child, key, depth + 1) {
                return Some(found);
            }
        }
    }
    None
}

fn json_text(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| match find_key(value, key, 0)? {
        Value::String(text) if !text.is_empty() => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(boolean) => Some(boolean.to_string()),
        _ => None,
    })
}

// --- app --------------------------------------------------------------------

pub struct App {
    sandboxes: Vec<Sandbox>,
    selected: usize,
    grid_offset: usize,
    visible_rows: usize,
    mode: Mode,
    wizard: Wizard,
    info: Option<SandboxDetails>,
    pending_external: Option<PendingCommand>,
    message: String,
    should_quit: bool,
    click_targets: Vec<ClickTarget>,
}

impl App {
    pub fn new() -> Self {
        let mut app = Self {
            sandboxes: Vec::new(),
            selected: 0,
            grid_offset: 0,
            visible_rows: 1,
            mode: Mode::Normal,
            wizard: Wizard::new(),
            info: None,
            pending_external: None,
            message: String::new(),
            should_quit: false,
            click_targets: Vec::new(),
        };
        app.refresh_sandboxes();
        app
    }

    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn wizard(&self) -> &Wizard {
        &self.wizard
    }

    pub fn info(&self) -> Option<&SandboxDetails> {
        self.info.as_ref()
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn set_message(&mut self, message: impl Into<String>) {
        self.message = message.into();
    }

    /// Taken by the event loop when an Exec button was clicked. The terminal
    /// is suspended before the returned command is run.
    pub fn take_external_command(&mut self) -> Option<PendingCommand> {
        self.pending_external.take()
    }

    pub fn sandboxes(&self) -> &[Sandbox] {
        &self.sandboxes
    }

    /// Sandbox blocks plus the trailing "+ New Sandbox" button.
    pub fn block_count(&self) -> usize {
        self.sandboxes.len() + 1
    }

    pub fn selected_index(&self) -> usize {
        self.selected
    }

    pub fn is_new_sandbox_selected(&self) -> bool {
        self.selected == self.sandboxes.len()
    }

    pub fn grid_offset(&self) -> usize {
        self.grid_offset
    }

    /// Called by the renderer so keyboard scrolling matches what is on screen.
    pub fn set_visible_rows(&mut self, rows: usize) {
        self.visible_rows = rows.max(1);
        self.clamp_navigation();
    }

    /// Called by the renderer on every frame with the areas that are clickable.
    pub fn set_click_targets(&mut self, targets: Vec<ClickTarget>) {
        self.click_targets = targets;
    }

    #[cfg(test)]
    pub fn click_targets(&self) -> &[ClickTarget] {
        &self.click_targets
    }

    /// Handle a mouse event against the most recently rendered hit regions.
    pub fn handle_mouse(&mut self, mouse: MouseEvent) {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                // Reverse order: modal widgets are drawn last and must win
                // when they overlap the grid underneath them.
                if let Some(target) = self
                    .click_targets
                    .iter()
                    .rev()
                    .find(|target| target.contains(mouse.column, mouse.row))
                {
                    let action = target.action;
                    self.handle_click(action);
                }
            }
            MouseEventKind::ScrollUp if self.mode == Mode::Normal => self.move_vertical(-1),
            MouseEventKind::ScrollDown if self.mode == Mode::Normal => self.move_vertical(1),
            _ => {}
        }
    }

    fn handle_click(&mut self, action: ClickAction) {
        match action {
            ClickAction::Select(index) if self.mode == Mode::Normal => self.select(index),
            ClickAction::OpenWizard if self.mode == Mode::Normal => self.open_wizard(),
            ClickAction::CloseWizard if self.mode == Mode::Wizard => self.close_wizard(),
            ClickAction::CreateSandbox if self.mode == Mode::Wizard => self.create_from_button(),
            ClickAction::Start(index) if self.mode == Mode::Normal => self.start_sandbox(index),
            ClickAction::Stop(index) if self.mode == Mode::Normal => self.stop_sandbox(index),
            ClickAction::Exec(index)
                if self.mode == Mode::Normal && self.sandbox_is_running(index) =>
            {
                self.exec_sandbox(index)
            }
            ClickAction::Remove(index) if self.mode == Mode::Normal => self.remove_sandbox(index),
            ClickAction::Info(index)
                if self.mode == Mode::Normal && self.sandbox_is_running(index) =>
            {
                self.open_info(index)
            }
            ClickAction::CloseInfo if self.mode == Mode::Info => self.close_info(),
            _ => {}
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }

        match self.mode {
            Mode::Normal => match key.code {
                KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
                KeyCode::Left | KeyCode::Char('h') => self.move_horizontal(-1),
                KeyCode::Right | KeyCode::Char('l') => self.move_horizontal(1),
                KeyCode::Up | KeyCode::Char('k') => self.move_vertical(-1),
                KeyCode::Down | KeyCode::Char('j') => self.move_vertical(1),
                KeyCode::PageUp => self.move_page(-1),
                KeyCode::PageDown => self.move_page(1),
                KeyCode::Home => self.select(0),
                KeyCode::End => self.select(self.block_count() - 1),
                KeyCode::Enter if self.is_new_sandbox_selected() => self.open_wizard(),
                KeyCode::Char('r') | KeyCode::Char('R') => self.refresh_sandboxes(),
                KeyCode::Char('s') | KeyCode::Char('S') if !self.is_new_sandbox_selected() => {
                    self.toggle_sandbox(self.selected);
                }
                KeyCode::Char('x') | KeyCode::Char('X')
                    if !self.is_new_sandbox_selected() && self.sandbox_is_running(self.selected) =>
                {
                    self.exec_sandbox(self.selected);
                }
                KeyCode::Char('d') | KeyCode::Char('D') if !self.is_new_sandbox_selected() => {
                    self.remove_sandbox(self.selected);
                }
                KeyCode::Char('i') | KeyCode::Char('I')
                    if !self.is_new_sandbox_selected() && self.sandbox_is_running(self.selected) =>
                {
                    self.open_info(self.selected);
                }
                _ => {}
            },
            Mode::Wizard => match self.wizard.handle_key(key, &mut self.message) {
                WizardOutcome::Continue => {}
                WizardOutcome::Cancelled => self.close_wizard(),
                WizardOutcome::Create(pending) => {
                    let name = self.wizard.name().to_owned();
                    self.finish_wizard(pending, &name);
                }
            },
            Mode::Info => match key.code {
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') | KeyCode::Char('i') => {
                    self.close_info()
                }
                _ => {}
            },
        }
    }

    fn open_info(&mut self, index: usize) {
        let Some(sandbox) = self.sandboxes.get(index).cloned() else {
            return;
        };
        self.selected = index;
        self.info = Some(inspect_sandbox(&sandbox));
        self.mode = Mode::Info;
    }

    fn close_info(&mut self) {
        self.mode = Mode::Normal;
        self.info = None;
        self.message.clear();
    }

    fn sandbox_is_running(&self, index: usize) -> bool {
        self.sandboxes
            .get(index)
            .is_some_and(|sandbox| sandbox.is_running())
    }

    /// `s` follows the state shown on the block: start a stopped sandbox,
    /// stop a running one.
    fn toggle_sandbox(&mut self, index: usize) {
        if self.sandbox_is_running(index) {
            self.stop_sandbox(index);
        } else {
            self.start_sandbox(index);
        }
    }

    fn start_sandbox(&mut self, index: usize) {
        let Some(name) = self.sandbox_name(index) else {
            return;
        };
        // `sbx` has no standalone start verb; exec starts a stopped sandbox.
        // A no-op command leaves it running after the command exits.
        let command = PendingCommand::new("sbx", &["exec", &name, "--", "true"]);
        self.run_sbx(command, format!("Sandbox '{name}' started."));
    }

    fn stop_sandbox(&mut self, index: usize) {
        let Some(name) = self.sandbox_name(index) else {
            return;
        };
        let command = PendingCommand::new("sbx", &["stop", &name]);
        self.run_sbx(command, format!("Sandbox '{name}' stopped."));
    }

    fn exec_sandbox(&mut self, index: usize) {
        let Some(name) = self.sandbox_name(index) else {
            return;
        };
        self.pending_external = Some(PendingCommand::new(
            "sbx",
            &["exec", "-it", &name, "bash"],
        ));
        self.message = format!("Opening external terminal for '{name}'...");
    }

    fn remove_sandbox(&mut self, index: usize) {
        let Some(name) = self.sandbox_name(index) else {
            return;
        };
        // The TUI has no TTY to answer sbx's confirmation prompt, so removal
        // is explicit and non-interactive here.
        let command = PendingCommand::new("sbx", &["remove", &name, "--force"]);
        self.run_sbx(command, format!("Sandbox '{name}' removed."));
    }

    fn sandbox_name(&self, index: usize) -> Option<String> {
        self.sandboxes.get(index).map(|sandbox| sandbox.name.clone())
    }

    fn open_wizard(&mut self) {
        self.wizard = Wizard::new();
        self.message.clear();
        self.mode = Mode::Wizard;
    }

    fn close_wizard(&mut self) {
        self.mode = Mode::Normal;
        self.wizard = Wizard::new();
        self.message = "Create sandbox cancelled.".to_owned();
    }

    fn create_from_button(&mut self) {
        if !self.wizard.is_ready() {
            self.message = "Finish the wizard steps before creating the sandbox.".to_owned();
            return;
        }
        match self.wizard.finish() {
            Ok(pending) => {
                let name = self.wizard.name().to_owned();
                self.finish_wizard(pending, &name);
            }
            Err((step, error)) => {
                self.wizard.go_to_step(step);
                self.message = error;
            }
        }
    }

    fn finish_wizard(&mut self, pending: PendingCommand, name: &str) {
        self.run_sbx(pending, format!("Sandbox '{name}' created."));
        self.mode = Mode::Normal;
        self.wizard = Wizard::new();
    }

    fn select(&mut self, index: usize) {
        self.selected = index.min(self.block_count() - 1);
        self.keep_selection_visible();
    }

    fn move_horizontal(&mut self, delta: isize) {
        let next = self.selected as isize + delta;
        if next < 0 || next >= self.block_count() as isize {
            return;
        }
        // Don't wrap from the end of a row into the start of the next one.
        if delta < 0 && self.selected.is_multiple_of(GRID_COLUMNS) {
            return;
        }
        if delta > 0 && (next as usize).is_multiple_of(GRID_COLUMNS) {
            return;
        }
        self.select(next as usize);
    }

    fn move_vertical(&mut self, delta: isize) {
        let next = self.selected as isize + delta * GRID_COLUMNS as isize;
        if next >= 0 && next < self.block_count() as isize {
            self.select(next as usize);
        }
    }

    fn move_page(&mut self, direction: isize) {
        let step = (self.visible_rows * GRID_COLUMNS) as isize;
        let next = self.selected as isize + direction * step;
        if next < 0 {
            self.select(0);
        } else if next >= self.block_count() as isize {
            self.select(self.block_count() - 1);
        } else {
            self.select(next as usize);
        }
    }

    fn clamp_navigation(&mut self) {
        let rows = self.block_count().div_ceil(GRID_COLUMNS);
        self.selected = self.selected.min(self.block_count() - 1);
        self.grid_offset = self.grid_offset.min(rows.saturating_sub(self.visible_rows));
        self.keep_selection_visible();
    }

    fn keep_selection_visible(&mut self) {
        let row = self.selected / GRID_COLUMNS;
        if row < self.grid_offset {
            self.grid_offset = row;
        } else if row >= self.grid_offset + self.visible_rows {
            self.grid_offset = row + 1 - self.visible_rows;
        }
    }

    fn refresh_sandboxes(&mut self) {
        self.message.clear();
        match fetch_sandboxes() {
            Ok(sandboxes) => self.apply_statuses(sandboxes),
            Err(error) => self.message = format!("Could not list sandboxes: {error}"),
        }
    }

    /// Periodic tick: keep the running-state dots (and an open Info modal's
    /// status line) current without clearing the footer message or re-running
    /// the more expensive in-guest metrics commands.
    pub fn refresh_statuses(&mut self) {
        if let Ok(sandboxes) = fetch_sandboxes() {
            self.apply_statuses(sandboxes);
        }
    }

    fn apply_statuses(&mut self, sandboxes: Vec<Sandbox>) {
        if let Some(info) = self.info.as_mut()
            && let Some(fresh) = sandboxes.iter().find(|sandbox| sandbox.name == info.name)
        {
            info.status = fresh.status.clone();
        }
        self.sandboxes = sandboxes;
        self.clamp_navigation();
    }

    fn run_sbx(&mut self, pending: PendingCommand, success: String) {
        match pending.run() {
            Ok(output) if output.status.success() => {
                self.refresh_sandboxes();
                if self.message.is_empty() {
                    self.message = success;
                }
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let detail = stderr
                    .lines()
                    .find(|line| !line.trim().is_empty())
                    .unwrap_or("sbx command failed");
                self.message = format!("Could not run sbx: {detail}");
            }
            Err(error) => self.message = format!("Could not run sbx: {error}"),
        }
    }
}

#[cfg(test)]
impl App {
    pub fn with_sandboxes(names: &[&str]) -> Self {
        Self::with_sandbox_statuses(
            &names
                .iter()
                .map(|name| (*name, "running"))
                .collect::<Vec<_>>(),
        )
    }

    pub fn with_sandbox_statuses(entries: &[(&str, &str)]) -> Self {
        let mut app = Self::new();
        app.sandboxes = entries
            .iter()
            .map(|(name, status)| Sandbox::new((*name).to_owned(), (*status).to_owned()))
            .collect();
        app.clamp_navigation();
        app
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_validation() {
        assert!(validate_name("demo-tui").is_ok());
        assert!(validate_name("a").is_err());
        assert!(validate_name("default").is_err());
        assert!(validate_name(".bad").is_err());
        assert!(validate_name("bad name").is_err());
    }

    #[test]
    fn size_and_cpu_validation() {
        assert!(validate_gb_size("").is_ok());
        assert!(validate_gb_size("40").is_ok());
        assert!(validate_gb_size("20").is_ok());
        assert!(validate_gb_size("40g").is_err());
        assert!(validate_gb_size("40x").is_err());
        assert!(validate_cpus("2").is_ok());
        assert!(validate_cpus("0").is_ok());
        assert!(validate_cpus("abc").is_err());
    }

    #[test]
    fn memory_validation() {
        assert_eq!(parse_memory_mib("512m"), Some(512));
        assert_eq!(parse_memory_mib("8g"), Some(8192));
        assert!(validate_memory("256m").is_err());
        assert!(validate_memory("nonsense").is_err());
    }

    #[test]
    fn wizard_builds_expected_command() {
        let mut wizard = Wizard::new();
        wizard.name = "demo-tui".to_owned();
        wizard.root_size = "40".to_owned();
        wizard.docker_size = "20".to_owned();
        wizard.cpus = "2".to_owned();
        wizard.memory = "4g".to_owned();

        assert_eq!(
            wizard.preview(),
            "DOCKER_SANDBOXES_ROOT_SIZE=40g DOCKER_SANDBOXES_DOCKER_SIZE=20g sbx create --name demo-tui opencode --cpus 2 --memory 4g"
        );
    }

    #[test]
    fn sandbox_status_detects_running() {
        assert!(Sandbox::new("a".into(), "running".into()).is_running());
        assert!(Sandbox::new("a".into(), "● running (detached)".into()).is_running());
        assert!(!Sandbox::new("a".into(), "stopped".into()).is_running());
        assert!(!Sandbox::new("a".into(), "unknown".into()).is_running());
    }

    #[test]
    fn stopped_sandbox_ignores_running_only_keys() {
        let mut app = App::with_sandbox_statuses(&[("alpha", "stopped")]);
        for key in ['x', 'i'] {
            app.handle_key(KeyEvent::new(KeyCode::Char(key), KeyModifiers::NONE));
        }
        assert!(matches!(app.mode(), Mode::Normal));
        assert!(app.take_external_command().is_none());
    }

    #[test]
    fn sandbox_actions_build_expected_commands() {
        assert_eq!(
            PendingCommand::new("sbx", &["exec", "demo", "--", "true"]).preview(),
            "sbx exec demo -- true"
        );
        assert_eq!(
            PendingCommand::new("sbx", &["stop", "demo"]).preview(),
            "sbx stop demo"
        );
        assert_eq!(
            PendingCommand::new("sbx", &["exec", "-it", "demo", "bash"]).preview(),
            "sbx exec -it demo bash"
        );
        assert_eq!(
            PendingCommand::new("sbx", &["remove", "demo", "--force"]).preview(),
            "sbx remove demo --force"
        );
    }

    #[test]
    fn json_lookup_finds_nested_resource_fields() {
        let value: Value = serde_json::from_str(
            r#"{"name":"demo","resources":{"cpus":2,"memory":"4g"},"sizes":{"rootfs":"20g","docker":"10g"}}"#,
        )
        .unwrap();
        assert_eq!(json_text(&value, &["name"]).as_deref(), Some("demo"));
        assert_eq!(json_text(&value, &["cpus"]).as_deref(), Some("2"));
        assert_eq!(json_text(&value, &["memory"]).as_deref(), Some("4g"));
        assert_eq!(json_text(&value, &["rootfs"]).as_deref(), Some("20g"));
        assert_eq!(json_text(&value, &["docker"]).as_deref(), Some("10g"));
        assert_eq!(json_text(&value, &["missing"]), None);
    }

    #[test]
    fn size_formatting_uses_binary_units() {
        assert_eq!(format_size(512), "512 KiB");
        assert_eq!(format_size(1024), "1.0 MiB");
        assert_eq!(format_size(1024 * 1024), "1.0 GiB");
        assert_eq!(format_size(1024 * 1024 * 1024), "1.0 TiB");
    }

    #[test]
    fn vm_metrics_parser_reads_guest_report() {
        let output = concat!(
            "cpu=4\n",
            "mem_total_kb=2097152\n",
            "mem_free_kb=1048576\n",
            "Filesystem     1K-blocks      Used Available Use% Mounted on\n",
            "overlay         4194304    524288   3670016  13% /\n",
            "/dev/vde       2097152    524288   1048576  25% /var/lib/docker\n",
        );
        assert_eq!(
            parse_vm_metrics(output),
            VmMetrics {
                cpus: Some("4".to_owned()),
                mem_total_kib: Some(2097152),
                mem_free_kib: Some(1048576),
                root_total_kib: Some(4194304),
                root_free_kib: Some(3670016),
                docker_total_kib: Some(2097152),
                docker_free_kib: Some(1048576),
            }
        );
    }

    #[test]
    fn usage_derives_used_from_total_and_free() {
        assert_eq!(
            VmMetrics::usage(Some(2 * 1024 * 1024), Some(1024 * 1024)),
            (Some("1.0 GiB".to_owned()), Some("2.0 GiB".to_owned()))
        );
        assert_eq!(
            VmMetrics::usage(Some(4 * 1024 * 1024), Some(1024 * 1024)),
            (Some("3.0 GiB".to_owned()), Some("4.0 GiB".to_owned()))
        );
        assert_eq!(
            VmMetrics::usage(Some(1024), None),
            (None, Some("1.0 MiB".to_owned()))
        );
        assert_eq!(VmMetrics::usage(None, None), (None, None));
    }

    #[test]
    fn vm_metrics_prefers_standard_docker_mount() {
        let output = concat!(
            "cpu=8\n",
            "Filesystem 1K-blocks Used Available Use% Mounted on\n",
            "/dev/a 100 0 100 0% /var/lib/docker-data\n",
            "/dev/b 200 0 200 0% /var/lib/docker\n",
            "/dev/c 300 0 300 0% /\n",
        );
        let metrics = parse_vm_metrics(output);
        assert_eq!(metrics.cpus.as_deref(), Some("8"));
        assert_eq!(metrics.docker_total_kib, Some(200));
        assert_eq!(metrics.docker_free_kib, Some(200));
        assert_eq!(metrics.root_total_kib, Some(300));
    }

    #[test]
    fn vm_metrics_falls_back_to_any_docker_mount() {
        let output = concat!(
            "Filesystem 1K-blocks Used Available Use% Mounted on\n",
            "/dev/a 100 0 100 0% /mnt/custom-docker\n",
            "/dev/c 300 0 300 0% /\n",
        );
        let metrics = parse_vm_metrics(output);
        assert_eq!(metrics.docker_total_kib, Some(100));
        assert_eq!(metrics.mem_free_kib, None);
    }

    #[test]
    fn status_tick_updates_modal_without_clearing_message() {
        let mut app = App::with_sandbox_statuses(&[("alpha", "stopped")]);
        app.set_message("keep me");
        app.info = Some(SandboxDetails {
            name: "alpha".to_owned(),
            status: "stopped".to_owned(),
            ..SandboxDetails::default()
        });

        app.apply_statuses(vec![Sandbox::new("alpha".to_owned(), "running".to_owned())]);

        assert!(app.sandboxes()[0].is_running());
        assert_eq!(app.info().unwrap().status, "running");
        assert_eq!(app.message(), "keep me");
    }
}
