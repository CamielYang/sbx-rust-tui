use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use crate::app::{
    AGENTS, App, ClickAction, ClickTarget, GRID_COLUMNS, Mode, SandboxDetails, WIZARD_STEPS,
    Wizard, WizardStep,
};

const BLOCK_HEIGHT: u16 = 4;
const SLOT_HEIGHT: u16 = BLOCK_HEIGHT + 1;

pub fn render(frame: &mut Frame, app: &mut App) {
    let [body, footer_area] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(frame.area());

    let [title_area, grid_area] =
        Layout::vertical([Constraint::Length(3), Constraint::Min(BLOCK_HEIGHT)]).areas(body);

    // Keep the app's scroll offset in sync with the space actually available.
    app.set_visible_rows((grid_area.height / SLOT_HEIGHT) as usize);

    let interactive = app.mode() == Mode::Normal;
    let mode = app.mode();
    let mut click_targets = Vec::new();
    render_title(frame, title_area);
    render_grid(frame, grid_area, app, interactive, &mut click_targets);
    render_footer(frame, footer_area, app);

    match mode {
        Mode::Wizard => render_modal(frame, body, app.wizard(), &mut click_targets),
        Mode::Info => {
            if let Some(details) = app.info() {
                render_info_modal(frame, body, details, &mut click_targets);
            }
        }
        Mode::Normal => {}
    }

    app.set_click_targets(click_targets);
}

fn render_title(frame: &mut Frame, area: Rect) {
    let title = Paragraph::new(vec![
        Line::from(Span::styled(
            "Docker Sandbox Builder",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "Active sandboxes",
            Style::default().fg(Color::DarkGray),
        )),
    ])
    .alignment(Alignment::Center);
    frame.render_widget(title, area);
}

fn render_grid(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    interactive: bool,
    click_targets: &mut Vec<ClickTarget>,
) {
    let column_width = area.width / GRID_COLUMNS as u16;
    if column_width == 0 {
        return;
    }

    let dim = if interactive {
        Modifier::empty()
    } else {
        Modifier::DIM
    };
    let visible_rows = (area.height / SLOT_HEIGHT) as usize;

    for row in 0..visible_rows {
        let row_index = app.grid_offset() + row;
        let y = area.y + row as u16 * SLOT_HEIGHT;

        for column in 0..GRID_COLUMNS {
            let index = row_index * GRID_COLUMNS + column;
            if index >= app.block_count() {
                break;
            }

            let x = area.x + column as u16 * column_width;
            let width = if column + 1 == GRID_COLUMNS {
                area.width - column_width * (GRID_COLUMNS as u16 - 1)
            } else {
                column_width
            };
            let rect = Rect {
                x,
                y,
                width,
                height: BLOCK_HEIGHT,
            };
            // Whole-block target first: button targets are added by
            // render_block afterwards so they win on overlapping clicks.
            if interactive {
                let action = if index == app.sandboxes().len() {
                    ClickAction::OpenWizard
                } else {
                    ClickAction::Select(index)
                };
                click_targets.push(ClickTarget::new(rect, action));
            }
            render_block(frame, rect, app, index, dim, interactive, click_targets);
        }
    }
}

fn render_block(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    index: usize,
    dim: Modifier,
    interactive: bool,
    click_targets: &mut Vec<ClickTarget>,
) {
    let selected = index == app.selected_index();
    let is_button = index == app.sandboxes().len();

    let border_style = if selected {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD | dim)
    } else if is_button {
        Style::default().fg(Color::Yellow).add_modifier(dim)
    } else {
        Style::default().fg(Color::DarkGray).add_modifier(dim)
    };

    let text_style = if selected {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD | dim)
    } else if is_button {
        Style::default().fg(Color::Yellow).add_modifier(dim)
    } else {
        Style::default().add_modifier(dim)
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if is_button {
        let paragraph =
            Paragraph::new(Line::from(Span::styled("+ New Sandbox", text_style)))
                .alignment(Alignment::Center);
        frame.render_widget(paragraph, inner);
        return;
    }

    let sandbox = &app.sandboxes()[index];
    let [title_area, buttons_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(inner);

    let dot_color = if sandbox.is_running() {
        Color::Green
    } else {
        Color::Red
    };
    let title_line = Line::from(vec![
        Span::styled(
            "● ",
            Style::default().fg(dot_color).add_modifier(dim),
        ),
        Span::styled(sandbox.name.clone(), text_style),
    ])
    .alignment(Alignment::Left);
    frame.render_widget(Paragraph::new(title_line), title_area);

    let columns = Layout::horizontal([
        Constraint::Ratio(1, 4),
        Constraint::Ratio(1, 4),
        Constraint::Ratio(1, 4),
        Constraint::Ratio(1, 4),
    ])
    .split(buttons_area);

    let buttons = [
        ("Stop", Color::LightRed, ClickAction::Stop(index)),
        ("Exec", Color::LightGreen, ClickAction::Exec(index)),
        ("Remove", Color::Red, ClickAction::Remove(index)),
        ("Info", Color::LightBlue, ClickAction::Info(index)),
    ];
    for (cell, (label, color, action)) in columns.iter().zip(buttons) {
        let text = button_label(label, cell.width);
        let style = Style::default().fg(color).add_modifier(dim);
        frame.render_widget(
            Paragraph::new(text).alignment(Alignment::Center).style(style),
            *cell,
        );
        if interactive {
            click_targets.push(ClickTarget::new(*cell, action));
        }
    }
}

/// Trim a button label to the width of its column.
fn button_label(label: &str, width: u16) -> String {
    label.chars().take(width as usize).collect()
}

fn modal_rect(area: Rect) -> Option<Rect> {
    let width = area.width.saturating_sub(6).min(78);
    let height = area.height.saturating_sub(2).min(24);
    if width == 0 || height == 0 {
        return None;
    }
    Some(Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    })
}

/// Draw the shared modal frame and its [X] close button.
fn render_modal_chrome(
    frame: &mut Frame,
    rect: Rect,
    title: &str,
    close_action: ClickAction,
    click_targets: &mut Vec<ClickTarget>,
) -> Option<Rect> {
    frame.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(Color::Cyan))
        .style(Style::default().bg(Color::Black));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    // Close button sits on the top-right edge of the modal border.
    let close_width = 4.min(rect.width.saturating_sub(2));
    if close_width > 0 {
        let close_rect = Rect {
            x: rect.right().saturating_sub(close_width + 1),
            y: rect.y,
            width: close_width,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new("[X]").style(
                Style::default()
                    .fg(Color::Red)
                    .bg(Color::Black)
                    .add_modifier(Modifier::BOLD),
            ),
            close_rect,
        );
        // Include the adjacent border corner in the hit area: users tend to
        // click the very top-right cell of the modal.
        let close_hit_area = Rect {
            width: (close_width + 1).min(rect.width),
            ..close_rect
        };
        click_targets.push(ClickTarget::new(close_hit_area, close_action));
    }

    Some(inner)
}

fn render_modal(
    frame: &mut Frame,
    area: Rect,
    wizard: &Wizard,
    click_targets: &mut Vec<ClickTarget>,
) {
    let Some(rect) = modal_rect(area) else {
        return;
    };
    let Some(inner) = render_modal_chrome(
        frame,
        rect,
        " New Sandbox ",
        ClickAction::CloseWizard,
        click_targets,
    ) else {
        return;
    };

    // Reserve the last inner row for the Create button.
    let [content_area, button_area] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(inner);
    let paragraph = Paragraph::new(wizard_lines(wizard)).wrap(Wrap { trim: false });
    frame.render_widget(paragraph, content_area);

    let label = "[ Create Sandbox ]";
    let button_width = (label.len() as u16).min(button_area.width);
    if button_width > 0 {
        let button_rect = Rect {
            x: button_area.x + (button_area.width - button_width) / 2,
            y: button_area.y,
            width: button_width,
            height: 1,
        };
        let button_style = if wizard.is_ready() {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM)
        };
        frame.render_widget(
            Paragraph::new(label)
                .alignment(Alignment::Center)
                .style(button_style),
            button_rect,
        );
        if wizard.is_ready() {
            click_targets.push(ClickTarget::new(button_rect, ClickAction::CreateSandbox));
        }
    }
}

fn render_info_modal(
    frame: &mut Frame,
    area: Rect,
    details: &SandboxDetails,
    click_targets: &mut Vec<ClickTarget>,
) {
    let Some(rect) = modal_rect(area) else {
        return;
    };
    let Some(inner) = render_modal_chrome(
        frame,
        rect,
        " Sandbox Info ",
        ClickAction::CloseInfo,
        click_targets,
    ) else {
        return;
    };

    let paragraph = Paragraph::new(info_lines(details))
        .wrap(Wrap { trim: false })
        .style(Style::default().bg(Color::Black));
    frame.render_widget(paragraph, inner);
}

fn info_lines(details: &SandboxDetails) -> Vec<Line<'static>> {
    let dim = Style::default().fg(Color::DarkGray);
    let running = details.status.trim().to_ascii_lowercase().contains("running");
    let status_color = if running { Color::Green } else { Color::Red };
    let status_style = Style::default().fg(status_color).add_modifier(Modifier::BOLD);

    let mut lines = vec![
        Line::from(vec![
            Span::styled("Name:   ", dim),
            Span::styled(details.name.clone(), Style::default().fg(Color::Cyan)),
        ]),
        Line::from(vec![
            Span::styled("Status: ", dim),
            Span::styled("● ", status_style),
            Span::styled(details.status.clone(), status_style),
        ]),
        Line::raw(""),
        Line::from(Span::styled(
            "Resources",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        resource_line("CPU", details.cpus.as_deref()),
        usage_line(
            "Memory",
            details.mem_used.as_deref(),
            details.mem_total.as_deref(),
        ),
        usage_line(
            "Root size",
            details.root_used.as_deref(),
            details.root_total.as_deref(),
        ),
        usage_line(
            "Docker size",
            details.docker_used.as_deref(),
            details.docker_total.as_deref(),
        ),
    ];

    if let Some(error) = &details.error {
        lines.push(Line::raw(""));
        lines.push(Line::from(Span::styled(
            format!("Note: {error}"),
            Style::default().fg(Color::Yellow),
        )));
    }

    lines
}

fn resource_line(label: &str, value: Option<&str>) -> Line<'static> {
    labeled_line(label, value.unwrap_or("n/a").to_owned())
}

fn usage_line(label: &str, used: Option<&str>, total: Option<&str>) -> Line<'static> {
    let value = match (used, total) {
        (Some(used), Some(total)) => format!("{used} used / {total} total"),
        (Some(used), None) => format!("{used} used / n/a total"),
        (None, Some(total)) => format!("n/a used / {total} total"),
        (None, None) => "n/a".to_owned(),
    };
    labeled_line(label, value)
}

fn labeled_line(label: &str, value: String) -> Line<'static> {
    let name = format!("  {label}:");
    let padding = " ".repeat(15usize.saturating_sub(name.len()));
    Line::from(vec![
        Span::styled(
            format!("{name}{padding}"),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(value, Style::default().fg(Color::White)),
    ])
}

fn wizard_lines(wizard: &Wizard) -> Vec<Line<'static>> {
    let current_index = WIZARD_STEPS
        .iter()
        .position(|step| *step == wizard.step())
        .unwrap_or(0);

    let mut lines = vec![
        Line::from(Span::styled(
            wizard.step().prompt(),
            Style::default().fg(Color::DarkGray),
        )),
        Line::raw(""),
    ];

    for (index, step) in WIZARD_STEPS.iter().enumerate() {
        let marker = if index < current_index {
            "[✓]"
        } else if index == current_index {
            "[>]"
        } else {
            "[ ]"
        };
        let style = if index == current_index {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        let mut value = wizard.value_for(*step);
        if value.is_empty() {
            value = if *step == WizardStep::Name {
                "<required>".to_owned()
            } else {
                "<empty>".to_owned()
            };
        }
        if index == current_index && step.is_text() {
            value.push('▌');
        }

        let mut spans = vec![
            Span::styled(format!("{marker} "), style),
            Span::styled(step.label(), style),
        ];
        if *step == WizardStep::Name {
            spans.push(Span::styled(
                "*",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ));
        }
        spans.push(Span::styled(": ", style));
        spans.push(Span::styled(value, style));
        lines.push(Line::from(spans));
    }

    lines.push(Line::raw(""));
    if let Some(error) = wizard.field_error() {
        lines.push(Line::from(Span::styled(
            format!("Error: {error}"),
            Style::default().fg(Color::Red),
        )));
    }
    if let Some(warning) = wizard.warning() {
        lines.push(Line::from(Span::styled(
            warning,
            Style::default().fg(Color::Yellow),
        )));
    }

    if wizard.step() == WizardStep::Agent {
        lines.extend(agent_options(wizard.agent_index()));
    }

    if wizard.step() == WizardStep::Confirm {
        lines.push(Line::raw(""));
        lines.push(Line::from(Span::styled(
            "Press y/Enter or click Create Sandbox.",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )));
    }

    lines
}

/// Two-column list of selectable agents, with the current one marked.
fn agent_options(selected: usize) -> Vec<Line<'static>> {
    let column_width = AGENTS.iter().map(|agent| agent.len()).max().unwrap_or(0) + 4;

    let mut lines = vec![Line::from(Span::styled(
        "Agents (↑/↓ or j/k to change):",
        Style::default().fg(Color::DarkGray),
    ))];

    for row in 0..AGENTS.len().div_ceil(2) {
        let mut spans = Vec::new();
        for column in 0..2 {
            let index = row * 2 + column;
            let Some(agent) = AGENTS.get(index) else {
                continue;
            };
            if column == 1 {
                let first_width = 2 + AGENTS[index - 1].len();
                spans.push(Span::raw(" ".repeat(column_width - first_width)));
            }
            let marker = if index == selected { ">" } else { " " };
            let style = if index == selected {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            spans.push(Span::styled(format!("{marker} {agent}"), style));
        }
        lines.push(Line::from(spans));
    }

    lines
}

fn render_footer(frame: &mut Frame, area: Rect, app: &App) {
    let hints = match app.mode() {
        Mode::Normal => {
            " click: Stop/Exec/Remove/Info   s/x/d/i keys   wheel scroll   ←/→/↑/↓ move   r refresh   q quit "
        }
        Mode::Wizard => match app.wizard().step() {
            WizardStep::Agent => " ↑/↓ choose agent   ↵ next   Esc back / click [X] close ",
            WizardStep::Confirm => " ↵/y create   n/Esc back   click Create Sandbox ",
            _ => " type value   Ctrl+U clear   ↵ next   Esc back / click [X] close ",
        },
        Mode::Info => " Esc/q/i/↵ or click [X] close ",
    };

    let line = Line::from(vec![
        Span::styled(hints, Style::default().fg(Color::Black).bg(Color::White)),
        Span::raw(" "),
        Span::styled(app.message(), Style::default().fg(Color::LightBlue)),
    ]);

    frame.render_widget(Paragraph::new(line), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{
        KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    };
    use ratatui::{Terminal, backend::TestBackend};

    fn click(app: &mut App, action: ClickAction) {
        let target = app
            .click_targets()
            .iter()
            .find(|target| target.action == action)
            .copied()
            .unwrap_or_else(|| panic!("no click target for {action:?}"));
        // Select/OpenWizard cover the whole block; click the title row so a
        // button in the lower row doesn't take the event.
        let row = if matches!(action, ClickAction::Select(_) | ClickAction::OpenWizard) {
            target.area.y + 1
        } else {
            target.area.y + target.area.height / 2
        };
        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: target.area.x + target.area.width / 2,
            row,
            modifiers: KeyModifiers::NONE,
        });
    }

    fn snapshot(width: u16, height: u16, app: &mut App) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| render(frame, app)).unwrap();
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol().to_owned())
                    .collect::<String>()
            })
            .collect()
    }

    #[test]
    fn dashboard_shows_title_grid_and_button() {
        let mut app = App::with_sandbox_statuses(&[
            ("alpha", "running"),
            ("beta", "stopped"),
            ("gamma", "running"),
            ("delta", "stopped"),
        ]);
        let screen = snapshot(90, 24, &mut app).join("\n");

        assert!(screen.contains("Docker Sandbox Builder"));
        assert!(screen.contains("alpha"));
        assert!(screen.contains("gamma"));
        assert!(screen.contains("+ New Sandbox"));
    }

    #[test]
    fn sandbox_blocks_have_four_button_targets() {
        let mut app = App::with_sandboxes(&["alpha", "beta"]);
        snapshot(90, 24, &mut app);

        for action in [
            ClickAction::Stop(0),
            ClickAction::Exec(0),
            ClickAction::Remove(0),
            ClickAction::Info(0),
        ] {
            assert!(
                app.click_targets().iter().any(|target| target.action == action),
                "missing click target for {action:?}"
            );
        }
    }

    #[test]
    fn status_dot_is_green_only_for_running_sandboxes() {
        let mut app = App::with_sandbox_statuses(&[("alpha", "running"), ("beta", "stopped")]);
        let mut terminal = Terminal::new(TestBackend::new(90, 24)).unwrap();
        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        let buffer = terminal.backend().buffer();

        // Blocks are 30 columns wide; the dot is the first inner cell of the
        // title row (y = 3 title rows + 1 border).
        assert_eq!(buffer[(1, 4)].fg, Color::Green);
        assert_eq!(buffer[(31, 4)].fg, Color::Red);
    }

    #[test]
    fn exec_button_prepares_external_terminal_command() {
        let mut app = App::with_sandboxes(&["alpha"]);
        snapshot(90, 24, &mut app);

        click(&mut app, ClickAction::Exec(0));
        let pending = app.take_external_command().expect("exec should be pending");
        assert_eq!(pending.preview(), "sbx exec -it alpha bash");
    }

    #[test]
    fn info_button_opens_modal_with_resources() {
        let mut app = App::with_sandboxes(&["alpha"]);
        snapshot(90, 24, &mut app);

        click(&mut app, ClickAction::Info(0));
        assert!(matches!(app.mode(), Mode::Info));

        let screen = snapshot(90, 24, &mut app).join("\n");
        assert!(screen.contains("Sandbox Info"));
        assert!(screen.contains("Name:"));
        assert!(screen.contains("alpha"));
        assert!(screen.contains("Status:"));
        assert!(screen.contains("Resources"));
        assert!(screen.contains("CPU:"));
        assert!(screen.contains("Memory:"));
        assert!(screen.contains("Root size:"));
        assert!(screen.contains("Docker size:"));

        click(&mut app, ClickAction::CloseInfo);
        assert!(matches!(app.mode(), Mode::Normal));
    }

    #[test]
    fn usage_line_formats_used_and_total() {
        let line = usage_line("Memory", Some("1.0 GiB"), Some("2.0 GiB"));
        let text: String = line
            .spans
            .iter()
            .map(|span| span.content.to_string())
            .collect();
        assert!(text.contains("Memory:"));
        assert!(text.contains("1.0 GiB used / 2.0 GiB total"));

        let partial = usage_line("Root size", None, Some("40g"));
        let text: String = partial
            .spans
            .iter()
            .map(|span| span.content.to_string())
            .collect();
        assert!(text.contains("n/a used / 40g total"));
    }

    #[test]
    fn wizard_renders_as_modal() {
        let mut app = App::with_sandboxes(&["alpha"]);
        app.handle_key(crossterm::event::KeyEvent::new(
            KeyCode::End,
            KeyModifiers::NONE,
        ));
        app.handle_key(crossterm::event::KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        ));
        assert!(matches!(app.mode(), Mode::Wizard));

        let screen = snapshot(90, 24, &mut app).join("\n");
        assert!(screen.contains("New Sandbox"));
        assert!(screen.contains("Name*"));
        assert!(screen.contains("<required>"));
    }

    #[test]
    fn mouse_clicks_select_blocks_and_open_wizard() {
        let mut app = App::with_sandboxes(&["alpha", "beta"]);
        snapshot(90, 24, &mut app);

        click(&mut app, ClickAction::Select(1));
        assert_eq!(app.selected_index(), 1);

        snapshot(90, 24, &mut app);
        click(&mut app, ClickAction::OpenWizard);
        assert!(matches!(app.mode(), Mode::Wizard));
    }

    #[test]
    fn modal_has_close_and_create_buttons() {
        let mut app = App::with_sandboxes(&["alpha"]);
        app.handle_key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        let screen = snapshot(90, 24, &mut app).join("\n");
        assert!(screen.contains("[X]"));
        assert!(screen.contains("[ Create Sandbox ]"));

        click(&mut app, ClickAction::CloseWizard);
        assert!(matches!(app.mode(), Mode::Normal));
    }

    #[test]
    fn create_button_is_disabled_until_review() {
        let mut app = App::with_sandboxes(&["alpha"]);
        app.handle_key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        snapshot(90, 24, &mut app);

        assert!(
            app.click_targets()
                .iter()
                .all(|target| target.action != ClickAction::CreateSandbox)
        );

        for character in "demo-tui".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
        }
        for _ in 0..6 {
            app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        }
        assert!(app.wizard().is_ready());

        let screen = snapshot(90, 24, &mut app).join("\n");
        assert!(
            app.click_targets()
                .iter()
                .any(|target| target.action == ClickAction::CreateSandbox)
        );
        assert!(!screen.contains("sbx create"));
    }

    #[test]
    fn agent_step_lists_all_agents() {
        let mut app = App::with_sandboxes(&["alpha"]);
        app.handle_key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        for character in "demo-tui".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
        }
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.wizard().step(), WizardStep::Agent);

        let screen = snapshot(90, 24, &mut app).join("\n");
        assert!(screen.contains("Agents (↑/↓ or j/k to change):"));
        for agent in AGENTS {
            assert!(screen.contains(agent), "missing agent {agent}");
        }
        assert!(screen.contains("> opencode"));
    }

    #[test]
    fn mouse_wheel_moves_selection() {
        let mut app = App::with_sandboxes(&["alpha", "beta", "gamma", "delta"]);
        snapshot(90, 24, &mut app);

        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 5,
            row: 5,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(app.selected_index(), 3);
    }
}
