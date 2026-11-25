//! Ratcn-based dashboard for `sbx` sandboxes.
//!
//! Every control is either a ratcn component or themed paint queued through
//! ratcn's declaration context. The runtime owns focus, hover, hit testing, and
//! modal layering; [`App`](crate::app::App) owns the state and [`apply`] is the
//! single place where component messages become state changes.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratcn::runtime::{Component, DeclareCtx, FocusState};
use ratcn::{
    Button, Dialog, Input, InputState, List, ListItem, ListItemState, ProgressWidget, Select, Theme,
};

use crate::app::{AGENTS, App, CREATE_DIALOG, FormField, INFO_DIALOG, Sandbox, SandboxDetails};

/// Every interaction ratcn can emit.
#[derive(Clone, Debug)]
pub enum Msg {
    FocusChanged(FocusState),
    SandboxFocused(String, usize),
    SandboxSelected(String),
    SandboxScrolled(usize),
    OpenWizard,
    Refresh,
    Quit,
    CancelWizard,
    CreateSandbox,
    FormChanged(FormField, InputState),
    AgentOpenChanged(bool),
    AgentFocused(usize),
    AgentSelected(usize),
    ToggleSelected,
    ExecSelected,
    InfoSelected,
    RemoveSelected,
    CloseInfo,
}

/// Apply one runtime message. Creation locks the controls: late focus messages
/// are kept so the runtime stays in sync, but everything else is discarded
/// until the worker reports back.
pub fn apply(app: &mut App, msg: Msg) {
    if app.is_creating() && !matches!(msg, Msg::FocusChanged(_)) {
        return;
    }
    match msg {
        Msg::FocusChanged(focus) => *app.focus_mut() = focus,
        Msg::SandboxFocused(name, offset) => {
            app.select_named(&name);
            app.set_list_offset(offset);
        }
        Msg::SandboxSelected(name) => app.select_named(&name),
        Msg::SandboxScrolled(offset) => app.set_list_offset(offset),
        Msg::OpenWizard => app.open_wizard(),
        Msg::Refresh => {
            app.refresh_sandboxes();
            app.notify_success("Sandbox list refreshed", None);
        }
        Msg::Quit => app.quit(),
        Msg::CancelWizard => app.cancel_wizard(),
        Msg::CreateSandbox => app.create_from_form(),
        Msg::FormChanged(field, state) => app.set_form_state(field, state),
        Msg::AgentOpenChanged(open) => app.set_agent_open(open),
        Msg::AgentFocused(index) => app.set_agent_focus(index),
        Msg::AgentSelected(index) => app.set_agent_index(index),
        Msg::ToggleSelected => app.toggle_selected(),
        Msg::ExecSelected => app.exec_selected(),
        Msg::InfoSelected => app.open_selected_info(),
        Msg::RemoveSelected => app.remove_selected(),
        Msg::CloseInfo => app.close_info(),
    }
}

/// Declare one complete frame. Called from `Ratcn::render`.
pub fn declare(ctx: &mut DeclareCtx<'_, App, Msg>, app: &App, theme: &Theme) {
    let theme = *theme;
    let area = ctx.area();
    ctx.paint(move |paint| {
        paint.widget(
            Block::default().style(Style::default().bg(theme.background)),
            area,
        )
    });

    let vertical = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(area);

    paint_header(ctx, app, &theme, vertical[0]);
    paint_toolbar(ctx, app, &theme, vertical[1]);
    paint_body(ctx, app, &theme, vertical[2]);
    paint_footer(ctx, app, &theme, vertical[3]);

    // Modals are declared last so their layers cover the dashboard. Ids come
    // from App's ModalState, which Ratcn validates against this declaration.
    if app.modals_ref().is_open(CREATE_DIALOG) {
        ctx.modal(CREATE_DIALOG, create_dialog(app, &theme), area);
    }
    if app.modals_ref().is_open(INFO_DIALOG) {
        ctx.modal(INFO_DIALOG, info_dialog(app, &theme), area);
    }
}

fn paint_header(ctx: &mut DeclareCtx<'_, App, Msg>, app: &App, theme: &Theme, area: Rect) {
    let title = Span::styled(
        "  DOCKER SANDBOXES  ",
        Style::default()
            .fg(theme.primary_foreground)
            .bg(theme.primary)
            .add_modifier(Modifier::BOLD),
    );
    let subtitle = Span::styled(
        "  sbx dashboard",
        Style::default().fg(theme.muted_foreground),
    );
    let count = Span::styled(
        format!("  {} sandbox(es)  ", app.sandboxes().len()),
        Style::default().fg(theme.muted_foreground),
    );
    ctx.paint_widget(
        Paragraph::new(Line::from(vec![title, subtitle, count])),
        area,
    );
}

fn paint_toolbar(ctx: &mut DeclareCtx<'_, App, Msg>, app: &App, theme: &Theme, area: Rect) {
    let locked = app.is_creating();
    let new_button = Button::new("+ New sandbox")
        .on_press(|| Msg::OpenWizard)
        .disabled(locked);
    let refresh_button = Button::new("Refresh")
        .secondary()
        .on_press(|| Msg::Refresh)
        .disabled(locked);
    let quit_button = Button::new("Quit")
        .ghost()
        .on_press(|| Msg::Quit)
        .disabled(locked);

    let widths = [
        new_button.width(),
        refresh_button.width(),
        quit_button.width(),
    ];
    let columns = Layout::horizontal([
        Constraint::Length(widths[0] + 2),
        Constraint::Length(widths[1] + 2),
        Constraint::Length(widths[2] + 2),
        Constraint::Min(0),
    ])
    .split(area);

    ctx.component("toolbar-new", new_button, columns[0]);
    ctx.component("toolbar-refresh", refresh_button, columns[1]);
    ctx.component("toolbar-quit", quit_button, columns[2]);

    let hint = Span::styled(
        "  Tab moves focus · Enter activates",
        Style::default().fg(theme.muted_foreground),
    );
    ctx.paint_widget(Paragraph::new(Line::from(hint)), columns[3]);
}

fn paint_body(ctx: &mut DeclareCtx<'_, App, Msg>, app: &App, theme: &Theme, area: Rect) {
    let columns = Layout::horizontal([Constraint::Min(24), Constraint::Length(36)]).split(area);
    paint_sandbox_list(ctx, app, theme, columns[0]);
    paint_actions(ctx, app, theme, columns[1]);
}

fn paint_sandbox_list(ctx: &mut DeclareCtx<'_, App, Msg>, app: &App, theme: &Theme, area: Rect) {
    let theme = *theme;
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Sandboxes ")
        .border_style(Style::default().fg(theme.border))
        .style(Style::default().bg(theme.field));
    let inner = block.inner(area);

    let items: Vec<ListItem<String>> = if app.sandboxes().is_empty() {
        vec![
            ListItem::new(
                String::new(),
                "No sandboxes yet — press N or choose New sandbox.",
            )
            .disabled(true),
        ]
    } else {
        app.sandboxes()
            .iter()
            .map(|sandbox| ListItem::new(sandbox.name.clone(), sandbox.name.clone()))
            .collect()
    };

    let list = List::new(items)
        .item_focus(|state: &App| state.selected_name(), Msg::SandboxFocused)
        .selection(|state: &App| state.selected_name(), Msg::SandboxSelected)
        .scroll(|state: &App| state.list_offset(), Msg::SandboxScrolled)
        .focus_symbol("▸ ")
        .paint_item(move |state: &App, row: ListItemState<'_, String>| {
            let sandbox = state
                .sandboxes()
                .iter()
                .find(|candidate| candidate.name == *row.value);
            let running = sandbox.is_some_and(Sandbox::is_running);
            let name_style = if row.selected {
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD)
            } else if row.focused {
                Style::default().fg(theme.foreground)
            } else {
                Style::default().fg(theme.muted_foreground)
            };
            let status_style = if running {
                Style::default().fg(theme.accent)
            } else {
                Style::default().fg(theme.warning)
            };
            let status = sandbox
                .map(|sandbox| sandbox.status.as_str())
                .unwrap_or("empty");
            Line::from(vec![
                Span::styled(if running { "● " } else { "○ " }, status_style),
                Span::styled(row.label.to_owned(), name_style),
                Span::styled("  ", Style::default()),
                Span::styled(status.to_owned(), status_style),
            ])
        });

    ctx.paint_widget(block, area);
    ctx.component("sandbox-list", list, inner);
}

fn paint_actions(ctx: &mut DeclareCtx<'_, App, Msg>, app: &App, theme: &Theme, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Actions ")
        .border_style(Style::default().fg(theme.border))
        .style(Style::default().bg(theme.surface));
    let inner = block.inner(area);

    let rows = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .split(inner);

    let selected = app.selected_sandbox();
    let running = selected.is_some_and(Sandbox::is_running);
    let has_selection = selected.is_some();
    let locked = app.is_creating();

    let summary = match selected {
        Some(sandbox) => vec![
            Line::from(Span::styled(
                sandbox.name.clone(),
                Style::default()
                    .fg(theme.foreground)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                sandbox.status.clone(),
                Style::default().fg(if running { theme.accent } else { theme.warning }),
            )),
        ],
        None => vec![Line::from(Span::styled(
            "Select a sandbox",
            Style::default().fg(theme.muted_foreground),
        ))],
    };
    ctx.paint_widget(Paragraph::new(summary), rows[0]);

    let toggle = Button::new(if running { "Stop" } else { "Start" })
        .on_press(|| Msg::ToggleSelected)
        .disabled(!has_selection || locked);
    let exec = Button::new("Exec shell")
        .secondary()
        .on_press(|| Msg::ExecSelected)
        .disabled(!running || locked);
    let info = Button::new("Info")
        .secondary()
        .on_press(|| Msg::InfoSelected)
        .disabled(!running || locked);
    let remove = Button::new("Remove")
        .destructive()
        .on_press(|| Msg::RemoveSelected)
        .disabled(!has_selection || locked);

    place_button(ctx, "action-toggle", toggle, rows[1]);
    place_button(ctx, "action-exec", exec, rows[2]);
    place_button(ctx, "action-info", info, rows[3]);
    place_button(ctx, "action-remove", remove, rows[4]);

    ctx.paint_widget(block, area);
}

fn place_button(
    ctx: &mut DeclareCtx<'_, App, Msg>,
    id: &'static str,
    button: Button<Msg>,
    row: Rect,
) {
    let width = button.width();
    // All action buttons use ratcn's default Small size: a single row.
    let area = row.centered(Constraint::Length(width), Constraint::Length(1));
    ctx.component(id, button, area);
}

fn paint_footer(ctx: &mut DeclareCtx<'_, App, Msg>, app: &App, theme: &Theme, area: Rect) {
    let message = app.message();
    let color = if message.contains("Could not") || message.contains("Invalid") {
        theme.destructive
    } else if message.contains("Warning") {
        theme.warning
    } else {
        theme.muted_foreground
    };
    let message = if message.is_empty() {
        "N new · R refresh · S start/stop · X exec · I info · D remove · Ctrl+C quit"
    } else {
        message
    };
    ctx.paint_widget(
        Paragraph::new(Line::from(Span::styled(
            format!(" {message}"),
            Style::default().fg(color),
        ))),
        area,
    );
}

// --- create dialog ----------------------------------------------------------

fn create_dialog(app: &App, theme: &Theme) -> Dialog<App, Msg> {
    let theme = *theme;
    let form = app.form();
    let creating = app.is_creating();
    let name = text_input(
        FormField::Name,
        |state: &App| state.form().name(),
        "demo-tui",
        creating,
        form.invalid(FormField::Name),
    );
    let root = text_input(
        FormField::RootSize,
        |state: &App| state.form().root_size(),
        "40 (optional)",
        creating,
        form.invalid(FormField::RootSize),
    );
    let docker = text_input(
        FormField::DockerSize,
        |state: &App| state.form().docker_size(),
        "20 (optional)",
        creating,
        form.invalid(FormField::DockerSize),
    );
    let cpus = text_input(
        FormField::Cpus,
        |state: &App| state.form().cpus(),
        "2",
        creating,
        form.invalid(FormField::Cpus),
    );
    let memory = text_input(
        FormField::Memory,
        |state: &App| state.form().memory(),
        "4g",
        creating,
        form.invalid(FormField::Memory),
    );
    let agent = Select::new(
        AGENTS
            .iter()
            .enumerate()
            .map(|(index, agent)| ListItem::new(index, (*agent).to_owned())),
    )
    .placeholder("Choose an agent")
    .open(
        |state: &App| state.form().agent_open(),
        Msg::AgentOpenChanged,
    )
    .item_focus(
        |state: &App| Some(state.form().agent_focus()),
        Msg::AgentFocused,
    )
    .selection(
        |state: &App| Some(state.form().agent_index()),
        Msg::AgentSelected,
    )
    .max_visible_items(6)
    .disabled(creating);

    // The untouched empty name is not an error yet; the Create button still
    // validates it when pressed.
    let has_name = !form.name().value().is_empty();
    let error = if has_name { form.first_error() } else { None };
    let status = match error.as_ref() {
        Some((field, message)) => format!("{}: {message}", field.label()),
        None => form
            .warning()
            .unwrap_or_else(|| "Tab or click between fields · Esc cancels".to_owned()),
    };
    let status_color = if !has_name {
        if form.warning().is_some() {
            theme.warning
        } else {
            theme.muted_foreground
        }
    } else if error.is_some() {
        theme.destructive
    } else if form.warning().is_some() {
        theme.warning
    } else {
        theme.muted_foreground
    };
    let progress = app.progress();
    let spinner = if creating {
        format!(
            "{}  Creating sandbox '{}'…",
            app.spinner(),
            app.creating_name().unwrap_or("")
        )
    } else {
        "Ready to create".to_owned()
    };

    Dialog::new()
        .title("New sandbox")
        .description("Create a Docker Sandbox with sbx")
        .outer_width(70)
        .on_dismiss(|| Msg::CancelWizard)
        .content(9, move |ctx| {
            let rows = Layout::vertical(vec![Constraint::Length(1); 9]).split(ctx.area());
            field_row(ctx, rows[0], "Name", name, theme);
            field_row(ctx, rows[1], "Agent", agent, theme);
            field_row(ctx, rows[2], "Root size (GB)", root, theme);
            field_row(ctx, rows[3], "Docker data (GB)", docker, theme);
            field_row(ctx, rows[4], "CPUs", cpus, theme);
            field_row(ctx, rows[5], "Memory", memory, theme);

            ctx.paint_widget(
                Paragraph::new(Line::from(Span::styled(
                    format!(" {status}"),
                    Style::default().fg(status_color),
                ))),
                rows[6],
            );
            ctx.paint_widget(
                Paragraph::new(Line::from(Span::styled(
                    format!(" {spinner}"),
                    Style::default().fg(theme.primary),
                ))),
                rows[7],
            );
            // One bare bar: the row above carries the spinner text.
            ctx.paint_widget(ProgressWidget::new(progress).themed(&theme), rows[8]);
        })
        .action(
            "cancel",
            Button::new("Cancel")
                .ghost()
                .on_press(|| Msg::CancelWizard)
                .disabled(creating),
        )
        .action(
            "create",
            Button::new("Create sandbox")
                .on_press(|| Msg::CreateSandbox)
                .disabled(creating),
        )
}

fn text_input(
    field: FormField,
    read: impl Fn(&App) -> &InputState + 'static,
    placeholder: &'static str,
    disabled: bool,
    invalid: bool,
) -> Input<App, Msg> {
    Input::new()
        .value(
            move |state| read(state),
            move |state| Msg::FormChanged(field, state),
        )
        .placeholder(placeholder)
        .disabled(disabled)
        .invalid(invalid)
}

fn field_row<C>(
    ctx: &mut DeclareCtx<'_, App, Msg>,
    row: Rect,
    label: &'static str,
    component: C,
    theme: Theme,
) where
    C: Component<App, Msg> + 'static,
{
    let columns = Layout::horizontal([Constraint::Length(20), Constraint::Min(10)]).split(row);
    ctx.paint_widget(
        Paragraph::new(Line::from(Span::styled(
            format!(" {label}"),
            Style::default().fg(theme.muted_foreground),
        ))),
        columns[0],
    );
    ctx.component(format!("field-{label}"), component, columns[1]);
}

// --- info dialog ------------------------------------------------------------

fn info_dialog(app: &App, theme: &Theme) -> Dialog<App, Msg> {
    let Some(details) = app.info() else {
        return Dialog::new()
            .title("Sandbox details")
            .description("Details are no longer available.")
            .on_dismiss(|| Msg::CloseInfo)
            .action("close", Button::new("Close").on_press(|| Msg::CloseInfo));
    };
    let text = details_text(details);
    let title = format!("Sandbox · {}", details.name);
    let _ = theme;

    Dialog::new()
        .title(title)
        .description(text)
        .outer_width(66)
        .on_dismiss(|| Msg::CloseInfo)
        .action("close", Button::new("Close").on_press(|| Msg::CloseInfo))
}

fn details_text(details: &SandboxDetails) -> String {
    let mut lines = vec![
        detail_line("Status", Some(details.status.as_str())),
        detail_line("CPUs", details.cpus.as_deref()),
        detail_line(
            "Memory",
            join_usage(details.mem_used.as_deref(), details.mem_total.as_deref()).as_deref(),
        ),
        detail_line(
            "Root fs",
            join_usage(details.root_used.as_deref(), details.root_total.as_deref()).as_deref(),
        ),
        detail_line(
            "Docker data",
            join_usage(
                details.docker_used.as_deref(),
                details.docker_total.as_deref(),
            )
            .as_deref(),
        ),
    ];
    if let Some(error) = &details.error {
        lines.push(String::new());
        lines.push(format!("Note: {error}"));
    }
    lines.join("\n")
}

fn detail_line(label: &str, value: Option<&str>) -> String {
    let value = value.unwrap_or("—");
    format!("{label:<12} {value}")
}

fn join_usage(used: Option<&str>, total: Option<&str>) -> Option<String> {
    match (used, total) {
        (Some(used), Some(total)) => Some(format!("{used} / {total}")),
        (Some(used), None) => Some(used.to_owned()),
        (None, Some(total)) => Some(total.to_owned()),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{INFO_DIALOG, SandboxDetails};
    use ratatui::{Terminal, backend::TestBackend};
    use ratcn::runtime::Ratcn;

    fn render(app: &App) -> String {
        let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
        let theme = Theme::tokyo_night();
        let mut ratcn = Ratcn::new()
            .focus(|state: &App| state.focus_ref(), Msg::FocusChanged)
            .modals(|state: &App| state.modals_ref());
        terminal
            .draw(|frame| {
                ratcn.render(frame, frame.area(), app, &theme, |ctx| {
                    declare(ctx, app, &theme)
                });
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let width = buffer.area.width as usize;
        buffer
            .content()
            .chunks(width)
            .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn renders_dashboard_with_themated_controls() {
        let app = App::with_sandbox_statuses(&[("alpha", "running"), ("beta", "stopped")]);
        let screen = render(&app);
        assert!(screen.contains("DOCKER SANDBOXES"));
        assert!(screen.contains("alpha"));
        assert!(screen.contains("beta"));
        assert!(screen.contains("Exec shell"));
        assert!(screen.contains("Remove"));
    }

    #[test]
    fn renders_create_dialog() {
        let mut app = App::with_sandboxes(&[]);
        app.open_wizard();
        assert!(app.modals_ref().is_open(CREATE_DIALOG));
        let screen = render(&app);
        assert!(screen.contains("New sandbox"));
        assert!(screen.contains("Create sandbox"));
        assert!(screen.contains("demo-tui"));
        assert!(screen.contains("opencode"));

        // The agent dropdown is a popup layer inside the modal; it must be
        // declarable without colliding with the dialog's own ids.
        app.set_agent_open(true);
        let screen = render(&app);
        // The panel keeps the current agent in view, so the first visible row
        // is near the default selection rather than the top of the list.
        assert!(screen.contains("cursor"));
        assert!(screen.contains("opencode"));
    }

    #[test]
    fn renders_info_dialog() {
        let mut app = App::with_sandbox_statuses(&[("alpha", "running")]);
        app.show_info_for_test(SandboxDetails {
            name: "alpha".to_owned(),
            status: "running".to_owned(),
            cpus: Some("4".to_owned()),
            mem_used: Some("1.0 GiB".to_owned()),
            mem_total: Some("2.0 GiB".to_owned()),
            ..SandboxDetails::default()
        });
        assert!(app.modals_ref().is_open(INFO_DIALOG));
        let screen = render(&app);
        assert!(screen.contains("Sandbox"));
        assert!(screen.contains("1.0 GiB / 2.0 GiB"));
    }
}

#[cfg(test)]
mod interaction_tests {
    use super::*;
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{Terminal, backend::TestBackend};
    use ratcn::runtime::{EventResult, Ratcn};

    fn new_runtime() -> Ratcn<App, Msg> {
        Ratcn::new()
            .focus(|state: &App| state.focus_ref(), Msg::FocusChanged)
            .modals(|state: &App| state.modals_ref())
    }

    fn draw(terminal: &mut Terminal<TestBackend>, app: &App, ratcn: &mut Ratcn<App, Msg>) {
        let theme = Theme::tokyo_night();
        terminal
            .draw(|frame| {
                ratcn.render(frame, frame.area(), app, &theme, |ctx| {
                    declare(ctx, app, &theme)
                });
            })
            .unwrap();
    }

    #[test]
    fn enter_opens_dialog_and_escape_dismisses_it() {
        let mut app = App::with_sandboxes(&[]);
        let mut ratcn = new_runtime();
        let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
        draw(&mut terminal, &app, &mut ratcn);

        let enter = Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        match ratcn.handle_event(enter, &app) {
            EventResult::Emit(message) => apply(&mut app, message),
            other => panic!("Enter should activate the focused button, got {other:?}"),
        }
        assert!(app.modals_ref().is_open(CREATE_DIALOG));

        // The modal binding swallows events until the layer is redrawn.
        draw(&mut terminal, &app, &mut ratcn);
        let escape = Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        match ratcn.handle_event(escape, &app) {
            EventResult::Emit(message) => apply(&mut app, message),
            other => panic!("Esc should dismiss the dialog, got {other:?}"),
        }
        assert!(!app.modals_ref().is_open(CREATE_DIALOG));
    }
}
