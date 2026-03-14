use std::collections::HashMap;
use zed_extension_api::{self as zed, LanguageServerId, serde_json::Value, settings::LspSettings};

const ISABELLE_LANGUAGE_SERVER_ID: &str = "isabelle-lsp";

const DEFAULT_NATIVE_BINARY: &str = "isabelle";
const DEFAULT_BRIDGE_BINARY: &str = "isabelle-zed-lsp";

const ENV_BRIDGE_SOCKET: &str = "ISABELLE_BRIDGE_SOCKET";
const ENV_SESSION: &str = "ISABELLE_SESSION";
const ENV_BRIDGE_AUTOSTART_CMD: &str = "ISABELLE_BRIDGE_AUTOSTART_CMD";
const ENV_BRIDGE_AUTOSTART_TIMEOUT_MS: &str = "ISABELLE_BRIDGE_AUTOSTART_TIMEOUT_MS";

const SETTINGS_KEY_MODE: &str = "mode";
const SETTINGS_KEY_BRIDGE_SOCKET: &str = "bridge_socket";
const SETTINGS_KEY_SESSION: &str = "session";
const SETTINGS_KEY_BRIDGE_AUTOSTART_COMMAND: &str = "bridge_autostart_command";
const SETTINGS_KEY_BRIDGE_AUTOSTART_TIMEOUT_MS: &str = "bridge_autostart_timeout_ms";
const SETTINGS_KEY_NATIVE_LOGIC: &str = "native_logic";
const SETTINGS_KEY_NATIVE_NO_BUILD: &str = "native_no_build";
const SETTINGS_KEY_NATIVE_SESSION_DIRS: &str = "native_session_dirs";
const SETTINGS_KEY_NATIVE_EXTRA_ARGS: &str = "native_extra_args";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ServerMode {
    Native,
    Bridge,
}

struct IsabelleExtension;

impl IsabelleExtension {
    fn resolve_language_server_command(
        &self,
        worktree: &zed::Worktree,
        lsp_settings: Option<LspSettings>,
    ) -> zed::Command {
        let (binary, settings_json) = match lsp_settings {
            Some(settings) => (settings.binary, settings.settings),
            None => (None, None),
        };

        let mode = mode_from_settings(&settings_json);
        let command = resolve_command_path(worktree, mode, binary.as_ref());
        let args = resolve_command_args(mode, binary.as_ref(), &settings_json, worktree);
        let env = resolve_environment(worktree, mode, binary.as_ref(), &settings_json);

        zed::Command { command, args, env }
    }
}

impl zed::Extension for IsabelleExtension {
    fn new() -> Self {
        Self
    }

    fn language_server_command(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &zed::Worktree,
    ) -> zed::Result<zed::Command> {
        if language_server_id.as_ref() != ISABELLE_LANGUAGE_SERVER_ID {
            return Err(format!(
                "unsupported language server id: {}",
                language_server_id.as_ref()
            ));
        }

        let lsp_settings = LspSettings::for_worktree(language_server_id.as_ref(), worktree).ok();
        Ok(self.resolve_language_server_command(worktree, lsp_settings))
    }

    fn language_server_workspace_configuration(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &zed::Worktree,
    ) -> zed::Result<Option<zed::serde_json::Value>> {
        let settings = LspSettings::for_worktree(language_server_id.as_ref(), worktree)
            .ok()
            .and_then(|settings| settings.settings)
            .and_then(strip_control_settings);

        Ok(settings)
    }
}

fn mode_from_settings(settings_json: &Option<Value>) -> ServerMode {
    match setting_string(settings_json, SETTINGS_KEY_MODE)
        .as_deref()
        .map(|value| value.to_ascii_lowercase())
        .as_deref()
    {
        Some("bridge") => ServerMode::Bridge,
        _ => ServerMode::Native,
    }
}

fn resolve_command_path(
    worktree: &zed::Worktree,
    mode: ServerMode,
    binary: Option<&zed::settings::CommandSettings>,
) -> String {
    if let Some(path) = binary.and_then(|binary| binary.path.clone()) {
        return path;
    }

    match mode {
        ServerMode::Native => worktree
            .which(DEFAULT_NATIVE_BINARY)
            .unwrap_or_else(|| DEFAULT_NATIVE_BINARY.to_string()),
        ServerMode::Bridge => worktree
            .which(DEFAULT_BRIDGE_BINARY)
            .unwrap_or_else(|| DEFAULT_BRIDGE_BINARY.to_string()),
    }
}

fn resolve_command_args(
    mode: ServerMode,
    binary: Option<&zed::settings::CommandSettings>,
    settings_json: &Option<Value>,
    worktree: &zed::Worktree,
) -> Vec<String> {
    if let Some(args) = binary.and_then(|binary| binary.arguments.clone()) {
        return args;
    }

    match mode {
        ServerMode::Native => native_default_args(settings_json, worktree),
        ServerMode::Bridge => Vec::new(),
    }
}

fn native_default_args(settings_json: &Option<Value>, worktree: &zed::Worktree) -> Vec<String> {
    let mut args = vec!["vscode_server".to_string()];

    let logic = setting_string(settings_json, SETTINGS_KEY_NATIVE_LOGIC)
        .or_else(|| auto_logic_from_root(worktree));
    if let Some(logic) = logic {
        args.push("-l".to_string());
        args.push(logic);
    }

    if setting_bool(settings_json, SETTINGS_KEY_NATIVE_NO_BUILD).unwrap_or(false) {
        args.push("-n".to_string());
    }

    let mut session_dirs = setting_string_array(settings_json, SETTINGS_KEY_NATIVE_SESSION_DIRS);
    if worktree_has_session_root(worktree) {
        let root = worktree.root_path();
        if !session_dirs.iter().any(|dir| dir == &root) {
            session_dirs.push(root);
        }
    }

    for session_dir in session_dirs {
        args.push("-d".to_string());
        args.push(session_dir);
    }

    for extra in setting_string_array(settings_json, SETTINGS_KEY_NATIVE_EXTRA_ARGS) {
        args.push(extra);
    }

    args
}

fn worktree_has_session_root(worktree: &zed::Worktree) -> bool {
    worktree.read_text_file("ROOT").is_ok() || worktree.read_text_file("ROOTS").is_ok()
}

#[derive(Clone, Debug)]
struct SessionInfo {
    name: String,
    parent: Option<String>,
    origin: SessionOrigin,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SessionOrigin {
    WorktreeRoot,
    OtherRoot,
}

fn auto_logic_from_root(worktree: &zed::Worktree) -> Option<String> {
    let mut sessions = Vec::new();
    collect_sessions_from_root(worktree, "ROOT", SessionOrigin::WorktreeRoot, &mut sessions);

    if let Ok(roots) = worktree.read_text_file("ROOTS") {
        for line in roots.lines() {
            let Some(root_entry) = parse_roots_line(line) else {
                continue;
            };
            let root_path = format!("{}/ROOT", root_entry.trim_end_matches('/'));
            collect_sessions_from_root(
                worktree,
                &root_path,
                SessionOrigin::OtherRoot,
                &mut sessions,
            );
        }
    }

    pick_auto_logic(&sessions)
}

fn pick_auto_logic(sessions: &[SessionInfo]) -> Option<String> {
    let root_names = unique_session_names(
        sessions
            .iter()
            .filter(|session| session.origin == SessionOrigin::WorktreeRoot),
    );
    if root_names.len() == 1 {
        return root_names.into_iter().next();
    }

    let hol_names = unique_session_names(
        sessions
            .iter()
            .filter(|session| session.parent.as_deref() == Some("HOL")),
    );
    if hol_names.len() == 1 {
        return hol_names.into_iter().next();
    }

    None
}

fn unique_session_names<'a, I>(sessions: I) -> Vec<String>
where
    I: IntoIterator<Item = &'a SessionInfo>,
{
    let mut unique = Vec::new();
    for session in sessions {
        if !unique.contains(&session.name) {
            unique.push(session.name.clone());
        }
    }
    unique
}

fn collect_sessions_from_root(
    worktree: &zed::Worktree,
    path: &str,
    origin: SessionOrigin,
    out: &mut Vec<SessionInfo>,
) {
    let Ok(text) = worktree.read_text_file(path) else {
        return;
    };

    for line in text.lines() {
        if let Some((name, parent)) = parse_session_from_line(line) {
            out.push(SessionInfo {
                name,
                parent,
                origin,
            });
        }
    }
}

fn parse_session_from_line(line: &str) -> Option<(String, Option<String>)> {
    let trimmed = line.trim_start();
    if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("(*") {
        return None;
    }

    let trimmed = strip_unquoted_hash_comment(trimmed);
    let trimmed = trimmed.trim_start();
    if trimmed.is_empty() {
        return None;
    }

    let tokens = tokenize_root_line(trimmed);
    let first = tokens.first()?;
    if first != "session" {
        return None;
    }

    let name = tokens.get(1)?.clone();
    if name.is_empty() {
        return None;
    }

    let mut parent = None;
    for window in tokens.windows(2) {
        if window[0] == "=" {
            let candidate = window[1].clone();
            if !candidate.is_empty() {
                parent = Some(candidate);
            }
            break;
        }
    }

    Some((name, parent))
}

fn parse_roots_line(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }

    let trimmed = strip_unquoted_hash_comment(trimmed);
    let trimmed = trimmed.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some(quoted) = trimmed.strip_prefix('"') {
        let name = quoted.split('"').next().unwrap_or("").trim();
        if name.is_empty() {
            None
        } else {
            Some(name.to_string())
        }
    } else {
        Some(trimmed.to_string())
    }
}

fn strip_unquoted_hash_comment(line: &str) -> String {
    let mut out = String::new();
    let mut in_quotes = false;
    for ch in line.chars() {
        if ch == '"' {
            in_quotes = !in_quotes;
            out.push(ch);
            continue;
        }
        if ch == '#' && !in_quotes {
            break;
        }
        out.push(ch);
    }
    out
}

fn tokenize_root_line(line: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch.is_whitespace() {
            continue;
        }
        if ch == '"' {
            let mut token = String::new();
            while let Some(next) = chars.next() {
                if next == '"' {
                    break;
                }
                token.push(next);
            }
            tokens.push(token);
            continue;
        }

        let mut token = String::new();
        token.push(ch);
        while let Some(next) = chars.peek() {
            if next.is_whitespace() {
                break;
            }
            token.push(*next);
            chars.next();
        }
        tokens.push(token);
    }
    tokens
}

fn resolve_environment(
    worktree: &zed::Worktree,
    mode: ServerMode,
    binary: Option<&zed::settings::CommandSettings>,
    settings_json: &Option<Value>,
) -> Vec<(String, String)> {
    let mut env = merge_environment(
        worktree.shell_env(),
        binary.and_then(|binary| binary.env.clone()),
    );

    match mode {
        ServerMode::Native => env,
        ServerMode::Bridge => {
            let bridge_socket = setting_string(settings_json, SETTINGS_KEY_BRIDGE_SOCKET)
                .unwrap_or_else(|| default_bridge_socket(worktree));
            let session = setting_string(settings_json, SETTINGS_KEY_SESSION)
                .unwrap_or_else(|| default_session(worktree));

            ensure_env_var(&mut env, ENV_BRIDGE_SOCKET, bridge_socket);
            ensure_env_var(&mut env, ENV_SESSION, session);

            env
        }
    }
}

fn default_bridge_socket(worktree: &zed::Worktree) -> String {
    format!("/tmp/isabelle-{}.sock", worktree.id())
}

fn default_session(worktree: &zed::Worktree) -> String {
    format!("s{}", worktree.id())
}

fn merge_environment(
    mut base: Vec<(String, String)>,
    overrides: Option<HashMap<String, String>>,
) -> Vec<(String, String)> {
    if let Some(overrides) = overrides {
        for (key, value) in overrides {
            if is_restricted_override_env_var(&key) {
                continue;
            }
            upsert_env_var(&mut base, key, value);
        }
    }

    base
}

fn is_restricted_override_env_var(key: &str) -> bool {
    key == ENV_BRIDGE_AUTOSTART_CMD || key == ENV_BRIDGE_AUTOSTART_TIMEOUT_MS
}

fn upsert_env_var(env: &mut Vec<(String, String)>, key: String, value: String) {
    if let Some(existing) = env.iter_mut().find(|entry| entry.0 == key) {
        existing.1 = value;
        return;
    }

    env.push((key, value));
}

fn ensure_env_var(env: &mut Vec<(String, String)>, key: &str, value: String) {
    if env.iter().any(|entry| entry.0 == key) {
        return;
    }

    env.push((key.to_string(), value));
}

fn setting_string(settings_json: &Option<Value>, key: &str) -> Option<String> {
    settings_json
        .as_ref()
        .and_then(|value| value.get(key))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn setting_bool(settings_json: &Option<Value>, key: &str) -> Option<bool> {
    settings_json
        .as_ref()
        .and_then(|value| value.get(key))
        .and_then(Value::as_bool)
}

fn setting_string_array(settings_json: &Option<Value>, key: &str) -> Vec<String> {
    settings_json
        .as_ref()
        .and_then(|value| value.get(key))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn strip_control_settings(settings: Value) -> Option<Value> {
    let mut object = settings.as_object()?.clone();

    for key in [
        SETTINGS_KEY_MODE,
        SETTINGS_KEY_BRIDGE_SOCKET,
        SETTINGS_KEY_SESSION,
        SETTINGS_KEY_BRIDGE_AUTOSTART_COMMAND,
        SETTINGS_KEY_BRIDGE_AUTOSTART_TIMEOUT_MS,
        SETTINGS_KEY_NATIVE_LOGIC,
        SETTINGS_KEY_NATIVE_NO_BUILD,
        SETTINGS_KEY_NATIVE_SESSION_DIRS,
        SETTINGS_KEY_NATIVE_EXTRA_ARGS,
    ] {
        object.remove(key);
    }

    if object.is_empty() {
        None
    } else {
        Some(Value::Object(object))
    }
}

zed::register_extension!(IsabelleExtension);

#[cfg(test)]
mod tests {
    use super::*;
    use zed::serde_json::json;

    #[test]
    fn mode_defaults_to_native() {
        assert_eq!(mode_from_settings(&None), ServerMode::Native);
        assert_eq!(
            mode_from_settings(&Some(json!({ "mode": "native" }))),
            ServerMode::Native
        );
        assert_eq!(
            mode_from_settings(&Some(json!({ "mode": "bridge" }))),
            ServerMode::Bridge
        );
    }

    #[test]
    fn strip_control_settings_keeps_only_lsp_payload() {
        let input = json!({
            "mode": "bridge",
            "bridge_socket": "/tmp/isabelle.sock",
            "session": "s1",
            "bridge_autostart_command": "bridge --socket /tmp/isabelle.sock",
            "bridge_autostart_timeout_ms": 10000,
            "native_logic": "HOL",
            "native_no_build": false,
            "native_session_dirs": ["."],
            "native_extra_args": ["-v"],
            "isabelle": {
                "completion_limit": 200
            }
        });

        let output = strip_control_settings(input).expect("settings should not be empty");
        assert_eq!(output, json!({ "isabelle": { "completion_limit": 200 }}));
    }

    #[test]
    fn strip_control_settings_returns_none_for_control_only_input() {
        let input = json!({
            "mode": "bridge",
            "bridge_socket": "/tmp/isabelle.sock"
        });

        assert_eq!(strip_control_settings(input), None);
    }

    #[test]
    fn merge_environment_ignores_autostart_env_overrides() {
        let base = vec![(
            ENV_BRIDGE_AUTOSTART_CMD.to_string(),
            "bridge --socket /tmp/isabelle.sock".to_string(),
        )];
        let overrides = Some(HashMap::from([
            (
                ENV_BRIDGE_AUTOSTART_CMD.to_string(),
                "malicious-command".to_string(),
            ),
            (ENV_BRIDGE_AUTOSTART_TIMEOUT_MS.to_string(), "1".to_string()),
            ("ISABELLE_SESSION".to_string(), "s2".to_string()),
        ]));

        let merged = merge_environment(base, overrides);
        assert!(merged.iter().any(|(k, v)| {
            k == ENV_BRIDGE_AUTOSTART_CMD && v == "bridge --socket /tmp/isabelle.sock"
        }));
        assert!(
            !merged
                .iter()
                .any(|(k, v)| k == ENV_BRIDGE_AUTOSTART_CMD && v == "malicious-command")
        );
        assert!(
            !merged
                .iter()
                .any(|(k, _)| k == ENV_BRIDGE_AUTOSTART_TIMEOUT_MS)
        );
        assert!(merged.iter().any(|(k, v)| k == ENV_SESSION && v == "s2"));
    }

    #[test]
    fn parse_session_line_extracts_name_and_parent() {
        assert_eq!(
            parse_session_from_line("session Foo = HOL"),
            Some(("Foo".to_string(), Some("HOL".to_string())))
        );
        assert_eq!(
            parse_session_from_line("session \"Foo Bar\" in \"dir with space\" = \"HOL\" # comment"),
            Some(("Foo Bar".to_string(), Some("HOL".to_string())))
        );
        assert_eq!(
            parse_session_from_line("session Foo"),
            Some(("Foo".to_string(), None))
        );
        assert_eq!(parse_session_from_line("# session Foo = HOL"), None);
        assert_eq!(parse_session_from_line("(* session Foo = HOL *)"), None);
    }

    #[test]
    fn parse_roots_line_handles_quotes_and_comments() {
        assert_eq!(
            parse_roots_line("src/logic  # comment"),
            Some("src/logic".to_string())
        );
        assert_eq!(
            parse_roots_line("\"src with space\""),
            Some("src with space".to_string())
        );
        assert_eq!(parse_roots_line("# only comment"), None);
    }

    #[test]
    fn pick_auto_logic_prefers_single_root_session() {
        let sessions = vec![
            SessionInfo {
                name: "RootOnly".to_string(),
                parent: Some("HOL".to_string()),
                origin: SessionOrigin::WorktreeRoot,
            },
            SessionInfo {
                name: "Other".to_string(),
                parent: Some("HOL".to_string()),
                origin: SessionOrigin::OtherRoot,
            },
        ];

        assert_eq!(pick_auto_logic(&sessions), Some("RootOnly".to_string()));
    }

    #[test]
    fn pick_auto_logic_falls_back_to_single_hol_parent() {
        let sessions = vec![
            SessionInfo {
                name: "RootA".to_string(),
                parent: Some("Pure".to_string()),
                origin: SessionOrigin::WorktreeRoot,
            },
            SessionInfo {
                name: "RootB".to_string(),
                parent: Some("Pure".to_string()),
                origin: SessionOrigin::WorktreeRoot,
            },
            SessionInfo {
                name: "HolChild".to_string(),
                parent: Some("HOL".to_string()),
                origin: SessionOrigin::OtherRoot,
            },
        ];

        assert_eq!(pick_auto_logic(&sessions), Some("HolChild".to_string()));
    }

    #[test]
    fn pick_auto_logic_returns_none_for_multiple_candidates() {
        let sessions = vec![
            SessionInfo {
                name: "HolA".to_string(),
                parent: Some("HOL".to_string()),
                origin: SessionOrigin::OtherRoot,
            },
            SessionInfo {
                name: "HolB".to_string(),
                parent: Some("HOL".to_string()),
                origin: SessionOrigin::OtherRoot,
            },
        ];

        assert_eq!(pick_auto_logic(&sessions), None);
    }
}
