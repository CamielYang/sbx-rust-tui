use super::*;
use crate::app::test_support::TempWorkspaceDir;
use crate::app::{
    INFO_DIALOG, Mode, SET_CUSTOM_SECRET_DIALOG, SET_SECRET_DIALOG, Secret, WorkspaceMode,
};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};
use ratcn::runtime::{EventResult, Ratcn};
use std::path::Path;

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

fn type_text(
    terminal: &mut Terminal<TestBackend>,
    app: &mut App,
    ratcn: &mut Ratcn<App, Msg>,
    text: &str,
) {
    for character in text.chars() {
        let key = Event::Key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
        if let EventResult::Emit(message) = ratcn.handle_event(key, app) {
            apply(app, message);
        }
    }
    draw(terminal, app, ratcn);
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
        row: 10,
        modifiers: KeyModifiers::NONE,
    });
    if let EventResult::Emit(message) = ratcn.handle_event(moved, &app) {
        apply(&mut app, message);
    }
    assert_eq!(app.selected_name().as_deref(), Some("alpha"));

    click(&mut terminal, &mut app, &mut ratcn, 6, 10);
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
fn clicking_a_tab_switches_the_visible_section() {
    let mut app = App::with_sandboxes(&["alpha"]);
    let mut ratcn = new_runtime();
    let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
    draw(&mut terminal, &app, &mut ratcn);

    let emitted = click(&mut terminal, &mut app, &mut ratcn, 18, 2);
    assert!(
        emitted
            .iter()
            .any(|message| matches!(message, Msg::TabSelected(DashboardTab::Secrets)))
    );
    assert_eq!(app.active_tab(), DashboardTab::Secrets);
    let screen = screen(&terminal);
    assert!(screen.contains(" Secrets "));
    assert!(!screen.contains("+ New sandbox"));
    assert!(!screen.contains("alpha"));

    // The tab row keeps focus, so Left switches straight back to Sandboxes.
    let left = Event::Key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
    if let EventResult::Emit(message) = ratcn.handle_event(left, &app) {
        apply(&mut app, message);
    }
    assert_eq!(app.active_tab(), DashboardTab::Sandboxes);
    assert!(app.focus_ref().contains_path([TAB_ROW]));
}

#[test]
fn quit_button_in_the_title_row_quits() {
    let mut app = App::with_sandboxes(&[]);
    let mut ratcn = new_runtime();
    let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
    draw(&mut terminal, &app, &mut ratcn);

    let emitted = click(&mut terminal, &mut app, &mut ratcn, 95, 0);
    assert!(emitted.iter().any(|message| matches!(message, Msg::Quit)));
    assert!(app.should_quit());
}

#[test]
fn an_open_modal_keeps_focus_off_the_tab_row() {
    let mut app = App::with_sandboxes(&["alpha"]);
    app.select_tab(DashboardTab::Secrets);
    app.set_secret();
    *app.focus_mut() = FocusState::intent(["set-secret-dialog", "field-Secret"]);

    settle_focus(&mut app);
    assert!(app.focus_ref().contains_path(["set-secret-dialog"]));
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
    for (row, fill) in [(12, STOP_ORANGE), (13, EXEC_GRAY), (14, PORTS_CYAN)] {
        for x in 63..80 {
            assert_eq!(cell_bg(&terminal, x, row), fill, "fill at ({x}, {row})");
        }
        assert_ne!(
            cell_bg(&terminal, 80, row),
            fill,
            "button must stop at x=80"
        );
    }
}

#[test]
fn hovering_the_sandbox_list_only_highlights_the_row() {
    let mut app = App::with_sandbox_statuses(&[("alpha", "running"), ("beta", "stopped")]);
    let mut ratcn = new_runtime().hover_focus();
    let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
    draw(&mut terminal, &app, &mut ratcn);

    // A cell below the rows belongs to the list's backdrop, not a row.
    let resting_box = cell_bg(&terminal, 30, 16);
    let resting_row = cell_bg(&terminal, 6, 9);

    let moved = Event::Mouse(crossterm::event::MouseEvent {
        kind: crossterm::event::MouseEventKind::Moved,
        column: 6,
        row: 9,
        modifiers: KeyModifiers::NONE,
    });
    if let EventResult::Emit(message) = ratcn.handle_event(moved, &app) {
        apply(&mut app, message);
    }
    draw(&mut terminal, &app, &mut ratcn);

    assert_eq!(
        cell_bg(&terminal, 30, 16),
        resting_box,
        "hover must not recolor the whole sandbox box"
    );
    assert_ne!(
        cell_bg(&terminal, 6, 9),
        resting_row,
        "hover must still highlight the sandbox row"
    );
}

#[test]
fn action_buttons_change_color_on_hover() {
    let app = App::with_sandboxes(&["alpha"]);
    let mut ratcn = new_runtime();
    let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
    draw(&mut terminal, &app, &mut ratcn);
    let resting = cell_bg(&terminal, 70, 12);
    assert_eq!(resting, STOP_ORANGE);

    let moved = Event::Mouse(crossterm::event::MouseEvent {
        kind: crossterm::event::MouseEventKind::Moved,
        column: 70,
        row: 12,
        modifiers: KeyModifiers::NONE,
    });
    ratcn.handle_event(moved, &app);
    draw(&mut terminal, &app, &mut ratcn);
    assert_ne!(cell_bg(&terminal, 70, 12), resting);
}

#[test]
fn action_buttons_emit_only_their_own_command() {
    let mut running = App::with_sandboxes(&["alpha"]);
    let mut ratcn = new_runtime();
    let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
    draw(&mut terminal, &running, &mut ratcn);
    let emitted = click(&mut terminal, &mut running, &mut ratcn, 70, 12);
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
    let emitted = click(&mut terminal, &mut stopped, &mut ratcn, 70, 12);
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
    lines.iter().enumerate().rev().find_map(|(y, line)| {
        line.rfind(needle)
            .map(|byte| (line[..byte].chars().count() as u16, y as u16))
    })
}

#[test]
fn secret_row_delete_and_toolbar_refresh_are_clickable() {
    let mut app = App::with_secrets(&[("(global)", "github"), ("test", "nebius")]);
    app.select_tab(DashboardTab::Secrets);
    let mut ratcn = new_runtime();
    let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
    draw(&mut terminal, &app, &mut ratcn);

    let deletes: Vec<(u16, u16)> = screen(&terminal)
        .lines()
        .enumerate()
        .filter_map(|(y, line)| {
            line.rfind("Delete")
                .map(|byte| (line[..byte].chars().count() as u16, y as u16))
        })
        .collect();
    assert_eq!(deletes.len(), 2);
    for ((x, y), (name, scope, command)) in deletes.into_iter().zip([
        ("github", "(global)", "sbx secrets rm github -f"),
        ("nebius", "test", "sbx secrets rm nebius --sandbox test -f"),
    ]) {
        let emitted = click(&mut terminal, &mut app, &mut ratcn, x + 2, y);
        assert!(emitted.iter().any(|message| matches!(
            message,
            Msg::DeleteSecret(actual_name, actual_scope)
                if actual_name == name && actual_scope == scope
        )));
        assert!(app.modals_ref().is_open(CONFIRM_DIALOG));
        assert_eq!(
            app.pending_action(),
            Some(&PendingAction::DeleteSecret(
                name.to_owned(),
                scope.to_owned()
            ))
        );
        assert!(app.take_deleted_for_test().is_none());

        draw(&mut terminal, &app, &mut ratcn);
        let (cx, cy) = position_from_bottom(&screen(&terminal), "Delete secret")
            .expect("confirmation button on screen");
        let emitted = click(&mut terminal, &mut app, &mut ratcn, cx + 2, cy);
        assert!(
            emitted
                .iter()
                .any(|message| matches!(message, Msg::ConfirmAction))
        );
        assert!(!app.modals_ref().is_open(CONFIRM_DIALOG));
        assert_eq!(app.take_deleted_for_test().as_deref(), Some(command));
    }

    let (x, y) = position_from_bottom(&screen(&terminal), "Refresh").expect("refresh on screen");
    let emitted = click(&mut terminal, &mut app, &mut ratcn, x + 2, y);
    assert!(
        emitted
            .iter()
            .any(|message| matches!(message, Msg::RefreshSecrets))
    );
}

#[test]
fn custom_secret_row_delete_uses_the_env_key_and_placeholder() {
    let mut app = App::with_secrets(&[]);
    app.set_secrets_for_test(vec![
        Secret::custom("test", "API_KEY", ["api.example.com".to_owned()])
            .with_placeholder("sbx-cs-123"),
    ]);
    app.select_tab(DashboardTab::Secrets);
    let mut ratcn = new_runtime();
    let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
    draw(&mut terminal, &app, &mut ratcn);

    let (x, y) = screen(&terminal)
        .lines()
        .enumerate()
        .find_map(|(y, line)| {
            line.rfind("Delete")
                .map(|byte| (line[..byte].chars().count() as u16, y as u16))
        })
        .expect("custom row delete button");
    let emitted = click(&mut terminal, &mut app, &mut ratcn, x + 2, y);
    assert!(emitted.iter().any(|message| matches!(
        message,
        Msg::DeleteCustomSecret(env_key, scope, placeholder)
            if env_key == "API_KEY" && scope == "test" && placeholder == "sbx-cs-123"
    )));
    assert_eq!(
        app.pending_action(),
        Some(&PendingAction::DeleteCustomSecret(
            "API_KEY".to_owned(),
            "test".to_owned(),
            "sbx-cs-123".to_owned()
        ))
    );
    assert!(screen(&terminal).contains("Delete secret 'API_KEY' from sandbox 'test'?"));

    draw(&mut terminal, &app, &mut ratcn);
    let (cx, cy) = position_from_bottom(&screen(&terminal), "Delete secret")
        .expect("confirmation button on screen");
    click(&mut terminal, &mut app, &mut ratcn, cx + 2, cy);
    assert_eq!(
        app.take_deleted_for_test().as_deref(),
        Some("sbx secret rm --placeholder sbx-cs-123 --sandbox test -f")
    );
}

#[test]
fn clicking_a_secret_row_opens_its_info_dialog() {
    let mut app = App::with_secrets(&[("(global)", "github"), ("test", "nebius")]);
    app.select_tab(DashboardTab::Secrets);
    let mut ratcn = new_runtime();
    let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
    draw(&mut terminal, &app, &mut ratcn);

    let (x, y) = position_from_bottom(&screen(&terminal), "github").expect("github row");
    let emitted = click(&mut terminal, &mut app, &mut ratcn, x + 2, y);
    assert!(emitted.iter().any(|message| matches!(
        message,
        Msg::OpenSecretInfo(name, scope) if name == "github" && scope == "(global)"
    )));
    assert!(app.modals_ref().is_open(INFO_DIALOG));

    let text = screen(&terminal);
    assert!(text.contains("ENV VAR"));
    assert!(text.contains("GH_TOKEN, GITHUB_TOKEN"));
    assert!(text.contains("api.githubcopilot.com"));
}

#[test]
fn clicking_an_action_opens_its_confirmation_dialog() {
    let mut app = App::with_sandboxes(&["alpha"]);
    let mut ratcn = new_runtime();
    let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
    draw(&mut terminal, &app, &mut ratcn);

    let emitted = click(&mut terminal, &mut app, &mut ratcn, 70, 12);
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

    // Action rows: Stop (12), Exec shell (13), Publish ports (14).
    let emitted = click(&mut terminal, &mut app, &mut ratcn, 70, 14);
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
fn workspace_checkbox_reveals_the_path_and_change_button() {
    let mut app = App::with_sandboxes(&[]);
    app.open_wizard();
    let mut ratcn = new_runtime();
    let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
    draw(&mut terminal, &app, &mut ratcn);

    let (x, y) = position_from_bottom(&screen(&terminal), "Add workspace")
        .expect("workspace checkbox is on screen");
    let emitted = click(&mut terminal, &mut app, &mut ratcn, x + 2, y);
    assert!(
        emitted
            .iter()
            .any(|message| matches!(message, Msg::AddWorkspaceChanged(true)))
    );
    assert!(app.form().add_workspace());
    let path = app.form().workspace_path().to_owned();
    assert!(screen(&terminal).contains(&path));

    let (x, y) = position_from_bottom(&screen(&terminal), "Change").expect("change button");
    let emitted = click(&mut terminal, &mut app, &mut ratcn, x + 2, y);
    assert!(
        emitted
            .iter()
            .any(|message| matches!(message, Msg::ChangeWorkspace))
    );
    assert!(app.modals_ref().is_open(WORKSPACE_DIALOG));
    assert!(matches!(app.mode(), Mode::Workspace));
    assert_eq!(app.form().workspace_path(), path);
}

#[test]
fn workspace_mode_cycles_to_clone_and_adds_the_flag_before_the_path() {
    let mut app = App::with_sandboxes(&[]);
    app.open_wizard();
    app.set_add_workspace(true);
    app.set_workspace_path_for_test(Path::new("/work/repo"));
    let mut ratcn = new_runtime();
    let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
    draw(&mut terminal, &app, &mut ratcn);

    let (x, y) = position_from_bottom(&screen(&terminal), "direct").expect("mode cycle");
    let emitted = click(&mut terminal, &mut app, &mut ratcn, x, y);
    assert!(
        emitted
            .iter()
            .any(|message| matches!(message, Msg::WorkspaceModeChanged(1)))
    );
    assert_eq!(app.form().workspace_mode(), WorkspaceMode::Clone);
    assert!(
        app.form()
            .create_command()
            .preview()
            .ends_with("--clone /work/repo")
    );
}

#[test]
fn change_opens_the_picker_and_create_here_commits_the_browsed_folder() {
    let temp = TempWorkspaceDir::new();
    let child = temp.make_dir("child");
    let mut app = App::with_sandboxes(&[]);
    app.open_wizard();
    app.set_add_workspace(true);
    app.set_workspace_path_for_test(temp.path());
    let mut ratcn = new_runtime();
    let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
    draw(&mut terminal, &app, &mut ratcn);

    let (x, y) = position_from_bottom(&screen(&terminal), "Change").expect("change button");
    click(&mut terminal, &mut app, &mut ratcn, x + 2, y);
    assert!(app.modals_ref().is_open(WORKSPACE_DIALOG));
    let screen_text = screen(&terminal);
    assert!(screen_text.contains("Select workspace"));
    assert!(screen_text.contains("Create here"));

    // Entering a row browses into it without changing the form yet.
    let (x, y) = position_from_bottom(&screen_text, "child").expect("child folder row");
    let emitted = click(&mut terminal, &mut app, &mut ratcn, x + 2, y);
    assert!(
        emitted
            .iter()
            .any(|message| matches!(message, Msg::WorkspaceEntrySelected(_)))
    );
    assert_eq!(app.workspace_picker().unwrap().dir(), child.as_path());
    assert_eq!(
        app.form().workspace_path(),
        temp.path().to_string_lossy().as_ref()
    );

    // Create here commits the folder being shown, not the row under the cursor.
    let (x, y) = position_from_bottom(&screen(&terminal), "Create here").expect("create button");
    let emitted = click(&mut terminal, &mut app, &mut ratcn, x + 2, y);
    assert!(
        emitted
            .iter()
            .any(|message| matches!(message, Msg::CreateWorkspaceHere))
    );
    assert!(!app.modals_ref().is_open(WORKSPACE_DIALOG));
    assert!(app.modals_ref().is_open(CREATE_DIALOG));
    assert_eq!(
        app.form().workspace_path(),
        child.to_string_lossy().as_ref()
    );
    assert!(screen(&terminal).contains(child.to_string_lossy().as_ref()));
}

#[test]
fn picker_navigates_with_keys_and_escape_returns_to_the_wizard() {
    let temp = TempWorkspaceDir::new();
    let child = temp.make_dir("child");
    let mut app = App::with_sandboxes(&[]);
    app.open_wizard();
    app.set_add_workspace(true);
    app.set_workspace_path_for_test(temp.path());
    app.open_workspace_picker();
    let mut ratcn = new_runtime();
    let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
    draw(&mut terminal, &app, &mut ratcn);

    let press = |key: KeyCode, app: &mut App, ratcn: &mut Ratcn<App, Msg>| match ratcn
        .handle_event(Event::Key(KeyEvent::new(key, KeyModifiers::NONE)), app)
    {
        EventResult::Emit(message) => apply(app, message),
        other => panic!("key {key:?} was not handled: {other:?}"),
    };

    // The cursor starts on `..`; Down moves it to the only child folder.
    press(KeyCode::Down, &mut app, &mut ratcn);
    assert_eq!(app.workspace_picker().unwrap().focused(), Some("child"));
    press(KeyCode::Enter, &mut app, &mut ratcn);
    assert_eq!(app.workspace_picker().unwrap().dir(), child.as_path());

    press(KeyCode::Esc, &mut app, &mut ratcn);
    assert!(!app.modals_ref().is_open(WORKSPACE_DIALOG));
    assert!(app.modals_ref().is_open(CREATE_DIALOG));
    assert!(app.workspace_picker().is_none());
}

#[test]
fn add_port_button_publishes_the_default_mapping() {
    let mut app = App::with_sandboxes(&["alpha"]);
    app.show_ports_for_test("alpha", Vec::new());
    let mut ratcn = new_runtime();
    let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
    draw(&mut terminal, &app, &mut ratcn);

    let (x, y) =
        position_from_bottom(&screen(&terminal), "+ Add port").expect("add button is on screen");
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
fn set_secret_button_opens_a_dialog_that_selects_and_submits() {
    let mut app = App::with_sandboxes(&["alpha"]);
    app.select_tab(DashboardTab::Secrets);
    let mut ratcn = new_runtime();
    let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
    draw(&mut terminal, &app, &mut ratcn);

    let (x, y) = position_from_bottom(&screen(&terminal), "+ Set secret").expect("set button");
    let emitted = click(&mut terminal, &mut app, &mut ratcn, x + 2, y);
    assert!(
        emitted
            .iter()
            .any(|message| matches!(message, Msg::SetSecret))
    );
    assert!(app.modals_ref().is_open(SET_SECRET_DIALOG));

    // Pick github from the service panel.
    let (x, y) = position_from_bottom(&screen(&terminal), "anthropic").expect("service trigger");
    click(&mut terminal, &mut app, &mut ratcn, x + 2, y);
    assert!(app.secret_form().unwrap().service_open());
    let (x, y) = position_from_bottom(&screen(&terminal), "github").expect("github option");
    click(&mut terminal, &mut app, &mut ratcn, x + 2, y);
    assert_eq!(app.secret_form().unwrap().service(), Some("github"));

    // Pick alpha from the scope panel; "(global)" stays the first option.
    let (x, y) = position_from_bottom(&screen(&terminal), "(global)").expect("scope trigger");
    click(&mut terminal, &mut app, &mut ratcn, x + 2, y);
    assert!(app.secret_form().unwrap().scope_open());
    let (x, y) = position_from_bottom(&screen(&terminal), "alpha").expect("alpha option");
    click(&mut terminal, &mut app, &mut ratcn, x + 2, y);
    assert_eq!(app.secret_form().unwrap().scope(), Some("alpha"));

    // Type into the secret input.
    let (x, y) = position_from_bottom(&screen(&terminal), "Secret value").expect("secret input");
    click(&mut terminal, &mut app, &mut ratcn, x + 2, y);
    for character in "sk-test".chars() {
        let key = Event::Key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
        if let EventResult::Emit(message) = ratcn.handle_event(key, &app) {
            apply(&mut app, message);
        }
    }
    assert_eq!(app.secret_form().unwrap().secret().value(), "sk-test");

    let (x, y) = position_from_bottom(&screen(&terminal), "Set secret").expect("submit button");
    let emitted = click(&mut terminal, &mut app, &mut ratcn, x + 2, y);
    assert!(
        emitted
            .iter()
            .any(|message| matches!(message, Msg::SubmitSecret))
    );
    assert_eq!(
        app.take_set_for_test().as_deref(),
        Some("echo \"sk-test\" | sbx secret set github --sandbox alpha -f")
    );
    assert!(!app.modals_ref().is_open(SET_SECRET_DIALOG));
}

#[test]
fn set_custom_secret_button_adds_hosts_and_submits_their_command() {
    let mut app = App::with_sandboxes(&["alpha"]);
    app.select_tab(DashboardTab::Secrets);
    let mut ratcn = new_runtime();
    let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
    draw(&mut terminal, &app, &mut ratcn);

    let (x, y) = position_from_bottom(&screen(&terminal), "+ Set custom secret")
        .expect("custom secret toolbar button");
    let emitted = click(&mut terminal, &mut app, &mut ratcn, x + 2, y);
    assert!(
        emitted
            .iter()
            .any(|message| matches!(message, Msg::SetCustomSecret))
    );
    assert!(app.modals_ref().is_open(SET_CUSTOM_SECRET_DIALOG));

    let (x, y) = position_from_bottom(&screen(&terminal), "API_KEY").expect("env input");
    click(&mut terminal, &mut app, &mut ratcn, x + 2, y);
    type_text(&mut terminal, &mut app, &mut ratcn, "API_KEY");
    assert_eq!(
        app.custom_secret_form().unwrap().env_key().value(),
        "API_KEY"
    );

    // Pick alpha from the scope panel; the global store is the first option.
    let (x, y) = position_from_bottom(&screen(&terminal), "(global)").expect("scope trigger");
    click(&mut terminal, &mut app, &mut ratcn, x + 2, y);
    assert!(app.custom_secret_form().unwrap().scope_open());
    let (x, y) = position_from_bottom(&screen(&terminal), "alpha").expect("alpha option");
    click(&mut terminal, &mut app, &mut ratcn, x + 2, y);
    assert_eq!(app.custom_secret_form().unwrap().scope(), Some("alpha"));

    let (x, y) = position_from_bottom(&screen(&terminal), "Secret value").expect("secret input");
    click(&mut terminal, &mut app, &mut ratcn, x + 2, y);
    type_text(&mut terminal, &mut app, &mut ratcn, "test");
    assert_eq!(app.custom_secret_form().unwrap().secret().value(), "test");

    let (x, y) = position_from_bottom(&screen(&terminal), "api.example.com").expect("host input");
    click(&mut terminal, &mut app, &mut ratcn, x + 2, y);
    type_text(&mut terminal, &mut app, &mut ratcn, "api.example.com");

    let (x, y) = position_from_bottom(&screen(&terminal), "+ Add host").expect("add host button");
    let emitted = click(&mut terminal, &mut app, &mut ratcn, x + 2, y);
    assert!(
        emitted
            .iter()
            .any(|message| matches!(message, Msg::AddCustomHost))
    );
    assert_eq!(app.custom_secret_form().unwrap().hosts().len(), 2);

    // The new row is the last placeholder on screen, so it can be filled by
    // clicking the bottom-most matching input.
    let (x, y) = position_from_bottom(&screen(&terminal), "api.example.com").expect("new host");
    click(&mut terminal, &mut app, &mut ratcn, x + 2, y);
    type_text(&mut terminal, &mut app, &mut ratcn, "uploads.example.com");

    let (x, y) =
        position_from_bottom(&screen(&terminal), "Set custom secret").expect("submit button");
    let emitted = click(&mut terminal, &mut app, &mut ratcn, x + 2, y);
    assert!(
        emitted
            .iter()
            .any(|message| matches!(message, Msg::SubmitCustomSecret))
    );
    assert_eq!(
        app.take_set_for_test().as_deref(),
        Some(
            "sbx secret set-custom --sandbox alpha --host api.example.com --host uploads.example.com --env API_KEY --value test"
        )
    );
    assert!(!app.modals_ref().is_open(SET_CUSTOM_SECRET_DIALOG));
    assert!(app.custom_secret_form().is_none());
}

#[test]
fn overwrite_confirmation_button_is_clickable_above_the_set_secret_dialog() {
    let mut app = App::with_sandboxes(&["alpha"]);
    app.select_tab(DashboardTab::Secrets);
    app.set_secret();
    app.set_secret_state(InputState::new("sk-test"));
    app.set_secret_exists_for_test(true);
    app.submit_secret();

    let mut ratcn = new_runtime();
    let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
    draw(&mut terminal, &app, &mut ratcn);

    let (x, y) = position_from_bottom(&screen(&terminal), "Overwrite secret")
        .expect("overwrite button on the confirmation dialog");
    let emitted = click(&mut terminal, &mut app, &mut ratcn, x + 2, y);
    assert!(emitted.iter().any(|message| matches!(message, Msg::ConfirmAction)));
    assert!(app.take_set_for_test().is_some());
    assert!(!app.modals_ref().is_open(SET_SECRET_DIALOG));
}

#[test]
fn custom_overwrite_confirmation_button_is_clickable_above_the_custom_form() {
    let mut app = App::with_sandboxes(&["alpha"]);
    app.select_tab(DashboardTab::Secrets);
    app.set_custom_secret();
    app.set_custom_env_key(InputState::new("TEST_KEY"));
    app.set_custom_secret_value(InputState::new("sk-test"));
    app.set_custom_host(0, InputState::new("api.example.com"));
    app.set_secret_exists_for_test(true);
    app.submit_custom_secret();

    let mut ratcn = new_runtime();
    let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
    draw(&mut terminal, &app, &mut ratcn);

    let (x, y) = position_from_bottom(&screen(&terminal), "Overwrite secret")
        .expect("overwrite button on the confirmation dialog");
    let emitted = click(&mut terminal, &mut app, &mut ratcn, x + 2, y);
    assert!(
        emitted
            .iter()
            .any(|message| matches!(message, Msg::ConfirmAction))
    );
    assert_eq!(
        app.take_deleted_for_test().as_deref(),
        Some("sbx secret rm --placeholder docker-placeholder-value -f")
    );
    assert_eq!(
        app.take_set_for_test().as_deref(),
        Some("sbx secret set-custom --host api.example.com --env TEST_KEY --value sk-test")
    );
    assert!(!app.modals_ref().is_open(SET_CUSTOM_SECRET_DIALOG));
}

#[test]
fn escape_dismisses_the_set_secret_dialog() {
    let mut app = App::with_sandboxes(&["alpha"]);
    app.select_tab(DashboardTab::Secrets);
    app.set_secret();
    let mut ratcn = new_runtime();
    let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
    draw(&mut terminal, &app, &mut ratcn);

    let escape = Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    match ratcn.handle_event(escape, &app) {
        EventResult::Emit(message) => apply(&mut app, message),
        other => panic!("Esc should dismiss the dialog, got {other:?}"),
    }
    assert!(!app.modals_ref().is_open(SET_SECRET_DIALOG));
    assert!(app.secret_form().is_none());
    assert!(app.take_set_for_test().is_none());
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
