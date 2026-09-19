use std::ffi::OsString;
use std::path::{Path, PathBuf};

const DEFAULT_WORKTREE_PREFIX: &str = "worktree";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorktreeCommand {
    pub program: String,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExistingWorktree {
    pub path: PathBuf,
    pub branch: Option<String>,
    pub is_bare: bool,
    pub is_detached: bool,
    pub is_prunable: bool,
}

pub(crate) fn generated_branch_slug(seed: u64) -> String {
    let adjectives = [
        "brave", "calm", "clear", "green", "lucky", "quiet", "rapid", "silver",
    ];
    let nouns = [
        "river", "cloud", "field", "forest", "harbor", "meadow", "stone", "valley",
    ];
    let adjective = adjectives[(seed as usize) % adjectives.len()];
    let noun = nouns[((seed / adjectives.len() as u64) as usize) % nouns.len()];
    let suffix = seed & 0xffff;
    format!("{DEFAULT_WORKTREE_PREFIX}/{adjective}-{noun}-{suffix:04x}")
}

pub(crate) fn branch_to_path_slug(branch: &str) -> String {
    let mut slug = String::new();
    let mut last_was_dash = false;

    for ch in branch.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            last_was_dash = false;
        } else if !last_was_dash {
            slug.push('-');
            last_was_dash = true;
        }
    }

    let trimmed = slug.trim_matches('-').to_string();
    if trimmed.is_empty() {
        DEFAULT_WORKTREE_PREFIX.to_string()
    } else {
        trimmed
    }
}

pub(crate) fn expand_tilde_path(path: &str) -> PathBuf {
    expand_tilde_path_from_env(path, cfg!(windows), |key| std::env::var_os(key))
}

fn expand_tilde_path_from_env(
    path: &str,
    is_windows: bool,
    env: impl Fn(&str) -> Option<OsString> + Copy,
) -> PathBuf {
    if path == "~" {
        return home_dir_from_env(is_windows, env).unwrap_or_else(|_| PathBuf::from(path));
    }

    let tilde_rest = path.strip_prefix("~/").or_else(|| {
        if is_windows {
            path.strip_prefix("~\\")
        } else {
            None
        }
    });
    if let Some(rest) = tilde_rest {
        return home_dir_from_env(is_windows, env)
            .map(|home| join_tilde_rest(home, rest, is_windows))
            .unwrap_or_else(|_| PathBuf::from(path));
    }

    PathBuf::from(path)
}

fn join_tilde_rest(home: PathBuf, rest: &str, is_windows: bool) -> PathBuf {
    if is_windows {
        rest.split(['/', '\\'])
            .filter(|component| !component.is_empty())
            .fold(home, |path, component| path.join(component))
    } else {
        home.join(rest)
    }
}

fn home_dir_from_env(
    is_windows: bool,
    env: impl Fn(&str) -> Option<OsString>,
) -> Result<PathBuf, ()> {
    if !is_windows {
        return env("HOME").map(PathBuf::from).ok_or(());
    }

    if let Some(path) = usable_home_path(env("USERPROFILE")) {
        return Ok(path);
    }
    if let (Some(drive), Some(path)) = (
        usable_home_component(env("HOMEDRIVE")),
        usable_home_component(env("HOMEPATH")),
    ) {
        let path = path.to_string_lossy();
        if !path.starts_with(['\\', '/']) {
            return usable_home_path(env("HOME")).ok_or(());
        }
        let combined = format!("{}{}", drive.to_string_lossy(), path);
        if let Some(path) = usable_home_path(Some(OsString::from(combined))) {
            return Ok(path);
        }
    }

    usable_home_path(env("HOME")).ok_or(())
}

fn usable_home_path(value: Option<OsString>) -> Option<PathBuf> {
    let value = value?;
    if value.is_empty() || value == "~" {
        return None;
    }
    Some(PathBuf::from(value))
}

fn usable_home_component(value: Option<OsString>) -> Option<OsString> {
    let value = value?;
    if value.is_empty() || value == "~" {
        return None;
    }
    Some(value)
}

pub(crate) fn expand_tilde_absolute_path(path: &str) -> PathBuf {
    let path = expand_tilde_path(path);
    if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(&path))
            .unwrap_or(path)
    }
}

pub(crate) fn canonical_or_original(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn repository_git_command(repo_root: &Path, trust_repository: bool) -> std::process::Command {
    let mut command = crate::noninteractive_process::command("git");
    command.args(repository_git_args(repo_root, trust_repository));
    command
}

fn repository_git_args(repo_root: &Path, trust_repository: bool) -> Vec<String> {
    let mut args = Vec::new();
    if trust_repository {
        args.push("-c".to_string());
        args.push(format!("safe.directory={}", repo_root.display()));
    }
    args.push("-C".to_string());
    args.push(repo_root.display().to_string());
    args
}

pub(crate) fn default_checkout_path(root: &Path, repo_name: &str, branch: &str) -> PathBuf {
    root.join(repo_name).join(branch_to_path_slug(branch))
}

pub(crate) fn build_worktree_remove_command(
    repo_root: &Path,
    path: &Path,
    force: bool,
    trust_repository: bool,
) -> WorktreeCommand {
    let mut args = repository_git_args(repo_root, trust_repository);
    args.extend(["worktree".to_string(), "remove".to_string()]);
    if force {
        args.push("--force".to_string());
    }
    args.push(path.display().to_string());

    WorktreeCommand {
        program: "git".to_string(),
        args,
    }
}

pub(crate) fn is_dirty_worktree_remove_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    (lower.contains("contains modified or untracked files")
        && lower.contains("use --force to delete it"))
        || lower.contains("working trees containing submodules cannot be moved or removed")
}

pub(crate) fn is_not_working_tree_remove_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("is not a working tree") || lower.contains("is not a worktree")
}

pub(crate) fn worktree_dirty_remove_message(path: &Path) -> String {
    format!(
        "fatal: '{}' contains modified or untracked files, use --force to delete it",
        path.display()
    )
}

pub(crate) fn checkout_has_dirty_files(
    path: &Path,
    trust_repository: bool,
) -> Result<bool, String> {
    let output = repository_git_command(path, trust_repository)
        .args(["status", "--porcelain", "--untracked-files=all"])
        .output()
        .map_err(|err| err.to_string())?;

    if output.status.success() {
        return Ok(!output.stdout.is_empty());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !stderr.is_empty() {
        Err(stderr)
    } else if !stdout.is_empty() {
        Err(stdout)
    } else {
        Err(format!("git status failed with status {}", output.status))
    }
}

pub(crate) fn build_worktree_add_new_branch_command(
    repo_root: &Path,
    path: &Path,
    branch: &str,
    base: &str,
    trust_repository: bool,
) -> WorktreeCommand {
    let mut args = repository_git_args(repo_root, trust_repository);
    args.extend([
        "worktree".to_string(),
        "add".to_string(),
        "-b".to_string(),
        branch.to_string(),
        path.display().to_string(),
        base.to_string(),
    ]);
    WorktreeCommand {
        program: "git".to_string(),
        args,
    }
}

pub(crate) fn build_worktree_add_existing_branch_command(
    repo_root: &Path,
    path: &Path,
    branch: &str,
    trust_repository: bool,
) -> WorktreeCommand {
    let mut args = repository_git_args(repo_root, trust_repository);
    args.extend([
        "worktree".to_string(),
        "add".to_string(),
        path.display().to_string(),
        branch.to_string(),
    ]);
    WorktreeCommand {
        program: "git".to_string(),
        args,
    }
}

fn local_branch_exists(
    repo_root: &Path,
    branch: &str,
    trust_repository: bool,
) -> Result<bool, String> {
    let output = repository_git_command(repo_root, trust_repository)
        .args(["show-ref", "--verify", "--quiet"])
        .arg(format!("refs/heads/{branch}"))
        .output()
        .map_err(|err| err.to_string())?;

    if output.status.success() {
        return Ok(true);
    }
    if output.status.code() == Some(1) {
        return Ok(false);
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !stderr.is_empty() {
        Err(stderr)
    } else if !stdout.is_empty() {
        Err(stdout)
    } else {
        Err(format!("git show-ref failed with status {}", output.status))
    }
}

pub(crate) fn run_worktree_add_command(
    repo_root: &Path,
    path: &Path,
    branch: &str,
    base: &str,
    trust_repository: bool,
) -> Result<(), String> {
    let command = if local_branch_exists(repo_root, branch, trust_repository)? {
        build_worktree_add_existing_branch_command(repo_root, path, branch, trust_repository)
    } else {
        build_worktree_add_new_branch_command(repo_root, path, branch, base, trust_repository)
    };
    run_worktree_command(&command)
}

pub(crate) fn run_worktree_command(command: &WorktreeCommand) -> Result<(), String> {
    let output = crate::noninteractive_process::command(&command.program)
        // Removal errors are classified by Git's English diagnostics.
        .env("LC_ALL", "C")
        .args(&command.args)
        .output()
        .map_err(|err| err.to_string())?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let message = if stderr.is_empty() { stdout } else { stderr };
    Err(if message.is_empty() {
        format!("{} failed with status {}", command.program, output.status)
    } else {
        message
    })
}

pub(crate) fn run_worktree_remove_command_with_recovery(
    command: &WorktreeCommand,
    repo_root: &Path,
    path: &Path,
    force: bool,
    trust_repository: bool,
) -> Result<(), String> {
    match run_worktree_command(command) {
        Ok(()) => Ok(()),
        Err(err) if force && is_not_working_tree_remove_error(&err) => {
            if worktree_list_contains_path(repo_root, path, trust_repository)? {
                return Err(err);
            }
            if path.exists() {
                if !leftover_worktree_checkout_matches_repo(repo_root, path, trust_repository) {
                    return Err(err);
                }
                std::fs::remove_dir_all(path).map_err(|remove_err| {
                    format!(
                        "{err}; failed to remove leftover checkout {}: {remove_err}",
                        path.display()
                    )
                })?;
            }
            Ok(())
        }
        Err(err) => Err(err),
    }
}

fn leftover_worktree_checkout_matches_repo(
    repo_root: &Path,
    path: &Path,
    trust_repository: bool,
) -> bool {
    let git_file = path.join(".git");
    let Ok(content) = std::fs::read_to_string(&git_file) else {
        return false;
    };
    let Some(gitdir) = content.trim().strip_prefix("gitdir:") else {
        return false;
    };
    let gitdir = PathBuf::from(gitdir.trim());
    let gitdir = if gitdir.is_absolute() {
        gitdir
    } else {
        path.join(gitdir)
    };
    let Some(worktrees_dir) = git_common_worktrees_dir(repo_root, trust_repository) else {
        return false;
    };
    canonical_or_original(&gitdir).starts_with(canonical_or_original(&worktrees_dir))
}

fn git_common_worktrees_dir(repo_root: &Path, trust_repository: bool) -> Option<PathBuf> {
    let output = repository_git_command(repo_root, trust_repository)
        .args(["rev-parse", "--git-common-dir"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let common_dir = stdout.trim();
    if common_dir.is_empty() {
        None
    } else {
        let common_dir = PathBuf::from(common_dir);
        let common_dir = if common_dir.is_absolute() {
            common_dir
        } else {
            repo_root.join(common_dir)
        };
        Some(common_dir.join("worktrees"))
    }
}

pub(crate) fn parse_worktree_list_porcelain(output: &str) -> Vec<ExistingWorktree> {
    let mut entries = Vec::new();
    let mut path: Option<PathBuf> = None;
    let mut branch = None;
    let mut is_bare = false;
    let mut is_detached = false;
    let mut is_prunable = false;

    let finish = |entries: &mut Vec<ExistingWorktree>,
                  path: &mut Option<PathBuf>,
                  branch: &mut Option<String>,
                  is_bare: &mut bool,
                  is_detached: &mut bool,
                  is_prunable: &mut bool| {
        if let Some(path) = path.take() {
            entries.push(ExistingWorktree {
                path,
                branch: branch.take(),
                is_bare: *is_bare,
                is_detached: *is_detached,
                is_prunable: *is_prunable,
            });
        }
        *is_bare = false;
        *is_detached = false;
        *is_prunable = false;
    };

    for line in output.lines() {
        if line.trim().is_empty() {
            finish(
                &mut entries,
                &mut path,
                &mut branch,
                &mut is_bare,
                &mut is_detached,
                &mut is_prunable,
            );
            continue;
        }
        if let Some(value) = line.strip_prefix("worktree ") {
            path = Some(PathBuf::from(value));
        } else if let Some(value) = line.strip_prefix("branch ") {
            branch = Some(
                value
                    .strip_prefix("refs/heads/")
                    .unwrap_or(value)
                    .to_string(),
            );
        } else if line == "detached" {
            is_detached = true;
        } else if line == "bare" {
            is_bare = true;
        } else if line.starts_with("prunable") {
            is_prunable = true;
        }
    }

    finish(
        &mut entries,
        &mut path,
        &mut branch,
        &mut is_bare,
        &mut is_detached,
        &mut is_prunable,
    );
    entries
}

pub(crate) fn list_existing_worktrees(
    repo_root: &Path,
    trust_repository: bool,
) -> Result<Vec<ExistingWorktree>, String> {
    let output = repository_git_command(repo_root, trust_repository)
        .args(["worktree", "list", "--porcelain"])
        .output()
        .map_err(|err| err.to_string())?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Ok(parse_worktree_list_porcelain(&stdout));
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(if stderr.is_empty() {
        format!("git worktree list failed with status {}", output.status)
    } else {
        stderr
    })
}

fn worktree_list_contains_path(
    repo_root: &Path,
    path: &Path,
    trust_repository: bool,
) -> Result<bool, String> {
    let expected = canonical_or_original(path);
    Ok(list_existing_worktrees(repo_root, trust_repository)?
        .into_iter()
        .any(|entry| canonical_or_original(&entry.path) == expected))
}

// ---- cow backend ----

/// Selects the checkout provider behind Herdr's worktree actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorktreeBackend {
    Git,
    Cow,
}

impl WorktreeBackend {
    pub(crate) fn from_config(value: &str) -> Self {
        match value.trim() {
            "cow" => Self::Cow,
            _ => Self::Git,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CowPasture {
    pub name: String,
    pub path: PathBuf,
    pub source: PathBuf,
    pub branch: Option<String>,
    pub dirty: bool,
}

/// Default root cow uses for pastures; overridable for tests via env.
pub(crate) fn cow_pastures_directory() -> PathBuf {
    let dir = std::env::var("HERDR_COW_DIR")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "~/.cow/pastures".to_string());
    expand_tilde_absolute_path(&dir)
}

/// Resolves the `cow` executable: `HERDR_COW_BIN`, then `$PATH`, then the
/// usual install locations. The daemon's PATH is minimal, so Homebrew and
/// cargo bin dirs are probed explicitly even when absent from PATH.
fn cow_program() -> PathBuf {
    if let Some(custom) = std::env::var("HERDR_COW_BIN")
        .ok()
        .filter(|value| !value.trim().is_empty())
    {
        return PathBuf::from(custom);
    }
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).collect())
        .unwrap_or_default();
    for extra in [
        "/opt/homebrew/bin",
        "/usr/local/bin",
        "~/.cargo/bin",
        "~/.local/bin",
    ] {
        let dir = expand_tilde_absolute_path(extra);
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    dirs.iter()
        .map(|dir| dir.join("cow"))
        .find(|candidate| candidate.is_file())
        .unwrap_or_else(|| PathBuf::from("cow"))
}

fn cow_command() -> std::process::Command {
    crate::noninteractive_process::command(cow_program())
}

#[derive(serde::Deserialize)]
struct CowListEntry {
    name: String,
    path: PathBuf,
    source: PathBuf,
    #[serde(default)]
    branch: Option<String>,
    #[serde(default)]
    current_branch: Option<String>,
    #[serde(default)]
    dirty: bool,
}

/// `cow list --json`, optionally scoped to one source repository.
/// Callers that probe pasture membership (e.g. [`cow_pasture_source`])
/// treat an error as "no pastures".
pub(crate) fn list_cow_pastures(source: Option<&Path>) -> Result<Vec<CowPasture>, String> {
    let mut command = cow_command();
    command.args(["list", "--json"]);
    if let Some(source) = source {
        command.arg("--source").arg(source);
    }
    let output = command
        .output()
        .map_err(|err| format!("failed to run {}: {err}", cow_program().display()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            format!("cow list failed with status {}", output.status)
        } else {
            stderr
        });
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let entries: Vec<CowListEntry> = serde_json::from_str(&stdout)
        .map_err(|err| format!("cow list JSON parse failed: {err}"))?;
    Ok(entries
        .into_iter()
        .map(|entry| CowPasture {
            name: entry.name,
            path: entry.path,
            source: entry.source,
            branch: entry.current_branch.or(entry.branch),
            dirty: entry.dirty,
        })
        .collect())
}

/// The source repository of the pasture rooted at `repo_root`, if it is one.
pub(crate) fn cow_pasture_source(repo_root: &Path) -> Option<PathBuf> {
    let expected = canonical_or_original(repo_root);
    list_cow_pastures(None)
        .ok()?
        .into_iter()
        .find_map(|pasture| {
            (canonical_or_original(&pasture.path) == expected)
                .then(|| canonical_or_original(&pasture.source))
        })
}

/// `git worktree list` or `cow list`, presented as worktree entries.
/// The cow listing prepends the source checkout itself, mirroring how
/// `git worktree list` includes the main worktree.
pub(crate) fn list_worktrees(
    backend: WorktreeBackend,
    repo_root: &Path,
    trust_repository: bool,
) -> Result<Vec<ExistingWorktree>, String> {
    match backend {
        WorktreeBackend::Git => list_existing_worktrees(repo_root, trust_repository),
        WorktreeBackend::Cow => {
            let mut entries = Vec::new();
            let branch = crate::workspace::git_branch(repo_root);
            entries.push(ExistingWorktree {
                path: repo_root.to_path_buf(),
                is_detached: branch.is_none(),
                branch,
                is_bare: false,
                is_prunable: false,
            });
            for pasture in list_cow_pastures(Some(repo_root))? {
                entries.push(ExistingWorktree {
                    path: pasture.path,
                    is_detached: pasture.branch.is_none(),
                    branch: pasture.branch,
                    is_bare: false,
                    is_prunable: false,
                });
            }
            Ok(entries)
        }
    }
}

/// Creates a checkout: `git worktree add` or `cow create`.
/// Returns the checkout path, which cow decides itself (`--print-path`).
pub(crate) fn run_worktree_add(
    backend: WorktreeBackend,
    repo_root: &Path,
    path: &Path,
    branch: &str,
    base: &str,
    trust_repository: bool,
) -> Result<PathBuf, String> {
    match backend {
        WorktreeBackend::Git => {
            run_worktree_add_command(repo_root, path, branch, base, trust_repository)?;
            Ok(path.to_path_buf())
        }
        WorktreeBackend::Cow => run_cow_create(repo_root, branch),
    }
}

fn run_cow_create(repo_root: &Path, branch: &str) -> Result<PathBuf, String> {
    let name = branch_to_path_slug(branch);
    let output = cow_command()
        .env("LC_ALL", "C")
        .arg("create")
        .arg("--source")
        .arg(repo_root)
        .arg("--branch")
        .arg(branch)
        .arg("--print-path")
        .arg(&name)
        .output()
        .map_err(|err| format!("failed to run {}: {err}", cow_program().display()))?;

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !output.status.success() {
        return Err(if stderr.is_empty() {
            format!("cow create failed with status {}", output.status)
        } else {
            stderr
        });
    }
    if stdout.is_empty() {
        return Err("cow create did not report a pasture path".to_string());
    }
    Ok(PathBuf::from(stdout))
}

/// `cow remove` needs the pasture name, which is resolved from the
/// checkout path via `cow list` (falling back to the <repo>/<name> tail).
pub(crate) fn build_cow_remove_command(path: &Path, force: bool) -> WorktreeCommand {
    let expected = canonical_or_original(path);
    let name = list_cow_pastures(None)
        .ok()
        .and_then(|pastures| {
            pastures
                .into_iter()
                .find(|pasture| canonical_or_original(&pasture.path) == expected)
                .map(|pasture| pasture.name)
        })
        .unwrap_or_else(|| {
            let tail: Vec<String> = path
                .components()
                .rev()
                .take(2)
                .filter_map(|part| part.as_os_str().to_str().map(str::to_string))
                .collect();
            if tail.len() == 2 {
                format!("{}/{}", tail[1], tail[0])
            } else {
                path.display().to_string()
            }
        });
    let mut args = vec!["remove".to_string(), name, "-y".to_string()];
    if force {
        args.push("--force".to_string());
    }
    WorktreeCommand {
        program: cow_program().to_string_lossy().into_owned(),
        args,
    }
}

/// Serializes tests that mutate the cow-related process env, which is
/// shared by every test thread in the crate.
#[cfg(test)]
pub(crate) static COW_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Restores mutated env vars on drop; pair with [`COW_ENV_LOCK`].
#[cfg(test)]
pub(crate) struct CowEnvGuard {
    saved: Vec<(&'static str, Option<std::ffi::OsString>)>,
}

#[cfg(test)]
impl CowEnvGuard {
    pub(crate) fn set(vars: &[(&'static str, Option<&Path>)]) -> Self {
        let saved = vars
            .iter()
            .map(|(key, _)| (*key, std::env::var_os(key)))
            .collect();
        for (key, value) in vars {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
        Self { saved }
    }
}

#[cfg(test)]
impl Drop for CowEnvGuard {
    fn drop(&mut self) {
        for (key, value) in &self.saved {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
}

/// Writes an executable fake `cow` that logs its argv to `cow-stub.log`,
/// serves canned `list --json` output, and answers
/// `create ... --print-path <name>` with a pasture path under `dir`.
#[cfg(all(test, unix))]
pub(crate) fn write_cow_stub(dir: &Path, list_json: &str) -> PathBuf {
    let script = dir.join("cow-stub.sh");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\n\
             echo \"$@\" >> '{log}'\n\
             case \"$1\" in\n\
             list)\n\
             cat <<'HERDR_COW_STUB_JSON'\n\
             {list_json}\n\
             HERDR_COW_STUB_JSON\n\
             ;;\n\
             create)\n\
             shift\n\
             while [ $# -gt 0 ]; do\n\
             case \"$1\" in\n\
             --source|--branch) shift 2 ;;\n\
             --print-path) shift ;;\n\
             *) name=\"$1\"; shift ;;\n\
             esac\n\
             done\n\
             mkdir -p '{dir}/pastures'\n\
             echo '{dir}/pastures/'\"$name\"\n\
             ;;\n\
             esac\n",
            log = dir.join("cow-stub.log").display(),
            list_json = list_json,
            dir = dir.display(),
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(&script).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&script, perms).unwrap();
    script
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_temp_path(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!("herdr-{name}-{}-{nanos}", std::process::id()))
    }

    fn run_git(repo: &Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .status()
            .unwrap();
        assert!(
            status.success(),
            "git command failed: git -C {} {}",
            repo.display(),
            args.join(" ")
        );
    }

    fn create_committed_repo(name: &str) -> PathBuf {
        let repo = unique_temp_path(name);
        std::fs::create_dir_all(&repo).unwrap();
        run_git(&repo, &["init", "--quiet"]);
        run_git(&repo, &["config", "user.email", "herdr@example.invalid"]);
        run_git(&repo, &["config", "user.name", "Herdr Test"]);
        std::fs::write(repo.join("README.md"), "test\n").unwrap();
        run_git(&repo, &["add", "README.md"]);
        run_git(&repo, &["commit", "--quiet", "-m", "initial"]);
        repo
    }

    #[test]
    fn trusted_repository_git_args_are_request_scoped() {
        assert_eq!(
            repository_git_args(Path::new("/repo/herdr"), false),
            ["-C", "/repo/herdr"]
        );
        assert_eq!(
            repository_git_args(Path::new("/repo/herdr"), true),
            ["-c", "safe.directory=/repo/herdr", "-C", "/repo/herdr",]
        );
    }

    #[test]
    fn generated_branch_slug_is_worktree_namespaced_and_stable() {
        assert_eq!(generated_branch_slug(0), "worktree/brave-river-0000");
        assert_eq!(generated_branch_slug(9), "worktree/calm-cloud-0009");
    }

    #[test]
    fn parses_git_worktree_list_porcelain() {
        let output = "\
worktree /repo/main
HEAD abc
branch refs/heads/main

worktree /repo/issue
HEAD def
branch refs/heads/worktree/issue

worktree /repo/detached
HEAD fed
detached
prunable stale

";

        assert_eq!(
            parse_worktree_list_porcelain(output),
            vec![
                ExistingWorktree {
                    path: PathBuf::from("/repo/main"),
                    branch: Some("main".into()),
                    is_bare: false,
                    is_detached: false,
                    is_prunable: false,
                },
                ExistingWorktree {
                    path: PathBuf::from("/repo/issue"),
                    branch: Some("worktree/issue".into()),
                    is_bare: false,
                    is_detached: false,
                    is_prunable: false,
                },
                ExistingWorktree {
                    path: PathBuf::from("/repo/detached"),
                    branch: None,
                    is_bare: false,
                    is_detached: true,
                    is_prunable: true,
                },
            ]
        );
    }

    #[test]
    fn branch_to_path_slug_makes_branch_safe_folder_name() {
        assert_eq!(
            branch_to_path_slug("worktree/brave-river"),
            "worktree-brave-river"
        );
        assert_eq!(
            branch_to_path_slug("issue/137 Worktree Spaces"),
            "issue-137-worktree-spaces"
        );
        assert_eq!(branch_to_path_slug("///"), "worktree");
    }

    #[test]
    fn expand_tilde_path_uses_home_when_available() {
        assert_eq!(
            expand_tilde_path_from_env("~/.herdr/worktrees", false, |key| match key {
                "HOME" => Some("/home/me".into()),
                _ => None,
            }),
            PathBuf::from("/home/me/.herdr/worktrees")
        );
        assert_eq!(
            expand_tilde_path_from_env("/tmp/worktrees", false, |_| None),
            PathBuf::from("/tmp/worktrees")
        );
    }

    #[test]
    fn home_dir_uses_windows_profile_before_literal_home() {
        assert_eq!(
            home_dir_from_env(true, |key| match key {
                "HOME" => Some("~".into()),
                "USERPROFILE" => Some(r"C:\Users\herdr".into()),
                _ => None,
            }),
            Ok(PathBuf::from(r"C:\Users\herdr"))
        );
    }

    #[test]
    fn home_dir_uses_windows_drive_and_path_when_profile_is_missing() {
        assert_eq!(
            home_dir_from_env(true, |key| match key {
                "HOMEDRIVE" => Some("C:".into()),
                "HOMEPATH" => Some(r"\Users\herdr".into()),
                _ => None,
            }),
            Ok(PathBuf::from(r"C:\Users\herdr"))
        );
    }

    #[test]
    fn home_dir_rejects_incomplete_windows_drive_and_path() {
        assert_eq!(
            home_dir_from_env(true, |key| match key {
                "HOMEDRIVE" => Some("C:".into()),
                "HOMEPATH" => Some("".into()),
                _ => None,
            }),
            Err(())
        );
        assert_eq!(
            home_dir_from_env(true, |key| match key {
                "HOMEDRIVE" => Some("C:".into()),
                "HOMEPATH" => Some("Users\\herdr".into()),
                _ => None,
            }),
            Err(())
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_tilde_expansion_keeps_windows_separator_literal() {
        assert_eq!(
            expand_tilde_path_from_env(r"~\.herdr\worktrees", false, |key| match key {
                "HOME" => Some("/home/me".into()),
                _ => None,
            }),
            PathBuf::from(r"~\.herdr\worktrees")
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_tilde_expansion_normalizes_separators() {
        fn env(key: &str) -> Option<OsString> {
            match key {
                "HOME" => Some("~".into()),
                "USERPROFILE" => Some(r"C:\Users\herdr".into()),
                _ => None,
            }
        }

        let default_path = expand_tilde_path_from_env("~/.herdr/worktrees", true, env);
        assert_eq!(
            default_path,
            PathBuf::from(r"C:\Users\herdr\.herdr\worktrees")
        );
        assert_eq!(
            default_path.display().to_string(),
            r"C:\Users\herdr\.herdr\worktrees"
        );
        assert_eq!(
            expand_tilde_path_from_env(r"~\.herdr\worktrees", true, env),
            PathBuf::from(r"C:\Users\herdr\.herdr\worktrees")
        );
    }

    #[test]
    fn default_checkout_path_appends_repo_and_branch_slug() {
        assert_eq!(
            default_checkout_path(
                Path::new("/home/me/.herdr/worktrees"),
                "herdr",
                "worktree/brave-river",
            ),
            PathBuf::from("/home/me/.herdr/worktrees/herdr/worktree-brave-river")
        );
    }

    #[test]
    fn checkout_dirty_detection_reports_clean_and_dirty_worktrees() {
        let repo = create_committed_repo("worktree-dirty-detection-repo");
        let checkout = unique_temp_path("worktree-dirty-detection-checkout");
        run_git(
            &repo,
            &[
                "worktree",
                "add",
                "--quiet",
                "-b",
                "worktree/dirty-detection",
                checkout.to_str().unwrap(),
                "HEAD",
            ],
        );

        assert_eq!(checkout_has_dirty_files(&checkout, false), Ok(false));
        std::fs::write(checkout.join("README.md"), "dirty\n").unwrap();
        assert_eq!(checkout_has_dirty_files(&checkout, false), Ok(true));

        let remove = build_worktree_remove_command(&repo, &checkout, true, false);
        run_worktree_command(&remove).unwrap();
        let _ = std::fs::remove_dir_all(repo);
    }

    #[test]
    fn worktree_remove_command_preserves_branch_by_not_deleting_it() {
        let command = build_worktree_remove_command(
            Path::new("/repo/herdr"),
            Path::new("/w/herdr/issue-137"),
            false,
            false,
        );
        assert_eq!(command.program, "git");
        assert_eq!(
            command.args,
            vec![
                "-C",
                "/repo/herdr",
                "worktree",
                "remove",
                "/w/herdr/issue-137"
            ]
        );
    }

    #[test]
    fn forced_worktree_remove_command_uses_git_force_flag() {
        let command = build_worktree_remove_command(
            Path::new("/repo/herdr"),
            Path::new("/w/herdr/issue-137"),
            true,
            false,
        );
        assert_eq!(
            command.args,
            vec![
                "-C",
                "/repo/herdr",
                "worktree",
                "remove",
                "--force",
                "/w/herdr/issue-137"
            ]
        );
    }

    #[test]
    fn dirty_remove_error_detection_matches_git_force_hint() {
        assert!(is_dirty_worktree_remove_error(
            "fatal: '/w/herdr' contains modified or untracked files, use --force to delete it"
        ));
        assert!(!is_dirty_worktree_remove_error(
            "fatal: '/w/herdr' is a missing but already registered worktree"
        ));
        assert!(!is_dirty_worktree_remove_error(
            "fatal: '/w/herdr' contains a locked worktree, use --force only if you know why"
        ));
    }

    #[test]
    fn submodule_remove_error_requires_force_confirmation() {
        assert!(is_dirty_worktree_remove_error(
            "fatal: working trees containing submodules cannot be moved or removed"
        ));
    }

    #[test]
    fn submodule_worktree_removal_requires_explicit_force() {
        let repo = create_committed_repo("submodule-remove-repo");
        let submodule = create_committed_repo("submodule-remove-source");
        let checkout = unique_temp_path("submodule-remove-checkout");
        run_git(
            &repo,
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                "--quiet",
                submodule.to_str().unwrap(),
                "sub",
            ],
        );
        run_git(&repo, &["commit", "--quiet", "-am", "add submodule"]);
        let add = build_worktree_add_new_branch_command(
            &repo,
            &checkout,
            "worktree/submodule-remove",
            "HEAD",
            false,
        );
        run_worktree_command(&add).unwrap();
        run_git(
            &checkout,
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "update",
                "--init",
                "--quiet",
            ],
        );
        assert!(!checkout_has_dirty_files(&checkout, false).unwrap());

        let remove = build_worktree_remove_command(&repo, &checkout, false, false);
        let error = run_worktree_command(&remove).unwrap_err();
        assert!(is_dirty_worktree_remove_error(&error), "{error}");
        assert!(checkout.join("sub/README.md").exists());

        std::fs::write(checkout.join("sub/README.md"), "uncommitted change\n").unwrap();
        let error = run_worktree_command(&remove).unwrap_err();
        assert!(is_dirty_worktree_remove_error(&error), "{error}");
        assert!(checkout.join("sub/README.md").exists());

        let forced = build_worktree_remove_command(&repo, &checkout, true, false);
        run_worktree_remove_command_with_recovery(&forced, &repo, &checkout, true, false).unwrap();
        assert!(!checkout.exists());
        assert!(!worktree_list_contains_path(&repo, &checkout, false).unwrap());
        assert!(repo.join("sub/README.md").exists());

        std::fs::remove_dir_all(repo).unwrap();
        std::fs::remove_dir_all(submodule).unwrap();
    }

    #[test]
    fn worktree_add_command_creates_new_branch_from_base() {
        let command = build_worktree_add_new_branch_command(
            Path::new("/repo/herdr"),
            Path::new("/w/herdr/worktree-brave-river"),
            "worktree/brave-river",
            "HEAD",
            false,
        );
        assert_eq!(command.program, "git");
        assert_eq!(
            command.args,
            vec![
                "-C",
                "/repo/herdr",
                "worktree",
                "add",
                "-b",
                "worktree/brave-river",
                "/w/herdr/worktree-brave-river",
                "HEAD"
            ]
        );
    }

    #[test]
    fn worktree_add_command_checks_out_existing_branch() {
        let command = build_worktree_add_existing_branch_command(
            Path::new("/repo/herdr"),
            Path::new("/w/herdr/worktree-brave-river"),
            "worktree/brave-river",
            false,
        );
        assert_eq!(command.program, "git");
        assert_eq!(
            command.args,
            vec![
                "-C",
                "/repo/herdr",
                "worktree",
                "add",
                "/w/herdr/worktree-brave-river",
                "worktree/brave-river"
            ]
        );
    }

    #[test]
    fn run_worktree_add_and_remove_create_and_delete_checkout() {
        let repo = create_committed_repo("worktree-run-repo");
        let checkout = unique_temp_path("worktree-run-checkout");
        let branch = "worktree/test-create-remove";

        let add = build_worktree_add_new_branch_command(&repo, &checkout, branch, "HEAD", false);
        run_worktree_command(&add).unwrap();

        assert!(checkout.join("README.md").exists());
        let branch_name = std::process::Command::new("git")
            .arg("-C")
            .arg(&checkout)
            .args(["branch", "--show-current"])
            .output()
            .unwrap();
        assert!(branch_name.status.success());
        assert_eq!(
            String::from_utf8(branch_name.stdout).unwrap().trim(),
            branch
        );

        let remove = build_worktree_remove_command(&repo, &checkout, false, false);
        run_worktree_command(&remove).unwrap();
        assert!(!checkout.exists());

        let _ = std::fs::remove_dir_all(repo);
    }

    #[test]
    fn forced_worktree_remove_recovers_leftover_unregistered_checkout() {
        let repo = create_committed_repo("worktree-recovery-repo");
        let checkout = unique_temp_path("worktree-recovery-checkout");
        let branch = "worktree/recovery";

        let add = build_worktree_add_new_branch_command(&repo, &checkout, branch, "HEAD", false);
        run_worktree_command(&add).unwrap();
        let remove = build_worktree_remove_command(&repo, &checkout, true, false);
        run_worktree_command(&remove).unwrap();
        std::fs::create_dir_all(&checkout).unwrap();
        let stale_admin_dir = git_common_worktrees_dir(&repo, false)
            .unwrap()
            .join("stale");
        std::fs::write(
            checkout.join(".git"),
            format!("gitdir: {}\n", stale_admin_dir.display()),
        )
        .unwrap();
        std::fs::write(checkout.join("leftover"), "leftover\n").unwrap();

        run_worktree_remove_command_with_recovery(&remove, &repo, &checkout, true, false).unwrap();

        assert!(!checkout.exists());
        let _ = std::fs::remove_dir_all(repo);
    }

    #[test]
    fn forced_worktree_remove_recovery_keeps_unrelated_replacement_directory() {
        let repo = create_committed_repo("worktree-recovery-unrelated-repo");
        let checkout = unique_temp_path("worktree-recovery-unrelated-checkout");
        let branch = "worktree/recovery-unrelated";

        let add = build_worktree_add_new_branch_command(&repo, &checkout, branch, "HEAD", false);
        run_worktree_command(&add).unwrap();
        let remove = build_worktree_remove_command(&repo, &checkout, true, false);
        run_worktree_command(&remove).unwrap();
        std::fs::create_dir_all(&checkout).unwrap();
        std::fs::write(checkout.join("unrelated"), "do not delete\n").unwrap();

        let err = run_worktree_remove_command_with_recovery(&remove, &repo, &checkout, true, false)
            .expect_err("unrelated replacement directory should not be removed");

        assert!(is_not_working_tree_remove_error(&err));
        assert!(checkout.join("unrelated").exists());
        let _ = std::fs::remove_dir_all(checkout);
        let _ = std::fs::remove_dir_all(repo);
    }

    // ---- cow backend ----

    #[test]
    fn worktree_backend_from_config_selects_cow_only() {
        assert_eq!(WorktreeBackend::from_config("cow"), WorktreeBackend::Cow);
        assert_eq!(WorktreeBackend::from_config(" cow "), WorktreeBackend::Cow);
        assert_eq!(WorktreeBackend::from_config("git"), WorktreeBackend::Git);
        assert_eq!(WorktreeBackend::from_config(""), WorktreeBackend::Git);
        assert_eq!(
            WorktreeBackend::from_config("something-else"),
            WorktreeBackend::Git
        );
    }

    #[test]
    fn cow_pastures_directory_prefers_env_override() {
        let _lock = COW_ENV_LOCK.lock().unwrap();
        let dir = unique_temp_path("cow-dir-override");
        let _env = CowEnvGuard::set(&[("HERDR_COW_DIR", Some(&dir))]);
        assert_eq!(cow_pastures_directory(), dir);
    }

    #[test]
    fn cow_program_prefers_env_override() {
        let _lock = COW_ENV_LOCK.lock().unwrap();
        let _env = CowEnvGuard::set(&[("HERDR_COW_BIN", Some(Path::new("/custom/cow")))]);
        assert_eq!(cow_program(), Path::new("/custom/cow"));
    }

    #[cfg(unix)]
    #[test]
    fn cow_program_searches_path_dirs() {
        let _lock = COW_ENV_LOCK.lock().unwrap();
        let dir = unique_temp_path("cow-program-path");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("cow"), "").unwrap();
        // Keep system dirs so parallel tests can still spawn git/sh.
        let path = format!("{}:/usr/bin:/bin", dir.display());
        let _env = CowEnvGuard::set(&[
            ("HERDR_COW_BIN", Some(Path::new("  "))),
            ("PATH", Some(Path::new(&path))),
        ]);

        assert_eq!(cow_program(), dir.join("cow"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn cow_list_parses_pasture_entries() {
        let _lock = COW_ENV_LOCK.lock().unwrap();
        let dir = unique_temp_path("cow-list-entries");
        std::fs::create_dir_all(&dir).unwrap();
        let pasture = dir.join("pastures/repo/feat");
        let source = dir.join("src/repo");
        let stub = write_cow_stub(
            &dir,
            &format!(
                "[{{\"name\":\"repo/feat\",\"path\":\"{}\",\"source\":\"{}\",\"current_branch\":\"feat\",\"dirty\":true,\"is_worktree\":false}},\
                 {{\"name\":\"repo/det\",\"path\":\"{}\",\"source\":\"{}\",\"branch\":\"det\",\"dirty\":false}}]",
                pasture.display(),
                source.display(),
                dir.join("pastures/repo/det").display(),
                source.display(),
            ),
        );
        let _env = CowEnvGuard::set(&[("HERDR_COW_BIN", Some(&stub))]);

        let pastures = list_cow_pastures(None).unwrap();

        assert_eq!(
            pastures,
            vec![
                CowPasture {
                    name: "repo/feat".into(),
                    path: pasture.clone(),
                    source: source.clone(),
                    branch: Some("feat".into()),
                    dirty: true,
                },
                CowPasture {
                    name: "repo/det".into(),
                    path: dir.join("pastures/repo/det"),
                    source: source.clone(),
                    branch: Some("det".into()),
                    dirty: false,
                },
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn cow_pasture_source_maps_listed_pasture_to_source() {
        let _lock = COW_ENV_LOCK.lock().unwrap();
        let dir = unique_temp_path("cow-pasture-source");
        let pasture = create_committed_repo("cow-pasture-source-pasture");
        let source = create_committed_repo("cow-pasture-source-src");
        std::fs::create_dir_all(&dir).unwrap();
        let stub = write_cow_stub(
            &dir,
            &format!(
                "[{{\"name\":\"src/pasture\",\"path\":\"{}\",\"source\":\"{}\",\"current_branch\":\"feat\",\"dirty\":false}}]",
                pasture.display(),
                source.display(),
            ),
        );
        let _env = CowEnvGuard::set(&[("HERDR_COW_BIN", Some(&stub))]);

        assert_eq!(
            cow_pasture_source(&pasture),
            Some(canonical_or_original(&source))
        );
        assert_eq!(cow_pasture_source(&source), None);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&pasture);
        let _ = std::fs::remove_dir_all(&source);
    }

    #[cfg(unix)]
    #[test]
    fn list_worktrees_cow_includes_source_then_pastures() {
        let _lock = COW_ENV_LOCK.lock().unwrap();
        let dir = unique_temp_path("cow-list-worktrees");
        let repo = create_committed_repo("cow-list-worktrees-repo");
        let pasture = dir.join("pastures/repo/feat");
        std::fs::create_dir_all(&pasture).unwrap();
        let stub = write_cow_stub(
            &dir,
            &format!(
                "[{{\"name\":\"repo/feat\",\"path\":\"{}\",\"source\":\"{}\",\"current_branch\":\"feat\",\"dirty\":false}}]",
                pasture.display(),
                repo.display(),
            ),
        );
        let _env = CowEnvGuard::set(&[("HERDR_COW_BIN", Some(&stub))]);

        let entries = list_worktrees(WorktreeBackend::Cow, &repo, false).unwrap();

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].path, repo);
        assert!(entries[0].branch.is_some());
        assert!(!entries[0].is_detached);
        assert_eq!(entries[1].path, pasture);
        assert_eq!(entries[1].branch.as_deref(), Some("feat"));
        let log = std::fs::read_to_string(dir.join("cow-stub.log")).unwrap();
        assert!(
            log.contains(&format!("--source {}", repo.display())),
            "expected cow list scoped to the source repo, got: {log}"
        );
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[cfg(unix)]
    #[test]
    fn run_worktree_add_cow_returns_reported_path() {
        let _lock = COW_ENV_LOCK.lock().unwrap();
        let dir = unique_temp_path("cow-add");
        let repo = create_committed_repo("cow-add-repo");
        std::fs::create_dir_all(&dir).unwrap();
        let stub = write_cow_stub(&dir, "[]");
        let _env = CowEnvGuard::set(&[("HERDR_COW_BIN", Some(&stub))]);

        let created = run_worktree_add(
            WorktreeBackend::Cow,
            &repo,
            &dir.join("ignored-git-path"),
            "feature/thing",
            "HEAD",
            false,
        )
        .unwrap();

        assert_eq!(created, dir.join("pastures/feature-thing"));
        let log = std::fs::read_to_string(dir.join("cow-stub.log")).unwrap();
        assert!(
            log.contains(&format!(
                "create --source {} --branch feature/thing --print-path feature-thing",
                repo.display()
            )),
            "unexpected cow create invocation: {log}"
        );
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[cfg(unix)]
    #[test]
    fn cow_remove_command_prefers_listed_pasture_name() {
        let _lock = COW_ENV_LOCK.lock().unwrap();
        let dir = unique_temp_path("cow-remove-listed");
        let pasture = dir.join("pastures/repo/feat");
        std::fs::create_dir_all(&pasture).unwrap();
        let stub = write_cow_stub(
            &dir,
            &format!(
                "[{{\"name\":\"custom/pasture-name\",\"path\":\"{}\",\"source\":\"/src/repo\",\"current_branch\":\"feat\",\"dirty\":false}}]",
                pasture.display(),
            ),
        );
        let _env = CowEnvGuard::set(&[("HERDR_COW_BIN", Some(&stub))]);

        let command = build_cow_remove_command(&pasture, true);

        assert_eq!(command.program, stub.display().to_string());
        assert_eq!(
            command.args,
            vec!["remove", "custom/pasture-name", "-y", "--force"]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cow_remove_command_falls_back_to_path_tail() {
        let _lock = COW_ENV_LOCK.lock().unwrap();
        // No listed pasture matches: the name is derived from the
        // <repo>/<name> tail of the checkout path. Works whether or not a
        // real cow binary is installed, since this path cannot be listed.
        let command = build_cow_remove_command(Path::new("/pastures/repo/feat"), false);

        assert_eq!(command.args, vec!["remove", "repo/feat", "-y"]);
    }
}
