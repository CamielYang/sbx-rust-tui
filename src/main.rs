mod app;
mod ui;

use std::io::{self, Stdout};
use std::time::{Duration, Instant};

use app::{App, INFO_REFRESH_INTERVAL, Mode, STATUS_REFRESH_INTERVAL};
use crossterm::{
    cursor,
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use ratcn::crossterm::InputModes;
use ratcn::runtime::{EventResult, Ratcn, TabWrap};
use ratcn::{Theme, ToasterWidget};

use ui::Msg;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
type Tui = Terminal<CrosstermBackend<Stdout>>;

/// Read input at roughly 60 Hz. A 100 ms poll made every click feel delayed;
/// CLI work now runs on workers, so polling this often is cheap.
const INPUT_POLL_INTERVAL: Duration = Duration::from_millis(16);

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
    let theme = Theme::default();
    let mut ratcn = Ratcn::new()
        .focus(|state: &App| state.focus_ref(), Msg::FocusChanged)
        .modals(|state: &App| state.modals_ref())
        .tab_wrap(TabWrap::Wrap)
        .hover_focus();
    // Keep the guard alive for the event loop: ratcn needs mouse input for its
    // components, and Input for pasted values. It is dropped around Exec so the
    // external terminal gets a clean input mode, then re-enabled on resume.
    let mut input_modes = Some(InputModes::new().mouse().paste().enable()?);

    let mut next_status_check = Instant::now() + STATUS_REFRESH_INTERVAL;
    let mut next_info_check = Instant::now() + INFO_REFRESH_INTERVAL;

    while !app.should_quit() {
        // Collect finished worker results before drawing so the UI shows fresh
        // data on the very next frame. None of these calls wait on a process.
        app.poll_jobs();
        app.poll_creation();
        app.poll_info();
        app.tick();
        draw(terminal, app, &mut ratcn, &theme)?;

        // Exec opens a real terminal, so leave the alternate screen and raw
        // mode first, then rebuild the TUI once the child is done.
        if let Some(pending) = app.take_external_command() {
            // Dropping the guard turns bracketed paste and mouse capture back
            // off; the external shell may change terminal modes itself.
            input_modes.take();
            restore_terminal(terminal)?;
            let result = pending.run_in_external_terminal();
            *terminal = init_terminal()?;
            input_modes = Some(InputModes::new().mouse().paste().enable()?);
            if let Err(error) = result {
                app.notify_error("Could not open terminal", error);
            }
        }

        let now = Instant::now();
        if now >= next_status_check {
            // Status listing is queued on a worker. Skip it while creating so
            // the spinner keeps animating; the next tick refreshes instead.
            if !app.is_creating() {
                app.refresh_statuses();
            }
            next_status_check = now + STATUS_REFRESH_INTERVAL;
        }

        if now >= next_info_check {
            // Only an open Info dialog needs live metrics. The guest-side CPU
            // sample runs on a worker, so this never blocks the event loop.
            if app.mode() == Mode::Info {
                app.refresh_info();
            }
            next_info_check = now + INFO_REFRESH_INTERVAL;
        }

        if event::poll(INPUT_POLL_INTERVAL)? {
            let event = event::read()?;
            if let Event::Key(key) = event {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                // Global escape hatch, available even while creation locks the UI.
                if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    app.quit();
                    continue;
                }
                match ratcn.handle_event(event, app) {
                    EventResult::Emit(message) => ui::apply(app, message),
                    EventResult::Consumed => {}
                    // Unclaimed keys become app shortcuts (N, R, S, X, I, D, q).
                    EventResult::Ignored => app.handle_global_key(key),
                }
            } else if let EventResult::Emit(message) = ratcn.handle_event(event, app) {
                ui::apply(app, message);
            }
        }
    }
    Ok(())
}

fn draw(
    terminal: &mut Tui,
    app: &mut App,
    ratcn: &mut Ratcn<App, Msg>,
    theme: &Theme,
) -> Result<()> {
    // A status tick may hide the action button that held focus; move focus to
    // a control that is still on screen before ratcn resolves it.
    ui::settle_focus(app);
    terminal.draw(|frame| {
        let now = app.now();
        app.prune_toasts(now);
        let area = frame.area();
        ratcn.render(frame, area, app, theme, |ctx| ui::declare(ctx, app, theme));
        // Toasts are themed ratcn widgets but take no input, so they are drawn
        // as the final layer by the host, as in ratcn's own examples.
        frame.render_widget(ToasterWidget::new(app.toasts(), now).themed(theme), area);
    })?;
    Ok(())
}
