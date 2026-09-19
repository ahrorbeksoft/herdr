use super::*;

/// One display line inside the overlay body. Section status lines are never
/// selectable; entry rows carry their flat index into `filtered_rows()`.
enum DevServerRow<'a> {
    Header(&'a ClientDevServerSection),
    Status(String),
    Entry(usize, &'a ClientDevServerEntry),
}

fn dev_server_body_rows<'a>(
    overlay: &'a ClientDevServersOverlay,
    p: &Palette,
) -> Vec<DevServerRow<'a>> {
    let query = overlay.query.as_str();
    let mut rows = Vec::new();
    let mut flat = 0usize;
    for section in &overlay.sections {
        rows.push(DevServerRow::Header(section));
        match &section.state {
            ClientDevServerSectionState::Loading => {
                rows.push(DevServerRow::Status("loading…".to_owned()));
            }
            ClientDevServerSectionState::Offline => {
                let (_, state, _) = endpoint_status_presentation(section.status, p);
                rows.push(DevServerRow::Status(format!("endpoint {state}")));
            }
            ClientDevServerSectionState::Unsupported => {
                rows.push(DevServerRow::Status(
                    "server does not support dev servers".to_owned(),
                ));
            }
            ClientDevServerSectionState::Error(message) => {
                rows.push(DevServerRow::Status(message.clone()));
            }
            ClientDevServerSectionState::Ready(entries) => {
                let mut shown = 0usize;
                for entry in entries {
                    if !entry.matches_query(query) {
                        continue;
                    }
                    rows.push(DevServerRow::Entry(flat, entry));
                    flat += 1;
                    shown += 1;
                }
                if shown == 0 {
                    rows.push(DevServerRow::Status(if entries.is_empty() {
                        "no dev servers".to_owned()
                    } else {
                        "no matches".to_owned()
                    }));
                }
            }
        }
    }
    rows
}

fn dev_server_uptime(uptime_seconds: Option<u64>) -> String {
    let Some(seconds) = uptime_seconds else {
        return String::new();
    };
    if seconds >= 3600 {
        format!("{}h {}m", seconds / 3600, seconds % 3600 / 60)
    } else if seconds >= 60 {
        format!("{}m {}s", seconds / 60, seconds % 60)
    } else {
        format!("{seconds}s")
    }
}

pub(super) fn render_dev_servers_overlay(
    b: &mut Buffer,
    overlay: &ClientDevServersOverlay,
    p: &Palette,
) -> Option<OverlayRender> {
    let body_rows = dev_server_body_rows(overlay, p);
    let content_lines = body_rows.len() as u16;
    // Chrome: title, search, separator above the body; status and hint below.
    let popup_height = content_lines.saturating_add(7).clamp(12, 30);
    let q = popup(b.area, 100, popup_height)?;
    let i = panel(b, q, p.accent, p.panel_bg)?;
    let base = Style::default()
        .bg(p.panel_bg)
        .remove_modifier(Modifier::DIM);
    let muted = base.fg(p.overlay0);
    put_text(
        b,
        i.x,
        i.y,
        i.width,
        " dev servers",
        base.fg(p.text).add_modifier(Modifier::BOLD),
    );
    let close = Rect::new(i.right().saturating_sub(13), i.y, 13, 1);
    button(
        b,
        close,
        if overlay.search_focused {
            " esc back "
        } else {
            " esc close "
        },
        Style::default()
            .fg(contrast(p))
            .bg(p.accent)
            .add_modifier(Modifier::BOLD),
    );

    let search = Rect::new(i.x, i.y + 1, i.width, 1);
    let total_servers: usize = overlay
        .sections
        .iter()
        .map(|section| match &section.state {
            ClientDevServerSectionState::Ready(entries) => entries.len(),
            _ => 0,
        })
        .sum();
    let filtered_servers = overlay.filtered_rows().len();
    let count = if filtered_servers == total_servers {
        format!("{total_servers} servers")
    } else {
        format!("{filtered_servers}/{total_servers} servers")
    };
    put_text(
        b,
        search.x,
        search.y,
        search.width,
        &if overlay.search_focused || !overlay.query.as_str().is_empty() {
            format!(" / {}", overlay.query.as_str())
        } else {
            " / filter by port, process, or workspace".to_owned()
        },
        base.fg(if overlay.search_focused {
            p.text
        } else {
            p.overlay0
        }),
    );
    let cursor = if overlay.search_focused {
        text_editor::render(
            b,
            Rect::new(
                search.x + 3,
                search.y,
                search.width.saturating_sub(4 + display_width(&count)),
                1,
            ),
            &overlay.query,
            base.fg(p.text),
        )
    } else {
        None
    };
    put_right_text(b, search, search.y, &count, muted);
    put_text(
        b,
        i.x,
        i.y + 2,
        i.width,
        &"─".repeat(i.width as usize),
        base.fg(p.surface1),
    );

    let body = Rect::new(i.x, i.y + 3, i.width, i.height.saturating_sub(5));
    // Scroll is derived from the selected row — render never mutates state.
    let selected_flat = overlay.selected_flat_index();
    let selected_visual = selected_flat.and_then(|flat| {
        body_rows
            .iter()
            .position(|row| matches!(row, DevServerRow::Entry(index, _) if *index == flat))
    });
    let visible = usize::from(body.height.max(1));
    let scroll = selected_visual
        .map(|position| {
            position
                .saturating_sub(visible.saturating_sub(1))
                .min(body_rows.len().saturating_sub(visible))
        })
        .unwrap_or(0);
    let mut row_hits = Vec::new();
    for (offset, row) in body_rows
        .iter()
        .enumerate()
        .skip(scroll)
        .take(visible)
        .map(|(position, row)| (position - scroll, row))
    {
        let rect = Rect::new(body.x, body.y + offset as u16, body.width, 1);
        match row {
            DevServerRow::Header(section) => {
                put_text(
                    b,
                    rect.x,
                    rect.y,
                    rect.width,
                    &format!(" {}", section.label),
                    base.fg(p.accent).add_modifier(Modifier::BOLD),
                );
                let (glyph, state, color) = endpoint_status_presentation(section.status, p);
                let signal = if section.status == ClientEndpointStatus::Online {
                    glyph.to_owned()
                } else {
                    format!("{glyph} {state}")
                };
                put_right_text(b, rect, rect.y, &signal, base.fg(color));
            }
            DevServerRow::Status(message) => {
                put_text(
                    b,
                    rect.x,
                    rect.y,
                    rect.width,
                    &format!("   {message}"),
                    muted,
                );
            }
            DevServerRow::Entry(flat, entry) => {
                let selected = Some(*flat) == selected_flat;
                let terminating = overlay
                    .terminating
                    .get(&(entry.endpoint_id.clone(), entry.pid));
                let style = if selected {
                    base.fg(contrast(p))
                        .bg(p.accent)
                        .add_modifier(Modifier::BOLD)
                } else {
                    base.fg(p.text)
                };
                b.set_style(rect, style);
                let listeners = entry
                    .listeners
                    .iter()
                    .map(|listener| format!("{}:{}", listener.address, listener.port))
                    .collect::<Vec<_>>()
                    .join(", ");
                let context = [entry.workspace_name.as_deref(), entry.pane_title.as_deref()]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(" · ");
                let uptime = dev_server_uptime(entry.uptime_seconds);
                let mut label = format!("   {listeners} · {} (pid {})", entry.name, entry.pid);
                if !context.is_empty() {
                    label.push_str(&format!(" · {context}"));
                }
                if !uptime.is_empty() {
                    label.push_str(&format!(" · up {uptime}"));
                }
                put_text(b, rect.x, rect.y, rect.width, &label, style);
                row_hits.push((rect, *flat));
                let badge = match terminating {
                    Some(ClientDevServerTermination::Sent) => Some("terminating…"),
                    Some(ClientDevServerTermination::StillRunning) => {
                        Some("still running — ↵ force")
                    }
                    None => None,
                };
                if let Some(badge) = badge {
                    let badge_style = if selected {
                        style
                    } else {
                        base.fg(p.yellow).add_modifier(Modifier::BOLD)
                    };
                    put_right_text(b, rect, rect.y, badge, badge_style);
                }
            }
        }
    }
    if body_rows.is_empty() {
        put_text(
            b,
            body.x,
            body.y,
            body.width,
            " no connected endpoints",
            muted,
        );
    }

    if let Some(error) = overlay.error.as_deref() {
        put_text(
            b,
            i.x,
            i.bottom() - 2,
            i.width,
            &format!(" {error}"),
            base.fg(p.red),
        );
    }
    put_text(
        b,
        i.x,
        i.bottom() - 1,
        i.width,
        if overlay.search_focused {
            " type to filter · select ↑↓ · apply enter · back esc"
        } else {
            " select j/k · terminate enter · filter / · refresh r · close esc"
        },
        muted,
    );

    Some(OverlayRender {
        area: q,
        cancel: close,
        dev_server_popup: q,
        dev_server_search: search,
        dev_server_rows: row_hits,
        cursor,
        ..OverlayRender::default()
    })
}
