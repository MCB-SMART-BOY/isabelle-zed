use std::collections::HashMap;
use zed_extension_api::{self as zed, LanguageServerId, serde_json::Value, settings::LspSettings};

const ISABELLE_LANGUAGE_SERVER_ID: &str = "isabelle-lsp";

const DEFAULT_NATIVE_BINARY: &str = "isabelle";
const DEFAULT_BRIDGE_BINARY: &str = "isabelle-zed-lsp";

const DEFAULT_BRIDGE_SOCKET: &str = "/tmp/isabelle.sock";
const DEFAULT_SESSION: &str = "s1";

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
        let args = resolve_command_args(mode, binary.as_ref(), &settings_json);
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
) -> Vec<String> {
    if let Some(args) = binary.and_then(|binary| binary.arguments.clone()) {
        return args;
    }

    match mode {
        ServerMode::Native => native_default_args(settings_json),
        ServerMode::Bridge => Vec::new(),
    }
}

fn native_default_args(settings_json: &Option<Value>) -> Vec<String> {
    let mut args = vec!["vscode_server".to_string()];

    if let Some(logic) = setting_string(settings_json, SETTINGS_KEY_NATIVE_LOGIC) {
        args.push("-l".to_string());
        args.push(logic);
    }

    if setting_bool(settings_json, SETTINGS_KEY_NATIVE_NO_BUILD).unwrap_or(false) {
        args.push("-n".to_string());
    }

    for session_dir in setting_string_array(settings_json, SETTINGS_KEY_NATIVE_SESSION_DIRS) {
        args.push("-d".to_string());
        args.push(session_dir);
    }

    args
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
                .unwrap_or_else(|| DEFAULT_BRIDGE_SOCKET.to_string());
            let session = setting_string(settings_json, SETTINGS_KEY_SESSION)
                .unwrap_or_else(|| DEFAULT_SESSION.to_string());

            ensure_env_var(&mut env, ENV_BRIDGE_SOCKET, bridge_socket);
            ensure_env_var(&mut env, ENV_SESSION, session);

            if let Some(command) =
                setting_string(settings_json, SETTINGS_KEY_BRIDGE_AUTOSTART_COMMAND)
            {
                upsert_env_var(&mut env, ENV_BRIDGE_AUTOSTART_CMD.to_string(), command);
            }

            if let Some(timeout_ms) =
                setting_u64(settings_json, SETTINGS_KEY_BRIDGE_AUTOSTART_TIMEOUT_MS)
            {
                upsert_env_var(
                    &mut env,
                    ENV_BRIDGE_AUTOSTART_TIMEOUT_MS.to_string(),
                    timeout_ms.to_string(),
                );
            }

            env
        }
    }
}

fn merge_environment(
    mut base: Vec<(String, String)>,
    overrides: Option<HashMap<String, String>>,
) -> Vec<(String, String)> {
    if let Some(overrides) = overrides {
        for (key, value) in overrides {
            upsert_env_var(&mut base, key, value);
        }
    }

    base
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

fn setting_u64(settings_json: &Option<Value>, key: &str) -> Option<u64> {
    settings_json
        .as_ref()
        .and_then(|value| value.get(key))
        .and_then(Value::as_u64)
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
}
