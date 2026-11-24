use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
};

use crate::app::{App, Mode};

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

fn render_list(frame: &mut Frame, area: ratatui::layout::Rect, app: &App) {
    let items = app
        .commands()
        .iter()
        .enumerate()
        .map(|(index, command)| {
            let style = if index == app.selected_index() {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD | Modifier::REVERSED)
            } else {
                Style::default()
            };
            ListItem::new(Line::from(Span::styled(command.name, style)))
        })
        .collect::<Vec<_>>();

    let list = List::new(items).block(Block::default().borders(Borders::ALL).title(" Commands "));
    frame.render_widget(list, area);
}

fn render_detail(frame: &mut Frame, area: ratatui::layout::Rect, app: &App) {
    let Some(command) = app.commands().get(app.selected_index()) else {
        return;
    };

    let mut lines = vec![
        Line::from(Span::styled(
            command.name,
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::raw(command.description)),
        Line::raw(""),
        Line::from(Span::styled(
            "Command",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            command.preview(),
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

fn render_footer(frame: &mut Frame, area: ratatui::layout::Rect, app: &App) {
    let hints = match app.mode() {
        Mode::Normal => " ↑/↓ select   ↵ confirm   PgUp/PgDn scroll   c clear   q quit ",
        Mode::Confirm => " ↵/y run   n/Esc cancel ",
    };

    let line = Line::from(vec![
        Span::styled(hints, Style::default().fg(Color::Black).bg(Color::White)),
        Span::raw(" "),
        Span::styled(app.message(), Style::default().fg(Color::LightBlue)),
    ]);

    frame.render_widget(Paragraph::new(line), area);
}
