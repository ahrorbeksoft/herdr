use super::endpoint_requests::request_id;
use super::*;
use crate::api::schema::{DevServerEntry, DevServerListener, Method, ResponseResult};
use crate::client::endpoint::{ClientEndpointId, ClientEndpointStatus};

fn server_entry(pid: u32, name: &str, port: u16) -> DevServerEntry {
    DevServerEntry {
        pid,
        name: name.into(),
        command: Some(format!("{name} --serve")),
        listeners: vec![DevServerListener {
            address: "127.0.0.1".into(),
            port,
        }],
        uptime_seconds: Some(90),
        pane_id: Some("pane_1".into()),
        workspace_id: Some("ws_1".into()),
        workspace_name: Some("client-shell".into()),
        pane_title: Some("web".into()),
        cwd: Some("/repo".into()),
    }
}

fn list_result(servers: Vec<DevServerEntry>) -> ResponseResult {
    ResponseResult::DevServerList { servers }
}

fn profile(hex: &str, label: &str) -> crate::client::endpoint::SavedSshEndpoint {
    crate::client::endpoint::SavedSshEndpoint {
        id: crate::client::endpoint::ProfileId::parse(hex).unwrap(),
        label: label.into(),
        target: "dev@example".into(),
        session: "agents".into(),
        enabled: true,
    }
}

fn endpoint_request<'a>(
    actions: &'a [ClientShellAction],
    endpoint_id: &ClientEndpointId,
) -> &'a crate::api::schema::Request {
    actions
        .iter()
        .find_map(|action| match action {
            ClientShellAction::Endpoint {
                endpoint_id: target,
                request,
                ..
            } if target == endpoint_id => Some(request.as_ref()),
            _ => None,
        })
        .expect("endpoint request")
}

fn endpoint_requests<'a>(
    actions: &'a [ClientShellAction],
    endpoint_id: &ClientEndpointId,
) -> Vec<&'a crate::api::schema::Request> {
    actions
        .iter()
        .filter_map(|action| match action {
            ClientShellAction::Endpoint {
                endpoint_id: target,
                request,
                ..
            } if target == endpoint_id => Some(request.as_ref()),
            _ => None,
        })
        .collect()
}

fn open(state: &mut ClientShellState) -> Vec<ClientShellAction> {
    let mut outcome = ClientShellInput::default();
    state.open_dev_servers_overlay(&mut outcome);
    outcome.actions
}

fn overlay(state: &ClientShellState) -> &ClientDevServersOverlay {
    let Some(ClientShellOverlay::DevServers(overlay)) = state.overlay.as_ref() else {
        panic!("expected dev servers overlay");
    };
    overlay
}

fn section<'a>(
    overlay: &'a ClientDevServersOverlay,
    endpoint_id: &ClientEndpointId,
) -> &'a ClientDevServerSection {
    overlay
        .sections
        .iter()
        .find(|section| &section.endpoint_id == endpoint_id)
        .expect("endpoint section")
}

fn ready_pids(overlay: &ClientDevServersOverlay, endpoint_id: &ClientEndpointId) -> Vec<u32> {
    match &section(overlay, endpoint_id).state {
        ClientDevServerSectionState::Ready(entries) => {
            entries.iter().map(|entry| entry.pid).collect()
        }
        state => panic!("expected ready section, got {state:?}"),
    }
}

#[test]
fn dev_servers_menu_entry_opens_overlay_and_fans_out() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    let remote = super::endpoint_requests::add_remote(&mut state);
    let index = super::super::global_menu::global_menu_items(state.snapshot.as_deref().unwrap())
        .iter()
        .position(|(_, action)| {
            *action == super::super::global_menu::ClientGlobalMenuAction::DevServers
        })
        .expect("dev servers menu entry");
    let mut outcome = ClientShellInput::default();
    state.activate_global_menu_item(index, &mut outcome);
    assert!(matches!(
        &state.overlay,
        Some(ClientShellOverlay::DevServers(_))
    ));
    for endpoint_id in [ClientEndpointId::Local, remote] {
        let request = endpoint_request(&outcome.actions, &endpoint_id);
        assert!(matches!(&request.method, Method::ServerDevServers(_)));
    }
}

#[test]
fn dev_servers_sections_cover_offline_and_unsupported_endpoints() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    let build = profile("11111111111111111111111111111111", "Build");
    let legacy = profile("22222222222222222222222222222222", "Legacy");
    let offline = ClientEndpointId::Ssh(build.id.clone());
    let unsupported = ClientEndpointId::Ssh(legacy.id.clone());
    state.set_endpoint_catalog(&[build, legacy]);
    state.set_endpoint_status(&offline, ClientEndpointStatus::Reconnecting);
    state.set_endpoint_status(&unsupported, ClientEndpointStatus::Online);
    state.set_endpoint_methods_for(&unsupported, Some(Vec::new()));
    let mut projection = snapshot();
    projection.boot_id = "legacy-boot".into();
    state.set_endpoint_snapshot(&unsupported, Box::new(projection));

    let actions = open(&mut state);
    let overlay = overlay(&state);
    assert_eq!(overlay.sections.len(), 3);
    assert!(matches!(
        section(overlay, &ClientEndpointId::Local).state,
        ClientDevServerSectionState::Loading
    ));
    assert!(matches!(
        section(overlay, &offline).state,
        ClientDevServerSectionState::Offline
    ));
    assert!(matches!(
        section(overlay, &unsupported).state,
        ClientDevServerSectionState::Unsupported
    ));
    // Only the local endpoint received a request.
    assert_eq!(actions.len(), 1);
    endpoint_request(&actions, &ClientEndpointId::Local);
}

#[test]
fn dev_servers_result_populates_rows_and_selection_survives_refresh() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    let actions = open(&mut state);
    let (repaint, _) = state.handle_endpoint_result(
        "boot-1",
        request_id(&actions),
        Ok(list_result(vec![
            server_entry(10, "node", 3000),
            server_entry(11, "python", 8080),
        ])),
    );
    assert!(repaint);
    assert_eq!(
        ready_pids(overlay(&state), &ClientEndpointId::Local),
        vec![10, 11]
    );

    state.move_dev_servers_selection(1);
    assert_eq!(
        overlay(&state).selected,
        Some((ClientEndpointId::Local, 11))
    );

    // A refresh that reorders the list keeps the selected pid highlighted;
    // rows display sorted by primary port regardless of server order.
    let actions = open(&mut state);
    state.handle_endpoint_result(
        "boot-1",
        request_id(&actions),
        Ok(list_result(vec![
            server_entry(11, "python", 8080),
            server_entry(10, "node", 3000),
        ])),
    );
    assert_eq!(
        overlay(&state).selected,
        Some((ClientEndpointId::Local, 11))
    );
    assert_eq!(overlay(&state).selected_flat_index(), Some(1));

    // When the selected process exits, selection falls back to the first row.
    let actions = open(&mut state);
    state.handle_endpoint_result(
        "boot-1",
        request_id(&actions),
        Ok(list_result(vec![server_entry(10, "node", 3000)])),
    );
    assert_eq!(overlay(&state).selected_flat_index(), Some(0));
}

#[test]
fn dev_servers_query_matches_ports_names_commands_and_context() {
    let entry =
        ClientDevServerEntry::from_server(ClientEndpointId::Local, server_entry(42, "vite", 5173));
    for query in [
        "vite",
        "5173",
        "127.0.0.1",
        "42",
        "--serve",
        "client-shell",
        "web",
        "pane_1",
        "ws_1",
        "/repo",
        "VITE",
    ] {
        assert!(entry.matches_query(query), "query {query:?} should match");
    }
    for query in ["postgres", "9999", "other-workspace"] {
        assert!(
            !entry.matches_query(query),
            "query {query:?} should not match"
        );
    }
    assert!(entry.matches_query(""));
}

#[test]
fn dev_servers_primary_listener_prefers_wildcard_then_lowest_port() {
    // A wildcard/externally reachable bind beats a lower loopback-only port.
    let mut raw = server_entry(1, "vite", 0);
    raw.listeners = vec![
        DevServerListener {
            address: "127.0.0.1".into(),
            port: 3000,
        },
        DevServerListener {
            address: "*".into(),
            port: 5173,
        },
    ];
    let entry = ClientDevServerEntry::from_server(ClientEndpointId::Local, raw);
    assert_eq!(
        entry.primary_listener().map(|listener| listener.port),
        Some(5173)
    );
    assert_eq!(entry.url().as_deref(), Some("http://localhost:5173"));

    // Within one reachability class the lowest port wins.
    let mut raw = server_entry(1, "node", 0);
    raw.listeners = vec![
        DevServerListener {
            address: "0.0.0.0".into(),
            port: 8080,
        },
        DevServerListener {
            address: "*".into(),
            port: 3000,
        },
    ];
    let entry = ClientDevServerEntry::from_server(ClientEndpointId::Local, raw);
    assert_eq!(entry.url().as_deref(), Some("http://localhost:3000"));

    // Loopback-only processes pick their lowest loopback port.
    let mut raw = server_entry(1, "node", 0);
    raw.listeners = vec![
        DevServerListener {
            address: "127.0.0.1".into(),
            port: 9000,
        },
        DevServerListener {
            address: "::1".into(),
            port: 4000,
        },
    ];
    let entry = ClientDevServerEntry::from_server(ClientEndpointId::Local, raw);
    assert_eq!(entry.url().as_deref(), Some("http://[::1]:4000"));
}

#[test]
fn dev_servers_url_host_formats_each_bind_style() {
    for (address, url) in [
        ("*", "http://localhost:3000"),
        ("0.0.0.0", "http://localhost:3000"),
        ("::", "http://localhost:3000"),
        ("[::]", "http://localhost:3000"),
        ("127.0.0.1", "http://127.0.0.1:3000"),
        ("::1", "http://[::1]:3000"),
        ("[::1]", "http://[::1]:3000"),
        ("192.168.1.20", "http://192.168.1.20:3000"),
    ] {
        let mut raw = server_entry(1, "node", 3000);
        raw.listeners = vec![DevServerListener {
            address: address.into(),
            port: 3000,
        }];
        let entry = ClientDevServerEntry::from_server(ClientEndpointId::Local, raw);
        assert_eq!(entry.url().as_deref(), Some(url), "address {address:?}");
    }
}

#[test]
fn dev_servers_query_matches_secondary_listener_port_and_url() {
    let mut raw = server_entry(42, "vite", 5173);
    raw.listeners = vec![
        DevServerListener {
            address: "*".into(),
            port: 5173,
        },
        DevServerListener {
            address: "127.0.0.1".into(),
            port: 58085,
        },
    ];
    let entry = ClientDevServerEntry::from_server(ClientEndpointId::Local, raw);
    // The hidden secondary listener still filters in.
    assert!(entry.matches_query("58085"));
    assert!(entry.matches_query("127.0.0.1:58085"));
    // Composed URLs match too — the wildcard bind reads as localhost.
    assert!(entry.matches_query("localhost:5173"));
    assert!(entry.matches_query("http://127.0.0.1:58085"));
    assert!(!entry.matches_query("9999"));
}

#[test]
fn dev_servers_d_terminates_and_x_escalates_to_force() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    let actions = open(&mut state);
    state.handle_endpoint_result(
        "boot-1",
        request_id(&actions),
        Ok(list_result(vec![server_entry(42, "node", 3000)])),
    );

    // Leave the filter; `d` sends a graceful kill to the owning endpoint.
    state.handle_input_bytes(b"\r");
    let outcome = state.handle_input_bytes(b"d");
    let kill = endpoint_request(&outcome.actions, &ClientEndpointId::Local);
    assert!(matches!(
        &kill.method,
        Method::ProcessKill(params) if params.pid == 42 && !params.force
    ));

    // `x` escalates to a force kill even while the graceful one is in flight.
    let outcome = state.handle_input_bytes(b"x");
    let force = endpoint_request(&outcome.actions, &ClientEndpointId::Local);
    assert!(matches!(
        &force.method,
        Method::ProcessKill(params) if params.pid == 42 && params.force
    ));
}

#[test]
fn dev_servers_terminate_targets_owning_endpoint_then_escalates() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    let remote = super::endpoint_requests::add_remote(&mut state);

    let actions = open(&mut state);
    let (repaint, _) = state.handle_endpoint_result(
        "remote-boot",
        &endpoint_request(&actions, &remote).id,
        Ok(list_result(vec![server_entry(42, "node", 3000)])),
    );
    assert!(repaint);
    assert_eq!(ready_pids(overlay(&state), &remote), vec![42]);

    // The remote row is the only selectable row; activating it must target
    // the endpoint that owns it, not the presented local endpoint.
    let mut outcome = ClientShellInput::default();
    state.dev_servers_terminate_selected(&mut outcome);
    let kill = endpoint_request(&outcome.actions, &remote);
    let kill_id = kill.id.clone();
    assert!(matches!(
        &kill.method,
        Method::ProcessKill(params) if params.pid == 42 && !params.force
    ));
    assert!(endpoint_requests(&outcome.actions, &ClientEndpointId::Local).is_empty());

    // The graceful kill confirms by re-listing the endpoint.
    let (_, follow_up) =
        state.handle_endpoint_result("remote-boot", &kill_id, Ok(ResponseResult::Ok {}));
    let refresh = endpoint_request(&follow_up, &remote);
    let refresh_id = refresh.id.clone();
    assert!(matches!(&refresh.method, Method::ServerDevServers(_)));

    // The process survived the TERM — the next activation escalates to force.
    let (repaint, _) = state.handle_endpoint_result(
        "remote-boot",
        &refresh_id,
        Ok(list_result(vec![server_entry(42, "node", 3000)])),
    );
    assert!(repaint);
    assert!(matches!(
        overlay(&state).terminating.get(&(remote.clone(), 42)),
        Some(ClientDevServerTermination::StillRunning)
    ));

    let mut outcome = ClientShellInput::default();
    state.dev_servers_terminate_selected(&mut outcome);
    let force = endpoint_request(&outcome.actions, &remote);
    let force_id = force.id.clone();
    assert!(matches!(
        &force.method,
        Method::ProcessKill(params) if params.pid == 42 && params.force
    ));

    let (_, follow_up) =
        state.handle_endpoint_result("remote-boot", &force_id, Ok(ResponseResult::Ok {}));
    let refresh_id = endpoint_request(&follow_up, &remote).id.clone();
    state.handle_endpoint_result("remote-boot", &refresh_id, Ok(list_result(Vec::new())));
    assert!(overlay(&state).terminating.is_empty());
    assert!(ready_pids(overlay(&state), &remote).is_empty());
}

#[test]
fn dev_servers_scoped_result_completes_on_inactive_endpoint() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    let remote = super::endpoint_requests::add_remote(&mut state);
    assert_ne!(state.active_endpoint_id, remote);

    let actions = open(&mut state);
    let (repaint, _) = state.handle_endpoint_result(
        "remote-boot",
        &endpoint_request(&actions, &remote).id,
        Ok(list_result(vec![server_entry(7, "node", 3000)])),
    );
    assert!(repaint);
    assert_eq!(ready_pids(overlay(&state), &remote), vec![7]);
    // The local section is still loading; nothing was retargeted.
    assert!(matches!(
        section(overlay(&state), &ClientEndpointId::Local).state,
        ClientDevServerSectionState::Loading
    ));
}

#[test]
fn dev_servers_request_dropped_on_boot_mismatch_marks_section() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    let remote = super::endpoint_requests::add_remote(&mut state);
    let actions = open(&mut state);
    let request = endpoint_request(&actions, &remote).id.clone();

    // The remote rebooted between request and response.
    let (repaint, actions) =
        state.handle_endpoint_result("different-boot", &request, Ok(list_result(Vec::new())));
    assert!(!repaint);
    assert!(actions.is_empty());
    assert!(matches!(
        section(overlay(&state), &remote).state,
        ClientDevServerSectionState::Error(_)
    ));
}

#[test]
fn dev_servers_keyboard_filter_selection_and_terminate() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    let actions = open(&mut state);
    state.handle_endpoint_result(
        "boot-1",
        request_id(&actions),
        Ok(list_result(vec![
            server_entry(10, "node", 3000),
            server_entry(11, "postgres", 5432),
        ])),
    );

    // Typing filters while the search field is focused.
    let outcome = state.handle_input_bytes(b"postgres");
    assert!(outcome.repaint);
    assert_eq!(overlay(&state).filtered_rows().len(), 1);

    // Enter accepts the query; a second Enter terminates the selected row.
    state.handle_input_bytes(b"\r");
    assert!(!overlay(&state).search_focused);
    let outcome = state.handle_input_bytes(b"\r");
    let kill = endpoint_request(&outcome.actions, &ClientEndpointId::Local);
    assert!(matches!(
        &kill.method,
        Method::ProcessKill(params) if params.pid == 11 && !params.force
    ));

    // Escape backs out of the search field first, then closes the overlay.
    let mut outcome = ClientShellInput::default();
    state.route_overlay_key(
        &crate::input::TerminalKey::new(KeyCode::Char('/'), KeyModifiers::NONE),
        &mut outcome,
    );
    assert!(overlay(&state).search_focused);
    state.handle_input_bytes(b"\x1b");
    assert!(!overlay(&state).search_focused);
    assert!(state.overlay.is_some());
    state.handle_input_bytes(b"\x1b");
    assert!(state.overlay.is_none());
}

#[test]
fn dev_servers_pointer_row_selects_then_confirms() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    let actions = open(&mut state);
    state.handle_endpoint_result(
        "boot-1",
        request_id(&actions),
        Ok(list_result(vec![server_entry(10, "node", 3000)])),
    );

    // First pointer activation selects; the same row again confirms.
    let mut outcome = ClientShellInput::default();
    state.dev_servers_pointer_row(0, true, &mut outcome);
    assert_eq!(
        overlay(&state).selected,
        Some((ClientEndpointId::Local, 10))
    );
    assert!(outcome.actions.is_empty());
    state.dev_servers_pointer_row(0, true, &mut outcome);
    let kill = endpoint_request(&outcome.actions, &ClientEndpointId::Local);
    assert!(matches!(&kill.method, Method::ProcessKill(_)));
}

#[test]
fn dev_servers_disconnect_marks_section_and_cancels_scoped_requests() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    let remote = super::endpoint_requests::add_remote(&mut state);
    let actions = open(&mut state);
    let remote_request = endpoint_request(&actions, &remote).id.clone();
    assert_eq!(state.pending_requests.len(), 2);

    state.mark_endpoint_disconnected(&remote);
    // Only the scoped request was cancelled; the local fan-out survives.
    assert_eq!(state.pending_requests.len(), 1);
    assert!(!state.pending_requests.contains_key(&remote_request));
    assert!(matches!(
        section(overlay(&state), &remote).state,
        ClientDevServerSectionState::Offline
    ));
    assert!(matches!(
        section(overlay(&state), &ClientEndpointId::Local).state,
        ClientDevServerSectionState::Loading
    ));
}

#[test]
fn dev_servers_dispatch_drains_inactive_endpoint_lane() {
    use crate::client::endpoint::{EndpointNegotiation, EndpointRegistry};
    use crate::client::endpoint_commands::EndpointCommands;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct RecordingTransport {
        sent: Arc<Mutex<Vec<ClientMessage>>>,
    }

    impl crate::client::endpoint::EndpointTransport for RecordingTransport {
        fn send(&mut self, message: &ClientMessage) -> std::io::Result<()> {
            self.sent.lock().unwrap().push(message.clone());
            Ok(())
        }
    }

    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    let remote = super::endpoint_requests::add_remote(&mut state);
    let actions = open(&mut state);
    let remote_request = endpoint_request(&actions, &remote).id.clone();

    let remote_sent = Arc::new(Mutex::new(Vec::new()));
    let mut endpoints = EndpointRegistry::new(
        RecordingTransport::default(),
        1,
        EndpointNegotiation::default(),
    );
    endpoints.insert(
        remote.clone(),
        RecordingTransport {
            sent: remote_sent.clone(),
        },
        2,
        EndpointNegotiation::default(),
        // The remote endpoint is connected but does not own a surface.
        false,
    );
    assert_ne!(endpoints.active_id(), &remote);

    let mut commands = EndpointCommands::default();
    let mut scheduled = None;
    crate::client::shell_runtime::dispatch_client_shell_actions(
        actions,
        &mut commands,
        &mut endpoints,
        Some(&mut state),
        &mut Vec::new(),
        &mut scheduled,
    )
    .unwrap();

    let sent = remote_sent.lock().unwrap();
    assert!(sent.iter().any(|message| matches!(
        message,
        ClientMessage::ClientShellEndpointRequest { request, .. }
            if request.contains(&remote_request)
    )));
    drop(sent);
    // The remote lane is in flight now; the local request also drained.
    assert!(commands.accepts_response(&remote, 2, "remote-boot", &remote_request));
}

#[test]
fn dev_servers_overlay_renders_sections_and_row_hits() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    let remote = super::endpoint_requests::add_remote(&mut state);
    let actions = open(&mut state);
    state.handle_endpoint_result(
        "boot-1",
        &endpoint_request(&actions, &ClientEndpointId::Local).id,
        Ok(list_result(vec![server_entry(10, "node", 3000)])),
    );
    state.handle_endpoint_result(
        "remote-boot",
        &endpoint_request(&actions, &remote).id,
        Ok(list_result(vec![server_entry(42, "vite", 5173)])),
    );
    state.set_pane_surface(surface());
    state.compose(106, 40).unwrap();
    assert_eq!(state.hits.dev_server_rows.len(), 2);
    assert_eq!(state.hits.dev_server_url_rows.len(), 2);
    assert!(!state.hits.dev_server_search.is_empty());
    assert!(!state.hits.dev_server_popup.is_empty());
}

#[test]
fn dev_servers_row_shows_url_badge_name_and_meta() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    let actions = open(&mut state);
    let mut vite = server_entry(42, "vite", 5173);
    vite.listeners = vec![
        DevServerListener {
            address: "*".into(),
            port: 5173,
        },
        DevServerListener {
            address: "127.0.0.1".into(),
            port: 58085,
        },
    ];
    state.handle_endpoint_result("boot-1", request_id(&actions), Ok(list_result(vec![vite])));
    state.set_pane_surface(surface());
    let frame = state.compose(106, 40).unwrap();
    let rows = frame_rows(&frame);
    let row = rows
        .iter()
        .find(|row| row.contains("http://localhost:5173"))
        .expect("dev server row")
        .clone();
    // The extra listener collapses into +N; raw bind addresses stay hidden.
    assert!(row.contains("http://localhost:5173+1"), "row: {row}");
    // Process and context are separate columns now — padded, not ` · ` joined.
    assert!(row.contains("vite"), "row: {row}");
    assert!(row.contains("client-shell · web"), "row: {row}");
    assert!(!row.contains("vite · client-shell"), "row: {row}");
    assert!(row.contains("pid 42 · up 1m 30s"), "row: {row}");
    assert!(!row.contains("(pid"), "row: {row}");
    assert!(!row.contains("127.0.0.1:58085"), "row: {row}");
    // The URL span is a click target of its own inside the row.
    assert_eq!(state.hits.dev_server_url_rows.len(), 1);
    let (rect, flat) = state.hits.dev_server_url_rows[0];
    assert_eq!(flat, 0);
    assert_eq!(rect.width, "http://localhost:5173".len() as u16);
}

#[test]
fn dev_servers_columns_align_across_rows() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    let actions = open(&mut state);
    let mut vite = server_entry(42, "vite", 5173);
    vite.listeners = vec![
        DevServerListener {
            address: "*".into(),
            port: 5173,
        },
        DevServerListener {
            address: "127.0.0.1".into(),
            port: 58085,
        },
    ];
    let node = server_entry(10, "node", 3000);
    state.handle_endpoint_result(
        "boot-1",
        request_id(&actions),
        Ok(list_result(vec![vite, node])),
    );
    state.set_pane_surface(surface());
    let frame = state.compose(106, 40).unwrap();
    let rows = frame_rows(&frame);
    let vite_row = rows
        .iter()
        .find(|row| row.contains("http://localhost:5173"))
        .expect("vite row");
    let node_row = rows
        .iter()
        .find(|row| row.contains("http://127.0.0.1:3000"))
        .expect("node row");
    // Different URL/badge lengths still put every column at the same offset.
    // Compare character columns — row prefixes contain multi-byte `─`/`│`.
    let column_of =
        |row: &str, needle: &str| row.find(needle).map(|byte| row[..byte].chars().count());
    assert_eq!(
        column_of(vite_row, "vite"),
        column_of(node_row, "node"),
        "vite: {vite_row:?}\nnode: {node_row:?}"
    );
    assert_eq!(
        column_of(vite_row, "client-shell"),
        column_of(node_row, "client-shell")
    );
    assert_eq!(column_of(vite_row, "pid 42"), column_of(node_row, "pid 10"));
}

#[test]
fn dev_servers_columns_truncate_when_narrow() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    let actions = open(&mut state);
    let mut vite = server_entry(42, "vite", 5173);
    vite.listeners = vec![
        DevServerListener {
            address: "*".into(),
            port: 5173,
        },
        DevServerListener {
            address: "127.0.0.1".into(),
            port: 58085,
        },
    ];
    let node = server_entry(10, "node", 3000);
    state.handle_endpoint_result(
        "boot-1",
        request_id(&actions),
        Ok(list_result(vec![vite, node])),
    );
    state.set_pane_surface(surface());
    let frame = state.compose(48, 40).unwrap();
    let rows = frame_rows(&frame);
    // Context, then process, shrink to nothing; the URL cell keeps a viable
    // `http://…` truncation and the right meta stays put.
    let vite_row = rows
        .iter()
        .find(|row| row.contains("pid 42"))
        .expect("vite row");
    assert!(vite_row.contains("http://local…"), "row: {vite_row}");
    assert!(vite_row.contains("pid 42 · up 1m 30s"), "row: {vite_row}");
    assert!(!vite_row.contains("client-shell"), "row: {vite_row}");
    let node_row = rows
        .iter()
        .find(|row| row.contains("pid 10"))
        .expect("node row");
    assert!(node_row.contains("…"), "row: {node_row}");
}

#[test]
fn dev_servers_url_click_emits_open_url_action() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    let actions = open(&mut state);
    let mut vite = server_entry(42, "vite", 5173);
    vite.listeners = vec![DevServerListener {
        address: "*".into(),
        port: 5173,
    }];
    state.handle_endpoint_result("boot-1", request_id(&actions), Ok(list_result(vec![vite])));
    state.set_pane_surface(surface());
    state.compose(106, 40).unwrap();
    let (rect, _) = state.hits.dev_server_url_rows[0];
    let mut outcome = ClientShellInput::default();
    state.handle_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: rect.x + 2,
            row: rect.y,
            modifiers: KeyModifiers::NONE,
        },
        &mut outcome,
    );
    assert!(matches!(
        outcome.actions.as_slice(),
        [ClientShellAction::OpenSafeWebUrl(url)] if url == "http://localhost:5173"
    ));
    // The click opened the link instead of selecting or terminating the row.
    assert!(overlay(&state).selected.is_none());
    assert!(endpoint_requests(&outcome.actions, &ClientEndpointId::Local).is_empty());
}
