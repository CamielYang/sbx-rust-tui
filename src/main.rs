mod app;
mod ui;

use std::io::{self, Stdout};
use std::time::Duration;

use app::App;
use crossterm::{
    cursor,
    event::{self, Event, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
type Tui = Terminal<CrosstermBackend<Stdout>>;

fn main() -> Result<()> {
    install_panic_hook();

    let mut terminal = init_terminal()?;
    let mut app = App::new();
    let result = run(&mut terminal, &mut app);

    restore_terminal(&mut terminal)?;
    result
}

fn init_terminal() -> Result<Tui> {
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen, cursor::Hide)?;
    let terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    Ok(terminal)
}

fn restore_terminal(terminal: &mut Tui) -> Result<()> {
    terminal.show_cursor()?;
    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen, cursor::Show)?;
    Ok(())
}

/// Make sure a panic doesn't leave the terminal in raw mode / alternate screen.
fn install_panic_hook() {
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, cursor::Show);
        original_hook(panic_info);
    }));
}

fn run(terminal: &mut Tui, app: &mut App) -> Result<()> {
    while !app.should_quit() {
        terminal.draw(|frame| ui::render(frame, app))?;

        // Ignore key-release and repeat events; handle each press once.
        if event::poll(Duration::from_millis(100))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            app.handle_key(key);
        }
    }
    Ok(())
}
