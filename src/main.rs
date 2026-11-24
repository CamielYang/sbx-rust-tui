mod app;
mod ui;

use std::io::{self, Stdout};
use std::time::Duration;

use app::App;
use crossterm::{
    cursor,
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind},
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
    execute!(
        io::stdout(),
        EnterAlternateScreen,
        EnableMouseCapture,
        cursor::Hide
    )?;
    let terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    Ok(terminal)
}

fn restore_terminal(terminal: &mut Tui) -> Result<()> {
    terminal.show_cursor()?;
    disable_raw_mode()?;
    execute!(
        io::stdout(),
        LeaveAlternateScreen,
        DisableMouseCapture,
        cursor::Show
    )?;
    Ok(())
}

/// Make sure a panic doesn't leave the terminal in raw mode / alternate screen.
fn install_panic_hook() {
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = disable_raw_mode();
        let _ = execute!(
            io::stdout(),
            LeaveAlternateScreen,
            DisableMouseCapture,
            cursor::Show
        );
        original_hook(panic_info);
    }));
}

fn run(terminal: &mut Tui, app: &mut App) -> Result<()> {
    while !app.should_quit() {
        terminal.draw(|frame| ui::render(frame, app))?;

        // Exec opens a real terminal, so leave the alternate screen and raw
        // mode first, then rebuild the TUI once the child is done.
        if let Some(pending) = app.take_external_command() {
            restore_terminal(terminal)?;
            let result = pending.run_in_external_terminal();
            *terminal = init_terminal()?;
            match result {
                Ok(()) => app.set_message("External terminal closed."),
                Err(error) => app.set_message(format!("Could not open terminal: {error}")),
            }
        }

        if event::poll(Duration::from_millis(100))? {
            match event::read()? {
                // Ignore key-release and repeat events; handle each press once.
                Event::Key(key) if key.kind == KeyEventKind::Press => app.handle_key(key),
                Event::Mouse(mouse) => app.handle_mouse(mouse),
                _ => {}
            }
        }
    }
    Ok(())
}
