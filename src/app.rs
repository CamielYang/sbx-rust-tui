use std::io;
use std::process::Command as ProcessCommand;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

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

/// An action bound to a rectangle on screen by the renderer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClickAction {
    Select(usize),
    OpenWizard,
    CloseWizard,
    CreateSandbox,
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

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Wizard,
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

    pub fn name(&self) -> &str {
        &self.name
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
            WizardStep::RootSize => validate_size(&self.root_size).err(),
            WizardStep::DockerSize => validate_size(&self.docker_size).err(),
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

// --- sandbox list -----------------------------------------------------------

/// Fetch the names of the sandboxes known to `sbx` (one per line with `-q`).
pub fn list_sandboxes() -> Result<Vec<String>, String> {
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
        .map(str::to_owned)
        .collect())
}

// --- app --------------------------------------------------------------------

pub struct App {
    sandboxes: Vec<String>,
    selected: usize,
    grid_offset: usize,
    visible_rows: usize,
    mode: Mode,
    wizard: Wizard,
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

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn sandboxes(&self) -> &[String] {
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
        }
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
        self.execute(pending, name);
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
        match list_sandboxes() {
            Ok(names) => self.sandboxes = names,
            Err(error) => self.message = format!("Could not list sandboxes: {error}"),
        }
        self.clamp_navigation();
    }

    fn execute(&mut self, pending: PendingCommand, name: &str) {
        match pending.run() {
            Ok(output) if output.status.success() => {
                self.refresh_sandboxes();
                if self.message.is_empty() {
                    self.message = format!("Sandbox '{name}' created.");
                }
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let detail = stderr
                    .lines()
                    .find(|line| !line.trim().is_empty())
                    .unwrap_or("sbx create failed");
                self.message = format!("Could not create sandbox: {detail}");
            }
            Err(error) => self.message = format!("Could not run sbx: {error}"),
        }
    }
}

#[cfg(test)]
impl App {
    pub fn with_sandboxes(names: &[&str]) -> Self {
        let mut app = Self::new();
        app.sandboxes = names.iter().map(|name| (*name).to_owned()).collect();
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
