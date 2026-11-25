use std::io;
use std::process::Command as ProcessCommand;

/// A ready-to-run command, including any environment overrides for the child.
#[derive(Clone)]
pub struct PendingCommand {
    pub(super) program: String,
    pub(super) args: Vec<String>,
    pub(super) env: Vec<(String, String)>,
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

/// The most useful error line from a failed command. Prefer an `error:` line
/// (and a following `try:` hint) over unrelated progress output; fall back to
/// the first non-empty stderr line when no explicit error line is present.
pub(super) fn stderr_detail(output: &std::process::Output, fallback: &str) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let lines: Vec<&str> = stderr.lines().collect();
    if let Some(index) = lines.iter().position(|line| {
        line.trim_start()
            .to_ascii_lowercase()
            .starts_with("error:")
    }) {
        let mut detail = lines[index].trim().to_owned();
        if let Some(try_line) = lines[index + 1..]
            .iter()
            .map(|line| line.trim())
            .find(|line| !line.is_empty())
            .filter(|line| line.to_ascii_lowercase().starts_with("try:"))
        {
            detail.push('\n');
            detail.push_str(try_line);
        }
        return detail;
    }
    stderr
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or(fallback)
        .to_owned()
}

fn quote_if_needed(part: &str) -> String {
    if part.contains(char::is_whitespace) {
        format!("'{part}'")
    } else {
        part.to_owned()
    }
}
