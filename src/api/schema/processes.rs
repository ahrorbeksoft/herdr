use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ProcessKillParams {
    pub pid: u32,
    /// Send SIGKILL instead of SIGTERM on Unix; always a hard kill on Windows.
    #[serde(default)]
    pub force: bool,
}

/// A single TCP listen socket owned by a dev-server process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct DevServerListener {
    pub address: String,
    pub port: u16,
}

/// A process listening on TCP ports inside a pane's process tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct DevServerEntry {
    pub pid: u32,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    pub listeners: Vec<DevServerListener>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uptime_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
}
