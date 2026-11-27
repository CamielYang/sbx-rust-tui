//! The dashboard frame: header, toolbar, sandbox list, actions, and footer.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Padding, Paragraph};
use ratcn::color::{DISABLED_DIM, FOCUS_SHIFT, HOVER_SHIFT, away_from, dim, nearest_to};
use ratcn::runtime::DeclareCtx;
use ratcn::{
    Button, ButtonStyle, List, ListItem, ListItemState, ListStyle, Tab as UiTab, Tabs,
    TabsActivation, Theme,
};

use super::*;
use crate::app::Secret;

pub(super) fn paint_header(
    ctx: &mut DeclareCtx<'_, App, Msg>,
    app: &App,
    theme: &Theme,
    area: Rect,
) {
    let title = Span::styled(
        "  DOCKER SANDBOXES RUST ",
        Style::default()
            .fg(theme.primary_foreground)
            .bg(theme.primary)
            .add_modifier(Modifier::BOLD),
    );
    let count = Span::styled(
        format!("  {} sandbox(es)  ", app.sandboxes().len()),
        Style::default().fg(theme.muted_foreground),
    );
    let columns = header_columns(area);

    ctx.paint_widget(Paragraph::new(Line::from(vec![title, count])), columns[0]);
}

/// Quit lives on the title row, outside the tabs. It is declared last so
/// startup focus still lands on the first sandbox action, not on Quit.
pub(super) fn paint_quit(ctx: &mut DeclareCtx<'_, App, Msg>, app: &App, area: Rect) {
    let columns = header_columns(area);
    let quit_button = Button::new("Quit")
        .ghost()
        .on_press(|| Msg::Quit)
        .disabled(app.is_creating());
    ctx.component("toolbar-quit", quit_button, columns[1]);
}

fn header_columns(area: Rect) -> [Rect; 2] {
    let quit_width = Button::<Msg>::new("Quit").width() + 2;
    let columns =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(quit_width)]).split(area);
    [columns[0], columns[1]]
}

/// The tab row switches which section owns the body below it. Arrows switch
/// tabs directly, and a click selects a tab without a second confirmation.
pub(super) fn paint_tabs(ctx: &mut DeclareCtx<'_, App, Msg>, app: &App, area: Rect) {
    let tabs = Tabs::new([
        UiTab::new(DashboardTab::Sandboxes, "Sandboxes"),
        UiTab::new(DashboardTab::Secrets, "Secrets"),
    ])
    .selection(|app: &App| Some(app.active_tab()), Msg::TabSelected)
    .activation(TabsActivation::Automatic)
    .disabled(app.is_creating());
    ctx.component(TAB_ROW, tabs, area);
}

pub(super) fn paint_toolbar(
    ctx: &mut DeclareCtx<'_, App, Msg>,
    app: &App,
    theme: &Theme,
    area: Rect,
) {
    let locked = app.is_creating();
    let new_button = Button::new("+ New sandbox")
        .on_press(|| Msg::OpenWizard)
        .disabled(locked);
    let refresh_button = Button::new("Refresh")
        .secondary()
        .on_press(|| Msg::Refresh)
        .disabled(locked);

    let columns = Layout::horizontal([
        Constraint::Length(new_button.width() + 2),
        Constraint::Length(refresh_button.width() + 2),
        Constraint::Min(0),
    ])
    .split(area);

    ctx.component("toolbar-new", new_button, columns[0]);
    ctx.component("toolbar-refresh", refresh_button, columns[1]);

    let hint = Span::styled(
        "  Tab moves focus · Enter activates",
        Style::default().fg(theme.muted_foreground),
    );
    ctx.paint_widget(Paragraph::new(Line::from(hint)), columns[2]);
}

pub(super) fn paint_body(ctx: &mut DeclareCtx<'_, App, Msg>, app: &App, theme: &Theme, area: Rect) {
    let columns = Layout::horizontal([Constraint::Min(24), Constraint::Length(36)]).split(area);
    paint_sandbox_list(ctx, app, theme, columns[0]);
    paint_actions(ctx, app, theme, columns[1]);
}

/// Paint the single white outline around everything a tab shows, and return
/// the padded area its toolbar and body should be drawn into.
pub(super) fn paint_content_frame(
    ctx: &mut DeclareCtx<'_, App, Msg>,
    theme: &Theme,
    area: Rect,
) -> Rect {
    let frame = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::White))
        .style(Style::default().bg(theme.surface))
        .padding(Padding::new(1, 1, 1, 1));
    let inner = frame.inner(area);
    ctx.paint_widget(frame, area);
    inner
}

/// The Secrets tab keeps the toolbar in the same row as the sandboxes tab,
/// with its own three buttons.
pub(super) fn paint_secrets_toolbar(ctx: &mut DeclareCtx<'_, App, Msg>, theme: &Theme, area: Rect) {
    let set_button = Button::new("+ Set secret").on_press(|| Msg::SetSecret);
    let set_custom_button = Button::new("+ Set custom secret").on_press(|| Msg::SetCustomSecret);
    let refresh_button = Button::new("Refresh")
        .secondary()
        .on_press(|| Msg::RefreshSecrets);

    let columns = Layout::horizontal([
        Constraint::Length(set_button.width() + 2),
        Constraint::Length(set_custom_button.width() + 2),
        Constraint::Length(refresh_button.width() + 2),
        Constraint::Min(0),
    ])
    .split(area);

    ctx.component("secrets-set", set_button, columns[0]);
    ctx.component("secrets-set-custom", set_custom_button, columns[1]);
    ctx.component("secrets-refresh", refresh_button, columns[2]);

    let hint = Span::styled(
        "  Tab moves focus · Enter activates",
        Style::default().fg(theme.muted_foreground),
    );
    ctx.paint_widget(Paragraph::new(Line::from(hint)), columns[3]);
}

/// A single pane holding the secret inventory: scope, name, and env var in
/// columns. Each row opens its details; a row button deletes the entry.
pub(super) fn paint_secrets_body(
    ctx: &mut DeclareCtx<'_, App, Msg>,
    app: &App,
    theme: &Theme,
    area: Rect,
) {
    let block = pane_block(" Secrets ", theme.field, theme.border);
    let inner = block.inner(area);
    ctx.paint_widget(block, area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let delete_width = Button::<Msg>::new("Delete").width();
    let columns = Layout::horizontal([
        Constraint::Length(16),
        Constraint::Length(20),
        Constraint::Min(0),
        Constraint::Length(delete_width + 2),
    ])
    .split(inner);
    let name_width = columns[1].width.saturating_sub(1) as usize;
    let env_width = columns[2].width.saturating_sub(1) as usize;
    ctx.paint_widget(
        Paragraph::new(secret_header_line(name_width, env_width, theme)),
        inner,
    );

    let visible = (inner.height - 1) as usize;
    if app.secrets().is_empty() {
        ctx.paint_widget(
            Paragraph::new(Line::from(Span::styled(
                " No secrets yet.",
                Style::default().fg(theme.muted_foreground),
            ))),
            Rect::new(inner.x, inner.y + 1, inner.width, 1),
        );
        return;
    }

    for (index, secret) in app.secrets().iter().take(visible).enumerate() {
        let row = Rect::new(inner.x, inner.y + 1 + index as u16, inner.width, 1);
        let name = secret.name.clone();
        let scope = secret.scope.clone();
        // An invisible ghost button covers the text columns. It is declared
        // first so the row's spans paint over it; at rest there is nothing to
        // see, while hover and focus fill the row behind the text.
        ctx.component(
            format!("secrets-row-{index}"),
            Button::new("")
                .ghost()
                .on_press(move || Msg::OpenSecretInfo(name.clone(), scope.clone())),
            Rect::new(
                inner.x,
                row.y,
                columns[0].width + columns[1].width + columns[2].width,
                1,
            ),
        );
        ctx.paint_widget(
            Paragraph::new(secret_row_line(secret, name_width, env_width, theme)),
            row,
        );

        // Custom rows carry the CLI placeholder, not a service name, so their
        // Delete button goes through the same removal path as an overwrite.
        let delete = match secret.env_key() {
            Some(env_key) => Msg::DeleteCustomSecret(
                env_key.to_owned(),
                secret.scope.clone(),
                secret.placeholder.clone().unwrap_or_default(),
            ),
            None => Msg::DeleteSecret(secret.name.clone(), secret.scope.clone()),
        };
        ctx.component(
            format!("secrets-delete-{index}"),
            Button::new("Delete")
                .ghost()
                .on_press(move || delete.clone()),
            Rect::new(columns[3].x, row.y, delete_width.min(columns[3].width), 1),
        );
    }
}

fn secret_header_line(name_width: usize, env_width: usize, theme: &Theme) -> Line<'static> {
    let style = Style::default()
        .fg(theme.muted_foreground)
        .add_modifier(Modifier::BOLD);
    Line::from(vec![
        Span::styled(format!(" {:<15}", "SCOPE"), style),
        Span::styled(format!("{:<name_width$}", "NAME"), style),
        Span::styled(format!("{:<env_width$}", "ENV VAR"), style),
        // Two leading cells match the button's padding, as in the ports table.
        Span::styled("  DELETE", style),
    ])
}

fn secret_row_line(
    secret: &Secret,
    name_width: usize,
    env_width: usize,
    theme: &Theme,
) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!(" {:<15}", secret.scope),
            Style::default().fg(theme.muted_foreground),
        ),
        Span::styled(
            format!("{:<name_width$}", secret.name),
            Style::default().fg(theme.foreground),
        ),
        Span::styled(
            format!("{:<env_width$}", secret.env_vars_label()),
            Style::default().fg(theme.muted_foreground),
        ),
    ])
}

/// Inner panes keep the theme's border; the white outline belongs to the
/// wrapper, so these stay quiet inside it.
fn pane_block(title: &str, fill: Color, border: Color) -> Block<'_> {
    Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(border))
        .style(Style::default().bg(fill))
}

/// Hovering shows the cursor row, and hover focus puts focus on the list. Let
/// that row carry the feedback: the well stays at one color through rest,
/// focus, and hover.
fn sandbox_list_style(theme: &Theme) -> ListStyle {
    let mut style = ListStyle::from_theme(theme);
    style.focused_background = style.background;
    style.hovered_background = style.background;
    style
}

fn paint_sandbox_list(ctx: &mut DeclareCtx<'_, App, Msg>, app: &App, theme: &Theme, area: Rect) {
    let theme = *theme;
    let block = pane_block(" Sandboxes ", theme.field, theme.border);
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
        .style(sandbox_list_style)
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
            let status_style = Style::default().fg(status_color(running, &theme));
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

// Status and action hues, in the same saturated register as the Tokyo Night palette.
pub(super) const RUNNING_GREEN: Color = Color::Rgb(158, 206, 106);
pub(super) const START_GREEN: Color = RUNNING_GREEN;
pub(super) const STOP_ORANGE: Color = Color::Rgb(255, 158, 100);
pub(super) const EXEC_GRAY: Color = Color::Rgb(122, 132, 166);
pub(super) const REMOVE_RED: Color = Color::Rgb(247, 118, 142);
pub(super) const INFO_BLUE: Color = Color::Rgb(122, 162, 247);
pub(super) const PORTS_CYAN: Color = Color::Rgb(115, 200, 211);

/// A running sandbox is green in both the list and the Actions summary;
/// stopped sandboxes keep the theme's warning hue.
fn status_color(running: bool, theme: &Theme) -> Color {
    if running {
        RUNNING_GREEN
    } else {
        theme.warning
    }
}

/// A filled button keeps its action hue in every state. Focus deepens the fill,
/// hover lifts it, and both change visibly even when the button is focused —
/// ratcn resolves hover before focus, so the pointer always wins.
pub(super) fn action_style(fill: Color) -> impl Fn(&Theme) -> ButtonStyle {
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
    let block = pane_block(" Actions ", theme.surface, theme.border);
    let inner = block.inner(area);
    // The pane's surface goes down first; buttons paint their fills on top.
    ctx.paint_widget(block, area);

    // Row 1 is the blank gap between the summary and the first button.
    let rows = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(1),
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
            Line::from(vec![
                Span::styled("status: ", Style::default().fg(theme.muted_foreground)),
                Span::styled(
                    sandbox.status.clone(),
                    Style::default().fg(status_color(running, theme)),
                ),
            ]),
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
        place_button(ctx, id, button, rows[row + 2], button_width);
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

pub(super) fn paint_footer(
    ctx: &mut DeclareCtx<'_, App, Msg>,
    theme: &Theme,
    area: Rect,
) {
    // Notifications live in toasts; the footer is a permanent shortcut legend.
    ctx.paint_widget(
        Paragraph::new(Line::from(Span::styled(
            " N new · R refresh · S start/stop · X exec · P ports · I info · D remove · Ctrl+C quit",
            Style::default().fg(theme.muted_foreground),
        ))),
        area,
    );
}
