use std::ffi::OsStr;
use std::io;
use std::path::PathBuf;

use super::form::{
    parse_memory_mib, validate_cpus, validate_gb_size, validate_memory, validate_name,
};
use super::ports::parse_ports;
use super::sandbox::{VmMetrics, format_size, json_text, parse_vm_metrics};
use super::test_support::TempWorkspaceDir;
use super::*;
use serde_json::Value;

#[test]
fn name_validation() {
    assert!(validate_name("sandbox-name").is_ok());
    assert!(validate_name("a").is_err());
    assert!(validate_name("default").is_err());
    assert!(validate_name(".bad").is_err());
    assert!(validate_name("bad name").is_err());
}

#[test]
fn size_and_cpu_validation() {
    assert!(validate_gb_size("").is_ok());
    assert!(validate_gb_size("40").is_ok());
    assert!(validate_gb_size("20").is_ok());
    assert!(validate_gb_size("40g").is_err());
    assert!(validate_gb_size("40x").is_err());
    assert!(validate_cpus("2").is_ok());
    assert!(validate_cpus("0").is_ok());
    assert!(validate_cpus("abc").is_err());
}

#[test]
fn memory_validation() {
    assert_eq!(parse_memory_mib("512m"), Some(512));
    assert_eq!(parse_memory_mib("8g"), Some(8192));
    assert!(validate_memory("256m").is_err());
    assert!(validate_memory("nonsense").is_err());
}

#[test]
fn form_builds_expected_commands() {
    let mut form = Form::new();
    form.name = InputState::new("sandbox-name");
    form.root_size = InputState::new("40");
    form.docker_size = InputState::new("20");
    form.cpus = InputState::new("2");
    form.memory = InputState::new("4g");

    assert_eq!(
        form.create_command().preview(),
        "DOCKER_SANDBOXES_ROOT_SIZE=40g DOCKER_SANDBOXES_DOCKER_SIZE=20g sbx create --name sandbox-name opencode --cpus 2 --memory 4g"
    );
    assert_eq!(
        form.keep_alive_command().preview(),
        "sbx run -d --name sandbox-name"
    );
    assert_eq!(
        form.preview(),
        "DOCKER_SANDBOXES_ROOT_SIZE=40g DOCKER_SANDBOXES_DOCKER_SIZE=20g sbx create --name sandbox-name opencode --cpus 2 --memory 4g && sbx run -d --name sandbox-name"
    );
    assert!(form.first_error().is_none());
}

#[test]
fn workspace_mode_controls_the_clone_flag_and_path_position() {
    let mut form = Form::new();
    form.name = InputState::new("sandbox-name");
    form.cpus = InputState::new("2");
    form.memory = InputState::new("4g");
    form.workspace_path = "/work/repo".to_owned();
    form.set_add_workspace(true);

    assert_eq!(form.workspace_mode(), WorkspaceMode::Mount);
    assert_eq!(
        form.create_command().preview(),
        "sbx create --name sandbox-name opencode --cpus 2 --memory 4g /work/repo"
    );

    form.set_workspace_mode(1);
    assert_eq!(form.workspace_mode(), WorkspaceMode::Clone);
    assert_eq!(
        form.create_command().preview(),
        "sbx create --name sandbox-name opencode --cpus 2 --memory 4g --clone /work/repo"
    );

    form.set_add_workspace(false);
    assert_eq!(
        form.create_command().preview(),
        "sbx create --name sandbox-name opencode --cpus 2 --memory 4g"
    );
}

#[test]
fn selecting_an_agent_closes_the_picker() {
    let mut app = App::with_sandboxes(&[]);
    app.open_wizard();
    app.set_agent_open(true);
    assert!(app.form().agent_open());

    app.set_agent_index(0);
    assert_eq!(app.form().agent_index(), 0);
    assert_eq!(app.form().agent_focus(), 0);
    assert!(!app.form().agent_open());
}

#[test]
fn windows_home_prefers_user_profile() {
    let home = super::form::windows_home_from(
        Some(OsStr::new(r"C:\Users\test")),
        Some(OsStr::new("D:")),
        Some(OsStr::new(r"\Users\other")),
    );
    assert_eq!(home, Some(PathBuf::from(r"C:\Users\test")));
}

#[test]
fn windows_home_uses_home_drive_and_path_when_no_profile() {
    let home = super::form::windows_home_from(
        None,
        Some(OsStr::new("C:")),
        Some(OsStr::new(r"\Users\test")),
    );
    assert_eq!(home, Some(PathBuf::from("C:").join(r"\Users\test")));
}

#[test]
fn windows_home_ignores_empty_variables() {
    assert_eq!(
        super::form::windows_home_from(Some(OsStr::new("")), None, None),
        None
    );
    assert_eq!(
        super::form::windows_home_from(
            None,
            Some(OsStr::new("")),
            Some(OsStr::new(r"\Users\test"))
        ),
        None
    );
}

#[test]
fn form_reports_first_invalid_field() {
    let mut form = Form::new();
    assert_eq!(
        form.first_error().map(|(field, _)| field),
        Some(FormField::Name)
    );

    form.name = InputState::new("demo");
    form.cpus = InputState::new("many");
    assert_eq!(
        form.first_error().map(|(field, _)| field),
        Some(FormField::Cpus)
    );
    assert!(form.invalid(FormField::Cpus));
    assert!(!form.invalid(FormField::Name));
}

#[cfg(unix)]
fn fake_output(success: bool, stderr: &str) -> std::process::Output {
    use std::os::unix::process::ExitStatusExt;
    std::process::Output {
        status: std::process::ExitStatus::from_raw(if success { 0 } else { 1 }),
        stdout: Vec::new(),
        stderr: stderr.as_bytes().to_vec(),
    }
}

#[cfg(windows)]
fn fake_output(success: bool, stderr: &str) -> std::process::Output {
    use std::os::windows::process::ExitStatusExt;
    std::process::Output {
        status: std::process::ExitStatus::from_raw(if success { 0 } else { 1 }),
        stdout: Vec::new(),
        stderr: stderr.as_bytes().to_vec(),
    }
}

#[test]
fn creation_ignores_input_until_worker_reports() {
    let mut app = App::with_sandboxes(&[]);
    app.begin_creation_for_test("demo");
    assert!(app.is_creating());
    assert_eq!(app.creating_name(), Some("demo"));

    for key in [
        KeyCode::Char('q'),
        KeyCode::Esc,
        KeyCode::Enter,
        KeyCode::Char('y'),
        KeyCode::Char('r'),
    ] {
        app.handle_global_key(KeyEvent::new(key, KeyModifiers::NONE));
    }
    assert!(app.is_creating());
    assert!(matches!(app.mode(), Mode::Wizard));

    // Ctrl+C remains available as the escape hatch.
    app.handle_global_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
    assert!(app.should_quit());

    // The test worker's channel is disconnected: poll reports the failure
    // instead of leaving the UI stuck in the creating state.
    app.poll_creation();
    assert!(!app.is_creating());
    assert!(matches!(app.mode(), Mode::Wizard));
    assert!(app.message().contains("creation worker stopped"));
}

#[test]
fn start_creation_does_not_block() {
    let mut app = App::with_sandboxes(&[]);
    #[cfg(unix)]
    let commands = vec![
        PendingCommand::new("true", &[]),
        PendingCommand::new("true", &[]),
    ];
    #[cfg(windows)]
    let commands = vec![
        PendingCommand::new("cmd", &["/C", "exit", "0"]),
        PendingCommand::new("cmd", &["/C", "exit", "0"]),
    ];
    app.start_creation(commands, "demo");
    assert!(app.is_creating());
    assert_eq!(app.creating_name(), Some("demo"));
}

#[test]
fn creation_success_closes_wizard() {
    let mut app = App::with_sandboxes(&[]);
    app.begin_creation_for_test("demo");
    app.complete_creation_for_test("demo", Ok(fake_output(true, "")));
    assert!(!app.is_creating());
    assert!(matches!(app.mode(), Mode::Normal));
}

#[test]
fn creation_failure_keeps_wizard_open_for_retry() {
    let mut app = App::with_sandboxes(&[]);
    app.begin_creation_for_test("demo");
    app.complete_creation_for_test("demo", Err(io::Error::other("boom")));
    assert!(!app.is_creating());
    assert!(matches!(app.mode(), Mode::Wizard));
    assert!(app.message().contains("boom"));
}

#[test]
fn spinner_advances_only_while_creating() {
    let mut app = App::with_sandboxes(&[]);
    app.tick();
    let idle = app.spinner();
    app.tick();
    assert_eq!(app.spinner(), idle);

    app.begin_creation_for_test("demo");
    let first = app.spinner();
    app.tick();
    assert_ne!(app.spinner(), first);
}

#[test]
fn sandbox_status_detects_running() {
    assert!(Sandbox::new("a".into(), "running".into()).is_running());
    assert!(Sandbox::new("a".into(), "● running (detached)".into()).is_running());
    assert!(!Sandbox::new("a".into(), "stopped".into()).is_running());
    assert!(!Sandbox::new("a".into(), "unknown".into()).is_running());
}

#[test]
fn stopped_sandbox_ignores_running_only_actions() {
    let mut app = App::with_sandbox_statuses(&[("alpha", "stopped")]);
    app.exec_selected();
    app.open_selected_info();
    assert!(matches!(app.mode(), Mode::Normal));
    assert!(app.take_external_command().is_none());
}

#[test]
fn sandbox_actions_build_expected_commands() {
    assert_eq!(
        App::start_command("demo").preview(),
        "sbx run -d --name demo"
    );
    assert_eq!(
        PendingCommand::new("sbx", &["stop", "demo"]).preview(),
        "sbx stop demo"
    );
    assert_eq!(
        PendingCommand::new("sbx", &["exec", "-it", "demo", "bash"]).preview(),
        "sbx exec -it demo bash"
    );
    assert_eq!(
        PendingCommand::new("sbx", &["remove", "demo", "--force"]).preview(),
        "sbx remove demo --force"
    );
}

#[test]
fn json_lookup_finds_nested_resource_fields() {
    let value: Value = serde_json::from_str(
        r#"{"name":"demo","resources":{"cpus":2,"memory":"4g"},"sizes":{"rootfs":"20g","docker":"10g"}}"#,
    )
    .unwrap();
    assert_eq!(json_text(&value, &["name"]).as_deref(), Some("demo"));
    assert_eq!(json_text(&value, &["cpus"]).as_deref(), Some("2"));
    assert_eq!(json_text(&value, &["memory"]).as_deref(), Some("4g"));
    assert_eq!(json_text(&value, &["rootfs"]).as_deref(), Some("20g"));
    assert_eq!(json_text(&value, &["docker"]).as_deref(), Some("10g"));
    assert_eq!(json_text(&value, &["missing"]), None);
}

#[test]
fn size_formatting_uses_binary_units() {
    assert_eq!(format_size(512), "512 KiB");
    assert_eq!(format_size(1024), "1.0 MiB");
    assert_eq!(format_size(1024 * 1024), "1.0 GiB");
    assert_eq!(format_size(1024 * 1024 * 1024), "1.0 TiB");
}

#[test]
fn vm_metrics_parser_reads_guest_report() {
    let output = concat!(
        "cpu=4\n",
        "cpu_usage=37\n",
        "mem_total_kb=2097152\n",
        "mem_free_kb=1048576\n",
        "Filesystem     1K-blocks      Used Available Use% Mounted on\n",
        "overlay         4194304    524288   3670016  13% /\n",
        "/dev/vde       2097152    524288   1048576  25% /var/lib/docker\n",
    );
    assert_eq!(
        parse_vm_metrics(output),
        VmMetrics {
            cpus: Some("4".to_owned()),
            cpu_usage: Some(37),
            mem_total_kib: Some(2097152),
            mem_free_kib: Some(1048576),
            root_total_kib: Some(4194304),
            root_free_kib: Some(3670016),
            docker_total_kib: Some(2097152),
            docker_free_kib: Some(1048576),
        }
    );
}

#[test]
fn usage_derives_used_from_total_and_free() {
    assert_eq!(
        VmMetrics::usage(Some(2 * 1024 * 1024), Some(1024 * 1024)),
        (Some("1.0 GiB".to_owned()), Some("2.0 GiB".to_owned()))
    );
    assert_eq!(
        VmMetrics::usage(Some(4 * 1024 * 1024), Some(1024 * 1024)),
        (Some("3.0 GiB".to_owned()), Some("4.0 GiB".to_owned()))
    );
    assert_eq!(
        VmMetrics::usage(Some(1024), None),
        (None, Some("1.0 MiB".to_owned()))
    );
    assert_eq!(VmMetrics::usage(None, None), (None, None));
}

#[test]
fn vm_metrics_prefers_standard_docker_mount() {
    let output = concat!(
        "cpu=8\n",
        "Filesystem 1K-blocks Used Available Use% Mounted on\n",
        "/dev/a 100 0 100 0% /var/lib/docker-data\n",
        "/dev/b 200 0 200 0% /var/lib/docker\n",
        "/dev/c 300 0 300 0% /\n",
    );
    let metrics = parse_vm_metrics(output);
    assert_eq!(metrics.cpus.as_deref(), Some("8"));
    assert_eq!(metrics.docker_total_kib, Some(200));
    assert_eq!(metrics.docker_free_kib, Some(200));
    assert_eq!(metrics.root_total_kib, Some(300));
}

#[test]
fn vm_metrics_falls_back_to_any_docker_mount() {
    let output = concat!(
        "Filesystem 1K-blocks Used Available Use% Mounted on\n",
        "/dev/a 100 0 100 0% /mnt/custom-docker\n",
        "/dev/c 300 0 300 0% /\n",
    );
    let metrics = parse_vm_metrics(output);
    assert_eq!(metrics.docker_total_kib, Some(100));
    assert_eq!(metrics.mem_free_kib, None);
}

#[test]
fn cpu_usage_is_parsed_only_when_in_range() {
    let metrics = parse_vm_metrics("cpu=4\ncpu_usage=101\n");
    assert_eq!(metrics.cpus.as_deref(), Some("4"));
    assert_eq!(metrics.cpu_usage, None);

    let metrics = parse_vm_metrics("cpu_usage=100\n");
    assert_eq!(metrics.cpu_usage, Some(100));
}

#[test]
fn destructive_actions_wait_for_confirmation() {
    let mut app = App::with_sandbox_statuses(&[("alpha", "stopped")]);
    app.start_selected();
    assert!(matches!(app.mode(), Mode::Confirm));
    assert!(app.modals_ref().is_open(CONFIRM_DIALOG));
    assert_eq!(
        app.pending_action(),
        Some(&PendingAction::Start("alpha".to_owned()))
    );
    assert!(app.take_confirmed_for_test().is_none());

    app.cancel_action();
    assert!(matches!(app.mode(), Mode::Normal));
    assert!(app.pending_action().is_none());
    assert_eq!(app.message(), "Start cancelled.");

    app.toggle_selected();
    assert_eq!(
        app.pending_action(),
        Some(&PendingAction::Start("alpha".to_owned()))
    );
    app.confirm_action();
    assert!(matches!(app.mode(), Mode::Normal));
    assert_eq!(
        app.take_confirmed_for_test(),
        Some(PendingAction::Start("alpha".to_owned()))
    );
}

#[test]
fn stop_and_remove_request_the_right_action() {
    let mut running = App::with_sandboxes(&["alpha"]);
    running.stop_selected();
    assert_eq!(
        running.pending_action(),
        Some(&PendingAction::Stop("alpha".to_owned()))
    );
    running.confirm_action();
    assert_eq!(
        running.take_confirmed_for_test(),
        Some(PendingAction::Stop("alpha".to_owned()))
    );

    let mut stopped = App::with_sandbox_statuses(&[("beta", "stopped")]);
    stopped.remove_selected();
    assert_eq!(
        stopped.pending_action(),
        Some(&PendingAction::Remove("beta".to_owned()))
    );
    stopped.confirm_action();
    assert_eq!(
        stopped.take_confirmed_for_test(),
        Some(PendingAction::Remove("beta".to_owned()))
    );
}

#[test]
fn confirmation_cancels_when_status_changed_or_sandbox_vanished() {
    let mut app = App::with_sandbox_statuses(&[("alpha", "stopped")]);
    app.start_selected();
    app.apply_statuses(vec![Sandbox::new("alpha".to_owned(), "running".to_owned())]);
    app.confirm_action();
    assert!(app.take_confirmed_for_test().is_none());
    assert!(app.message().contains("Sandbox state changed"));

    app.remove_selected();
    app.apply_statuses(vec![]);
    app.confirm_action();
    assert!(app.take_confirmed_for_test().is_none());
}

#[test]
fn global_shortcuts_are_inert_while_confirming() {
    let mut app = App::with_sandbox_statuses(&[("alpha", "stopped")]);
    app.remove_selected();
    for key in [KeyCode::Char('q'), KeyCode::Char('s'), KeyCode::Char('d')] {
        app.handle_global_key(KeyEvent::new(key, KeyModifiers::NONE));
    }
    assert!(!app.should_quit());
    assert!(matches!(app.mode(), Mode::Confirm));
    assert_eq!(
        app.pending_action(),
        Some(&PendingAction::Remove("alpha".to_owned()))
    );

    // Ctrl+C is still the escape hatch.
    app.handle_global_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
    assert!(app.should_quit());
}

#[test]
fn polling_info_installs_fresh_metrics_and_drops_stale_ones() {
    let mut app = App::with_sandboxes(&["alpha"]);
    app.info = Some(SandboxDetails {
        name: "alpha".to_owned(),
        status: "running".to_owned(),
        ..SandboxDetails::default()
    });
    app.queue_info_result_for_test(
        "alpha",
        SandboxDetails {
            name: "alpha".to_owned(),
            status: "running".to_owned(),
            cpus: Some("4".to_owned()),
            cpu_usage: Some(42),
            ..SandboxDetails::default()
        },
    );
    assert!(app.is_refreshing_info());
    app.poll_info();
    assert!(!app.is_refreshing_info());
    assert_eq!(app.info().unwrap().cpus.as_deref(), Some("4"));
    assert_eq!(app.info().unwrap().cpu_usage, Some(42));

    app.queue_info_result_for_test(
        "other",
        SandboxDetails {
            name: "other".to_owned(),
            cpus: Some("99".to_owned()),
            ..SandboxDetails::default()
        },
    );
    app.poll_info();
    assert_eq!(app.info().unwrap().name, "alpha");
    assert_eq!(app.info().unwrap().cpus.as_deref(), Some("4"));
}

#[test]
fn status_tick_updates_modal_without_clearing_toast() {
    let mut app = App::with_sandbox_statuses(&[("alpha", "stopped")]);
    app.notify_info("keep me", None);
    app.info = Some(SandboxDetails {
        name: "alpha".to_owned(),
        status: "stopped".to_owned(),
        ..SandboxDetails::default()
    });

    app.apply_statuses(vec![Sandbox::new("alpha".to_owned(), "running".to_owned())]);

    assert!(app.sandboxes()[0].is_running());
    assert_eq!(app.info().unwrap().status, "running");
    assert_eq!(app.message(), "keep me");
}

#[test]
fn port_listing_parser_reads_published_ports() {
    let rows = br#"[
        {"host_ip":"127.0.0.1","host_port":60002,"sandbox_port":3000,"protocol":"tcp4"},
        {"host_ip":"127.0.0.1","host_port":60003,"sandbox_port":3001,"protocol":"tcp4"}
    ]"#;
    assert_eq!(
        parse_ports(rows).unwrap(),
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
                host_ip: Some("127.0.0.1".to_owned()),
            },
        ]
    );
    assert_eq!(
        parse_ports(br#"{"ports":[{"host_port":8080,"sandbox_port":80}]}"#).unwrap()[0].protocol,
        "tcp"
    );
}

#[test]
fn port_form_builds_publish_command() {
    let form = PortForm::new();
    assert_eq!(
        form.pending_command("demo").preview(),
        "sbx port publish demo 8080:3000"
    );
    assert!(!form.invalid(PortField::Host));
    assert!(!form.invalid(PortField::Sandbox));

    let mut form = form;
    form.host = InputState::new("9000");
    form.sandbox = InputState::new("3001");
    assert_eq!(
        form.pending_command("demo").preview(),
        "sbx port publish demo 9000:3001"
    );
}

#[test]
fn port_validation_rejects_bad_values() {
    let mut form = PortForm::new();
    for value in ["0", "65536", "abc", ""] {
        form.host = InputState::new(value);
        assert!(form.invalid(PortField::Host), "accepted port {value:?}");
    }
    form.host = InputState::new("8080");
    form.sandbox = InputState::new("abc");
    assert_eq!(
        form.first_error().as_deref(),
        Some("Sandbox port: Enter a port between 1 and 65535.")
    );
}

#[test]
fn ports_dialog_only_opens_for_running_sandbox() {
    let mut app = App::with_sandbox_statuses(&[("alpha", "stopped")]);
    app.open_ports_selected();
    assert!(matches!(app.mode(), Mode::Normal));
    assert!(app.ports().is_none());

    app.apply_statuses(vec![Sandbox::new("alpha".to_owned(), "running".to_owned())]);
    app.show_ports_for_test("alpha", Vec::new());
    assert!(matches!(app.mode(), Mode::Ports));
    assert!(app.modals_ref().is_open(PORTS_DIALOG));
}

#[test]
fn publish_port_records_the_command() {
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
    assert_eq!(app.ports().unwrap().ports.len(), 1);

    app.publish_port();
    assert_eq!(
        app.take_published_for_test().as_deref(),
        Some("sbx port publish alpha 8080:3000")
    );

    app.set_port_state(PortField::Host, InputState::new("9000"));
    app.set_port_state(PortField::Sandbox, InputState::new("3001"));
    app.publish_port();
    assert_eq!(
        app.take_published_for_test().as_deref(),
        Some("sbx port publish alpha 9000:3001")
    );
}

#[test]
fn unpublish_port_records_the_command() {
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

    app.unpublish_port(60002, 3000);
    assert_eq!(
        app.take_unpublished_for_test().as_deref(),
        Some("sbx port unpublish alpha 60002:3000")
    );
}

#[test]
fn invalid_ports_are_not_published() {
    let mut app = App::with_sandboxes(&["alpha"]);
    app.show_ports_for_test("alpha", Vec::new());
    app.set_port_state(PortField::Host, InputState::new("nope"));
    app.publish_port();

    assert!(app.take_published_for_test().is_none());
    assert!(app.message().contains("Invalid port"));
    assert!(
        app.ports()
            .unwrap()
            .error
            .as_deref()
            .is_some_and(|error| error.contains("Host port"))
    );
}

#[test]
fn stderr_detail_prefers_error_line_and_try_hint() {
    use std::process::{Command, Output};

    use super::commands::stderr_detail;

    let status = if cfg!(windows) {
        Command::new("cmd").args(["/C", "exit 1"]).status().unwrap()
    } else {
        Command::new("false").status().unwrap()
    };
    let output = Output {
        status,
        stdout: Vec::new(),
        stderr: b"    o opencode - now ships as a kit; resolving docker.io/sbx/opencode:latest\n\n  error: sandbox 'test' already exists; connect to the existing sandbox\n    try: sbx run --name test\n"
            .to_vec(),
    };

    assert_eq!(
        stderr_detail(&output, "sbx command failed"),
        "error: sandbox 'test' already exists; connect to the existing sandbox\ntry: sbx run --name test"
    );
}

#[test]
fn workspace_picker_browses_folders_and_commits_the_current_one() {
    let temp = TempWorkspaceDir::new();
    let inner = temp.make_dir("inner");
    temp.make_dir("alpha");
    std::fs::write(temp.path().join("notes.txt"), "not a folder").unwrap();

    let mut app = App::with_sandboxes(&[]);
    app.open_wizard();
    app.set_workspace_path_for_test(temp.path());
    app.open_workspace_picker();

    assert!(matches!(app.mode(), Mode::Workspace));
    let picker = app.workspace_picker().unwrap();
    assert_eq!(picker.dir(), temp.path());
    assert_eq!(picker.entries(), [WORKSPACE_PARENT, "alpha", "inner"]);

    app.select_workspace_entry("inner".to_owned());
    let picker = app.workspace_picker().unwrap();
    assert_eq!(picker.dir(), inner.as_path());
    assert_eq!(picker.entries(), [WORKSPACE_PARENT]);

    app.select_workspace_entry(WORKSPACE_PARENT.to_owned());
    assert_eq!(app.workspace_picker().unwrap().dir(), temp.path());

    app.choose_workspace_dir();
    assert_eq!(
        app.form().workspace_path(),
        temp.path().to_string_lossy().as_ref()
    );
    assert!(app.workspace_picker().is_none());
    assert!(app.modals_ref().is_open(CREATE_DIALOG));
    assert!(matches!(app.mode(), Mode::Wizard));
}

#[test]
fn workspace_picker_only_opens_from_the_wizard() {
    let mut app = App::with_sandboxes(&[]);
    app.open_workspace_picker();
    assert!(app.workspace_picker().is_none());
    assert!(matches!(app.mode(), Mode::Normal));
}

#[test]
fn cancelling_the_picker_keeps_the_original_workspace_path() {
    let temp = TempWorkspaceDir::new();
    temp.make_dir("child");
    let mut app = App::with_sandboxes(&[]);
    app.open_wizard();
    app.set_workspace_path_for_test(temp.path());
    let original = app.form().workspace_path().to_owned();

    app.open_workspace_picker();
    app.select_workspace_entry("child".to_owned());
    app.cancel_workspace_picker();

    assert_eq!(app.form().workspace_path(), original);
    assert!(app.workspace_picker().is_none());
    assert!(app.modals_ref().is_open(CREATE_DIALOG));
}

#[test]
fn workspace_picker_starts_at_the_nearest_existing_directory() {
    let temp = TempWorkspaceDir::new();
    let missing = temp.path().join("does-not-exist").join("workspace");

    let mut app = App::with_sandboxes(&[]);
    app.open_wizard();
    app.set_workspace_path_for_test(&missing);
    app.open_workspace_picker();

    assert_eq!(app.workspace_picker().unwrap().dir(), temp.path());
}
