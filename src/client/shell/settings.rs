use super::*;
use crossterm::event::{KeyCode, KeyModifiers};

pub(super) fn normalized_theme_name(name: &str) -> String {
    name.to_lowercase().replace([' ', '_'], "-")
}

/// Write a terminal escape sequence to the host terminal. Instant and
/// side-effect free — used for live Ghostty color previews while browsing
/// the theme list. Gated out of test builds so unit tests never emit OSC.
fn emit_host_sequence(sequence: &str) {
    #[cfg(not(test))]
    {
        use std::io::Write as _;
        let _ = std::io::stdout().write_all(sequence.as_bytes());
        let _ = std::io::stdout().flush();
    }
    #[cfg(test)]
    let _ = sequence;
}

/// All entries shown in the settings theme list: built-in themes first,
/// then a `ghostty` follow entry and every discovered Ghostty theme.
fn theme_choices() -> Vec<ClientThemeChoice> {
    let mut choices: Vec<ClientThemeChoice> = crate::config::THEME_NAMES
        .iter()
        .map(|name| ClientThemeChoice {
            label: (*name).to_owned(),
            value: (*name).to_owned(),
        })
        .collect();
    let ghostty_names = crate::config::ghostty_theme_names();
    if ghostty_names.is_empty() {
        return choices;
    }
    choices.push(ClientThemeChoice {
        label: "ghostty (follow)".to_owned(),
        value: "ghostty".to_owned(),
    });
    choices.extend(ghostty_names.into_iter().map(|name| ClientThemeChoice {
        value: format!("{}{}", crate::config::GHOSTTY_THEME_PREFIX, name),
        label: name,
    }));
    choices
}

fn theme_choice_index(choices: &[ClientThemeChoice], name: &str) -> usize {
    let normalized = normalized_theme_name(name);
    choices
        .iter()
        .position(|choice| normalized_theme_name(&choice.value) == normalized)
        .unwrap_or(0)
}

fn indicator_index(style: crate::config::StatusIndicatorStyle) -> usize {
    usize::from(style == crate::config::StatusIndicatorStyle::Symbols)
}

fn toast_index(delivery: crate::config::ToastDelivery) -> usize {
    match delivery {
        crate::config::ToastDelivery::Off => 0,
        crate::config::ToastDelivery::Herdr => 1,
        crate::config::ToastDelivery::Terminal => 2,
        crate::config::ToastDelivery::System => 3,
    }
}

pub(super) fn integration_needs_install(info: &crate::api::schema::IntegrationInfo) -> bool {
    info.state == crate::api::schema::IntegrationState::Outdated
        || info.available && info.state == crate::api::schema::IntegrationState::NotInstalled
}

impl ClientShellState {
    pub(super) fn open_settings_overlay(&mut self) {
        let choices = theme_choices();
        self.overlay = Some(ClientShellOverlay::Settings(ClientSettingsOverlay {
            section: ClientSettingsSection::Theme,
            selected: theme_choice_index(&choices, &self.config.theme_name),
            original_theme_name: self.config.theme_name.clone(),
            original_palette: self.config.palette.clone(),
            theme_choices: choices,
            query: TextEditor::default(),
            search_focused: false,
            integrations: Vec::new(),
            integration_messages: Vec::new(),
            loading_integrations: false,
            installing_integrations: false,
        }));
    }

    fn selected_index_for_settings_section(&self, section: ClientSettingsSection) -> usize {
        match section {
            ClientSettingsSection::Theme => match &self.overlay {
                Some(ClientShellOverlay::Settings(settings)) => {
                    theme_choice_index(&settings.theme_choices, &self.config.theme_name)
                }
                _ => 0,
            },
            ClientSettingsSection::Indicators => indicator_index(self.config.status_indicators),
            ClientSettingsSection::Sound => usize::from(!self.config.sound_enabled),
            ClientSettingsSection::Toast => toast_index(self.config.toast_delivery),
            ClientSettingsSection::Integrations => 0,
        }
    }

    pub(super) fn select_settings_section(
        &mut self,
        section: ClientSettingsSection,
        outcome: &mut ClientShellInput,
    ) {
        let selected = self.selected_index_for_settings_section(section);
        let request_integrations = matches!(section, ClientSettingsSection::Integrations)
            && matches!(
                self.overlay,
                Some(ClientShellOverlay::Settings(ClientSettingsOverlay {
                    loading_integrations: false,
                    installing_integrations: false,
                    ..
                }))
            );
        if let Some(ClientShellOverlay::Settings(settings)) = self.overlay.as_mut() {
            settings.section = section;
            settings.selected = selected;
            settings.search_focused = false;
        }
        if request_integrations {
            self.queue_integration_list(outcome, true);
        }
        outcome.repaint = true;
    }

    fn move_settings_section(&mut self, delta: isize, outcome: &mut ClientShellInput) {
        let Some(ClientShellOverlay::Settings(settings)) = self.overlay.as_ref() else {
            return;
        };
        let current = ClientSettingsSection::ALL
            .iter()
            .position(|section| *section == settings.section)
            .unwrap_or(0);
        let next = (current as isize + delta).rem_euclid(ClientSettingsSection::ALL.len() as isize)
            as usize;
        self.select_settings_section(ClientSettingsSection::ALL[next], outcome);
    }

    fn settings_choice_count(&self) -> usize {
        match self.overlay.as_ref() {
            Some(ClientShellOverlay::Settings(settings)) => match settings.section {
                ClientSettingsSection::Theme => settings.filtered_theme_indices().len(),
                ClientSettingsSection::Indicators | ClientSettingsSection::Sound => 2,
                ClientSettingsSection::Toast => 4,
                ClientSettingsSection::Integrations => settings.integrations.len(),
            },
            _ => 0,
        }
    }

    pub(super) fn move_settings_selection(&mut self, delta: isize) {
        let count = self.settings_choice_count();
        let Some(ClientShellOverlay::Settings(settings)) = self.overlay.as_mut() else {
            return;
        };
        if count == 0 {
            settings.selected = 0;
            return;
        }
        if settings.section == ClientSettingsSection::Theme {
            // `selected` is a theme-choices index; move within the filtered
            // positions like the worktree open picker.
            let filtered = settings.filtered_theme_indices();
            let position = filtered
                .iter()
                .position(|index| *index == settings.selected)
                .unwrap_or(0);
            let next = (position as isize + delta)
                .clamp(0, filtered.len().saturating_sub(1) as isize)
                as usize;
            settings.selected = filtered[next];
        } else {
            settings.selected = (settings.selected as isize + delta)
                .clamp(0, count.saturating_sub(1) as isize)
                as usize;
        }
        if settings.section == ClientSettingsSection::Theme {
            self.preview_selected_theme();
        }
    }

    pub(super) fn select_settings_choice(&mut self, index: usize) {
        let count = self.settings_choice_count();
        if let Some(ClientShellOverlay::Settings(settings)) = self.overlay.as_mut() {
            if count > 0 {
                settings.selected = if settings.section == ClientSettingsSection::Theme {
                    index.min(settings.theme_choices.len().saturating_sub(1))
                } else {
                    index.min(count - 1)
                };
            }
        }
        if matches!(
            self.overlay,
            Some(ClientShellOverlay::Settings(ClientSettingsOverlay {
                section: ClientSettingsSection::Theme,
                ..
            }))
        ) {
            self.preview_selected_theme();
        }
    }

    fn preview_selected_theme(&mut self) {
        let Some(ClientShellOverlay::Settings(settings)) = self.overlay.as_ref() else {
            return;
        };
        let Some(choice) = settings
            .selected_theme_index()
            .and_then(|index| settings.theme_choices.get(index))
        else {
            return;
        };
        let value = choice.value.clone();
        self.config.theme_name = value.clone();
        self.config.palette =
            crate::app::client_palette_for_theme(&self.config.theme_runtime, &value);
        // Instant host-side preview: OSC sequences recolor the terminal
        // without touching Ghostty's config. Enter persists through
        // sync_ghostty_theme; Esc restores via reset sequences.
        match crate::config::ghostty_name_for_herdr_theme(&value)
            .and_then(|name| crate::config::load_ghostty_theme(&name).map(|spec| (name, spec)))
        {
            Some((name, spec)) => {
                emit_host_sequence(&crate::config::ghostty_preview_sequence(&spec));
                self.ghostty_osc_theme = Some(name);
            }
            None => self.restore_ghostty_preview(),
        }
    }

    /// Persist the effective herdr theme to Ghostty's config
    /// (`theme = ...`, or a `dark:X,light:Y` pair when herdr auto-switches)
    /// and ask Ghostty to reload. Only writes when the value changed.
    pub(crate) fn sync_ghostty_theme(&mut self) {
        // Unit tests must set GHOSTTY_CONFIG to opt into real file access.
        if cfg!(test) && std::env::var_os(crate::config::GHOSTTY_CONFIG_ENV).is_none() {
            return;
        }
        let Some(desired) = crate::app::ghostty_theme_value_for_runtime(
            &self.config.theme_runtime,
            self.host_appearance,
        ) else {
            return;
        };
        let path = crate::config::ghostty_config_path();
        if !path.exists() {
            return;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            return;
        };
        let Some(updated) = crate::config::ghostty_config_sync_update(&content, &desired) else {
            return;
        };
        if std::fs::write(&path, updated).is_err() {
            return;
        }
        // SIGUSR2 reloads Ghostty's configuration.
        #[cfg(all(unix, not(test)))]
        {
            let reloaded = std::process::Command::new("pkill")
                .args(["-USR2", "-i", "-x", "ghostty"])
                .status()
                .is_ok_and(|status| status.success());
            if !reloaded {
                let _ = std::process::Command::new("pkill")
                    .args(["-USR2", "-x", "ghostty"])
                    .status();
            }
        }
    }

    /// Undo a host-terminal OSC preview, restoring the terminal's
    /// configured colors.
    pub(super) fn restore_ghostty_preview(&mut self) {
        if self.ghostty_osc_theme.take().is_some() {
            emit_host_sequence(&crate::config::ghostty_preview_reset_sequence());
        }
    }

    /// The applied theme is now the end state: keep the OSC preview when it
    /// already shows the theme Ghostty is about to load (zero flicker), and
    /// reset to the configured colors otherwise.
    fn settle_ghostty_preview(&mut self) {
        let resolved = crate::app::ghostty_theme_value_for_runtime(
            &self.config.theme_runtime,
            self.host_appearance,
        )
        .and_then(|value| {
            crate::app::ghostty_theme_name_for_appearance(&value, self.host_appearance)
        });
        if resolved.is_some() && resolved == self.ghostty_osc_theme {
            self.ghostty_osc_theme = None;
        } else {
            self.restore_ghostty_preview();
        }
    }

    pub(super) fn cancel_settings_overlay(&mut self) {
        let Some(ClientShellOverlay::Settings(settings)) = self.overlay.take() else {
            return;
        };
        self.config.theme_name = settings.original_theme_name;
        self.config.palette = settings.original_palette;
        self.restore_ghostty_preview();
    }

    fn save_settings_edit(
        &mut self,
        edit: crate::config::ConfigEdit<'_>,
        outcome: &mut ClientShellInput,
    ) -> bool {
        if let Err(error) = crate::config::write_edit(edit) {
            self.set_endpoint_error(error);
            outcome.repaint = true;
            return false;
        }
        self.reload_client_config();
        self.push_endpoint_method_with_kind(
            crate::api::schema::Method::ServerReloadConfig(
                crate::api::schema::EmptyParams::default(),
            ),
            PendingEndpointKind::ReloadConfig,
            outcome,
        );
        outcome.repaint = true;
        true
    }

    pub(super) fn apply_settings_choice(&mut self, outcome: &mut ClientShellInput) {
        let Some(ClientShellOverlay::Settings(settings)) = self.overlay.as_ref() else {
            return;
        };
        let section = settings.section;
        let selected = settings.selected;
        match section {
            ClientSettingsSection::Theme => {
                let Some(name) = settings
                    .selected_theme_index()
                    .and_then(|index| settings.theme_choices.get(index))
                    .map(|choice| choice.value.clone())
                else {
                    return;
                };
                if self.save_settings_edit(crate::config::ConfigEdit::Theme(&name), outcome) {
                    // The previewed Ghostty theme is the applied end state.
                    self.settle_ghostty_preview();
                    self.overlay = None;
                }
            }
            ClientSettingsSection::Indicators => {
                let style = if selected == 0 {
                    crate::config::StatusIndicatorStyle::Dots
                } else {
                    crate::config::StatusIndicatorStyle::Symbols
                };
                self.save_settings_edit(
                    crate::config::ConfigEdit::StatusIndicators(style),
                    outcome,
                );
            }
            ClientSettingsSection::Sound => {
                self.save_settings_edit(crate::config::ConfigEdit::Sound(selected == 0), outcome);
            }
            ClientSettingsSection::Toast => {
                let delivery = match selected {
                    0 => crate::config::ToastDelivery::Off,
                    1 => crate::config::ToastDelivery::Herdr,
                    2 => crate::config::ToastDelivery::Terminal,
                    _ => crate::config::ToastDelivery::System,
                };
                self.save_settings_edit(
                    crate::config::ConfigEdit::ToastDelivery(delivery),
                    outcome,
                );
            }
            ClientSettingsSection::Integrations => self.install_recommended_integrations(outcome),
        }
        if section != ClientSettingsSection::Theme {
            // A non-theme apply leaves the theme unchanged — undo any
            // Ghostty preview writes from browsing the theme list.
            self.restore_ghostty_preview();
        }
    }

    fn queue_integration_list(&mut self, outcome: &mut ClientShellInput, clear_messages: bool) {
        if let Some(ClientShellOverlay::Settings(settings)) = self.overlay.as_mut() {
            settings.loading_integrations = true;
            if clear_messages {
                settings.integration_messages.clear();
            }
        }
        if !self.push_endpoint_method_with_kind(
            crate::api::schema::Method::IntegrationList(crate::api::schema::EmptyParams::default()),
            PendingEndpointKind::IntegrationList,
            outcome,
        ) {
            if let Some(ClientShellOverlay::Settings(settings)) = self.overlay.as_mut() {
                settings.loading_integrations = false;
            }
        }
    }

    fn install_recommended_integrations(&mut self, outcome: &mut ClientShellInput) {
        if self.pending_integration_installs > 0 {
            return;
        }
        let targets = match self.overlay.as_ref() {
            Some(ClientShellOverlay::Settings(settings)) => settings
                .integrations
                .iter()
                .filter(|integration| integration_needs_install(integration))
                .map(|integration| integration.target)
                .collect::<Vec<_>>(),
            _ => return,
        };
        if targets.is_empty() {
            return;
        }
        if let Some(ClientShellOverlay::Settings(settings)) = self.overlay.as_mut() {
            settings.installing_integrations = true;
            settings.integration_messages.clear();
        }
        self.pending_integration_installs = 0;
        for target in targets {
            if self.push_endpoint_method_with_kind(
                crate::api::schema::Method::IntegrationInstall(
                    crate::api::schema::IntegrationInstallParams { target },
                ),
                PendingEndpointKind::IntegrationInstall,
                outcome,
            ) {
                self.pending_integration_installs += 1;
            }
        }
        if self.pending_integration_installs == 0 {
            if let Some(ClientShellOverlay::Settings(settings)) = self.overlay.as_mut() {
                settings.installing_integrations = false;
            }
        }
        outcome.repaint = true;
    }

    pub(super) fn handle_settings_endpoint_result(
        &mut self,
        kind: PendingEndpointKind,
        result: Result<crate::api::schema::ResponseResult, ClientShellEndpointError>,
    ) -> (bool, Vec<ClientShellAction>) {
        match kind {
            PendingEndpointKind::IntegrationList => {
                if let Some(ClientShellOverlay::Settings(settings)) = self.overlay.as_mut() {
                    settings.loading_integrations = false;
                    match result {
                        Ok(crate::api::schema::ResponseResult::IntegrationList {
                            integrations,
                        }) => {
                            settings.integrations = integrations;
                            settings.selected = settings
                                .selected
                                .min(settings.integrations.len().saturating_sub(1));
                        }
                        Ok(_) => {
                            self.set_endpoint_error(
                                "endpoint returned an unexpected integration list result",
                            );
                        }
                        Err(_) => {}
                    }
                }
                (true, Vec::new())
            }
            PendingEndpointKind::IntegrationInstall => {
                let cancelled = result
                    .as_ref()
                    .is_err_and(|error| error.code.as_deref() == Some("endpoint_cancelled"));
                self.pending_integration_installs =
                    self.pending_integration_installs.saturating_sub(1);
                if let Some(ClientShellOverlay::Settings(settings)) = self.overlay.as_mut() {
                    match result {
                        Ok(crate::api::schema::ResponseResult::IntegrationInstall {
                            details,
                            ..
                        }) => settings.integration_messages.extend(details.messages),
                        Ok(_) => settings
                            .integration_messages
                            .push("endpoint returned an unexpected integration result".into()),
                        Err(error) => settings.integration_messages.push(error.message),
                    }
                    settings.installing_integrations = self.pending_integration_installs > 0;
                }
                let actions = if !cancelled
                    && self.pending_integration_installs == 0
                    && matches!(self.overlay, Some(ClientShellOverlay::Settings(_)))
                {
                    let mut deferred = ClientShellInput::default();
                    self.queue_integration_list(&mut deferred, false);
                    deferred.actions
                } else {
                    Vec::new()
                };
                (true, actions)
            }
            _ => (false, Vec::new()),
        }
    }

    pub(super) fn route_settings_key(
        &mut self,
        key: &crate::input::TerminalKey,
        outcome: &mut ClientShellInput,
    ) -> bool {
        if !matches!(self.overlay, Some(ClientShellOverlay::Settings(_))) {
            return false;
        }
        let (code, modifiers) = crate::config::normalize_key_combo((key.code, key.modifiers));
        if code == KeyCode::Esc {
            // First Esc leaves the theme filter; second Esc closes.
            if matches!(
                self.overlay,
                Some(ClientShellOverlay::Settings(ClientSettingsOverlay {
                    search_focused: true,
                    ..
                }))
            ) {
                if let Some(ClientShellOverlay::Settings(settings)) = self.overlay.as_mut() {
                    settings.search_focused = false;
                }
                outcome.repaint = true;
                return true;
            }
            if !matches!(
                self.overlay,
                Some(ClientShellOverlay::Settings(ClientSettingsOverlay {
                    installing_integrations: true,
                    ..
                }))
            ) {
                self.cancel_settings_overlay();
                outcome.repaint = true;
            }
            return true;
        }
        // Filter input for the theme list (same pattern as the worktree
        // open picker): while search is focused, printable keys edit the
        // query and selection follows the first match.
        let theme_search_focused = matches!(
            self.overlay,
            Some(ClientShellOverlay::Settings(ClientSettingsOverlay {
                section: ClientSettingsSection::Theme,
                search_focused: true,
                ..
            }))
        );
        if theme_search_focused {
            let mut handled = false;
            let mut preview = false;
            if let Some(ClientShellOverlay::Settings(settings)) = self.overlay.as_mut() {
                if let Some(content_changed) = settings.query.handle_key(key) {
                    if content_changed {
                        if let Some(first) = settings.filtered_theme_indices().first().copied() {
                            settings.selected = first;
                        }
                        preview = true;
                    }
                    handled = true;
                    outcome.repaint = true;
                }
            }
            if preview {
                self.preview_selected_theme();
            }
            if handled {
                return true;
            }
        }
        let theme_section = matches!(
            self.overlay,
            Some(ClientShellOverlay::Settings(ClientSettingsOverlay {
                section: ClientSettingsSection::Theme,
                ..
            }))
        );
        if theme_section && code == KeyCode::Char('/') && modifiers.is_empty() {
            if let Some(ClientShellOverlay::Settings(settings)) = self.overlay.as_mut() {
                settings.search_focused = true;
            }
            outcome.repaint = true;
            return true;
        }
        if matches!(code, KeyCode::Tab | KeyCode::Right | KeyCode::Char('l'))
            && modifiers.is_empty()
        {
            self.move_settings_section(1, outcome);
            return true;
        }
        if matches!(code, KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h'))
            && modifiers.difference(KeyModifiers::SHIFT).is_empty()
        {
            self.move_settings_section(-1, outcome);
            return true;
        }
        if matches!(code, KeyCode::Up | KeyCode::Char('k')) && modifiers.is_empty() {
            self.move_settings_selection(-1);
            outcome.repaint = true;
            return true;
        }
        if matches!(code, KeyCode::Down | KeyCode::Char('j')) && modifiers.is_empty() {
            self.move_settings_selection(1);
            outcome.repaint = true;
            return true;
        }
        if matches!(code, KeyCode::Enter | KeyCode::Char(' ')) && modifiers.is_empty() {
            self.apply_settings_choice(outcome);
            return true;
        }
        // Typeahead: any other printable character in the theme list starts
        // a filter query directly (j/k/h/l keep their navigation roles).
        if theme_section
            && matches!(code, KeyCode::Char(_))
            && modifiers.difference(KeyModifiers::SHIFT).is_empty()
        {
            let mut content_changed = false;
            if let Some(ClientShellOverlay::Settings(settings)) = self.overlay.as_mut() {
                settings.search_focused = true;
                if let Some(changed) = settings.query.handle_key(key) {
                    content_changed = changed;
                    if changed {
                        if let Some(first) = settings.filtered_theme_indices().first().copied() {
                            settings.selected = first;
                        }
                    }
                }
            }
            if content_changed {
                self.preview_selected_theme();
            }
            outcome.repaint = true;
            return true;
        }
        true
    }
}
