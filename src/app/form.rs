use std::ffi::OsStr;
use std::path::PathBuf;

use ratcn::InputState;

use super::{AGENTS, PendingCommand};

/// The editable fields shown in the themed create dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormField {
    Name,
    RootSize,
    DockerSize,
    Cpus,
    Memory,
}

/// How the workspace chosen in the create dialog is attached to a sandbox.
/// `direct` bind-mounts the host directory; `clone` copies the repository into
/// the container and keeps its commits available through `sbx`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspaceMode {
    Direct,
    Clone,
}

impl WorkspaceMode {
    /// Labels for ratcn's `Cycle`, in cycling order.
    pub const OPTIONS: [&'static str; 2] = ["direct", "clone"];

    pub(super) fn from_index(index: usize) -> Self {
        if index == 1 {
            Self::Clone
        } else {
            Self::Direct
        }
    }

    pub fn index(self) -> usize {
        match self {
            Self::Direct => 0,
            Self::Clone => 1,
        }
    }
}

/// Text and selection state for the create dialog. Ratcn's `Input`, `Select`
/// and `Cycle` components read and replace these values through messages.
#[derive(Clone)]
pub struct Form {
    pub(super) name: InputState,
    pub(super) agent_index: usize,
    pub(super) agent_focus: usize,
    pub(super) agent_open: bool,
    pub(super) root_size: InputState,
    pub(super) docker_size: InputState,
    pub(super) cpus: InputState,
    pub(super) memory: InputState,
    pub(super) add_workspace: bool,
    pub(super) workspace_path: String,
    pub(super) workspace_mode: WorkspaceMode,
}

impl Form {
    pub(super) fn new() -> Self {
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
            cpus: InputState::new(""),
            memory: InputState::new(""),
            add_workspace: false,
            workspace_path: default_workspace_path(),
            workspace_mode: WorkspaceMode::Direct,
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

    pub fn add_workspace(&self) -> bool {
        self.add_workspace
    }

    pub fn workspace_path(&self) -> &str {
        &self.workspace_path
    }

    pub fn workspace_mode(&self) -> WorkspaceMode {
        self.workspace_mode
    }

    pub fn set_add_workspace(&mut self, enabled: bool) {
        self.add_workspace = enabled;
    }

    pub fn set_workspace_mode(&mut self, index: usize) {
        self.workspace_mode =
            WorkspaceMode::from_index(index.min(WorkspaceMode::OPTIONS.len() - 1));
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
        format!(
            "{} && {}",
            self.create_command().preview(),
            self.keep_alive_command().preview()
        )
    }

    pub fn create_command(&self) -> PendingCommand {
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

        if self.add_workspace {
            if self.workspace_mode == WorkspaceMode::Clone {
                args.push("--clone".to_owned());
            }
            // The workspace is the command's final positional argument.
            args.push(self.workspace_path.trim().to_owned());
        }

        PendingCommand {
            program: "sbx".to_owned(),
            args,
            env,
            shell_preview: None,
        }
    }

    /// Keep the newly created sandbox active. `sbx create` can leave it idle and
    /// it may close; detaching a run keeps it around for later attach/exec.
    pub fn keep_alive_command(&self) -> PendingCommand {
        let name = self.name.value().trim();
        PendingCommand::new("sbx", &["run", "-d", "--name", name])
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

pub(super) fn validate_name(name: &str) -> Result<(), String> {
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
pub(super) fn validate_gb_size(value: &str) -> Result<(), String> {
    if value.is_empty() || value.bytes().all(|byte| byte.is_ascii_digit()) {
        Ok(())
    } else {
        Err("Enter a whole number of GB, e.g. 40.".to_owned())
    }
}

pub(super) fn validate_cpus(value: &str) -> Result<(), String> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(());
    }
    value
        .parse::<u32>()
        .map(|_| ())
        .map_err(|_| "CPUs must be a whole number (0 = auto).".to_owned())
}

pub(super) fn parse_memory_mib(value: &str) -> Option<u64> {
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

pub(super) fn validate_memory(value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Ok(());
    }
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

/// The workspace location offered before the user opens the folder picker.
fn default_workspace_path() -> String {
    home_dir()
        .or_else(|| std::env::current_dir().ok())
        .map(|path| path.to_string_lossy().into_owned())
        // Rare, but keep a placeholder if even the current directory is gone.
        .unwrap_or_else(|| "~".to_owned())
}

/// Windows does not normally define `HOME`, so check its home variables first
/// there. Elsewhere `HOME` wins; the Windows variables are still a useful
/// fallback when running under an unusual environment.
fn home_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME");
    let user_profile = std::env::var_os("USERPROFILE");
    let home_drive = std::env::var_os("HOMEDRIVE");
    let home_path = std::env::var_os("HOMEPATH");

    let unix_home = non_empty(home.as_deref()).map(PathBuf::from);
    let windows_home = windows_home_from(
        user_profile.as_deref(),
        home_drive.as_deref(),
        home_path.as_deref(),
    );

    #[cfg(windows)]
    let result = windows_home.or(unix_home);
    #[cfg(not(windows))]
    let result = unix_home.or(windows_home);
    result
}

fn non_empty(value: Option<&OsStr>) -> Option<&OsStr> {
    value.filter(|value| !value.is_empty())
}

/// `USERPROFILE` is the modern Windows home; older setups only provide the
/// `HOMEDRIVE` + `HOMEPATH` pair.
pub(super) fn windows_home_from(
    user_profile: Option<&OsStr>,
    home_drive: Option<&OsStr>,
    home_path: Option<&OsStr>,
) -> Option<PathBuf> {
    if let Some(profile) = non_empty(user_profile) {
        return Some(PathBuf::from(profile));
    }
    match (non_empty(home_drive), non_empty(home_path)) {
        (Some(drive), Some(path)) => Some(PathBuf::from(drive).join(path)),
        _ => None,
    }
}
