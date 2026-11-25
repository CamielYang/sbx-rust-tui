//! Ratcn-based dashboard for `sbx` sandboxes.
//!
//! Every control is either a ratcn component or themed paint queued through
//! ratcn's declaration context. The runtime owns focus, hover, hit testing, and
//! modal layering; [`App`](crate::app::App) owns the state and [`apply`] is the
//! single place where component messages become state changes.

use ratatui::layout::{Constraint, Layout};
use ratatui::style::Style;
use ratatui::widgets::Block;
use ratcn::runtime::{ChildId, DeclareCtx, FocusState};
use ratcn::{InputState, Theme};

use crate::app::{
    App, CONFIRM_DIALOG, CREATE_DIALOG, FormField, INFO_DIALOG, PORTS_DIALOG, PortField, Sandbox,
    WORKSPACE_DIALOG,
};

mod dashboard;
mod dialogs;
mod widgets;

use self::dashboard::{paint_body, paint_footer, paint_header, paint_toolbar};
use self::dialogs::{confirm_dialog, create_dialog, info_dialog, ports_dialog, workspace_dialog};

// The UI tests reach these helpers through `use super::*`.
#[cfg(test)]
use self::dashboard::{
    EXEC_GRAY, INFO_BLUE, PORTS_CYAN, REMOVE_RED, RUNNING_GREEN, START_GREEN, STOP_ORANGE,
    action_style,
};
#[cfg(test)]
use self::dialogs::{cancel_style, cpu_text};
#[cfg(test)]
use crate::app::{PendingAction, PortMapping};
#[cfg(test)]
use ratatui::style::Color;

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
    AddWorkspaceChanged(bool),
    ChangeWorkspace,
    CancelWorkspace,
    CreateWorkspaceHere,
    WorkspaceEntryFocused(String, usize),
    WorkspaceEntrySelected(String),
    WorkspaceScrolled(usize),
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
        Msg::AddWorkspaceChanged(enabled) => app.set_add_workspace(enabled),
        Msg::ChangeWorkspace => app.open_workspace_picker(),
        Msg::CancelWorkspace => app.cancel_workspace_picker(),
        Msg::CreateWorkspaceHere => app.choose_workspace_dir(),
        Msg::WorkspaceEntryFocused(name, scroll) => app.focus_workspace_entry(name, scroll),
        Msg::WorkspaceEntrySelected(name) => app.select_workspace_entry(name),
        Msg::WorkspaceScrolled(offset) => app.set_workspace_scroll(offset),
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
        Constraint::Length(1), // title
        Constraint::Length(1), // spacing
        Constraint::Length(1), // toolbar buttons
        Constraint::Length(1), // spacing
        Constraint::Min(0),    // sandboxes + actions
        Constraint::Length(1), // spacing
        Constraint::Length(1), // footer
    ])
    .split(area);

    paint_header(ctx, app, &theme, vertical[0]);
    paint_toolbar(ctx, app, &theme, vertical[2]);
    paint_body(ctx, app, &theme, vertical[4]);
    paint_footer(ctx, &theme, vertical[6]);

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
    // Declared last: the picker stacks above the create wizard that opened it.
    if app.modals_ref().is_open(WORKSPACE_DIALOG) {
        ctx.modal(WORKSPACE_DIALOG, workspace_dialog(app, &theme), area);
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod interaction_tests;
