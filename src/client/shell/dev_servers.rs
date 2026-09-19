use super::*;

/// Server-owned runtime methods this overlay consumes. Both are advertised per
/// endpoint and accepted on inactive client-shell surfaces, so fan-out and
/// termination reach every connected server, not only the presented one.
pub(super) const DEV_SERVERS_METHOD: &str = "server.dev_servers";
pub(super) const PROCESS_KILL_METHOD: &str = "process.kill";

/// Initial per-endpoint presentation state. Endpoints that cannot answer are
/// still listed so the overlay reads as a whole-fleet view.
fn dev_server_section_state(endpoint: &ClientShellEndpoint) -> ClientDevServerSectionState {
    match endpoint.status {
        ClientEndpointStatus::Online => match endpoint.methods.as_ref() {
            Some(methods) if !methods.contains(DEV_SERVERS_METHOD) => {
                ClientDevServerSectionState::Unsupported
            }
            // `None` means the negotiation did not report methods (older or
            // test endpoints); the request attempt decides.
            _ if endpoint.snapshot.is_some() => ClientDevServerSectionState::Loading,
            _ => ClientDevServerSectionState::Offline,
        },
        _ => ClientDevServerSectionState::Offline,
    }
}

impl ClientShellState {
    pub(super) fn open_dev_servers_overlay(&mut self, outcome: &mut ClientShellInput) {
        if matches!(self.overlay, Some(ClientShellOverlay::DevServers(_))) {
            self.refresh_dev_servers(outcome);
            outcome.repaint = true;
            return;
        }
        let sections = self
            .endpoints
            .iter()
            .map(|endpoint| ClientDevServerSection {
                endpoint_id: endpoint.endpoint_id.clone(),
                label: endpoint.label.clone(),
                status: endpoint.status,
                state: dev_server_section_state(endpoint),
            })
            .collect();
        self.overlay = Some(ClientShellOverlay::DevServers(ClientDevServersOverlay {
            sections,
            selected: None,
            query: TextEditor::default(),
            search_focused: true,
            error: None,
            terminating: HashMap::new(),
        }));
        self.refresh_dev_servers(outcome);
        outcome.repaint = true;
    }

    /// Fan `server.dev_servers` out to every endpoint that can answer. Each
    /// request is endpoint-scoped so it keeps working while another endpoint
    /// holds the presented surface. Sections also track catalog changes —
    /// endpoints removed while the overlay is open drop out, new ones append.
    pub(super) fn refresh_dev_servers(&mut self, outcome: &mut ClientShellInput) {
        let catalog = self
            .endpoints
            .iter()
            .map(|endpoint| {
                let supported = endpoint.status == ClientEndpointStatus::Online
                    && endpoint.snapshot.is_some()
                    && endpoint
                        .methods
                        .as_ref()
                        .is_none_or(|methods| methods.contains(DEV_SERVERS_METHOD));
                (
                    endpoint.endpoint_id.clone(),
                    endpoint.label.clone(),
                    endpoint.status,
                    dev_server_section_state(endpoint),
                    supported,
                )
            })
            .collect::<Vec<_>>();
        if let Some(ClientShellOverlay::DevServers(overlay)) = self.overlay.as_mut() {
            overlay.sections.retain(|section| {
                catalog
                    .iter()
                    .any(|(endpoint_id, ..)| &section.endpoint_id == endpoint_id)
            });
            overlay
                .terminating
                .retain(|(endpoint_id, _), _| catalog.iter().any(|(id, ..)| id == endpoint_id));
            for (endpoint_id, label, status, state, supported) in &catalog {
                let sync_state = |state: &ClientDevServerSectionState| match state {
                    ClientDevServerSectionState::Unsupported => {
                        ClientDevServerSectionState::Unsupported
                    }
                    ClientDevServerSectionState::Loading => ClientDevServerSectionState::Loading,
                    _ => ClientDevServerSectionState::Offline,
                };
                match overlay
                    .sections
                    .iter_mut()
                    .find(|section| &section.endpoint_id == endpoint_id)
                {
                    Some(section) => {
                        section.label = label.clone();
                        section.status = *status;
                        // A fresh request replaces whatever the section showed;
                        // without one the section falls back to connectivity
                        // and capability, so stale rows never survive a drop.
                        section.state = if *supported {
                            ClientDevServerSectionState::Loading
                        } else {
                            match state {
                                ClientDevServerSectionState::Unsupported => {
                                    ClientDevServerSectionState::Unsupported
                                }
                                _ => ClientDevServerSectionState::Offline,
                            }
                        };
                    }
                    None => overlay.sections.push(ClientDevServerSection {
                        endpoint_id: endpoint_id.clone(),
                        label: label.clone(),
                        status: *status,
                        state: sync_state(state),
                    }),
                }
            }
        }
        for (endpoint_id, _, _, _, supported) in catalog {
            if supported {
                let sent = self.push_endpoint_method_for(
                    &endpoint_id,
                    crate::api::schema::Method::ServerDevServers(
                        crate::api::schema::EmptyParams::default(),
                    ),
                    PendingEndpointKind::DevServerList {
                        endpoint_id: endpoint_id.clone(),
                    },
                    outcome,
                );
                if !sent {
                    if let Some(ClientShellOverlay::DevServers(overlay)) = self.overlay.as_mut() {
                        overlay.set_error(&endpoint_id, "endpoint is not ready".to_owned());
                    }
                }
            }
        }
    }

    /// A snapshot landed for `endpoint_id` while the overlay is open — the
    /// section may now be able to answer (or newly not). Returns the actions
    /// to dispatch, so callers that lack an outcome can still send.
    pub(crate) fn dev_servers_endpoint_ready(
        &mut self,
        endpoint_id: &ClientEndpointId,
    ) -> Vec<ClientShellAction> {
        if !matches!(self.overlay, Some(ClientShellOverlay::DevServers(_))) {
            return Vec::new();
        }
        let endpoint = self
            .endpoints
            .iter()
            .find(|endpoint| &endpoint.endpoint_id == endpoint_id);
        let Some(endpoint) = endpoint else {
            return Vec::new();
        };
        let supported = endpoint.status == ClientEndpointStatus::Online
            && endpoint.snapshot.is_some()
            && endpoint
                .methods
                .as_ref()
                .is_none_or(|methods| methods.contains(DEV_SERVERS_METHOD));
        let (label, status) = (endpoint.label.clone(), endpoint.status);
        let section_state = dev_server_section_state(endpoint);
        if let Some(ClientShellOverlay::DevServers(overlay)) = self.overlay.as_mut() {
            match overlay
                .sections
                .iter_mut()
                .find(|section| &section.endpoint_id == endpoint_id)
            {
                Some(section) => {
                    section.label = label;
                    section.status = status;
                    section.state = if supported {
                        ClientDevServerSectionState::Loading
                    } else {
                        section_state
                    };
                }
                None => overlay.sections.push(ClientDevServerSection {
                    endpoint_id: endpoint_id.clone(),
                    label,
                    status,
                    state: section_state,
                }),
            }
        }
        let mut outcome = ClientShellInput::default();
        if supported {
            self.push_endpoint_method_for(
                endpoint_id,
                crate::api::schema::Method::ServerDevServers(
                    crate::api::schema::EmptyParams::default(),
                ),
                PendingEndpointKind::DevServerList {
                    endpoint_id: endpoint_id.clone(),
                },
                &mut outcome,
            );
        }
        outcome.actions
    }

    /// An endpoint-scoped request outlived the boot it targeted (the server
    /// restarted or the connection cycled before answering).
    pub(super) fn dev_servers_request_dropped(&mut self, endpoint_id: &ClientEndpointId) {
        let Some(ClientShellOverlay::DevServers(overlay)) = self.overlay.as_mut() else {
            return;
        };
        let kills_dropped = overlay
            .terminating
            .keys()
            .any(|(entry_endpoint, _)| entry_endpoint == endpoint_id);
        overlay
            .terminating
            .retain(|(entry_endpoint, _), _| entry_endpoint != endpoint_id);
        if let Some(section) = overlay
            .sections
            .iter_mut()
            .find(|section| &section.endpoint_id == endpoint_id)
        {
            if matches!(section.state, ClientDevServerSectionState::Loading) {
                section.state = ClientDevServerSectionState::Error("server restarted".to_owned());
            }
        }
        if kills_dropped {
            overlay.error = Some("server restarted while terminating a process".to_owned());
        }
    }

    /// Disconnect/retire bookkeeping for an open overlay. Unsupported stays —
    /// it is a capability fact, not a connectivity one.
    pub(super) fn dev_servers_endpoint_disconnected(&mut self, endpoint_id: &ClientEndpointId) {
        let status = self.endpoint_status(endpoint_id);
        let Some(ClientShellOverlay::DevServers(overlay)) = self.overlay.as_mut() else {
            return;
        };
        overlay
            .terminating
            .retain(|(entry_endpoint, _), _| entry_endpoint != endpoint_id);
        if let Some(section) = overlay
            .sections
            .iter_mut()
            .find(|section| &section.endpoint_id == endpoint_id)
        {
            if let Some(status) = status {
                section.status = status;
            }
            if !matches!(section.state, ClientDevServerSectionState::Unsupported) {
                section.state = ClientDevServerSectionState::Offline;
            }
        }
    }

    pub(super) fn move_dev_servers_selection(&mut self, delta: isize) {
        if let Some(ClientShellOverlay::DevServers(overlay)) = self.overlay.as_mut() {
            overlay.move_selection(delta);
        }
    }

    /// The failure message when a termination request cannot even be sent —
    /// distinct from `push_endpoint_method_for`'s silent refusal so the overlay
    /// can say why.
    fn dev_server_termination_unavailable(&self, endpoint_id: &ClientEndpointId) -> String {
        let endpoint = self
            .endpoints
            .iter()
            .find(|endpoint| &endpoint.endpoint_id == endpoint_id);
        let label = endpoint
            .map(|endpoint| endpoint.label.as_str())
            .unwrap_or("Server");
        match endpoint.map(|endpoint| endpoint.status) {
            Some(ClientEndpointStatus::Online) => {
                if endpoint
                    .and_then(|endpoint| endpoint.methods.as_ref())
                    .is_some_and(|methods| !methods.contains(PROCESS_KILL_METHOD))
                {
                    format!("{label} does not support process termination")
                } else {
                    format!("{label} is not ready")
                }
            }
            _ => format!("{label} is offline"),
        }
    }

    /// Terminate the highlighted server: graceful first, `force` only once a
    /// refresh proved the pid still listens. Requests go to the endpoint that
    /// owns the row, which is not necessarily the active one.
    pub(super) fn dev_servers_terminate_selected(&mut self, outcome: &mut ClientShellInput) {
        let Some(ClientShellOverlay::DevServers(overlay)) = self.overlay.as_ref() else {
            return;
        };
        let Some((section_index, entry_index)) = overlay.selected_row() else {
            return;
        };
        let Some(entry) = overlay.entry_at(section_index, entry_index) else {
            return;
        };
        let endpoint_id = entry.endpoint_id.clone();
        let pid = entry.pid;
        let key = (endpoint_id.clone(), pid);
        match overlay.terminating.get(&key) {
            // A kill is already in flight; a second activation is a no-op.
            Some(ClientDevServerTermination::Sent) => return,
            Some(ClientDevServerTermination::StillRunning) => {}
            None => {}
        }
        let force = matches!(
            overlay.terminating.get(&key),
            Some(ClientDevServerTermination::StillRunning)
        );
        let sent = self.push_endpoint_method_for(
            &endpoint_id,
            crate::api::schema::Method::ProcessKill(crate::api::schema::ProcessKillParams {
                pid,
                force,
            }),
            PendingEndpointKind::ProcessKill {
                endpoint_id: endpoint_id.clone(),
                pid,
            },
            outcome,
        );
        let unavailable = (!sent).then(|| self.dev_server_termination_unavailable(&endpoint_id));
        if let Some(ClientShellOverlay::DevServers(overlay)) = self.overlay.as_mut() {
            if sent {
                overlay
                    .terminating
                    .insert(key, ClientDevServerTermination::Sent);
                overlay.error = None;
            } else {
                overlay.error = unavailable;
            }
        }
        outcome.repaint = true;
    }

    pub(super) fn route_dev_servers_key(
        &mut self,
        key: &crate::input::TerminalKey,
        outcome: &mut ClientShellInput,
    ) -> bool {
        if !matches!(self.overlay, Some(ClientShellOverlay::DevServers(_))) {
            return false;
        }
        let (code, modifiers) = crate::config::normalize_key_combo((key.code, key.modifiers));
        let search_focused = matches!(
            self.overlay,
            Some(ClientShellOverlay::DevServers(ClientDevServersOverlay {
                search_focused: true,
                ..
            }))
        );
        match code {
            KeyCode::Esc => {
                if search_focused {
                    if let Some(ClientShellOverlay::DevServers(overlay)) = self.overlay.as_mut() {
                        overlay.search_focused = false;
                    }
                } else {
                    self.overlay = None;
                }
                outcome.repaint = true;
            }
            // While filtering, Enter accepts the query; a second Enter on the
            // resulting selection is what terminates. That keeps a typed
            // search from ever signalling a process.
            KeyCode::Enter if search_focused => {
                if let Some(ClientShellOverlay::DevServers(overlay)) = self.overlay.as_mut() {
                    overlay.search_focused = false;
                }
                outcome.repaint = true;
            }
            KeyCode::Enter => self.dev_servers_terminate_selected(outcome),
            _ if search_focused => {
                if let Some(ClientShellOverlay::DevServers(overlay)) = self.overlay.as_mut() {
                    if let Some(content_changed) = overlay.query.handle_key(key) {
                        let _ = content_changed;
                        outcome.repaint = true;
                        return true;
                    }
                }
                match code {
                    KeyCode::Up => {
                        self.move_dev_servers_selection(-1);
                        outcome.repaint = true;
                    }
                    KeyCode::Down => {
                        self.move_dev_servers_selection(1);
                        outcome.repaint = true;
                    }
                    KeyCode::Char('n' | 'p')
                        if modifiers == crossterm::event::KeyModifiers::CONTROL =>
                    {
                        self.move_dev_servers_selection(if code == KeyCode::Char('n') {
                            1
                        } else {
                            -1
                        });
                        outcome.repaint = true;
                    }
                    _ => {}
                }
            }
            KeyCode::Char('/') if modifiers.is_empty() => {
                if let Some(ClientShellOverlay::DevServers(overlay)) = self.overlay.as_mut() {
                    overlay.search_focused = true;
                }
                outcome.repaint = true;
            }
            KeyCode::Char('j') | KeyCode::Down if modifiers.is_empty() => {
                self.move_dev_servers_selection(1);
                outcome.repaint = true;
            }
            KeyCode::Char('k') | KeyCode::Up if modifiers.is_empty() => {
                self.move_dev_servers_selection(-1);
                outcome.repaint = true;
            }
            KeyCode::Home if modifiers.is_empty() => {
                if let Some(ClientShellOverlay::DevServers(overlay)) = self.overlay.as_mut() {
                    overlay.select_flat_index(0);
                }
                outcome.repaint = true;
            }
            KeyCode::End if modifiers.is_empty() => {
                if let Some(ClientShellOverlay::DevServers(overlay)) = self.overlay.as_mut() {
                    let last = overlay.filtered_rows().len().saturating_sub(1);
                    overlay.select_flat_index(last);
                }
                outcome.repaint = true;
            }
            KeyCode::Char('r') if modifiers.is_empty() => {
                self.refresh_dev_servers(outcome);
                outcome.repaint = true;
            }
            _ => {}
        }
        true
    }

    pub(super) fn insert_dev_servers_text(&mut self, text: &str) -> bool {
        match self.overlay.as_mut() {
            Some(ClientShellOverlay::DevServers(overlay)) if overlay.search_focused => {
                overlay.query.insert(text);
                true
            }
            _ => false,
        }
    }

    /// Mouse hover/click target: `flat` indexes `filtered_rows()`. A click on
    /// the already-selected row confirms termination.
    pub(super) fn dev_servers_pointer_row(
        &mut self,
        flat: usize,
        activate: bool,
        outcome: &mut ClientShellInput,
    ) {
        let previous = match self.overlay.as_ref() {
            Some(ClientShellOverlay::DevServers(overlay)) => overlay.selected.clone(),
            _ => None,
        };
        let Some(ClientShellOverlay::DevServers(overlay)) = self.overlay.as_mut() else {
            return;
        };
        let key = overlay.select_flat_index(flat);
        if activate && key.is_some() && key == previous {
            self.dev_servers_terminate_selected(outcome);
        }
        outcome.repaint = true;
    }

    pub(super) fn handle_dev_server_endpoint_result(
        &mut self,
        kind: PendingEndpointKind,
        result: Result<crate::api::schema::ResponseResult, ClientShellEndpointError>,
        outcome: &mut ClientShellInput,
    ) -> bool {
        use crate::api::schema::ResponseResult;

        match (kind, result) {
            (
                PendingEndpointKind::DevServerList { endpoint_id },
                Ok(ResponseResult::DevServerList { servers }),
            ) => {
                let Some(ClientShellOverlay::DevServers(overlay)) = self.overlay.as_mut() else {
                    return false;
                };
                let listed: HashSet<u32> = servers.iter().map(|entry| entry.pid).collect();
                let entries = servers
                    .into_iter()
                    .map(|entry| ClientDevServerEntry::from_server(endpoint_id.clone(), entry))
                    .collect();
                overlay.set_ready(&endpoint_id, entries);
                overlay.terminating.retain(|(entry_endpoint, pid), state| {
                    if entry_endpoint != &endpoint_id {
                        return true;
                    }
                    if listed.contains(pid) {
                        // The pid survived a graceful kill — one more Enter
                        // escalates to force.
                        *state = ClientDevServerTermination::StillRunning;
                        true
                    } else {
                        false
                    }
                });
                true
            }
            (PendingEndpointKind::DevServerList { endpoint_id }, Ok(_)) => {
                if let Some(ClientShellOverlay::DevServers(overlay)) = self.overlay.as_mut() {
                    overlay.set_error(
                        &endpoint_id,
                        "endpoint returned an unexpected dev-servers result".to_owned(),
                    );
                }
                true
            }
            (PendingEndpointKind::DevServerList { endpoint_id }, Err(error)) => {
                let Some(ClientShellOverlay::DevServers(overlay)) = self.overlay.as_mut() else {
                    return false;
                };
                // A failed refresh keeps the stale list visible; a failed first
                // load becomes the section's state.
                let has_entries = overlay
                    .sections
                    .iter()
                    .find(|section| section.endpoint_id == endpoint_id)
                    .is_some_and(|section| {
                        matches!(section.state, ClientDevServerSectionState::Ready(_))
                    });
                if has_entries {
                    let label = overlay
                        .sections
                        .iter()
                        .find(|section| section.endpoint_id == endpoint_id)
                        .map(|section| section.label.clone())
                        .unwrap_or_default();
                    overlay.error = Some(format!("{label}: {}", error.message));
                } else {
                    overlay.set_error(&endpoint_id, error.message);
                }
                true
            }
            (PendingEndpointKind::ProcessKill { endpoint_id, pid }, Ok(ResponseResult::Ok {})) => {
                if !matches!(self.overlay, Some(ClientShellOverlay::DevServers(_))) {
                    return false;
                }
                // Observe whether the pid actually exited by re-listing; the
                // list response advances or clears the termination state.
                let sent = self.push_endpoint_method_for(
                    &endpoint_id,
                    crate::api::schema::Method::ServerDevServers(
                        crate::api::schema::EmptyParams::default(),
                    ),
                    PendingEndpointKind::DevServerList {
                        endpoint_id: endpoint_id.clone(),
                    },
                    outcome,
                );
                if !sent {
                    let error = self.dev_server_termination_unavailable(&endpoint_id);
                    if let Some(ClientShellOverlay::DevServers(overlay)) = self.overlay.as_mut() {
                        overlay.terminating.remove(&(endpoint_id, pid));
                        overlay.error = Some(error);
                    }
                }
                true
            }
            (PendingEndpointKind::ProcessKill { endpoint_id, pid }, Ok(_)) => {
                if let Some(ClientShellOverlay::DevServers(overlay)) = self.overlay.as_mut() {
                    overlay.terminating.remove(&(endpoint_id, pid));
                    overlay.error =
                        Some("endpoint returned an unexpected process.kill result".to_owned());
                }
                true
            }
            (PendingEndpointKind::ProcessKill { endpoint_id, pid }, Err(error)) => {
                if let Some(ClientShellOverlay::DevServers(overlay)) = self.overlay.as_mut() {
                    overlay.terminating.remove(&(endpoint_id, pid));
                    overlay.error = Some(error.message);
                }
                true
            }
            _ => false,
        }
    }
}
