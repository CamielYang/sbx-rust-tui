//! Ratcn-based dashboard for `sbx` sandboxes.
//!
//! Every control is either a ratcn component or themed paint queued through
//! ratcn's declaration context. The runtime owns focus, hover, hit testing, and
//! modal layering; [`App`](crate::app::App) owns the state and [`apply`] is the
//! single place where component messages become state changes.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::widgets::Block;
use ratcn::runtime::{ChildId, DeclareCtx, FocusState};
use ratcn::{InputState, Theme};

use crate::app::{
    App, CONFIRM_DIALOG, CREATE_DIALOG, DashboardTab, FormField, INFO_DIALOG, POLICY_DIALOG,
    PORTS_DIALOG, PortField, SET_CUSTOM_SECRET_DIALOG, SET_SECRET_DIALOG, Sandbox, SecretId,
    TAB_ROW, WORKSPACE_DIALOG,
};

mod dashboard;
mod dialogs;
mod widgets;

use self::dashboard::{
    paint_body, paint_content_frame, paint_footer, paint_header, paint_quit, paint_secrets_body,
    paint_secrets_toolbar, paint_tabs, paint_toolbar,
};
use self::dialogs::{
    confirm_dialog, create_dialog, info_dialog, policy_dialog, ports_dialog,
    set_custom_secret_dialog, set_secret_dialog, workspace_dialog,
};

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
    PolicyFocused(usize),
    PolicySelected(usize),
    SandboxFocused(String, usize),
    SandboxSelected(String),
    SandboxScrolled(usize),
    TabSelected(DashboardTab),
    OpenWizard,
    Refresh,
    SetSecret,
    CancelSetSecret,
    SetCustomSecret,
    CancelSetCustomSecret,
    CustomSecretEnvChanged(InputState),
    CustomSecretScopeOpenChanged(bool),
    CustomSecretScopeFocused(usize),
    CustomSecretScopeSelected(usize),
    CustomSecretValueChanged(InputState),
    CustomSecretHostChanged(usize, InputState),
    AddCustomHost,
    RemoveCustomHost(usize),
    SubmitCustomSecret,
    SecretServiceOpenChanged(bool),
    SecretServiceFocused(usize),
    SecretServiceSelected(usize),
    SecretScopeOpenChanged(bool),
    SecretScopeFocused(usize),
    SecretScopeSelected(usize),
    SecretValueChanged(InputState),
    SubmitSecret,
    RefreshSecrets,
    /// Secret name followed by its scope (the sandbox name, or "(global)").
    DeleteSecret(String, String),
    /// Custom secret env key, scope, and CLI placeholder, in that order.
    DeleteCustomSecret(String, String, String),
    /// A clicked secret row: its stable id followed by scope. Custom rows carry
    /// their env key, since their `(custom)` label is not unique.
    OpenSecretInfo(SecretId, String),
    Quit,
    CancelWizard,
    CreateSandbox,
    FormChanged(FormField, InputState),
    AgentOpenChanged(bool),
    AgentFocused(usize),
    AgentSelected(usize),
    AddWorkspaceChanged(bool),
    WorkspaceModeChanged(usize),
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

/// One-cell inset around the dashboard. The footer is deliberately excluded
/// so its shortcut legend stays flush with the bottom edge.
const LAYOUT_PADDING_Y: u16 = 1;
const LAYOUT_PADDING_X: u16 = 2;
const FOOTER_HEIGHT: u16 = 1;

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
        Msg::PolicyFocused(index) => app.set_policy_focus(index),
        Msg::PolicySelected(index) => app.select_network_policy(index),
        // Moving the cursor (hover or arrow keys) only moves the highlight.
        // Clicking or Enter commits the row as the Actions pane's target.
        Msg::SandboxFocused(name, offset) => app.focus_list_item(&name, offset),
        Msg::SandboxSelected(name) => app.focus_sandbox(&name),
        Msg::SandboxScrolled(offset) => app.set_list_offset(offset),
        Msg::TabSelected(tab) => app.select_tab(tab),
        Msg::OpenWizard => app.open_wizard(),
        Msg::Refresh => app.request_sandbox_refresh(),
        Msg::SetSecret => app.set_secret(),
        Msg::CancelSetSecret => app.cancel_set_secret(),
        Msg::SetCustomSecret => app.set_custom_secret(),
        Msg::CancelSetCustomSecret => app.cancel_custom_secret(),
        Msg::CustomSecretEnvChanged(state) => app.set_custom_env_key(state),
        Msg::CustomSecretScopeOpenChanged(open) => app.set_custom_scope_open(open),
        Msg::CustomSecretScopeFocused(index) => app.set_custom_scope_focus(index),
        Msg::CustomSecretScopeSelected(index) => app.set_custom_scope(index),
        Msg::CustomSecretValueChanged(state) => app.set_custom_secret_value(state),
        Msg::CustomSecretHostChanged(index, state) => app.set_custom_host(index, state),
        Msg::AddCustomHost => app.add_custom_host(),
        Msg::RemoveCustomHost(index) => app.remove_custom_host(index),
        Msg::SubmitCustomSecret => app.submit_custom_secret(),
        Msg::SecretServiceOpenChanged(open) => app.set_secret_service_open(open),
        Msg::SecretServiceFocused(index) => app.set_secret_service_focus(index),
        Msg::SecretServiceSelected(index) => app.set_secret_service(index),
        Msg::SecretScopeOpenChanged(open) => app.set_secret_scope_open(open),
        Msg::SecretScopeFocused(index) => app.set_secret_scope_focus(index),
        Msg::SecretScopeSelected(index) => app.set_secret_scope(index),
        Msg::SecretValueChanged(state) => app.set_secret_state(state),
        Msg::SubmitSecret => app.submit_secret(),
        Msg::RefreshSecrets => app.request_secret_refresh(),
        Msg::DeleteSecret(name, scope) => app.delete_secret(&name, &scope),
        Msg::DeleteCustomSecret(env_key, scope, placeholder) => {
            app.delete_custom_secret(&env_key, &scope, &placeholder)
        }
        Msg::OpenSecretInfo(id, scope) => app.open_secret_info(&id, &scope),
        Msg::Quit => app.quit(),
        Msg::CancelWizard => app.cancel_wizard(),
        Msg::CreateSandbox => app.create_from_form(),
        Msg::FormChanged(field, state) => app.set_form_state(field, state),
        Msg::AgentOpenChanged(open) => app.set_agent_open(open),
        Msg::AgentFocused(index) => app.set_agent_focus(index),
        Msg::AgentSelected(index) => app.set_agent_index(index),
        Msg::AddWorkspaceChanged(enabled) => app.set_add_workspace(enabled),
        Msg::WorkspaceModeChanged(index) => app.set_workspace_mode(index),
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
    // A modal owns focus while it is open. Reparking the path on the tab row
    // here would pull it off the dialog's fields every frame.
    if app.modals_ref().top().is_some() {
        return;
    }
    if app.active_tab() != DashboardTab::Sandboxes {
        *app.focus_mut() = FocusState::intent([TAB_ROW]);
        return;
    }
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

    // Everything above the footer lives inside a one-cell frame. The footer
    // keeps the full width and sits on the bottom row, outside that frame.
    let padded = Rect {
        x: area.x.saturating_add(LAYOUT_PADDING_X),
        y: area.y.saturating_add(LAYOUT_PADDING_Y),
        width: area.width.saturating_sub(LAYOUT_PADDING_X * 2),
        height: area
            .height
            .saturating_sub(LAYOUT_PADDING_Y * 2 + FOOTER_HEIGHT),
    };
    let footer = Rect {
        x: area.x,
        y: area
            .y
            .saturating_add(area.height.saturating_sub(FOOTER_HEIGHT)),
        width: area.width,
        height: FOOTER_HEIGHT.min(area.height),
    };

    let vertical = Layout::vertical([
        Constraint::Length(1), // title + quit
        Constraint::Length(1), // spacing
        Constraint::Length(1), // tabs
        Constraint::Min(0),    // white-bordered content wrapper
    ])
    .split(padded);

    paint_header(ctx, app, &theme, vertical[0]);
    let content = paint_content_frame(ctx, &theme, vertical[3]);
    let content_rows = Layout::vertical([
        Constraint::Length(1), // toolbar buttons
        Constraint::Length(1), // spacing
        Constraint::Min(0),    // active tab's section
    ])
    .split(content);
    match app.active_tab() {
        DashboardTab::Sandboxes => {
            paint_toolbar(ctx, app, &theme, content_rows[0]);
            paint_body(ctx, app, &theme, content_rows[2]);
        }
        DashboardTab::Secrets => {
            paint_secrets_toolbar(ctx, &theme, content_rows[0]);
            paint_secrets_body(ctx, app, &theme, content_rows[2]);
        }
    }
    // Declared after the sandbox controls so startup focus still lands on
    // "+ New sandbox"; the visual row stays just below the title spacing.
    paint_tabs(ctx, app, vertical[2]);
    paint_footer(ctx, &theme, footer);
    paint_quit(ctx, app, vertical[0]);

    // Modals are declared last so their layers cover the dashboard. Ids come
    // from App's ModalState, which Ratcn validates against this declaration.
    // The first-run policy choice is alone on the stack, so it can be declared
    // before the others without changing their layering.
    if app.modals_ref().is_open(POLICY_DIALOG) {
        ctx.modal(POLICY_DIALOG, policy_dialog(app, &theme), area);
    }
    if app.modals_ref().is_open(CREATE_DIALOG) {
        ctx.modal(CREATE_DIALOG, create_dialog(app, &theme), area);
    }
    if app.modals_ref().is_open(INFO_DIALOG) {
        ctx.modal(INFO_DIALOG, info_dialog(app, &theme), area);
    }
    if app.modals_ref().is_open(PORTS_DIALOG) {
        ctx.modal(PORTS_DIALOG, ports_dialog(app, &theme), area);
    }
    // Declared last: the picker stacks above the create wizard that opened it.
    if app.modals_ref().is_open(WORKSPACE_DIALOG) {
        ctx.modal(WORKSPACE_DIALOG, workspace_dialog(app, &theme), area);
    }
    if app.modals_ref().is_open(SET_SECRET_DIALOG) {
        ctx.modal(SET_SECRET_DIALOG, set_secret_dialog(app, &theme), area);
    }
    if app.modals_ref().is_open(SET_CUSTOM_SECRET_DIALOG) {
        ctx.modal(
            SET_CUSTOM_SECRET_DIALOG,
            set_custom_secret_dialog(app, &theme),
            area,
        );
    }
    // Declared last so an overwrite confirmation stacks above the set secret
    // dialog that asked for it.
    if app.modals_ref().is_open(CONFIRM_DIALOG) {
        ctx.modal(CONFIRM_DIALOG, confirm_dialog(app), area);
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod interaction_tests;
