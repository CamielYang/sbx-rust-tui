//! Ratcn-based dashboard for `sbx` sandboxes.
//!
//! Every control is either a ratcn component or themed paint queued through
//! ratcn's declaration context. The runtime owns focus, hover, hit testing, and
//! modal layering; [`App`](crate::app::App) owns the state and [`apply`] is the
//! single place where component messages become state changes.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratcn::color::{DISABLED_DIM, FOCUS_SHIFT, HOVER_SHIFT, away_from, dim, nearest_to};
use ratcn::runtime::{ChildId, Component, DeclareCtx, FocusState};
use ratcn::{
    Button, ButtonStyle, Dialog, Input, InputState, List, ListItem, ListItemState, ProgressWidget,
    Select, Theme,
};

use crate::app::{
    AGENTS, App, CONFIRM_DIALOG, CREATE_DIALOG, FormField, INFO_DIALOG, PORTS_DIALOG,
    PendingAction, PortField, PortMapping, Sandbox, SandboxDetails,
};

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
    StartSelected,
    StopSelected,
    ExecSelected,
    InfoSelected,
    RemoveSelected,
    OpenPorts,
    ClosePorts,
    PortsChanged(PortField, InputState),
    PublishPort,
    UnpublishPort(u16, u16),
    ConfirmAction,
    CancelAction,
    CloseInfo,
}

const ACTION_START: &str = "action-start";
const ACTION_STOP: &str = "action-stop";
const ACTION_EXEC: &str = "action-exec";
const ACTION_REMOVE: &str = "action-remove";
const ACTION_INFO: &str = "action-info";
const ACTION_PORTS: &str = "action-ports";

/// Apply one runtime message. Creation locks the controls: late focus messages
/// are kept so the runtime stays in sync, but everything else is discarded
/// until the worker reports back.
pub fn apply(app: &mut App, msg: Msg) {
    if app.is_creating() && !matches!(msg, Msg::FocusChanged(_)) {
        return;
    }
    match msg {
        Msg::FocusChanged(focus) => *app.focus_mut() = focus,
        // Moving the cursor (hover or arrow keys) only moves the highlight.
        // Clicking or Enter commits the row as the Actions pane's target.
        Msg::SandboxFocused(name, offset) => app.focus_list_item(&name, offset),
        Msg::SandboxSelected(name) => app.focus_sandbox(&name),
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
        Msg::StartSelected => app.start_selected(),
        Msg::StopSelected => app.stop_selected(),
        Msg::ExecSelected => app.exec_selected(),
        Msg::InfoSelected => app.open_selected_info(),
        Msg::RemoveSelected => app.remove_selected(),
        Msg::OpenPorts => app.open_ports_selected(),
        Msg::ClosePorts => app.close_ports(),
        Msg::PortsChanged(field, state) => app.set_port_state(field, state),
        Msg::PublishPort => app.publish_port(),
        Msg::UnpublishPort(host, sandbox) => app.unpublish_port(host, sandbox),
        Msg::ConfirmAction => app.confirm_action(),
        Msg::CancelAction => app.cancel_action(),
        Msg::CloseInfo => app.close_info(),
    }
}

/// A status tick can hide the action button holding keyboard focus. Ratcn
/// leaves a path parked on a component that is no longer declared, and Tab
/// cannot move from there, so hand focus back to the list instead.
pub fn settle_focus(app: &mut App) {
    let Some(id) = app.focus_ref().path().first().map(ChildId::as_str) else {
        return;
    };
    if !id.starts_with("action-") {
        return;
    }
    let selected = app.selected_sandbox();
    let running = selected.is_some_and(Sandbox::is_running);
    let valid = match id {
        ACTION_START => !running && selected.is_some(),
        ACTION_STOP | ACTION_EXEC | ACTION_INFO | ACTION_PORTS => running,
        ACTION_REMOVE => selected.is_some(),
        _ => false,
    };
    if !valid {
        *app.focus_mut() = FocusState::intent(["sandbox-list"]);
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
    if app.modals_ref().is_open(CONFIRM_DIALOG) {
        ctx.modal(CONFIRM_DIALOG, confirm_dialog(app), area);
    }
    if app.modals_ref().is_open(PORTS_DIALOG) {
        ctx.modal(PORTS_DIALOG, ports_dialog(app, &theme), area);
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
        .item_focus(|state: &App| state.focused_name(), Msg::SandboxFocused)
        .selection(|state: &App| state.selected_name(), Msg::SandboxSelected)
        .scroll(|state: &App| state.list_offset(), Msg::SandboxScrolled)
        // The cursor glyph is drawn by `paint_item` below. Ratcn's own
        // `focus_symbol` reserves its gutter only while the list has focus,
        // which shifts every row when hover moves focus elsewhere.
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
                // Two cells are reserved on every row, whether or not the
                // cursor is visible, so the row text never shifts sideways.
                Span::styled(
                    if row.focused { "▸ " } else { "  " },
                    Style::default().fg(theme.primary),
                ),
                Span::styled(if running { "● " } else { "○ " }, status_style),
                Span::styled(row.label.to_owned(), name_style),
                Span::styled("  ", Style::default()),
                Span::styled(status.to_owned(), status_style),
            ])
        });

    ctx.paint_widget(block, area);
    ctx.component("sandbox-list", list, inner);
}

// Action hues, in the same saturated register as the Tokyo Night palette.
const START_GREEN: Color = Color::Rgb(158, 206, 106);
const STOP_ORANGE: Color = Color::Rgb(255, 158, 100);
const EXEC_GRAY: Color = Color::Rgb(122, 132, 166);
const REMOVE_RED: Color = Color::Rgb(247, 118, 142);
const INFO_BLUE: Color = Color::Rgb(122, 162, 247);
const PORTS_CYAN: Color = Color::Rgb(115, 200, 211);

/// A filled button keeps its action hue in every state. Focus deepens the fill,
/// hover lifts it, and both change visibly even when the button is focused —
/// ratcn resolves hover before focus, so the pointer always wins.
fn action_style(fill: Color) -> impl Fn(&Theme) -> ButtonStyle {
    move |theme: &Theme| {
        let pressed = nearest_to(theme.background);
        let raised = away_from(theme.background);
        ButtonStyle {
            foreground: theme.background,
            background: fill,
            border: None,
            focused_foreground: theme.background,
            focused_background: dim(fill, pressed, FOCUS_SHIFT),
            focused_border: None,
            hovered_foreground: theme.background,
            hovered_background: dim(fill, raised, HOVER_SHIFT),
            hovered_border: None,
            disabled_foreground: theme.muted_foreground,
            disabled_background: dim(fill, theme.surface, DISABLED_DIM),
            disabled_border: None,
        }
    }
}

/// (id, label, hue, message) for the actions valid in the current state.
type Action = (&'static str, &'static str, Color, fn() -> Msg);

const RUNNING_ACTIONS: &[Action] = &[
    (ACTION_STOP, "Stop", STOP_ORANGE, || Msg::StopSelected),
    (ACTION_EXEC, "Exec shell", EXEC_GRAY, || Msg::ExecSelected),
    (ACTION_PORTS, "Publish ports", PORTS_CYAN, || Msg::OpenPorts),
    (ACTION_INFO, "Info", INFO_BLUE, || Msg::InfoSelected),
    (ACTION_REMOVE, "Remove", REMOVE_RED, || Msg::RemoveSelected),
];

const STOPPED_ACTIONS: &[Action] = &[
    (ACTION_START, "Start", START_GREEN, || Msg::StartSelected),
    (ACTION_REMOVE, "Remove", REMOVE_RED, || Msg::RemoveSelected),
];

fn paint_actions(ctx: &mut DeclareCtx<'_, App, Msg>, app: &App, theme: &Theme, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Actions ")
        .border_style(Style::default().fg(theme.border))
        .style(Style::default().bg(theme.surface));
    let inner = block.inner(area);
    // The pane's surface goes down first; buttons paint their fills on top.
    ctx.paint_widget(block, area);

    let rows = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .split(inner);

    let selected = app.selected_sandbox();
    let running = selected.is_some_and(Sandbox::is_running);
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

    // Only declare actions that mean something right now. A hidden action is
    // not in the focus order and cannot be clicked by stale pointer state.
    let actions = if selected.is_none() {
        &[][..]
    } else if running {
        RUNNING_ACTIONS
    } else {
        STOPPED_ACTIONS
    };
    // One width for every action, independent of which set is visible, so the
    // column stays put when a sandbox changes state.
    let button_width = RUNNING_ACTIONS
        .iter()
        .chain(STOPPED_ACTIONS.iter())
        .map(|(_, label, _, _)| Button::<Msg>::new(*label).width())
        .max()
        .unwrap_or(0);
    for (row, (id, label, fill, message)) in actions.iter().enumerate() {
        let button = Button::new(*label)
            .style(action_style(*fill))
            .on_press(*message)
            .disabled(locked);
        place_button(ctx, id, button, rows[row + 1], button_width);
    }
}

fn place_button(
    ctx: &mut DeclareCtx<'_, App, Msg>,
    id: &'static str,
    button: Button<Msg>,
    row: Rect,
    width: u16,
) {
    // All action buttons use ratcn's default Small size: a single row. Pin
    // them to the pane's left edge instead of centering each label width.
    let area = Rect::new(row.x, row.y, width.min(row.width), 1);
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
        "N new · R refresh · S start/stop · X exec · P ports · I info · D remove · Ctrl+C quit"
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
    let creating_message = if creating {
        format!(
            "{}  Creating sandbox '{}'…",
            app.spinner(),
            app.creating_name().unwrap_or("")
        )
    } else {
        String::new()
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
                    format!(" {creating_message}"),
                    Style::default().fg(theme.primary),
                ))),
                rows[7],
            );
            // One bare bar: the row above carries the creation message.
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
    input(
        move |state| Msg::FormChanged(field, state),
        read,
        placeholder,
        disabled,
        invalid,
    )
}

/// An input bound to an app field through a caller-supplied message, so the
/// create form and the ports dialog can share the same themed widget.
fn input(
    on_change: impl Fn(InputState) -> Msg + 'static,
    read: impl Fn(&App) -> &InputState + 'static,
    placeholder: &'static str,
    disabled: bool,
    invalid: bool,
) -> Input<App, Msg> {
    Input::new()
        .value(move |state| read(state), on_change)
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

// --- confirmation dialog ----------------------------------------------------

/// The last gate before Start/Stop/Remove reaches `sbx`. The action's hue is
/// kept on the confirm button so the dialog reads like the action it replaces.
fn confirm_dialog(app: &App) -> Dialog<App, Msg> {
    let Some(action) = app.pending_action() else {
        return Dialog::new()
            .title("Confirm action")
            .description("No action is pending.")
            .on_dismiss(|| Msg::CancelAction)
            .action("close", Button::new("Close").on_press(|| Msg::CancelAction));
    };

    let (title, description, label, fill) = match action {
        PendingAction::Start(name) => (
            "Confirm start",
            format!("Start sandbox '{name}'?"),
            "Start sandbox",
            START_GREEN,
        ),
        PendingAction::Stop(name) => (
            "Confirm stop",
            format!("Stop sandbox '{name}'?"),
            "Stop sandbox",
            STOP_ORANGE,
        ),
        PendingAction::Remove(name) => (
            "Confirm remove",
            format!("Permanently remove sandbox '{name}' and its data?"),
            "Remove sandbox",
            REMOVE_RED,
        ),
    };

    Dialog::new()
        .title(title)
        .description(description)
        .outer_width(62)
        .on_dismiss(|| Msg::CancelAction)
        .action(
            "cancel",
            Button::new("Cancel").ghost().on_press(|| Msg::CancelAction),
        )
        .action(
            "confirm",
            Button::new(label)
                .style(action_style(fill))
                .on_press(|| Msg::ConfirmAction),
        )
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
        detail_line("CPUs", cpu_text(details).as_deref()),
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

/// Show core count alone until the guest reports a usage sample, then add the
/// percentage in the requested `{cores} ({percentage usage})` form.
fn cpu_text(details: &SandboxDetails) -> Option<String> {
    let cores = details.cpus.as_deref()?;
    Some(match details.cpu_usage {
        Some(usage) => format!("{cores} ({usage}%)"),
        None => cores.to_owned(),
    })
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

// --- published ports dialog -------------------------------------------------

/// A table of `sbx port ls` rows with an Add port button above it. The two
/// inputs default to the example mapping (`8080:3000`), so pressing the button
/// publishes that mapping directly or any pair typed into the fields.
fn ports_dialog(app: &App, theme: &Theme) -> Dialog<App, Msg> {
    let Some(view) = app.ports() else {
        return Dialog::new()
            .title("Published ports")
            .description("Ports are no longer available.")
            .on_dismiss(|| Msg::ClosePorts)
            .action("close", Button::new("Close").on_press(|| Msg::ClosePorts));
    };

    let theme = *theme;
    let name = view.name.clone();
    let ports = view.ports.clone();
    let error = view.error.clone();
    let host = input(
        |state| Msg::PortsChanged(PortField::Host, state),
        |state: &App| state.port_form().host(),
        "8080",
        false,
        app.port_invalid(PortField::Host),
    );
    let sandbox = input(
        |state| Msg::PortsChanged(PortField::Sandbox, state),
        |state: &App| state.port_form().sandbox(),
        "3000",
        false,
        app.port_invalid(PortField::Sandbox),
    );
    let add_button = Button::new("+ Add port").on_press(|| Msg::PublishPort);
    let add_width = add_button.width();
    let unpublish_width = Button::<Msg>::new("Unpublish").width();

    Dialog::new()
        .title(format!("Published ports · {name}"))
        .description("Ports published to the host")
        .outer_width(72)
        .on_dismiss(|| Msg::ClosePorts)
        .content(12, move |ctx| {
            let rows = Layout::vertical([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Min(0),
                Constraint::Length(1),
            ])
            .split(ctx.area());

            // Button, both inputs, and their short labels share one row.
            let add = Layout::horizontal([
                Constraint::Length(add_width),
                Constraint::Length(6),
                Constraint::Length(12),
                Constraint::Length(6),
                Constraint::Length(12),
                Constraint::Min(0),
            ])
            .split(rows[0]);
            ctx.component("ports-add", add_button, add[0]);
            for (area, label) in [(add[1], " host "), (add[3], " port ")] {
                ctx.paint_widget(
                    Paragraph::new(Line::from(Span::styled(
                        label,
                        Style::default().fg(theme.muted_foreground),
                    ))),
                    area,
                );
            }
            ctx.component("ports-host", host, add[2]);
            ctx.component("ports-sandbox", sandbox, add[4]);

            // One blank row separates the inputs from the table, which gets
            // its own border so the rows read as a distinct group.
            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme.border))
                .style(Style::default().bg(theme.surface));
            let inner = block.inner(rows[2]);
            ctx.paint_widget(block, rows[2]);

            if inner.height == 0 || inner.width == 0 {
                return;
            }
            let columns = Layout::horizontal([
                Constraint::Length(16),
                Constraint::Length(18),
                Constraint::Min(0),
            ])
            .split(inner);
            ctx.paint_widget(Paragraph::new(port_header_line(&theme)), inner);

            let visible = (inner.height - 1) as usize;
            if ports.is_empty() {
                ctx.paint_widget(
                    Paragraph::new(Line::from(Span::styled(
                        " No published ports.",
                        Style::default().fg(theme.muted_foreground),
                    ))),
                    Rect::new(inner.x, inner.y + 1, inner.width, 1),
                );
            } else {
                for (index, mapping) in ports.iter().take(visible).enumerate() {
                    let row = Rect::new(inner.x, inner.y + 1 + index as u16, inner.width, 1);
                    ctx.paint_widget(Paragraph::new(port_row_line(mapping, &theme)), row);

                    let width = unpublish_width.min(columns[2].width);
                    let button_area =
                        Rect::new(columns[2].x + columns[2].width - width, row.y, width, 1);
                    let (host_port, sandbox_port) = (mapping.host_port, mapping.sandbox_port);
                    ctx.component(
                        format!("ports-unpublish-{index}"),
                        Button::new("Unpublish")
                            .ghost()
                            .on_press(move || Msg::UnpublishPort(host_port, sandbox_port)),
                        button_area,
                    );
                }
            }

            let status = match error {
                Some(error) => Some(Span::styled(
                    format!(" {error}"),
                    Style::default().fg(theme.destructive),
                )),
                None if ports.len() > visible => Some(Span::styled(
                    format!("  Showing {visible} of {} published ports", ports.len()),
                    Style::default().fg(theme.muted_foreground),
                )),
                None => None,
            };
            if let Some(status) = status {
                ctx.paint_widget(Paragraph::new(Line::from(status)), rows[3]);
            }
        })
        .action(
            "close",
            Button::new("Close").ghost().on_press(|| Msg::ClosePorts),
        )
}

fn port_header_line(theme: &Theme) -> Line<'static> {
    let style = Style::default()
        .fg(theme.muted_foreground)
        .add_modifier(Modifier::BOLD);
    Line::from(vec![
        Span::styled(format!(" {:<16}", "HOST PORT"), style),
        Span::styled(format!("{:<18}", "SANDBOX PORT"), style),
        Span::styled("ACTION", style),
    ])
}

fn port_row_line(mapping: &PortMapping, theme: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!(" {:<16}", mapping.host_port),
            Style::default().fg(theme.foreground),
        ),
        Span::styled(
            format!("{:<18}", mapping.sandbox_port),
            Style::default().fg(theme.foreground),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{INFO_DIALOG, PORTS_DIALOG, PortMapping, SandboxDetails};
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

    fn column_of(rows: &str, needle: &str) -> Option<usize> {
        rows.lines()
            .find_map(|line| line.find(needle).map(|byte| line[..byte].chars().count()))
    }

    #[test]
    fn sandbox_rows_do_not_shift_when_focus_moves() {
        let mut app = App::with_sandboxes(&["alpha"]);
        *app.focus_mut() = FocusState::intent(["sandbox-list"]);
        let focused = render(&app);
        *app.focus_mut() = FocusState::intent([ACTION_STOP]);
        let unfocused = render(&app);

        assert_eq!(column_of(&focused, "alpha"), column_of(&unfocused, "alpha"));
        assert!(focused.contains("▸"), "the cursor glyph should still show");
        assert!(!unfocused.contains("▸"));
    }

    #[test]
    fn running_actions_show_only_running_commands() {
        let app = App::with_sandboxes(&["alpha"]);
        let screen = render(&app);
        for label in ["Stop", "Exec shell", "Remove", "Info"] {
            assert!(screen.contains(label), "missing running action: {label}");
        }
        assert!(
            !screen.contains("Start"),
            "Start must be hidden while running"
        );
    }

    #[test]
    fn stopped_actions_show_only_stopped_commands() {
        let app = App::with_sandbox_statuses(&[("alpha", "stopped")]);
        let screen = render(&app);
        assert!(screen.contains("Start"));
        assert!(screen.contains("Remove"));
        for label in ["Stop", "Exec shell", "Info"] {
            assert!(
                !screen.contains(label),
                "unexpected running action: {label}"
            );
        }
    }

    #[test]
    fn action_styles_keep_their_hue_and_change_on_focus_and_hover() {
        let theme = Theme::tokyo_night();
        for fill in [START_GREEN, STOP_ORANGE, EXEC_GRAY, REMOVE_RED, INFO_BLUE] {
            let style = action_style(fill)(&theme);
            assert_eq!(style.background, fill);
            assert_eq!(style.focused_foreground, theme.background);
            assert_ne!(style.focused_background, fill, "focus must change the fill");
            assert_ne!(style.hovered_background, fill, "hover must change the fill");
        }
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
        assert!(!screen.contains("Ready to create"));

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
            cpu_usage: Some(37),
            mem_used: Some("1.0 GiB".to_owned()),
            mem_total: Some("2.0 GiB".to_owned()),
            ..SandboxDetails::default()
        });
        assert!(app.modals_ref().is_open(INFO_DIALOG));
        let screen = render(&app);
        assert!(screen.contains("Sandbox"));
        assert!(screen.contains("4 (37%)"));
        assert!(screen.contains("1.0 GiB / 2.0 GiB"));
    }

    #[test]
    fn renders_ports_dialog_with_table_and_add_button() {
        let mut app = App::with_sandboxes(&["alpha"]);
        app.show_ports_for_test(
            "alpha",
            vec![
                PortMapping {
                    host_port: 60002,
                    sandbox_port: 3000,
                    protocol: "tcp4".to_owned(),
                    host_ip: Some("127.0.0.1".to_owned()),
                },
                PortMapping {
                    host_port: 60003,
                    sandbox_port: 3001,
                    protocol: "tcp4".to_owned(),
                    host_ip: None,
                },
            ],
        );
        assert!(app.modals_ref().is_open(PORTS_DIALOG));
        let screen = render(&app);
        assert!(screen.contains("Published ports"));
        assert!(screen.contains("+ Add port"));
        assert!(screen.contains(" host "));
        assert!(screen.contains(" port "));
        assert!(screen.contains("HOST PORT"));
        assert!(screen.contains("SANDBOX PORT"));
        assert!(!screen.contains("PROTOCOL"));
        assert!(screen.contains("60002"));
        assert!(screen.contains("60003"));
        assert!(screen.contains("Unpublish"));
        assert!(!screen.contains("sbx port publish"));
    }

    #[test]
    fn cpu_text_adds_usage_only_when_sampled() {
        let details = SandboxDetails {
            cpus: Some("8".to_owned()),
            cpu_usage: Some(12),
            ..SandboxDetails::default()
        };
        assert_eq!(cpu_text(&details).as_deref(), Some("8 (12%)"));

        let details = SandboxDetails {
            cpu_usage: None,
            ..details
        };
        assert_eq!(cpu_text(&details).as_deref(), Some("8"));
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

    fn cell_bg(terminal: &Terminal<TestBackend>, x: u16, y: u16) -> Color {
        terminal
            .backend()
            .buffer()
            .cell((x, y))
            .expect("cell on screen")
            .bg
    }

    fn screen(terminal: &Terminal<TestBackend>) -> String {
        let buffer = terminal.backend().buffer();
        let width = buffer.area.width as usize;
        buffer
            .content()
            .chunks(width)
            .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn click(
        terminal: &mut Terminal<TestBackend>,
        app: &mut App,
        ratcn: &mut Ratcn<App, Msg>,
        column: u16,
        row: u16,
    ) -> Vec<Msg> {
        let mut messages = Vec::new();
        for kind in [
            crossterm::event::MouseEventKind::Moved,
            crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
            crossterm::event::MouseEventKind::Up(crossterm::event::MouseButton::Left),
        ] {
            let event = Event::Mouse(crossterm::event::MouseEvent {
                kind,
                column,
                row,
                modifiers: KeyModifiers::NONE,
            });
            if let EventResult::Emit(message) = ratcn.handle_event(event, app) {
                messages.push(message.clone());
                apply(app, message);
            }
            draw(terminal, app, ratcn);
        }
        messages
    }

    #[test]
    fn clicking_a_sandbox_focuses_its_actions() {
        let mut app = App::with_sandbox_statuses(&[("alpha", "running"), ("beta", "stopped")]);
        let mut ratcn = new_runtime();
        let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
        draw(&mut terminal, &app, &mut ratcn);

        // Hovering the cursor is not a commitment: the Actions pane still
        // targets the sandbox that was selected before the pointer arrived.
        let moved = Event::Mouse(crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::Moved,
            column: 6,
            row: 4,
            modifiers: KeyModifiers::NONE,
        });
        if let EventResult::Emit(message) = ratcn.handle_event(moved, &app) {
            apply(&mut app, message);
        }
        assert_eq!(app.selected_name().as_deref(), Some("alpha"));

        click(&mut terminal, &mut app, &mut ratcn, 6, 4);
        assert_eq!(app.selected_name().as_deref(), Some("beta"));
        assert!(app.focus_ref().contains_path(["sandbox-list"]));

        let screen = screen(&terminal);
        assert!(screen.contains("beta"));
        assert!(screen.contains("Start"));
        assert!(screen.contains("Remove"));
        assert!(!screen.contains("Stop"));
        assert!(!screen.contains("Exec shell"));
        assert!(!screen.contains("Info"));
    }

    #[test]
    fn hiding_an_action_does_not_park_focus_on_it() {
        let mut app = App::with_sandbox_statuses(&[("alpha", "stopped")]);
        *app.focus_mut() = FocusState::intent([ACTION_STOP]);
        settle_focus(&mut app);
        assert!(app.focus_ref().contains_path(["sandbox-list"]));

        *app.focus_mut() = FocusState::intent([ACTION_START]);
        settle_focus(&mut app);
        assert!(app.focus_ref().contains_path([ACTION_START]));
    }

    #[test]
    fn action_buttons_share_one_left_aligned_column() {
        let app = App::with_sandboxes(&["alpha"]);
        let mut ratcn = new_runtime();
        let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
        draw(&mut terminal, &app, &mut ratcn);
        // "Publish ports" sets the shared width: 13 label cells + 4 of padding.
        for (row, fill) in [(5, STOP_ORANGE), (6, EXEC_GRAY), (7, PORTS_CYAN)] {
            for x in 65..82 {
                assert_eq!(cell_bg(&terminal, x, row), fill, "fill at ({x}, {row})");
            }
            assert_ne!(
                cell_bg(&terminal, 82, row),
                fill,
                "button must stop at x=82"
            );
        }
    }

    #[test]
    fn action_buttons_change_color_on_hover() {
        let app = App::with_sandboxes(&["alpha"]);
        let mut ratcn = new_runtime();
        let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
        draw(&mut terminal, &app, &mut ratcn);
        let resting = cell_bg(&terminal, 70, 5);
        assert_eq!(resting, STOP_ORANGE);

        let moved = Event::Mouse(crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::Moved,
            column: 70,
            row: 5,
            modifiers: KeyModifiers::NONE,
        });
        ratcn.handle_event(moved, &app);
        draw(&mut terminal, &app, &mut ratcn);
        assert_ne!(cell_bg(&terminal, 70, 5), resting);
    }

    #[test]
    fn action_buttons_emit_only_their_own_command() {
        let mut running = App::with_sandboxes(&["alpha"]);
        let mut ratcn = new_runtime();
        let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
        draw(&mut terminal, &running, &mut ratcn);
        let emitted = click(&mut terminal, &mut running, &mut ratcn, 70, 5);
        assert!(
            emitted
                .iter()
                .any(|message| matches!(message, Msg::StopSelected))
        );
        assert!(
            !emitted
                .iter()
                .any(|message| matches!(message, Msg::StartSelected))
        );

        let mut stopped = App::with_sandbox_statuses(&[("alpha", "stopped")]);
        let mut ratcn = new_runtime();
        draw(&mut terminal, &stopped, &mut ratcn);
        let emitted = click(&mut terminal, &mut stopped, &mut ratcn, 70, 5);
        assert!(
            emitted
                .iter()
                .any(|message| matches!(message, Msg::StartSelected))
        );
        assert!(
            !emitted
                .iter()
                .any(|message| matches!(message, Msg::StopSelected))
        );
    }

    fn position_from_bottom(screen: &str, needle: &str) -> Option<(u16, u16)> {
        let lines: Vec<_> = screen.lines().collect();
        lines
            .iter()
            .enumerate()
            .rev()
            .find_map(|(y, line)| line.rfind(needle).map(|x| (x as u16, y as u16)))
    }

    #[test]
    fn clicking_an_action_opens_its_confirmation_dialog() {
        let mut app = App::with_sandboxes(&["alpha"]);
        let mut ratcn = new_runtime();
        let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
        draw(&mut terminal, &app, &mut ratcn);

        let emitted = click(&mut terminal, &mut app, &mut ratcn, 70, 5);
        assert!(
            emitted
                .iter()
                .any(|message| matches!(message, Msg::StopSelected))
        );
        assert!(app.modals_ref().is_open(CONFIRM_DIALOG));
        assert_eq!(
            app.pending_action(),
            Some(&PendingAction::Stop("alpha".to_owned()))
        );
        assert!(app.take_confirmed_for_test().is_none());

        let screen = screen(&terminal);
        assert!(screen.contains("Confirm stop"));
        assert!(screen.contains("Stop sandbox 'alpha'?"));
        assert!(screen.contains("Cancel"));
    }

    #[test]
    fn clicking_publish_ports_opens_the_ports_dialog() {
        let mut app = App::with_sandboxes(&["alpha"]);
        let mut ratcn = new_runtime();
        let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
        draw(&mut terminal, &app, &mut ratcn);

        // Action rows: Stop (5), Exec shell (6), Publish ports (7).
        let emitted = click(&mut terminal, &mut app, &mut ratcn, 70, 7);
        assert!(
            emitted
                .iter()
                .any(|message| matches!(message, Msg::OpenPorts))
        );
        assert!(app.modals_ref().is_open(PORTS_DIALOG));
        let screen = screen(&terminal);
        assert!(screen.contains("Published ports"));
        assert!(screen.contains("+ Add port"));
    }

    #[test]
    fn add_port_button_publishes_the_default_mapping() {
        let mut app = App::with_sandboxes(&["alpha"]);
        app.show_ports_for_test("alpha", Vec::new());
        let mut ratcn = new_runtime();
        let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
        draw(&mut terminal, &app, &mut ratcn);

        let (x, y) = position_from_bottom(&screen(&terminal), "+ Add port")
            .expect("add button is on screen");
        let emitted = click(&mut terminal, &mut app, &mut ratcn, x + 2, y);
        assert!(
            emitted
                .iter()
                .any(|message| matches!(message, Msg::PublishPort))
        );
        assert_eq!(
            app.take_published_for_test().as_deref(),
            Some("sbx port publish alpha 8080:3000")
        );
    }

    #[test]
    fn unpublish_button_records_the_row_mapping() {
        let mut app = App::with_sandboxes(&["alpha"]);
        app.show_ports_for_test(
            "alpha",
            vec![PortMapping {
                host_port: 60002,
                sandbox_port: 3000,
                protocol: "tcp4".to_owned(),
                host_ip: None,
            }],
        );
        let mut ratcn = new_runtime();
        let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
        draw(&mut terminal, &app, &mut ratcn);

        let (x, y) = position_from_bottom(&screen(&terminal), "Unpublish")
            .expect("unpublish button is on screen");
        let emitted = click(&mut terminal, &mut app, &mut ratcn, x + 2, y);
        assert!(
            emitted
                .iter()
                .any(|message| matches!(message, Msg::UnpublishPort(60002, 3000)))
        );
        assert_eq!(
            app.take_unpublished_for_test().as_deref(),
            Some("sbx port unpublish alpha 60002:3000")
        );
    }

    #[test]
    fn escape_dismisses_confirmation_without_running_the_action() {
        let mut app = App::with_sandbox_statuses(&[("alpha", "stopped")]);
        let mut ratcn = new_runtime();
        let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
        draw(&mut terminal, &app, &mut ratcn);

        app.start_selected();
        draw(&mut terminal, &app, &mut ratcn);
        assert!(app.modals_ref().is_open(CONFIRM_DIALOG));

        let escape = Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        match ratcn.handle_event(escape, &app) {
            EventResult::Emit(message) => apply(&mut app, message),
            other => panic!("Esc should dismiss the confirmation, got {other:?}"),
        }
        assert!(!app.modals_ref().is_open(CONFIRM_DIALOG));
        assert!(app.pending_action().is_none());
        assert!(app.take_confirmed_for_test().is_none());
        assert_eq!(app.message(), "Start cancelled.");
    }

    #[test]
    fn confirming_an_action_hands_it_to_the_app() {
        let mut app = App::with_sandboxes(&["alpha"]);
        let mut ratcn = new_runtime();
        let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
        draw(&mut terminal, &app, &mut ratcn);

        app.stop_selected();
        draw(&mut terminal, &app, &mut ratcn);
        let (x, y) = position_from_bottom(&screen(&terminal), "Stop sandbox")
            .expect("confirm button is on screen");
        let emitted = click(&mut terminal, &mut app, &mut ratcn, x + 2, y);
        assert!(
            emitted
                .iter()
                .any(|message| matches!(message, Msg::ConfirmAction))
        );
        assert!(!app.modals_ref().is_open(CONFIRM_DIALOG));
        assert_eq!(
            app.take_confirmed_for_test(),
            Some(PendingAction::Stop("alpha".to_owned()))
        );
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
