//! Small shared widgets used by the dashboard and its dialogs.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratcn::runtime::{Component, DeclareCtx};
use ratcn::{Input, InputState, Theme};

use super::*;

/// An input bound to an app field through a caller-supplied message, so the
/// create form and the ports dialog can share the same themed widget.
pub(super) fn input(
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

pub(super) fn field_row<C>(
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

/// One row in the sandbox-details table. Missing values get an em dash so the
/// columns stay aligned.
pub(super) fn detail_line(label: &str, value: Option<&str>) -> String {
    let value = value.unwrap_or("—");
    format!("{label:<12} {value}")
}

/// Show whichever half of a used/total pair is available.
pub(super) fn join_usage(used: Option<&str>, total: Option<&str>) -> Option<String> {
    match (used, total) {
        (Some(used), Some(total)) => Some(format!("{used} / {total}")),
        (Some(used), None) => Some(used.to_owned()),
        (None, Some(total)) => Some(total.to_owned()),
        (None, None) => None,
    }
}
