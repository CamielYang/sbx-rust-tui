use std::io;
use std::process::Command as ProcessCommand;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratcn::runtime::{FocusState, ModalState};
use ratcn::{InputState, Toast, ToasterState};
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

/// Stable ids for the ratcn modal layers.
pub const CREATE_DIALOG: &str = "create-dialog";
pub const INFO_DIALOG: &str = "info-dialog";
pub const CONFIRM_DIALOG: &str = "confirm-dialog";
pub const PORTS_DIALOG: &str = "ports-dialog";

const CREATE_TOAST: &str = "create-sandbox";

/// Advanced once per event-loop frame while `sbx create` runs in the worker.
pub const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// How often the dashboard re-reads `sbx ls` to keep running-state dots fresh.
pub const STATUS_REFRESH_INTERVAL: Duration = Duration::from_secs(5);

/// How often an open Info dialog re-collects live guest metrics.
pub const INFO_REFRESH_INTERVAL: Duration = Duration::from_secs(10);

/// Collected with `sbx exec <name> -- sh -c <script>`. Plain text keeps the
/// command portable across the minimal shells used inside the microVM. CPU
/// usage needs two `/proc/stat` samples, so the guest sleeps for one second
/// between them and reports a percentage of all cores.
const VM_METRICS_SCRIPT: &str = r#"
printf 'cpu=%s\n' "$(nproc 2>/dev/null || printf 0)"
awk '
  function sample(   line, fields, count, i, total, idle) {
    while ((getline line < "/proc/stat") > 0) {
      if (line ~ /^cpu[ \t]/) {
        count = split(line, fields, " ")
        total = 0
        for (i = 2; i <= count; i++) total += fields[i] + 0
        idle = fields[5] + 0 + fields[6] + 0
        close("/proc/stat")
        return total " " idle
      }
    }
    close("/proc/stat")
    return ""
  }
  BEGIN {
    before = sample()
    system("sleep 1")
    after = sample()
    if (before != "" && after != "") {
      split(before, first, " ")
      split(after, second, " ")
      total = second[1] - first[1]
      idle = second[2] - first[2]
      if (total > 0) {
        usage = 100 * (total - idle) / total
        if (usage < 0) usage = 0
        if (usage > 100) usage = 100
        printf "cpu_usage=%d\n", int(usage + 0.5)
      }
    }
  }
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

/// Which screen the app is logically on. Derived from the ratcn modal stack,
/// kept as a convenience for status text and tests.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Normal,
    Wizard,
    Info,
    Confirm,
    Ports,
}

// --- create form -------------------------------------------------------------

/// The editable fields shown in the themed create dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormField {
    Name,
    RootSize,
    DockerSize,
    Cpus,
    Memory,
}

/// Text and selection state for the create dialog. Ratcn's `Input` and
/// `Select` components read and replace these values through messages.
#[derive(Clone)]
pub struct Form {
    name: InputState,
    agent_index: usize,
    agent_focus: usize,
    agent_open: bool,
    root_size: InputState,
    docker_size: InputState,
    cpus: InputState,
    memory: InputState,
}

impl Form {
    fn new() -> Self {
        Self {
            name: InputState::new(""),
            agent_index: AGENTS
                .iter()
                .position(|agent| *agent == "opencode")
                .unwrap_or(0),
            agent_focus: AGENTS
                .iter()
                .position(|agent| *agent == "opencode")
                .unwrap_or(0),
            agent_open: false,
            root_size: InputState::new(""),
            docker_size: InputState::new(""),
            cpus: InputState::new("2"),
            memory: InputState::new(default_memory()),
        }
    }

    pub fn name(&self) -> &InputState {
        &self.name
    }

    pub fn agent_index(&self) -> usize {
        self.agent_index
    }

    pub fn agent_focus(&self) -> usize {
        self.agent_focus
    }

    pub fn agent_open(&self) -> bool {
        self.agent_open
    }

    pub fn root_size(&self) -> &InputState {
        &self.root_size
    }

    pub fn docker_size(&self) -> &InputState {
        &self.docker_size
    }

    pub fn cpus(&self) -> &InputState {
        &self.cpus
    }

    pub fn memory(&self) -> &InputState {
        &self.memory
    }

    pub fn set_agent_index(&mut self, index: usize) {
        self.agent_index = index.min(AGENTS.len().saturating_sub(1));
        self.agent_focus = self.agent_index;
        // Committing a choice also dismisses the picker. Focus, selection and
        // open state change in one message so the popup cannot linger.
        self.agent_open = false;
    }

    pub fn set_agent_focus(&mut self, index: usize) {
        self.agent_focus = index.min(AGENTS.len().saturating_sub(1));
    }

    pub fn set_agent_open(&mut self, open: bool) {
        self.agent_open = open;
    }

    pub fn field_error(&self, field: FormField) -> Option<String> {
        match field {
            FormField::Name => validate_name(self.name.value()).err(),
            FormField::RootSize => validate_gb_size(self.root_size.value()).err(),
            FormField::DockerSize => validate_gb_size(self.docker_size.value()).err(),
            FormField::Cpus => validate_cpus(self.cpus.value()).err(),
            FormField::Memory => validate_memory(self.memory.value()).err(),
        }
    }

    /// True when an input should paint in the theme's invalid color. The empty
    /// name is left neutral until the user has typed something.
    pub fn invalid(&self, field: FormField) -> bool {
        if field == FormField::Name && self.name.value().is_empty() {
            return false;
        }
        self.field_error(field).is_some()
    }

    pub fn first_error(&self) -> Option<(FormField, String)> {
        [
            FormField::Name,
            FormField::RootSize,
            FormField::DockerSize,
            FormField::Cpus,
            FormField::Memory,
        ]
        .into_iter()
        .find_map(|field| self.field_error(field).map(|error| (field, error)))
    }

    pub fn warning(&self) -> Option<String> {
        let mib = parse_memory_mib(self.memory.value())?;
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

    pub fn pending_command(&self) -> PendingCommand {
        let name = self.name.value().trim();
        let agent = AGENTS[self.agent_index];
        let mut args = vec![
            "create".to_owned(),
            "--name".to_owned(),
            name.to_owned(),
            agent.to_owned(),
        ];

        let cpus = self.cpus.value().trim();
        if !cpus.is_empty() {
            args.push("--cpus".to_owned());
            args.push(cpus.to_owned());
        }
        let memory = self.memory.value().trim();
        if !memory.is_empty() {
            args.push("--memory".to_owned());
            args.push(memory.to_owned());
        }

        let mut env = Vec::new();
        let root = self.root_size.value().trim();
        if !root.is_empty() {
            env.push(("DOCKER_SANDBOXES_ROOT_SIZE".to_owned(), format!("{root}g")));
        }
        let docker = self.docker_size.value().trim();
        if !docker.is_empty() {
            env.push((
                "DOCKER_SANDBOXES_DOCKER_SIZE".to_owned(),
                format!("{docker}g"),
            ));
        }

        PendingCommand {
            program: "sbx".to_owned(),
            args,
            env,
        }
    }
}

impl FormField {
    pub fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::RootSize => "Root size (GB)",
            Self::DockerSize => "Docker data (GB)",
            Self::Cpus => "CPUs",
            Self::Memory => "Memory",
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

// --- published ports --------------------------------------------------------

/// One row of the published-ports table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortMapping {
    pub host_port: u16,
    pub sandbox_port: u16,
    pub protocol: String,
    pub host_ip: Option<String>,
}

/// The ports currently published for the sandbox named in the dialog.
#[derive(Clone, Debug)]
pub struct PortsView {
    pub name: String,
    pub ports: Vec<PortMapping>,
    pub error: Option<String>,
}

/// Which end of a `host:sandbox` mapping is being edited.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortField {
    Host,
    Sandbox,
}

impl PortField {
    pub fn label(self) -> &'static str {
        match self {
            Self::Host => "Host port",
            Self::Sandbox => "Sandbox port",
        }
    }
}

/// The two editable ports behind the modal's **Add port** button. The defaults
/// match `sbx port publish <name> 8080:3000` so the button is useful on its own.
#[derive(Clone)]
pub struct PortForm {
    host: InputState,
    sandbox: InputState,
}

impl PortForm {
    fn new() -> Self {
        Self {
            host: InputState::new("8080"),
            sandbox: InputState::new("3000"),
        }
    }

    pub fn host(&self) -> &InputState {
        &self.host
    }

    pub fn sandbox(&self) -> &InputState {
        &self.sandbox
    }

    fn port(&self, field: PortField) -> Result<u16, String> {
        let state = match field {
            PortField::Host => &self.host,
            PortField::Sandbox => &self.sandbox,
        };
        parse_port(state.value())
    }

    /// True when an input cannot be turned into a usable port.
    pub fn invalid(&self, field: PortField) -> bool {
        self.port(field).is_err()
    }

    pub fn first_error(&self) -> Option<String> {
        [PortField::Host, PortField::Sandbox]
            .into_iter()
            .find_map(|field| {
                self.port(field)
                    .err()
                    .map(|error| format!("{}: {error}", field.label()))
            })
    }

    fn pending_command(&self, name: &str) -> PendingCommand {
        // Callers validate first; the fallbacks keep this helper total.
        let host = self.port(PortField::Host).unwrap_or(0);
        let sandbox = self.port(PortField::Sandbox).unwrap_or(0);
        PendingCommand::new(
            "sbx",
            &["port", "publish", name, &format!("{host}:{sandbox}")],
        )
    }
}

fn parse_port(value: &str) -> Result<u16, String> {
    let value = value.trim();
    match value.parse::<u16>() {
        Ok(port) if port > 0 => Ok(port),
        _ => Err("Enter a port between 1 and 65535.".to_owned()),
    }
}

/// Read published ports with `sbx port ls <name> --format json`. The input is
/// an array (or an object with a `ports` array), as emitted by current `sbx`.
pub fn fetch_ports(name: &str) -> Result<Vec<PortMapping>, String> {
    let output = ProcessCommand::new("sbx")
        .args(["port", "ls", name, "--format", "json"])
        .output()
        .map_err(|error| format!("Could not run sbx port ls: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(if stderr.is_empty() {
            format!("sbx port ls exited with {}.", output.status)
        } else {
            stderr
        });
    }

    parse_ports(&output.stdout)
}

fn parse_ports(stdout: &[u8]) -> Result<Vec<PortMapping>, String> {
    let value: Value = serde_json::from_slice(stdout)
        .map_err(|error| format!("Could not parse sbx port ls output: {error}"))?;
    let rows = value
        .as_array()
        .or_else(|| value.get("ports").and_then(Value::as_array))
        .ok_or_else(|| "Unexpected sbx port ls output.".to_owned())?;

    Ok(rows
        .iter()
        .filter_map(|row| {
            let host = json_text(row, &["host_port", "hostPort", "host"])?;
            let sandbox = json_text(
                row,
                &[
                    "sandbox_port",
                    "sandboxPort",
                    "guest_port",
                    "container_port",
                    "port",
                ],
            )?;
            Some(PortMapping {
                host_port: parse_port(&host).ok()?,
                sandbox_port: parse_port(&sandbox).ok()?,
                protocol: json_text(row, &["protocol"]).unwrap_or_else(|| "tcp".to_owned()),
                host_ip: json_text(row, &["host_ip", "hostIp"]),
            })
        })
        .collect())
}

/// Resource details shown in the info modal. Values come from live commands
/// run inside the microVM; the structured `sbx inspect` output is only a
/// fallback for fields the guest cannot report.
#[derive(Clone, Debug, Default)]
pub struct SandboxDetails {
    pub name: String,
    pub status: String,
    pub cpus: Option<String>,
    /// Percentage of all guest cores currently busy, sampled over one second.
    pub cpu_usage: Option<u8>,
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
    cpu_usage: Option<u8>,
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
        if let Some(value) = line.strip_prefix("cpu_usage=") {
            metrics.cpu_usage = value.parse::<u8>().ok().filter(|usage| *usage <= 100);
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
/// values; a command inside the microVM supplies the live numbers. Info calls
/// this on a worker when the dialog opens and every ten seconds after that.
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
            details.cpu_usage = metrics.cpu_usage;

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

/// A `sbx create` running on a worker thread. The UI keeps rendering (and
/// ignoring input) until the result arrives on the channel.
struct Creation {
    name: String,
    result: Receiver<io::Result<std::process::Output>>,
}

/// One in-flight metrics collection for an open Info dialog. The guest-side
/// CPU sample sleeps for a second, so this runs off the event-loop thread.
struct InfoRefresh {
    name: String,
    result: Receiver<SandboxDetails>,
}

/// An action waiting for the user to confirm it in the modal dialog. Only the
/// name is captured: the state is checked again when the user confirms, so a
/// periodic status refresh cannot start a stopped sandbox by accident.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PendingAction {
    Start(String),
    Stop(String),
    Remove(String),
}

impl PendingAction {
    fn verb(&self) -> &'static str {
        match self {
            Self::Start(_) => "Start",
            Self::Stop(_) => "Stop",
            Self::Remove(_) => "Remove",
        }
    }
}

pub struct App {
    sandboxes: Vec<Sandbox>,
    /// The row the list cursor is on. Hover and arrow keys move it; the
    /// Actions pane follows `selected`, which a click commits.
    cursor: usize,
    selected: usize,
    list_offset: usize,
    focus: FocusState,
    modals: ModalState,
    form: Form,
    info: Option<SandboxDetails>,
    info_refresh: Option<InfoRefresh>,
    ports: Option<PortsView>,
    port_form: PortForm,
    pending_action: Option<PendingAction>,
    pending_external: Option<PendingCommand>,
    creation: Option<Creation>,
    spinner_index: usize,
    message: String,
    toasts: ToasterState<'static>,
    started: Instant,
    should_quit: bool,
    /// In tests, `perform_action` records instead of running `sbx`.
    #[cfg(test)]
    confirmed_for_test: Option<PendingAction>,
    /// In tests, publishing records the command instead of running `sbx`.
    #[cfg(test)]
    published_for_test: Option<String>,
    /// In tests, unpublishing records the command instead of running `sbx`.
    #[cfg(test)]
    unpublished_for_test: Option<String>,
}

impl App {
    pub fn new() -> Self {
        let mut app = Self {
            sandboxes: Vec::new(),
            cursor: 0,
            selected: 0,
            list_offset: 0,
            focus: FocusState::default(),
            modals: ModalState::default(),
            form: Form::new(),
            info: None,
            info_refresh: None,
            ports: None,
            port_form: PortForm::new(),
            pending_action: None,
            pending_external: None,
            creation: None,
            spinner_index: 0,
            message: String::new(),
            toasts: ToasterState::new(),
            started: Instant::now(),
            should_quit: false,
            #[cfg(test)]
            confirmed_for_test: None,
            #[cfg(test)]
            published_for_test: None,
            #[cfg(test)]
            unpublished_for_test: None,
        };
        app.refresh_sandboxes();
        app
    }

    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    pub fn quit(&mut self) {
        self.should_quit = true;
    }

    /// Logical screen, derived from the modal stack so the two can never
    /// disagree about which layer is active.
    pub fn mode(&self) -> Mode {
        if self.modals.is_open(CREATE_DIALOG) {
            Mode::Wizard
        } else if self.modals.is_open(INFO_DIALOG) {
            Mode::Info
        } else if self.modals.is_open(CONFIRM_DIALOG) {
            Mode::Confirm
        } else if self.modals.is_open(PORTS_DIALOG) {
            Mode::Ports
        } else {
            Mode::Normal
        }
    }

    /// The action the confirmation dialog is asking about, if one is open.
    pub fn pending_action(&self) -> Option<&PendingAction> {
        self.pending_action.as_ref()
    }

    pub fn focus_ref(&self) -> &FocusState {
        &self.focus
    }

    pub fn focus_mut(&mut self) -> &mut FocusState {
        &mut self.focus
    }

    pub fn modals_ref(&self) -> &ModalState {
        &self.modals
    }

    pub fn form(&self) -> &Form {
        &self.form
    }

    pub fn set_form_state(&mut self, field: FormField, state: InputState) {
        match field {
            FormField::Name => self.form.name = state,
            FormField::RootSize => self.form.root_size = state,
            FormField::DockerSize => self.form.docker_size = state,
            FormField::Cpus => self.form.cpus = state,
            FormField::Memory => self.form.memory = state,
        }
    }

    pub fn set_agent_index(&mut self, index: usize) {
        self.form.set_agent_index(index);
    }

    pub fn set_agent_focus(&mut self, index: usize) {
        self.form.set_agent_focus(index);
    }

    pub fn set_agent_open(&mut self, open: bool) {
        self.form.set_agent_open(open);
    }

    pub fn info(&self) -> Option<&SandboxDetails> {
        self.info.as_ref()
    }

    pub fn ports(&self) -> Option<&PortsView> {
        self.ports.as_ref()
    }

    pub fn port_form(&self) -> &PortForm {
        &self.port_form
    }

    pub fn port_invalid(&self, field: PortField) -> bool {
        self.port_form.invalid(field)
    }

    pub fn set_port_state(&mut self, field: PortField, state: InputState) {
        match field {
            PortField::Host => self.port_form.host = state,
            PortField::Sandbox => self.port_form.sandbox = state,
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn now(&self) -> Duration {
        self.started.elapsed()
    }

    pub fn toasts(&self) -> &ToasterState<'static> {
        &self.toasts
    }

    pub fn prune_toasts(&mut self, now: Duration) -> bool {
        self.toasts.prune_expired(now)
    }

    fn notify(&mut self, toast: Toast<'static>, detail: Option<String>) {
        self.message = match detail {
            Some(ref detail) if !detail.is_empty() => format!("{}: {detail}", toast.title()),
            _ => toast.title().to_owned(),
        };
        let toast = match detail {
            Some(detail) if !detail.is_empty() => toast.with_description(detail),
            _ => toast,
        };
        let now = self.now();
        self.toasts.push(toast, now);
    }

    pub fn notify_info(&mut self, title: impl Into<String>, detail: Option<String>) {
        self.notify(Toast::info(title.into()), detail);
    }

    pub fn notify_success(&mut self, title: impl Into<String>, detail: Option<String>) {
        self.notify(Toast::success(title.into()), detail);
    }

    pub fn notify_error(&mut self, title: impl Into<String>, detail: impl Into<String>) {
        let detail = detail.into();
        self.notify(Toast::error(title.into()), Some(detail));
    }

    /// Taken by the event loop when an Exec button was clicked. The terminal
    /// is suspended before the returned command is run.
    pub fn take_external_command(&mut self) -> Option<PendingCommand> {
        self.pending_external.take()
    }

    /// True from the moment the create button is pressed until the worker
    /// thread reports back. Input is ignored during this window.
    pub fn is_creating(&self) -> bool {
        self.creation.is_some()
    }

    pub fn creating_name(&self) -> Option<&str> {
        self.creation
            .as_ref()
            .map(|creation| creation.name.as_str())
    }

    pub fn spinner(&self) -> &'static str {
        SPINNER_FRAMES[self.spinner_index % SPINNER_FRAMES.len()]
    }

    /// Fraction for the themed progress bar shown in the create dialog.
    pub fn progress(&self) -> f64 {
        (self.spinner_index % SPINNER_FRAMES.len()) as f64 / (SPINNER_FRAMES.len() - 1) as f64
    }

    /// Advance the spinner once per rendered frame, but only while a sandbox
    /// is actually being created.
    pub fn tick(&mut self) {
        if self.is_creating() {
            self.spinner_index = self.spinner_index.wrapping_add(1);
        }
    }

    /// Pick up a finished `sbx create` without blocking the event loop.
    pub fn poll_creation(&mut self) {
        let (name, result) = {
            let Some(creation) = self.creation.as_ref() else {
                return;
            };
            let result = match creation.result.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(Err(io::Error::other("creation worker stopped")))
                }
                Err(mpsc::TryRecvError::Empty) => None,
            };
            (creation.name.clone(), result)
        };
        if let Some(result) = result {
            self.creation = None;
            self.finish_creation(result, &name);
        }
    }

    /// Pick up a finished metrics collection without blocking the event loop.
    /// Results for a sandbox that is no longer shown are dropped.
    pub fn poll_info(&mut self) {
        let Some(refresh) = self.info_refresh.as_ref() else {
            return;
        };
        let result = match refresh.result.try_recv() {
            Ok(details) => Some(details),
            Err(mpsc::TryRecvError::Disconnected) => None,
            Err(mpsc::TryRecvError::Empty) => return,
        };
        let name = refresh.name.clone();
        self.info_refresh = None;
        if self.info.as_ref().is_some_and(|info| info.name == name) {
            match result {
                Some(details) => self.info = Some(details),
                None => {
                    if let Some(info) = self.info.as_mut() {
                        push_error(info, "Metrics worker stopped.".to_owned());
                    }
                }
            }
        }
    }

    /// True while the Info dialog is waiting for its metrics worker.
    pub fn is_refreshing_info(&self) -> bool {
        self.info_refresh.is_some()
    }

    /// Start a new metrics collection for the sandbox shown in Info. Called
    /// when the dialog opens and on the ten-second ticker; overlapping runs are
    /// skipped so slow guest commands cannot pile up.
    pub fn refresh_info(&mut self) {
        if self.is_refreshing_info() {
            return;
        }
        let Some(name) = self.info.as_ref().map(|info| info.name.clone()) else {
            return;
        };
        let Some(sandbox) = self
            .sandboxes
            .iter()
            .find(|sandbox| sandbox.name == name)
            .cloned()
        else {
            return;
        };
        self.start_info_refresh(&sandbox);
    }

    fn start_info_refresh(&mut self, sandbox: &Sandbox) {
        if self.is_refreshing_info() {
            return;
        }
        let name = sandbox.name.clone();
        let sandbox = sandbox.clone();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let _ = sender.send(inspect_sandbox(&sandbox));
        });
        self.info_refresh = Some(InfoRefresh {
            name,
            result: receiver,
        });
    }

    pub fn sandboxes(&self) -> &[Sandbox] {
        &self.sandboxes
    }

    /// The row the list cursor is on. Kept apart from the selected sandbox so
    /// hovering a row does not silently retarget the Actions pane.
    pub fn focused_name(&self) -> Option<String> {
        self.sandboxes.get(self.cursor).map(|s| s.name.clone())
    }

    pub fn selected_name(&self) -> Option<String> {
        self.sandboxes.get(self.selected).map(|s| s.name.clone())
    }

    pub fn selected_sandbox(&self) -> Option<&Sandbox> {
        self.sandboxes.get(self.selected)
    }

    pub fn list_offset(&self) -> usize {
        self.list_offset
    }

    pub fn set_list_offset(&mut self, offset: usize) {
        self.list_offset = offset.min(self.sandboxes.len().saturating_sub(1));
    }

    /// Move the cursor (and the visible window) without changing which sandbox
    /// the Actions pane acts on. Keyboard navigation and hover use this.
    pub fn focus_list_item(&mut self, name: &str, offset: usize) {
        if let Some(index) = self.sandboxes.iter().position(|s| s.name == name) {
            self.cursor = index;
        }
        self.set_list_offset(offset);
    }

    /// Commit a sandbox as the target of the Actions pane and put keyboard
    /// focus on the list so Tab continues into those actions.
    pub fn focus_sandbox(&mut self, name: &str) {
        self.select_named(name);
        *self.focus_mut() = FocusState::intent(["sandbox-list"]);
    }

    pub fn select_named(&mut self, name: &str) {
        if let Some(index) = self.sandboxes.iter().position(|s| s.name == name) {
            self.selected = index;
            self.cursor = index;
        }
    }

    /// Keys that are not claimed by a ratcn component still act as global
    /// shortcuts. Creation is modal: only Ctrl+C gets through.
    pub fn handle_global_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }
        if self.is_creating() || self.mode() != Mode::Normal {
            return;
        }

        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.quit(),
            KeyCode::Char('r') | KeyCode::Char('R') => {
                self.refresh_sandboxes();
                self.notify_success("Sandbox list refreshed", None);
            }
            KeyCode::Char('n') | KeyCode::Char('N') => self.open_wizard(),
            KeyCode::Char('s') | KeyCode::Char('S') => self.toggle_selected(),
            KeyCode::Char('x') | KeyCode::Char('X') => self.exec_selected(),
            KeyCode::Char('d') | KeyCode::Char('D') => self.remove_selected(),
            KeyCode::Char('i') | KeyCode::Char('I') => self.open_selected_info(),
            KeyCode::Char('p') | KeyCode::Char('P') => self.open_ports_selected(),
            _ => {}
        }
    }

    pub fn open_wizard(&mut self) {
        self.form = Form::new();
        let _ = self.modals.open(CREATE_DIALOG, &mut self.focus);
    }

    /// Cancel via the dialog's close button or Escape.
    pub fn cancel_wizard(&mut self) {
        self.close_wizard();
        self.notify_info("Create sandbox cancelled.", None);
    }

    fn close_wizard(&mut self) {
        if self
            .modals
            .top()
            .is_some_and(|id| id.as_str() == CREATE_DIALOG)
        {
            self.modals.close(&mut self.focus);
        }
        self.form = Form::new();
    }

    pub fn create_from_form(&mut self) {
        if let Some((field, error)) = self.form.first_error() {
            self.notify_error(
                "Invalid sandbox setting",
                format!("{}: {error}", field.label()),
            );
            return;
        }
        let pending = self.form.pending_command();
        let name = self.form.name.value().trim().to_owned();
        self.start_creation(pending, &name);
    }

    pub fn open_info(&mut self, name: &str) {
        let Some(sandbox) = self.sandboxes.iter().find(|s| s.name == name).cloned() else {
            return;
        };
        self.select_named(name);
        // Show the known values immediately; the guest-side metrics arrive on
        // a worker so the one-second CPU sample never freezes the TUI.
        self.info = Some(SandboxDetails {
            name: sandbox.name.clone(),
            status: sandbox.status.clone(),
            ..SandboxDetails::default()
        });
        self.info_refresh = None;
        self.start_info_refresh(&sandbox);
        let _ = self.modals.open(INFO_DIALOG, &mut self.focus);
    }

    pub fn close_info(&mut self) {
        if self
            .modals
            .top()
            .is_some_and(|id| id.as_str() == INFO_DIALOG)
        {
            self.modals.close(&mut self.focus);
        }
        // Invalidate any worker still running: its result must not land in a
        // dialog opened later for a different sandbox.
        self.info = None;
        self.info_refresh = None;
    }

    /// Show the published ports of the sandbox the Actions pane targets.
    pub fn open_ports_selected(&mut self) {
        if let Some(name) = self.selected_name()
            && self.is_running(&name)
        {
            self.open_ports(&name);
        }
    }

    /// Open (or refill) the ports dialog. Listing ports is a quick local `sbx`
    /// call, so the table is ready on the first frame; a failure is shown in
    /// the dialog instead of blocking it.
    pub fn open_ports(&mut self, name: &str) {
        if !self.is_running(name) {
            return;
        }
        self.select_named(name);
        self.port_form = PortForm::new();
        let (ports, error) = match fetch_ports(name) {
            Ok(ports) => (ports, None),
            Err(error) => (Vec::new(), Some(error)),
        };
        self.ports = Some(PortsView {
            name: name.to_owned(),
            ports,
            error,
        });
        let _ = self.modals.open(PORTS_DIALOG, &mut self.focus);
    }

    pub fn close_ports(&mut self) {
        if self
            .modals
            .top()
            .is_some_and(|id| id.as_str() == PORTS_DIALOG)
        {
            self.modals.close(&mut self.focus);
        }
        self.ports = None;
    }

    /// Re-read the table after a publish, keeping the dialog open.
    #[cfg_attr(test, allow(dead_code))]
    fn refresh_ports(&mut self) {
        let Some(view) = self.ports.as_mut() else {
            return;
        };
        let name = view.name.clone();
        match fetch_ports(&name) {
            Ok(ports) => {
                view.ports = ports;
                view.error = None;
            }
            Err(error) => view.error = Some(error),
        }
    }

    /// The **Add port** button: validate both fields, then run
    /// `sbx port publish <name> <host>:<sandbox>` and refresh the table.
    pub fn publish_port(&mut self) {
        if self.mode() != Mode::Ports {
            return;
        }
        let Some(name) = self.ports.as_ref().map(|view| view.name.clone()) else {
            return;
        };
        if let Some(error) = self.port_form.first_error() {
            if let Some(view) = self.ports.as_mut() {
                view.error = Some(error.clone());
            }
            self.notify_error("Invalid port", error);
            return;
        }

        let pending = self.port_form.pending_command(&name);

        #[cfg(test)]
        {
            self.published_for_test = Some(pending.preview());
        }

        #[cfg(not(test))]
        let host = self.port_form.port(PortField::Host).unwrap_or(0);
        #[cfg(not(test))]
        let sandbox = self.port_form.port(PortField::Sandbox).unwrap_or(0);

        #[cfg(not(test))]
        match pending.run() {
            Ok(output) if output.status.success() => {
                self.refresh_ports();
                self.notify_success(format!("Published {host}:{sandbox} for '{name}'."), None);
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let detail = stderr
                    .lines()
                    .find(|line| !line.trim().is_empty())
                    .unwrap_or("sbx port publish failed")
                    .to_owned();
                if let Some(view) = self.ports.as_mut() {
                    view.error = Some(detail.clone());
                }
                self.notify_error("Could not publish port", detail);
            }
            Err(error) => {
                let detail = error.to_string();
                if let Some(view) = self.ports.as_mut() {
                    view.error = Some(detail.clone());
                }
                self.notify_error("Could not publish port", detail);
            }
        }
    }

    /// The unpublish button on a table row: run
    /// `sbx port unpublish <name> <host>:<sandbox>` and refresh the table.
    pub fn unpublish_port(&mut self, host: u16, sandbox: u16) {
        if self.mode() != Mode::Ports {
            return;
        }
        let Some(name) = self.ports.as_ref().map(|view| view.name.clone()) else {
            return;
        };
        let pending = PendingCommand::new(
            "sbx",
            &["port", "unpublish", &name, &format!("{host}:{sandbox}")],
        );

        #[cfg(test)]
        {
            self.unpublished_for_test = Some(pending.preview());
        }

        #[cfg(not(test))]
        match pending.run() {
            Ok(output) if output.status.success() => {
                self.refresh_ports();
                self.notify_success(format!("Unpublished {host}:{sandbox} for '{name}'."), None);
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let detail = stderr
                    .lines()
                    .find(|line| !line.trim().is_empty())
                    .unwrap_or("sbx port unpublish failed")
                    .to_owned();
                if let Some(view) = self.ports.as_mut() {
                    view.error = Some(detail.clone());
                }
                self.notify_error("Could not unpublish port", detail);
            }
            Err(error) => {
                let detail = error.to_string();
                if let Some(view) = self.ports.as_mut() {
                    view.error = Some(detail.clone());
                }
                self.notify_error("Could not unpublish port", detail);
            }
        }
    }

    pub fn toggle_selected(&mut self) {
        let Some(name) = self.selected_name() else {
            return;
        };
        if self.is_running(&name) {
            self.request_action(PendingAction::Stop(name));
        } else {
            self.request_action(PendingAction::Start(name));
        }
    }

    /// The Actions pane only offers Start while a sandbox is stopped.
    pub fn start_selected(&mut self) {
        if let Some(name) = self.selected_name()
            && !self.is_running(&name)
        {
            self.request_action(PendingAction::Start(name));
        }
    }

    /// The Actions pane only offers Stop while a sandbox is running.
    pub fn stop_selected(&mut self) {
        if let Some(name) = self.selected_name()
            && self.is_running(&name)
        {
            self.request_action(PendingAction::Stop(name));
        }
    }

    /// Start/Stop/Remove never run directly from a button or shortcut: they
    /// first park here so the user can confirm the action in a dialog.
    fn request_action(&mut self, action: PendingAction) {
        if self.mode() != Mode::Normal {
            return;
        }
        self.pending_action = Some(action);
        let _ = self.modals.open(CONFIRM_DIALOG, &mut self.focus);
    }

    /// The dialog's Cancel button or Escape.
    pub fn cancel_action(&mut self) {
        let Some(action) = self.pending_action.take() else {
            return;
        };
        if self
            .modals
            .top()
            .is_some_and(|id| id.as_str() == CONFIRM_DIALOG)
        {
            self.modals.close(&mut self.focus);
        }
        self.notify_info(format!("{} cancelled.", action.verb()), None);
    }

    /// The dialog's confirm button. Recheck the sandbox state before acting:
    /// the five-second status ticker may have changed it while the dialog was
    /// open, and a removed sandbox should never be acted on.
    pub fn confirm_action(&mut self) {
        let Some(action) = self.pending_action.take() else {
            return;
        };
        if self
            .modals
            .top()
            .is_some_and(|id| id.as_str() == CONFIRM_DIALOG)
        {
            self.modals.close(&mut self.focus);
        }
        let should_be_running = matches!(&action, PendingAction::Stop(_));
        let still_valid = match &action {
            PendingAction::Start(name) | PendingAction::Stop(name) => {
                self.has_sandbox(name) && self.is_running(name) == should_be_running
            }
            PendingAction::Remove(name) => self.has_sandbox(name),
        };
        if still_valid {
            self.perform_action(action);
        } else {
            self.notify_info("Sandbox state changed; action cancelled.", None);
        }
    }

    fn perform_action(&mut self, action: PendingAction) {
        #[cfg(test)]
        {
            self.confirmed_for_test = Some(action);
        }
        #[cfg(not(test))]
        match action {
            PendingAction::Start(name) => self.start_sandbox(&name),
            PendingAction::Stop(name) => self.stop_sandbox(&name),
            PendingAction::Remove(name) => self.remove_sandbox(&name),
        }
    }

    #[cfg_attr(test, allow(dead_code))]
    fn start_sandbox(&mut self, name: &str) {
        let command = Self::start_command(name);
        self.run_sbx(command, format!("Sandbox '{name}' started."));
    }

    /// `sbx` has no standalone start verb. `run --name` re-attaches to the
    /// existing sandbox and starts it; `-d` leaves it running detached.
    fn start_command(name: &str) -> PendingCommand {
        PendingCommand::new("sbx", &["run", "-d", "--name", name])
    }

    #[cfg_attr(test, allow(dead_code))]
    fn stop_sandbox(&mut self, name: &str) {
        let command = PendingCommand::new("sbx", &["stop", name]);
        self.run_sbx(command, format!("Sandbox '{name}' stopped."));
    }

    pub fn exec_selected(&mut self) {
        let Some(name) = self.selected_name() else {
            return;
        };
        if !self.is_running(&name) {
            return;
        }
        self.pending_external = Some(PendingCommand::new("sbx", &["exec", "-it", &name, "bash"]));
        self.notify_info(format!("Opening external terminal for '{name}'..."), None);
    }

    pub fn remove_selected(&mut self) {
        if let Some(name) = self.selected_name() {
            self.request_action(PendingAction::Remove(name));
        }
    }

    #[cfg_attr(test, allow(dead_code))]
    fn remove_sandbox(&mut self, name: &str) {
        // The TUI has no TTY to answer sbx's confirmation prompt, so removal
        // is explicit and non-interactive here.
        let command = PendingCommand::new("sbx", &["remove", name, "--force"]);
        self.run_sbx(command, format!("Sandbox '{name}' removed."));
    }

    pub fn open_selected_info(&mut self) {
        if let Some(name) = self.selected_name()
            && self.is_running(&name)
        {
            self.open_info(&name);
        }
    }

    fn is_running(&self, name: &str) -> bool {
        self.sandboxes
            .iter()
            .find(|sandbox| sandbox.name == name)
            .is_some_and(Sandbox::is_running)
    }

    fn has_sandbox(&self, name: &str) -> bool {
        self.sandboxes.iter().any(|sandbox| sandbox.name == name)
    }

    fn start_creation(&mut self, pending: PendingCommand, name: &str) {
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let _ = sender.send(pending.run());
        });
        self.creation = Some(Creation {
            name: name.to_owned(),
            result: receiver,
        });
        self.spinner_index = 0;
        let now = self.now();
        self.toasts.push(
            Toast::loading(format!("Creating sandbox '{name}'"))
                .with_id(CREATE_TOAST)
                .persistent(),
            now,
        );
        self.message = format!("Creating sandbox '{name}'...");
    }

    pub fn refresh_sandboxes(&mut self) {
        match fetch_sandboxes() {
            Ok(sandboxes) => self.apply_statuses(sandboxes),
            Err(error) => self.notify_error("Could not list sandboxes", error),
        }
    }

    /// Periodic tick: keep running-state dots (and an open Info modal's status
    /// line) current without re-running the in-guest metrics commands.
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
        let last = self.sandboxes.len().saturating_sub(1);
        self.cursor = self.cursor.min(last);
        self.selected = self.selected.min(last);
        self.list_offset = self.list_offset.min(last);
    }

    #[cfg_attr(test, allow(dead_code))]
    fn run_sbx(&mut self, pending: PendingCommand, success: String) {
        self.handle_sbx_result(pending.run(), success);
    }

    fn handle_sbx_result(&mut self, result: io::Result<std::process::Output>, success: String) {
        match result {
            Ok(output) if output.status.success() => {
                self.refresh_statuses();
                self.notify_success(success, None);
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let detail = stderr
                    .lines()
                    .find(|line| !line.trim().is_empty())
                    .unwrap_or("sbx command failed")
                    .to_owned();
                self.notify_error("Could not run sbx", detail);
            }
            Err(error) => self.notify_error("Could not run sbx", error.to_string()),
        }
    }

    fn finish_creation(&mut self, result: io::Result<std::process::Output>, name: &str) {
        let succeeded = matches!(&result, Ok(output) if output.status.success());
        let _ = self.toasts.dismiss(CREATE_TOAST);
        self.handle_sbx_result(result, format!("Sandbox '{name}' created."));
        if succeeded {
            self.close_wizard();
        }
        // On failure the dialog stays open so the settings can be corrected or
        // the create can be retried.
    }
}

#[cfg(test)]
impl App {
    /// Put the app into the same state `start_creation` does, without spawning
    /// a real `sbx` process. The sender is dropped, so `poll_creation` observes
    /// a disconnected worker.
    pub fn begin_creation_for_test(&mut self, name: &str) {
        let (sender, receiver) = mpsc::channel();
        drop(sender);
        self.creation = Some(Creation {
            name: name.to_owned(),
            result: receiver,
        });
        self.spinner_index = 0;
        self.message = format!("Creating sandbox '{name}'...");
        self.toasts.push(
            Toast::loading(format!("Creating sandbox '{name}'"))
                .with_id(CREATE_TOAST)
                .persistent(),
            self.now(),
        );
        let _ = self.modals.open(CREATE_DIALOG, &mut self.focus);
    }

    pub fn complete_creation_for_test(
        &mut self,
        name: &str,
        result: io::Result<std::process::Output>,
    ) {
        self.creation = None;
        self.finish_creation(result, name);
    }

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
        app
    }

    pub fn show_info_for_test(&mut self, details: SandboxDetails) {
        self.info = Some(details);
        let _ = self.modals.open(INFO_DIALOG, &mut self.focus);
    }

    /// Open the ports dialog with a known table, without calling `sbx`.
    pub fn show_ports_for_test(&mut self, name: &str, ports: Vec<PortMapping>) {
        self.select_named(name);
        self.ports = Some(PortsView {
            name: name.to_owned(),
            ports,
            error: None,
        });
        let _ = self.modals.open(PORTS_DIALOG, &mut self.focus);
    }

    /// The command the ports dialog handed to `perform_action`-style test hook.
    pub fn take_published_for_test(&mut self) -> Option<String> {
        self.published_for_test.take()
    }

    /// The command a row's unpublish button handed to the test hook.
    pub fn take_unpublished_for_test(&mut self) -> Option<String> {
        self.unpublished_for_test.take()
    }

    /// Queue a fake metrics result so `poll_info` can be tested without
    /// running `sbx exec` inside the guest.
    pub fn queue_info_result_for_test(&mut self, name: &str, details: SandboxDetails) {
        let (sender, receiver) = mpsc::channel();
        sender.send(details).expect("test channel");
        self.info_refresh = Some(InfoRefresh {
            name: name.to_owned(),
            result: receiver,
        });
    }

    /// The action a confirmed dialog handed to `perform_action`. Tests use
    /// this instead of letting the app shell out to `sbx`.
    pub fn take_confirmed_for_test(&mut self) -> Option<PendingAction> {
        self.confirmed_for_test.take()
    }
}

#[cfg(test)]
mod tests;
