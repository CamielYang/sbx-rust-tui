//! Modal dialog builders: create wizard, confirmation, details, and ports.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratcn::{Button, Dialog, InputState, ListItem, ProgressWidget, Select, Theme};

use super::dashboard::{REMOVE_RED, START_GREEN, STOP_ORANGE, action_style};
use super::widgets::{detail_line, field_row, input, join_usage};
use super::*;
use crate::app::{AGENTS, FormField, PendingAction, PortField, PortMapping, SandboxDetails};

// --- create dialog ----------------------------------------------------------

pub(super) fn create_dialog(app: &App, theme: &Theme) -> Dialog<App, Msg> {
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
) -> ratcn::Input<App, Msg> {
    input(
        move |state| Msg::FormChanged(field, state),
        read,
        placeholder,
        disabled,
        invalid,
    )
}

// --- confirmation dialog ----------------------------------------------------

/// The last gate before Start/Stop/Remove reaches `sbx`. The action's hue is
/// kept on the confirm button so the dialog reads like the action it replaces.
pub(super) fn confirm_dialog(app: &App) -> Dialog<App, Msg> {
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

pub(super) fn info_dialog(app: &App, theme: &Theme) -> Dialog<App, Msg> {
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
pub(super) fn cpu_text(details: &SandboxDetails) -> Option<String> {
    let cores = details.cpus.as_deref()?;
    Some(match details.cpu_usage {
        Some(usage) => format!("{cores} ({usage}%)"),
        None => cores.to_owned(),
    })
}

// --- published ports dialog -------------------------------------------------

/// A table of `sbx port ls` rows with an Add port button above it. The two
/// inputs default to the example mapping (`8080:3000`), so pressing the button
/// publishes that mapping directly or any pair typed into the fields.
pub(super) fn ports_dialog(app: &App, theme: &Theme) -> Dialog<App, Msg> {
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
                Constraint::Length(9),
                Constraint::Length(12),
                Constraint::Min(0),
            ])
            .split(rows[0]);
            ctx.component("ports-add", add_button, add[0]);
            for (area, label) in [(add[1], " host "), (add[3], " sandbox ")] {
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

                    // Start at the ACTION column, not the far edge of the box.
                    let width = unpublish_width.min(columns[2].width);
                    let button_area = Rect::new(columns[2].x, row.y, width, 1);
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
        // Two leading cells match the button's own padding, so the label sits
        // directly above the button text.
        Span::styled("  ACTION", style),
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
