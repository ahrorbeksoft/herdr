# Fork changes

Customizations on top of upstream Herdr. This file is embedded into the binary and shown in the "What's new" dialog.

### Added
- Sync Herdr's theme with Ghostty. `theme.name = "ghostty"` follows Ghostty's configured theme, including `dark:X,light:Y` splits, and `theme.name = "ghostty:<name>"` pins a specific one. Discovered Ghostty themes also appear in the Settings theme list with a live terminal preview; applying one writes Ghostty's `theme =` line and reloads it, so both apps stay in lockstep. `HERDR_GHOSTTY_THEMES_DIR` and `GHOSTTY_CONFIG` override theme and config discovery.
- On macOS, `worktrees.backend = "cow"` creates worktree checkouts as APFS copy-on-write pastures through the `cow` CLI instead of linked Git worktrees. Pastures land under `~/.cow/pastures/<repo>/<branch-slug>` and are removed with `cow remove`; `HERDR_COW_BIN` and `HERDR_COW_DIR` override the executable and the pastures root.
- A "dev servers" item in the global menu lists TCP listeners owned by pane process trees across all connected machines, grouped by endpoint. Terminating a row sends SIGTERM first and escalates to SIGKILL when the process survives. The new `server.dev_servers` and `process.kill` socket methods expose the same data and scoped termination to scripts; `process.kill` only accepts pids inside a pane's process tree.
- The "What's new" dialog shows this fork's changelog instead of upstream release notes.

### Changed
- Self-update is disabled. `herdr update` refuses with a message, and background version and agent-manifest checks default to off (`[update] version_check`, `manifest_check`). Update by rebuilding this fork.
- Agent-facing docs (`herdr --skill` and `distribution/agent-guide.md`) describe this fork's behavior: cow pastures as complete local environments, the dev servers list with scoped `process.kill`, and disabled self-update.
