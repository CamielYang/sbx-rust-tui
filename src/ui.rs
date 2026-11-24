use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
};

use crate::app::{App, Mode, WIZARD_STEPS, Wizard, WizardStep};

pub fn render(frame: &mut Frame, app: &App) {
    let [main_area, footer_area] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(frame.area());

    let outer = Block::default()
        .borders(Borders::ALL)
        .title(" sbx-rust-tui — command launcher ");
    frame.render_widget(&outer, main_area);

    let inner = outer.inner(main_area);
    let [list_area, detail_area] =
        Layout::horizontal([Constraint::Percentage(38), Constraint::Percentage(62)]).areas(inner);

    render_list(frame, list_area, app);
    render_detail(frame, detail_area, app);
    render_footer(frame, footer_area, app);
}

fn render_list(frame: &mut Frame, area: Rect, app: &App) {
    let items = app
        .menu()
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let style = if index == app.selected_index() {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD | Modifier::REVERSED)
            } else {
                Style::default()
            };
            ListItem::new(Line::from(Span::styled(item.name(), style)))
        })
        .collect::<Vec<_>>();

    let list = List::new(items).block(Block::default().borders(Borders::ALL).title(" Commands "));
    frame.render_widget(list, area);
}

fn render_detail(frame: &mut Frame, area: Rect, app: &App) {
    if app.mode() == Mode::Wizard {
        render_wizard(frame, area, app.wizard());
        return;
    }

    let Some(item) = app.menu().get(app.selected_index()) else {
        return;
    };

    let mut lines = vec![
        Line::from(Span::styled(
            item.name(),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::raw(item.description())),
        Line::raw(""),
        Line::from(Span::styled(
            "Command",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            item.preview(),
            Style::default().fg(Color::Green),
        )),
        Line::raw(""),
    ];

    if app.mode() == Mode::Confirm {
        lines.push(Line::from(Span::styled(
            "Run this command? [y/Enter] confirm  [n/Esc] cancel",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::raw(""));
    }

    if let Some(result) = app.result() {
        let result_style = if result.success {
            Style::default().fg(Color::Green)
        } else {
            Style::default().fg(Color::Red)
        };
        lines.push(Line::from(Span::styled(
            result.command.clone(),
            result_style,
        )));
        lines.push(Line::raw(""));

        for line in result.stdout.lines() {
            lines.push(Line::from(Span::raw(line.to_owned())));
        }
        if !result.stderr.is_empty() {
            lines.push(Line::raw(""));
            lines.push(Line::from(Span::styled(
                "stderr",
                Style::default().fg(Color::Red),
            )));
            for line in result.stderr.lines() {
                lines.push(Line::from(Span::styled(
                    line.to_owned(),
                    Style::default().fg(Color::Red),
                )));
            }
        }
    }

    let detail = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(" Details "))
        .wrap(Wrap { trim: false })
        .scroll((app.output_scroll(), 0));

    frame.render_widget(detail, area);
}

fn render_wizard(frame: &mut Frame, area: Rect, wizard: &Wizard) {
    let current_index = WIZARD_STEPS
        .iter()
        .position(|step| *step == wizard.step())
        .unwrap_or(0);

    let mut lines = vec![
        Line::from(Span::styled(
            "Create a new sandbox",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
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
            "Press y/Enter to create the sandbox.",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )));
    }

    let detail = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" New sandbox "),
        )
        .wrap(Wrap { trim: false });

    frame.render_widget(detail, area);
}

fn render_footer(frame: &mut Frame, area: Rect, app: &App) {
    let hints = match app.mode() {
        Mode::Normal => " ↑/↓ select   ↵ confirm   PgUp/PgDn scroll   c clear   q quit ",
        Mode::Confirm => " ↵/y run   n/Esc cancel ",
        Mode::Wizard => match app.wizard().step() {
            WizardStep::Agent => " ↑/↓ choose agent   ↵ next   Esc back ",
            WizardStep::Confirm => " ↵/y create   n/Esc back ",
            _ => " type value   Ctrl+U clear   ↵ next   Esc back ",
        },
    };

    let line = Line::from(vec![
        Span::styled(hints, Style::default().fg(Color::Black).bg(Color::White)),
        Span::raw(" "),
        Span::styled(app.message(), Style::default().fg(Color::LightBlue)),
    ]);

    frame.render_widget(Paragraph::new(line), area);
}
