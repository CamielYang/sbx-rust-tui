use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use crate::app::{
    App, ClickAction, ClickTarget, GRID_COLUMNS, Mode, WIZARD_STEPS, Wizard, WizardStep,
};

const BLOCK_HEIGHT: u16 = 3;
const SLOT_HEIGHT: u16 = BLOCK_HEIGHT + 1;

pub fn render(frame: &mut Frame, app: &mut App) {
    let [body, footer_area] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(frame.area());

    let [title_area, grid_area] =
        Layout::vertical([Constraint::Length(3), Constraint::Min(BLOCK_HEIGHT)]).areas(body);

    // Keep the app's scroll offset in sync with the space actually available.
    app.set_visible_rows((grid_area.height / SLOT_HEIGHT) as usize);

    let mut click_targets = Vec::new();
    render_title(frame, title_area);
    render_grid(
        frame,
        grid_area,
        app,
        app.mode() == Mode::Wizard,
        &mut click_targets,
    );
    render_footer(frame, footer_area, app);

    if app.mode() == Mode::Wizard {
        render_modal(frame, body, app.wizard(), &mut click_targets);
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
    dimmed: bool,
    click_targets: &mut Vec<ClickTarget>,
) {
    let column_width = area.width / GRID_COLUMNS as u16;
    if column_width == 0 {
        return;
    }

    let dim = if dimmed {
        Modifier::DIM
    } else {
        Modifier::empty()
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
            render_block(frame, rect, app, index, dim);
            if !dimmed {
                let action = if index == app.sandboxes().len() {
                    ClickAction::OpenWizard
                } else {
                    ClickAction::Select(index)
                };
                click_targets.push(ClickTarget::new(rect, action));
            }
        }
    }
}

fn render_block(frame: &mut Frame, area: Rect, app: &App, index: usize, dim: Modifier) {
    let selected = index == app.selected_index();
    let is_button = index == app.sandboxes().len();
    let label = if is_button {
        "+ New Sandbox".to_owned()
    } else {
        app.sandboxes()[index].clone()
    };

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

    let paragraph =
        Paragraph::new(Line::from(Span::styled(label, text_style))).alignment(Alignment::Center);
    frame.render_widget(paragraph, inner);
}

fn render_modal(
    frame: &mut Frame,
    area: Rect,
    wizard: &Wizard,
    click_targets: &mut Vec<ClickTarget>,
) {
    let width = area.width.saturating_sub(6).min(78);
    let height = area.height.saturating_sub(4).min(24);
    if width == 0 || height == 0 {
        return;
    }

    let rect = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    };

    frame.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" New Sandbox ")
        .border_style(Style::default().fg(Color::Cyan))
        .style(Style::default().bg(Color::Black));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    // Close button sits on the top-right edge of the modal border.
    let close_width = 4.min(width.saturating_sub(2));
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
            width: (close_width + 1).min(width),
            ..close_rect
        };
        click_targets.push(ClickTarget::new(close_hit_area, ClickAction::CloseWizard));
    }

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
        frame.render_widget(
            Paragraph::new(label).alignment(Alignment::Center).style(
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            button_rect,
        );
        click_targets.push(ClickTarget::new(button_rect, ClickAction::CreateSandbox));
    }
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
            value = "<empty>".to_owned();
        }
        if index == current_index && step.is_text() {
            value.push('▌');
        }

        lines.push(Line::from(vec![
            Span::styled(format!("{marker} {}: ", step.label()), style),
            Span::styled(value, style),
        ]));
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

    if wizard.step() == WizardStep::Confirm {
        lines.push(Line::raw(""));
        lines.push(Line::from(Span::styled(
            wizard.preview(),
            Style::default().fg(Color::Green),
        )));
        lines.push(Line::from(Span::styled(
            "Press y/Enter or click Create Sandbox.",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )));
    }

    lines
}

fn render_footer(frame: &mut Frame, area: Rect, app: &App) {
    let hints = match app.mode() {
        Mode::Normal => {
            " click select / create   wheel scroll   ←/→/↑/↓ move   ↵ create   r refresh   q quit "
        }
        Mode::Wizard => match app.wizard().step() {
            WizardStep::Agent => " ↑/↓ choose agent   ↵ next   Esc back / click [X] close ",
            WizardStep::Confirm => " ↵/y create   n/Esc back   click Create Sandbox ",
            _ => " type value   Ctrl+U clear   ↵ next   Esc back / click [X] close ",
        },
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
        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: target.area.x + target.area.width / 2,
            row: target.area.y + target.area.height / 2,
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
        let mut app = App::with_sandboxes(&["alpha", "beta", "gamma", "delta"]);
        let screen = snapshot(90, 24, &mut app).join("\n");

        assert!(screen.contains("Docker Sandbox Builder"));
        assert!(screen.contains("alpha"));
        assert!(screen.contains("gamma"));
        assert!(screen.contains("+ New Sandbox"));
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
        assert!(screen.contains("Name"));
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
    fn create_button_validates_every_field() {
        let mut app = App::with_sandboxes(&["alpha"]);
        app.handle_key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        snapshot(90, 24, &mut app);

        // The Create button is usable from any step; an empty name is rejected
        // instead of running `sbx create` with no name.
        click(&mut app, ClickAction::CreateSandbox);
        assert!(matches!(app.mode(), Mode::Wizard));
        assert!(app.message().contains("Name must be 2-63 characters."));
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
