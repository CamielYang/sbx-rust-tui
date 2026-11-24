use std::io;
use std::process::Command as ProcessCommand;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

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

/// A ready-to-run command, including any environment overrides for the child.
#[derive(Clone)]
pub struct PendingCommand {
    program: String,
    args: Vec<String>,
    env: Vec<(String, String)>,
}

impl PendingCommand {
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
}

fn quote_if_needed(part: &str) -> String {
    if part.contains(char::is_whitespace) {
        format!("'{part}'")
    } else {
        part.to_owned()
    }
}

/// A command that can be launched from the TUI.
///
/// Add your own entries to `MENU` below. `{sandbox}` in `args` is replaced
/// with the `SANDBOX_NAME` environment variable at launch time.
#[derive(Clone, Copy)]
pub struct CommandSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub program: &'static str,
    pub args: &'static [&'static str],
}

impl CommandSpec {
    fn resolved_args(&self) -> Vec<String> {
        let sandbox = std::env::var("SANDBOX_NAME").unwrap_or_default();
        self.args
            .iter()
            .map(|arg| arg.replace("{sandbox}", &sandbox))
            .collect()
    }

    fn to_pending(self) -> PendingCommand {
        PendingCommand {
            program: self.program.to_owned(),
            args: self.resolved_args(),
            env: Vec::new(),
        }
    }

    pub fn preview(&self) -> String {
        self.to_pending().preview()
    }
}

#[derive(Clone)]
pub enum MenuItem {
    CreateSandbox,
    Command(CommandSpec),
}

impl MenuItem {
    pub fn name(&self) -> &'static str {
        match self {
            Self::CreateSandbox => "Create new sandbox (wizard)",
            Self::Command(command) => command.name,
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            Self::CreateSandbox => {
                "Walk through name, agent, disk sizes, CPUs and memory, then run sbx create."
            }
            Self::Command(command) => command.description,
        }
    }

    pub fn preview(&self) -> String {
        match self {
            Self::CreateSandbox => "opens an interactive create wizard".to_owned(),
            Self::Command(command) => command.preview(),
        }
    }
}

const MENU: &[MenuItem] = &[
    MenuItem::CreateSandbox,
    MenuItem::Command(CommandSpec {
        name: "List sandboxes",
        description: "Show all local and cloud sandboxes.",
        program: "sbx",
        args: &["ls"],
    }),
    MenuItem::Command(CommandSpec {
        name: "Inspect this sandbox",
        description: "Show details for the sandbox this TUI is running in.",
        program: "sbx",
        args: &["inspect", "{sandbox}"],
    }),
    MenuItem::Command(CommandSpec {
        name: "Published ports",
        description: "List ports published for the current sandbox.",
        program: "sbx",
        args: &["port", "ls", "{sandbox}"],
    }),
    MenuItem::Command(CommandSpec {
        name: "Network rules",
        description: "List active sandbox network policy rules.",
        program: "sbx",
        args: &["policy", "ls"],
    }),
    MenuItem::Command(CommandSpec {
        name: "Network log",
        description: "Show recent connections and why they were blocked.",
        program: "sbx",
        args: &["policy", "log"],
    }),
    MenuItem::Command(CommandSpec {
        name: "Docker containers",
        description: "Show running Docker containers.",
        program: "docker",
        args: &["ps"],
    }),
    MenuItem::Command(CommandSpec {
        name: "sbx version",
        description: "Show the installed Docker Sandboxes version.",
        program: "sbx",
        args: &["version"],
    }),
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Confirm,
    Wizard,
}

pub struct CommandResult {
    pub command: String,
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

// --- wizard -----------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
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
            Self::Name => "Sandbox name (2-63 ASCII letters, numbers, '-' or '.')",
            Self::Agent => "Pick an agent",
            Self::RootSize => "Root filesystem size, e.g. 40g (leave empty to skip)",
            Self::DockerSize => "Docker data size, e.g. 20g (leave empty to skip)",
            Self::Cpus => "Number of CPUs (positive integer, 0 = auto)",
            Self::Memory => "Memory limit in binary units, e.g. 512m or 8g",
            Self::Confirm => "Review the command before creating the sandbox",
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

    pub fn field_error(&self) -> Option<String> {
        match self.step {
            WizardStep::Name if self.name.is_empty() => None,
            WizardStep::Name => validate_name(&self.name).err(),
            WizardStep::RootSize => validate_size(&self.root_size).err(),
            WizardStep::DockerSize => validate_size(&self.docker_size).err(),
            WizardStep::Cpus => validate_cpus(&self.cpus).err(),
            WizardStep::Memory => validate_memory(&self.memory).err(),
            _ => None,
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
                self.root_size.trim().to_owned(),
            ));
        }
        if !self.docker_size.trim().is_empty() {
            env.push((
                "DOCKER_SANDBOXES_DOCKER_SIZE".to_owned(),
                self.docker_size.trim().to_owned(),
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

fn validate_size(value: &str) -> Result<(), String> {
    if value.is_empty() {
        return Ok(());
    }
    let split = value
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(value.len());
    let (number, unit) = value.split_at(split);
    let valid = !number.is_empty()
        && matches!(
            unit.to_ascii_lowercase().as_str(),
            "b" | "k"
                | "m"
                | "g"
                | "t"
                | "kb"
                | "mb"
                | "gb"
                | "tb"
                | "ki"
                | "mi"
                | "gi"
                | "ti"
                | "kib"
                | "mib"
                | "gib"
                | "tib"
        );
    if valid {
        Ok(())
    } else {
        Err("Use a size like 40g or 20g.".to_owned())
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

// --- app --------------------------------------------------------------------

pub struct App {
    menu: Vec<MenuItem>,
    selected: usize,
    mode: Mode,
    wizard: Wizard,
    result: Option<CommandResult>,
    output_scroll: u16,
    message: String,
    should_quit: bool,
}

impl App {
    pub fn new() -> Self {
        Self {
            menu: MENU.to_vec(),
            selected: 0,
            mode: Mode::Normal,
            wizard: Wizard::new(),
            result: None,
            output_scroll: 0,
            message: String::new(),
            should_quit: false,
        }
    }

    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    pub fn menu(&self) -> &[MenuItem] {
        &self.menu
    }

    pub fn selected_index(&self) -> usize {
        self.selected
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn wizard(&self) -> &Wizard {
        &self.wizard
    }

    pub fn result(&self) -> Option<&CommandResult> {
        self.result.as_ref()
    }

    pub fn output_scroll(&self) -> u16 {
        self.output_scroll
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }

        match self.mode {
            Mode::Normal => match key.code {
                KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
                KeyCode::Up | KeyCode::Char('k') => self.select_previous(),
                KeyCode::Down | KeyCode::Char('j') => self.select_next(),
                KeyCode::Enter => self.open_selected(),
                KeyCode::PageUp => self.scroll_output(-10),
                KeyCode::PageDown => self.scroll_output(10),
                KeyCode::Char('c') => {
                    self.result = None;
                    self.output_scroll = 0;
                    self.message.clear();
                }
                _ => {}
            },
            Mode::Confirm => match key.code {
                KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
                    if let Some(MenuItem::Command(command)) = self.menu.get(self.selected) {
                        let pending = command.to_pending();
                        self.execute(pending);
                    }
                    self.mode = Mode::Normal;
                }
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                    self.mode = Mode::Normal;
                    self.message = "Cancelled.".to_owned();
                }
                _ => {}
            },
            Mode::Wizard => match self.wizard.handle_key(key, &mut self.message) {
                WizardOutcome::Continue => {}
                WizardOutcome::Cancelled => {
                    self.mode = Mode::Normal;
                    self.message = "Create sandbox cancelled.".to_owned();
                }
                WizardOutcome::Create(pending) => {
                    self.execute(pending);
                    self.mode = Mode::Normal;
                }
            },
        }
    }

    fn open_selected(&mut self) {
        self.message.clear();
        match self.menu.get(self.selected) {
            Some(MenuItem::CreateSandbox) => {
                self.wizard = Wizard::new();
                self.mode = Mode::Wizard;
            }
            Some(MenuItem::Command(_)) => self.mode = Mode::Confirm,
            None => {}
        }
    }

    fn select_previous(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    fn select_next(&mut self) {
        if self.selected + 1 < self.menu.len() {
            self.selected += 1;
        }
    }

    fn scroll_output(&mut self, delta: i32) {
        self.output_scroll = (self.output_scroll as i32 + delta).max(0) as u16;
    }

    fn execute(&mut self, pending: PendingCommand) {
        let preview = pending.preview();
        match pending.run() {
            Ok(output) => {
                let success = output.status.success();
                self.message = if success {
                    "Command finished.".to_owned()
                } else {
                    format!("Command exited with {}.", output.status)
                };
                self.result = Some(CommandResult {
                    command: preview,
                    success,
                    stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                    stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                });
            }
            Err(error) => {
                self.message = format!("Could not start command: {error}");
                self.result = Some(CommandResult {
                    command: preview,
                    success: false,
                    stdout: String::new(),
                    stderr: error.to_string(),
                });
            }
        }
        self.output_scroll = 0;
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
        assert!(validate_size("").is_ok());
        assert!(validate_size("40g").is_ok());
        assert!(validate_size("20GiB").is_ok());
        assert!(validate_size("40x").is_err());
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
        wizard.root_size = "40g".to_owned();
        wizard.docker_size = "20g".to_owned();
        wizard.cpus = "2".to_owned();
        wizard.memory = "4g".to_owned();

        assert_eq!(
            wizard.preview(),
            "DOCKER_SANDBOXES_ROOT_SIZE=40g DOCKER_SANDBOXES_DOCKER_SIZE=20g sbx create --name demo-tui opencode --cpus 2 --memory 4g"
        );
    }
}
