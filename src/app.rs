use std::process::Command;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// A command that can be launched from the TUI.
///
/// Add your own entries to `COMMANDS` below. `{sandbox}` in `args` is replaced
/// with the `SANDBOX_NAME` environment variable at launch time.
#[derive(Clone, Copy)]
pub struct CommandSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub program: &'static str,
    pub args: &'static [&'static str],
}

impl CommandSpec {
    pub fn resolved_args(&self) -> Vec<String> {
        let sandbox = std::env::var("SANDBOX_NAME").unwrap_or_default();
        self.args
            .iter()
            .map(|arg| arg.replace("{sandbox}", &sandbox))
            .collect()
    }

    pub fn preview(&self) -> String {
        render_command(self.program, &self.resolved_args())
    }
}

fn render_command(program: &str, args: &[String]) -> String {
    std::iter::once(program.to_owned())
        .chain(args.iter().cloned())
        .map(|part| {
            if part.contains(char::is_whitespace) {
                format!("'{part}'")
            } else {
                part
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        name: "List sandboxes",
        description: "Show all local and cloud sandboxes.",
        program: "sbx",
        args: &["ls"],
    },
    CommandSpec {
        name: "Inspect this sandbox",
        description: "Show details for the sandbox this TUI is running in.",
        program: "sbx",
        args: &["inspect", "{sandbox}"],
    },
    CommandSpec {
        name: "Published ports",
        description: "List ports published for the current sandbox.",
        program: "sbx",
        args: &["port", "ls", "{sandbox}"],
    },
    CommandSpec {
        name: "Network rules",
        description: "List active sandbox network policy rules.",
        program: "sbx",
        args: &["policy", "ls"],
    },
    CommandSpec {
        name: "Network log",
        description: "Show recent connections and why they were blocked.",
        program: "sbx",
        args: &["policy", "log"],
    },
    CommandSpec {
        name: "Docker containers",
        description: "Show running Docker containers.",
        program: "docker",
        args: &["ps"],
    },
    CommandSpec {
        name: "sbx version",
        description: "Show the installed Docker Sandboxes version.",
        program: "sbx",
        args: &["version"],
    },
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Confirm,
}

pub struct CommandResult {
    pub command: String,
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

pub struct App {
    commands: Vec<CommandSpec>,
    selected: usize,
    mode: Mode,
    result: Option<CommandResult>,
    output_scroll: u16,
    message: String,
    should_quit: bool,
}

impl App {
    pub fn new() -> Self {
        Self {
            commands: COMMANDS.to_vec(),
            selected: 0,
            mode: Mode::Normal,
            result: None,
            output_scroll: 0,
            message: String::new(),
            should_quit: false,
        }
    }

    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    pub fn commands(&self) -> &[CommandSpec] {
        &self.commands
    }

    pub fn selected_index(&self) -> usize {
        self.selected
    }

    pub fn mode(&self) -> Mode {
        self.mode
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
                KeyCode::Enter => {
                    self.mode = Mode::Confirm;
                    self.message.clear();
                }
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
                    self.run_selected();
                    self.mode = Mode::Normal;
                }
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                    self.mode = Mode::Normal;
                    self.message = "Cancelled.".to_owned();
                }
                _ => {}
            },
        }
    }

    fn select_previous(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    fn select_next(&mut self) {
        if self.selected + 1 < self.commands.len() {
            self.selected += 1;
        }
    }

    fn scroll_output(&mut self, delta: i32) {
        self.output_scroll = (self.output_scroll as i32 + delta).max(0) as u16;
    }

    fn run_selected(&mut self) {
        let Some(command) = self.commands.get(self.selected) else {
            return;
        };

        let args = command.resolved_args();
        let rendered = render_command(command.program, &args);

        match Command::new(command.program).args(&args).output() {
            Ok(output) => {
                let success = output.status.success();
                self.message = if success {
                    "Command finished.".to_owned()
                } else {
                    format!("Command exited with {}.", output.status)
                };
                self.result = Some(CommandResult {
                    command: rendered,
                    success,
                    stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                    stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                });
            }
            Err(error) => {
                self.message = format!("Could not start command: {error}");
                self.result = Some(CommandResult {
                    command: rendered,
                    success: false,
                    stdout: String::new(),
                    stderr: error.to_string(),
                });
            }
        }
        self.output_scroll = 0;
    }
}
