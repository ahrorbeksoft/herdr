//! Dev-server discovery and scoped process termination for the JSON API.
//!
//! `server.dev_servers` reports TCP listeners owned by descendants of pane
//! shell processes; `process.kill` terminates a pid only while it sits inside
//! one of those pane process trees. The tree/matching logic below is pure so
//! it is testable without PTYs or live processes.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::api::schema::{DevServerEntry, DevServerListener, ProcessKillParams, ResponseResult};
use crate::app::api::responses::{encode_error, encode_success};
use crate::app::App;
use crate::platform::{ListeningSocket, ProcessEntry};

/// A pane's shell process plus the public metadata attached to that pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PaneShellRoot {
    pub(crate) shell_pid: u32,
    pub(crate) pane_id: Option<String>,
    pub(crate) workspace_id: Option<String>,
    pub(crate) workspace_name: Option<String>,
    pub(crate) pane_title: Option<String>,
}

/// parent pid -> child pids for the current process table.
fn children_by_parent(processes: &[ProcessEntry]) -> HashMap<u32, Vec<u32>> {
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    for process in processes {
        children
            .entry(process.parent_pid)
            .or_default()
            .push(process.pid);
    }
    children
}

/// Every pid inside each pane shell's current process tree, roots included.
///
/// Root pids are included so a pane whose shell `exec`'d a server still
/// reports — and can kill — that process. The set is recomputed from the
/// caller-provided table, so dead roots or reused pids simply do not match.
pub(crate) fn pane_scoped_pids(processes: &[ProcessEntry], shell_pids: &[u32]) -> HashSet<u32> {
    let children = children_by_parent(processes);
    let mut scoped = HashSet::new();
    let mut queue = VecDeque::new();
    for &root in shell_pids {
        if root != 0 && scoped.insert(root) {
            queue.push_back(root);
        }
    }
    while let Some(pid) = queue.pop_front() {
        let Some(child_pids) = children.get(&pid) else {
            continue;
        };
        for &child in child_pids {
            if child != 0 && scoped.insert(child) {
                queue.push_back(child);
            }
        }
    }
    scoped
}

/// Group TCP listeners owned by pane-descendant processes into entries.
///
/// `cwd_for_pid` resolves each matched process's working directory; callers
/// pass the platform lookup so the pure matching stays I/O-free.
pub(crate) fn dev_server_entries(
    processes: &[ProcessEntry],
    listeners: &[ListeningSocket],
    roots: &[PaneShellRoot],
    cwd_for_pid: impl Fn(u32) -> Option<String>,
) -> Vec<DevServerEntry> {
    let children = children_by_parent(processes);
    let by_pid: HashMap<u32, &ProcessEntry> = processes
        .iter()
        .map(|process| (process.pid, process))
        .collect();

    // Walk every pane tree once; first root wins when trees overlap.
    let mut owner_root: HashMap<u32, usize> = HashMap::new();
    for (index, root) in roots.iter().enumerate() {
        if root.shell_pid == 0 {
            continue;
        }
        let mut queue = VecDeque::from([root.shell_pid]);
        while let Some(pid) = queue.pop_front() {
            if owner_root.insert(pid, index).is_some() {
                continue;
            }
            if let Some(child_pids) = children.get(&pid) {
                queue.extend(child_pids.iter().copied().filter(|pid| *pid != 0));
            }
        }
    }

    let mut listeners_by_pid: HashMap<u32, Vec<DevServerListener>> = HashMap::new();
    for listener in listeners {
        if !owner_root.contains_key(&listener.pid) {
            continue;
        }
        // Herdr's own processes (the stable server or a `herdr-dev` build run
        // inside a pane) are infrastructure, not dev servers.
        if by_pid
            .get(&listener.pid)
            .is_some_and(|process| matches!(process.name.as_str(), "herdr" | "herdr-dev"))
        {
            continue;
        }
        listeners_by_pid
            .entry(listener.pid)
            .or_default()
            .push(DevServerListener {
                address: listener.address.clone(),
                port: listener.port,
            });
    }

    let mut servers: Vec<DevServerEntry> = listeners_by_pid
        .into_iter()
        .map(|(pid, mut listeners)| {
            listeners.sort_by(|a, b| a.port.cmp(&b.port).then_with(|| a.address.cmp(&b.address)));
            listeners.dedup();
            let root = &roots[owner_root[&pid]];
            let process = by_pid.get(&pid);
            DevServerEntry {
                pid,
                name: process.map(|entry| entry.name.clone()).unwrap_or_default(),
                command: process.and_then(|entry| entry.command.clone()),
                listeners,
                uptime_seconds: process.and_then(|entry| entry.uptime_seconds),
                pane_id: root.pane_id.clone(),
                workspace_id: root.workspace_id.clone(),
                workspace_name: root.workspace_name.clone(),
                pane_title: root.pane_title.clone(),
                cwd: cwd_for_pid(pid),
            }
        })
        .collect();
    servers.sort_by_key(|server| server.pid);
    servers
}

impl App {
    /// `server.dev_servers` — TCP listeners owned by pane process trees.
    pub(super) fn handle_server_dev_servers(&mut self, id: String) -> String {
        let roots = self.pane_shell_roots();
        let processes = crate::platform::process_table();
        let listeners = crate::platform::listening_tcp_sockets();
        let servers = dev_server_entries(&processes, &listeners, &roots, |pid| {
            crate::platform::process_cwd(pid).map(|cwd| cwd.display().to_string())
        });
        encode_success(id, ResponseResult::DevServerList { servers })
    }

    /// `process.kill` — terminate a pid that currently sits inside a pane's
    /// process tree. The scope is recomputed at request time; arbitrary pids
    /// outside pane trees are refused before any signal is sent.
    pub(super) fn handle_process_kill(&mut self, id: String, params: ProcessKillParams) -> String {
        let shell_pids: Vec<u32> = self
            .pane_shell_roots()
            .iter()
            .map(|root| root.shell_pid)
            .collect();
        let scoped = pane_scoped_pids(&crate::platform::process_table(), &shell_pids);
        if !scoped.contains(&params.pid) {
            return encode_error(
                id,
                "process_out_of_scope",
                format!("pid {} is not part of a pane process tree", params.pid),
            );
        }
        match crate::platform::terminate_process(params.pid, params.force) {
            Ok(()) => encode_success(id, ResponseResult::Ok {}),
            Err(err) => encode_error(id, "terminate_failed", err.to_string()),
        }
    }

    /// Shell pid plus public metadata for every pane with a live runtime.
    fn pane_shell_roots(&self) -> Vec<PaneShellRoot> {
        let mut roots = Vec::new();
        for (ws_idx, workspace) in self.state.workspaces.iter().enumerate() {
            let workspace_id = self.public_workspace_id(ws_idx);
            let workspace_name =
                workspace.display_name_from(&self.state.terminals, &self.terminal_runtimes);
            for tab in &workspace.tabs {
                for (pane_id, pane) in &tab.panes {
                    let Some(shell_pid) = self
                        .terminal_runtimes
                        .get(&pane.attached_terminal_id)
                        .and_then(|runtime| runtime.child_pid())
                    else {
                        continue;
                    };
                    let pane_title = self
                        .state
                        .terminals
                        .get(&pane.attached_terminal_id)
                        .and_then(|terminal| {
                            terminal
                                .manual_label
                                .clone()
                                .or_else(|| terminal.effective_presentation().title)
                        });
                    roots.push(PaneShellRoot {
                        shell_pid,
                        pane_id: self.public_pane_id(ws_idx, *pane_id),
                        workspace_id: Some(workspace_id.clone()),
                        workspace_name: Some(workspace_name.clone()),
                        pane_title,
                    });
                }
            }
        }
        roots
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn process(pid: u32, parent_pid: u32, name: &str) -> ProcessEntry {
        ProcessEntry {
            pid,
            parent_pid,
            name: name.into(),
            command: Some(format!("{name} --flag")),
            uptime_seconds: Some(42),
        }
    }

    fn listener(pid: u32, address: &str, port: u16) -> ListeningSocket {
        ListeningSocket {
            pid,
            address: address.into(),
            port,
        }
    }

    fn root(shell_pid: u32, pane: &str, workspace: &str) -> PaneShellRoot {
        PaneShellRoot {
            shell_pid,
            pane_id: Some(pane.into()),
            workspace_id: Some(workspace.into()),
            workspace_name: Some(format!("ws-{workspace}")),
            pane_title: Some(format!("title-{pane}")),
        }
    }

    #[test]
    fn dev_servers_match_nested_descendants_of_pane_shells() {
        // shell 100 -> npm 200 -> node 300 listens; unrelated 400 also listens.
        let processes = vec![
            process(100, 1, "zsh"),
            process(200, 100, "npm"),
            process(300, 200, "node"),
            process(400, 1, "nginx"),
        ];
        let listeners = vec![
            listener(300, "127.0.0.1", 3000),
            listener(400, "0.0.0.0", 80),
        ];
        let roots = vec![root(100, "w1:p1", "w1")];

        let servers = dev_server_entries(&processes, &listeners, &roots, |_| None);

        assert_eq!(servers.len(), 1);
        let server = &servers[0];
        assert_eq!(server.pid, 300);
        assert_eq!(server.name, "node");
        assert_eq!(server.command.as_deref(), Some("node --flag"));
        assert_eq!(server.uptime_seconds, Some(42));
        assert_eq!(server.pane_id.as_deref(), Some("w1:p1"));
        assert_eq!(server.workspace_id.as_deref(), Some("w1"));
        assert_eq!(server.workspace_name.as_deref(), Some("ws-w1"));
        assert_eq!(server.pane_title.as_deref(), Some("title-w1:p1"));
        assert_eq!(
            server.listeners,
            vec![DevServerListener {
                address: "127.0.0.1".into(),
                port: 3000,
            }]
        );
    }

    #[test]
    fn dev_servers_hide_herdr_processes() {
        // A `cargo run` herdr or herdr-dev server inside a pane is herdr
        // infrastructure, not a dev server.
        let processes = vec![
            process(100, 1, "zsh"),
            process(200, 100, "herdr"),
            process(201, 100, "herdr-dev"),
            process(202, 100, "node"),
        ];
        let listeners = vec![
            listener(200, "127.0.0.1", 4000),
            listener(201, "127.0.0.1", 4001),
            listener(202, "127.0.0.1", 4002),
        ];
        let roots = vec![root(100, "w1:p1", "w1")];

        let servers = dev_server_entries(&processes, &listeners, &roots, |_| None);
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].pid, 202);
    }

    #[test]
    fn dev_servers_exclude_unrelated_listeners() {
        let processes = vec![process(100, 1, "zsh"), process(999, 1, "sshd")];
        let listeners = vec![listener(999, "0.0.0.0", 22), listener(1, "*", 5000)];
        let roots = vec![root(100, "w1:p1", "w1")];

        assert!(dev_server_entries(&processes, &listeners, &roots, |_| None).is_empty());
    }

    #[test]
    fn dev_servers_group_multiple_ports_for_one_process() {
        let processes = vec![process(100, 1, "zsh"), process(300, 100, "node")];
        let listeners = vec![
            listener(300, "0.0.0.0", 8080),
            listener(300, "127.0.0.1", 3000),
            listener(300, "[::1]", 3000),
        ];
        let roots = vec![root(100, "w1:p1", "w1")];

        let servers = dev_server_entries(&processes, &listeners, &roots, |_| None);

        assert_eq!(servers.len(), 1);
        assert_eq!(
            servers[0].listeners,
            vec![
                DevServerListener {
                    address: "127.0.0.1".into(),
                    port: 3000,
                },
                DevServerListener {
                    address: "[::1]".into(),
                    port: 3000,
                },
                DevServerListener {
                    address: "0.0.0.0".into(),
                    port: 8080,
                },
            ]
        );
    }

    #[test]
    fn dev_servers_handle_shell_replaced_by_server_process() {
        // `exec node server.js` leaves the server at the shell's own pid.
        let processes = vec![process(100, 1, "node")];
        let listeners = vec![listener(100, "*", 8080)];
        let roots = vec![root(100, "w1:p1", "w1")];

        let servers = dev_server_entries(&processes, &listeners, &roots, |_| None);

        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].pid, 100);
    }

    #[test]
    fn dev_servers_ignore_dead_roots_and_missing_process_rows() {
        // Root 555 exited; pid reuse means a table row may not exist. The
        // listener owned by the dead root's stale pid still matches only if the
        // pid is in the live tree — here it is not a descendant of any row.
        let processes = vec![process(100, 1, "zsh"), process(200, 100, "node")];
        let listeners = vec![listener(555, "127.0.0.1", 7000)];
        let roots = vec![root(555, "w1:p1", "w1"), root(100, "w1:p2", "w1")];

        // The dead root pid itself is still scoped (a listener on the shell pid
        // is reported), so 555 shows up with no process metadata.
        let servers = dev_server_entries(&processes, &listeners, &roots, |_| None);
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].pid, 555);
        assert_eq!(servers[0].name, "");
        assert_eq!(servers[0].command, None);
        assert_eq!(servers[0].uptime_seconds, None);
        assert_eq!(servers[0].pane_id.as_deref(), Some("w1:p1"));

        // A listener on a pid never reachable from either root is excluded.
        let listeners = vec![listener(777, "127.0.0.1", 7001)];
        assert!(dev_server_entries(&processes, &listeners, &roots, |_| None).is_empty());
    }

    #[test]
    fn dev_servers_resolve_cwd_through_lookup() {
        let processes = vec![process(100, 1, "zsh"), process(200, 100, "node")];
        let listeners = vec![listener(200, "127.0.0.1", 3000)];
        let roots = vec![root(100, "w1:p1", "w1")];

        let servers = dev_server_entries(&processes, &listeners, &roots, |pid| {
            (pid == 200).then(|| "/repo/app".to_string())
        });

        assert_eq!(servers[0].cwd.as_deref(), Some("/repo/app"));
    }

    #[test]
    fn pane_scope_includes_roots_and_nested_descendants_only() {
        let processes = vec![
            process(100, 1, "zsh"),
            process(200, 100, "npm"),
            process(300, 200, "node"),
            process(101, 1, "zsh"),
            process(201, 101, "python"),
            process(999, 1, "sshd"),
        ];
        let scoped = pane_scoped_pids(&processes, &[100, 101]);

        assert!(scoped.contains(&100));
        assert!(scoped.contains(&200));
        assert!(scoped.contains(&300));
        assert!(scoped.contains(&101));
        assert!(scoped.contains(&201));
        assert!(!scoped.contains(&999));
        assert!(!scoped.contains(&1));
        assert!(!scoped.contains(&0));
    }

    #[test]
    fn pane_scope_refuses_pids_from_dead_or_reused_roots() {
        // Shell pid 555 is gone from the table; a same-numbered unrelated
        // process is only in scope if it actually appears under a live tree.
        let processes = vec![
            process(100, 1, "zsh"),
            process(555, 1, "node"), // reused pid, parented to init — not a child of 100
        ];
        let scoped = pane_scoped_pids(&processes, &[100]);

        assert!(!scoped.contains(&555));

        // The dead root pid is still in scope — its own shell is pane-owned.
        let scoped = pane_scoped_pids(&processes, &[555]);
        assert!(scoped.contains(&555));
    }
}
