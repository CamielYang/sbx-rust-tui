use super::*;
use crate::app::test_support::TempWorkspaceDir;
use crate::app::{INFO_DIALOG, PORTS_DIALOG, PortMapping, SandboxDetails};
use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};
use ratcn::runtime::Ratcn;

fn draw(app: &App) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
    let theme = Theme::tokyo_night();
    let mut ratcn = Ratcn::new()
        .focus(|state: &App| state.focus_ref(), Msg::FocusChanged)
        .modals(|state: &App| state.modals_ref());
    terminal
        .draw(|frame| {
            ratcn.render(frame, frame.area(), app, &theme, |ctx| {
                declare(ctx, app, &theme)
            });
        })
        .unwrap();
    terminal.backend().buffer().clone()
}

fn render(app: &App) -> String {
    let buffer = draw(app);
    let width = buffer.area.width as usize;
    buffer
        .content()
        .chunks(width)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn renders_dashboard_with_themated_controls() {
    let app = App::with_sandbox_statuses(&[("alpha", "running"), ("beta", "stopped")]);
    let screen = render(&app);
    assert!(screen.contains("DOCKER SANDBOXES"));
    assert!(screen.contains("alpha"));
    assert!(screen.contains("beta"));
    assert!(screen.contains("Exec shell"));
    assert!(screen.contains("Remove"));
}

#[test]
fn footer_keeps_controls_when_a_notification_arrives() {
    let mut app = App::with_sandbox_statuses(&[("alpha", "running")]);
    app.notify_error("Could not run sbx", "boom");

    let screen = render(&app);
    assert!(screen.contains(
        "N new · R refresh · S start/stop · X exec · P ports · I info · D remove · Ctrl+C quit"
    ));
    assert!(!screen.contains("Could not run sbx"));
    assert!(app.message().contains("boom"));
}

#[test]
fn actions_pane_labels_status_and_spaces_it_from_buttons() {
    let app = App::with_sandbox_statuses(&[("alpha", "running")]);
    let screen = render(&app);
    let rows: Vec<&str> = screen.lines().collect();
    let status_row = rows
        .iter()
        .position(|row| row.contains("status: running"))
        .unwrap();
    let first_button_row = rows.iter().position(|row| row.contains("Stop")).unwrap();
    assert_eq!(first_button_row, status_row + 2);
    assert!(
        rows[status_row + 1]
            .trim_matches(|c: char| c == '│' || c.is_whitespace())
            .is_empty()
    );
}

#[test]
fn running_status_is_green_in_list_and_actions() {
    let app = App::with_sandbox_statuses(&[("alpha", "running")]);
    let buffer = draw(&app);
    assert!(
        buffer
            .content()
            .iter()
            .any(|cell| cell.symbol() == "●" && cell.fg == RUNNING_GREEN),
        "the running dot in the sandbox list should be green"
    );

    let width = buffer.area.width as usize;
    let screen = render(&app);
    let row = screen
        .lines()
        .position(|line| line.contains("status: running"))
        .unwrap();
    let column = screen.lines().nth(row).unwrap().find("running").unwrap();
    assert_eq!(
        buffer.content()[row * width + column].fg,
        RUNNING_GREEN,
        "the Actions pane status should be green"
    );
}

/// Style of the first cell of a rendered button label.
fn label_style(buffer: &Buffer, label: &str) -> ratatui::style::Style {
    let width = buffer.area.width as usize;
    let (row, column) = buffer
        .content()
        .chunks(width)
        .enumerate()
        .find_map(|(row, cells)| {
            let text: String = cells.iter().map(|cell| cell.symbol()).collect();
            text.find(label).map(|column| (row, column))
        })
        .unwrap_or_else(|| panic!("rendered button {label:?} was not found"));
    buffer.content()[row * width + column].style()
}

#[test]
fn dialog_cancel_buttons_share_one_visible_fill() {
    let theme = Theme::tokyo_night();
    let style = cancel_style(&theme);
    let fills = [
        style.background,
        style.focused_background,
        style.hovered_background,
    ];
    let assert_cancel_style = |buffer: &Buffer, label: &str| {
        let cell = label_style(buffer, label);
        assert!(
            fills.iter().any(|fill| cell.bg == Some(*fill)),
            "{label} should use the shared cancel fill, got {:?}",
            cell.bg
        );
        assert_eq!(cell.fg, Some(style.foreground), "{label} foreground");
    };

    // The confirmation Cancel and info Close start focused; the wizard Cancel
    // and ports Close start unfocused because their dialogs have content above
    // the action row. All four still carry the same visible fill.
    let mut app = App::with_sandbox_statuses(&[("alpha", "stopped")]);
    app.start_selected();
    assert_eq!(
        label_style(&draw(&app), "Cancel").bg,
        Some(style.focused_background)
    );
    assert_cancel_style(&draw(&app), "Cancel");

    let mut app = App::with_sandboxes(&[]);
    app.open_wizard();
    assert_eq!(
        label_style(&draw(&app), "Cancel").bg,
        Some(style.background)
    );

    let mut app = App::with_sandbox_statuses(&[("alpha", "running")]);
    app.show_info_for_test(SandboxDetails {
        name: "alpha".to_owned(),
        ..SandboxDetails::default()
    });
    assert_cancel_style(&draw(&app), "Close");

    let mut app = App::with_sandboxes(&["alpha"]);
    app.show_ports_for_test("alpha", vec![]);
    assert_eq!(label_style(&draw(&app), "Close").bg, Some(style.background));
}

fn column_of(rows: &str, needle: &str) -> Option<usize> {
    rows.lines()
        .find_map(|line| line.find(needle).map(|byte| line[..byte].chars().count()))
}

#[test]
fn sandbox_rows_do_not_shift_when_focus_moves() {
    let mut app = App::with_sandboxes(&["alpha"]);
    *app.focus_mut() = FocusState::intent(["sandbox-list"]);
    let focused = render(&app);
    *app.focus_mut() = FocusState::intent([ACTION_STOP]);
    let unfocused = render(&app);

    assert_eq!(column_of(&focused, "alpha"), column_of(&unfocused, "alpha"));
    assert!(focused.contains("▸"), "the cursor glyph should still show");
    assert!(!unfocused.contains("▸"));
}

#[test]
fn running_actions_show_only_running_commands() {
    let app = App::with_sandboxes(&["alpha"]);
    let screen = render(&app);
    for label in ["Stop", "Exec shell", "Remove", "Info"] {
        assert!(screen.contains(label), "missing running action: {label}");
    }
    assert!(
        !screen.contains("Start"),
        "Start must be hidden while running"
    );
}

#[test]
fn stopped_actions_show_only_stopped_commands() {
    let app = App::with_sandbox_statuses(&[("alpha", "stopped")]);
    let screen = render(&app);
    assert!(screen.contains("Start"));
    assert!(screen.contains("Remove"));
    for label in ["Stop", "Exec shell", "Info"] {
        assert!(
            !screen.contains(label),
            "unexpected running action: {label}"
        );
    }
}

#[test]
fn action_styles_keep_their_hue_and_change_on_focus_and_hover() {
    let theme = Theme::tokyo_night();
    for fill in [START_GREEN, STOP_ORANGE, EXEC_GRAY, REMOVE_RED, INFO_BLUE] {
        let style = action_style(fill)(&theme);
        assert_eq!(style.background, fill);
        assert_eq!(style.focused_foreground, theme.background);
        assert_ne!(style.focused_background, fill, "focus must change the fill");
        assert_ne!(style.hovered_background, fill, "hover must change the fill");
    }
}

#[test]
fn renders_create_dialog() {
    let mut app = App::with_sandboxes(&[]);
    app.open_wizard();
    assert!(app.modals_ref().is_open(CREATE_DIALOG));
    let screen = render(&app);
    assert!(screen.contains("New sandbox"));
    assert!(screen.contains("Create sandbox"));
    assert!(screen.contains("sandbox-name"));
    assert!(screen.contains("opencode"));
    assert!(!screen.contains("Ready to create"));

    // Each field gets its own row with a blank row between neighbours.
    let rows: Vec<&str> = screen.lines().collect();
    let name_row = rows.iter().position(|row| row.contains("Name")).unwrap();
    let agent_row = rows.iter().position(|row| row.contains("Agent")).unwrap();
    assert_eq!(agent_row, name_row + 2);
    assert!(
        rows[name_row + 1]
            .trim_matches(|c: char| c == '│' || c.is_whitespace())
            .is_empty()
    );

    // The agent dropdown is a popup layer inside the modal; it must be
    // declarable without colliding with the dialog's own ids.
    app.set_agent_open(true);
    let screen = render(&app);
    // The panel keeps the current agent in view, so the first visible row
    // is near the default selection rather than the top of the list.
    assert!(screen.contains("cursor"));
    assert!(screen.contains("opencode"));
}

#[test]
fn workspace_option_reveals_path_and_change_button_when_enabled() {
    let mut app = App::with_sandboxes(&[]);
    app.open_wizard();
    let path = app.form().workspace_path().to_owned();
    let screen = render(&app);
    assert!(screen.contains("Add workspace"));
    assert!(!screen.contains(&path));
    assert!(!screen.contains("Change"));

    app.set_add_workspace(true);
    let screen = render(&app);
    assert!(screen.contains(&path));
    assert!(screen.contains("Change"));
}

#[test]
fn renders_workspace_picker_with_folders_and_create_here() {
    let temp = TempWorkspaceDir::new();
    temp.make_dir("projects");
    let mut app = App::with_sandboxes(&[]);
    app.open_wizard();
    app.set_workspace_path_for_test(temp.path());
    app.open_workspace_picker();

    let screen = render(&app);
    assert!(screen.contains("Select workspace"));
    assert!(screen.contains(&temp.path().to_string_lossy().into_owned()));
    assert!(screen.contains(".."));
    assert!(screen.contains("projects"));
    assert!(screen.contains("Create here"));
}

#[test]
fn renders_info_dialog() {
    let mut app = App::with_sandbox_statuses(&[("alpha", "running")]);
    app.show_info_for_test(SandboxDetails {
        name: "alpha".to_owned(),
        status: "running".to_owned(),
        cpus: Some("4".to_owned()),
        cpu_usage: Some(37),
        mem_used: Some("1.0 GiB".to_owned()),
        mem_total: Some("2.0 GiB".to_owned()),
        ..SandboxDetails::default()
    });
    assert!(app.modals_ref().is_open(INFO_DIALOG));
    let screen = render(&app);
    assert!(screen.contains("Sandbox"));
    assert!(screen.contains("4 (37%)"));
    assert!(screen.contains("1.0 GiB / 2.0 GiB"));
}

#[test]
fn renders_ports_dialog_with_table_and_add_button() {
    let mut app = App::with_sandboxes(&["alpha"]);
    app.show_ports_for_test(
        "alpha",
        vec![
            PortMapping {
                host_port: 60002,
                sandbox_port: 3000,
                protocol: "tcp4".to_owned(),
                host_ip: Some("127.0.0.1".to_owned()),
            },
            PortMapping {
                host_port: 60003,
                sandbox_port: 3001,
                protocol: "tcp4".to_owned(),
                host_ip: None,
            },
        ],
    );
    assert!(app.modals_ref().is_open(PORTS_DIALOG));
    let screen = render(&app);
    assert!(screen.contains("Published ports"));
    assert!(screen.contains("+ Add port"));
    assert!(screen.contains(" host "));
    assert!(screen.contains(" sandbox "));
    assert!(screen.contains("HOST PORT"));
    assert!(screen.contains("SANDBOX PORT"));
    assert!(!screen.contains("PROTOCOL"));
    assert!(screen.contains("60002"));
    assert!(screen.contains("60003"));
    assert!(screen.contains("Unpublish"));
    assert!(!screen.contains("sbx port publish"));
}

#[test]
fn cpu_text_adds_usage_only_when_sampled() {
    let details = SandboxDetails {
        cpus: Some("8".to_owned()),
        cpu_usage: Some(12),
        ..SandboxDetails::default()
    };
    assert_eq!(cpu_text(&details).as_deref(), Some("8 (12%)"));

    let details = SandboxDetails {
        cpu_usage: None,
        ..details
    };
    assert_eq!(cpu_text(&details).as_deref(), Some("8"));
}
