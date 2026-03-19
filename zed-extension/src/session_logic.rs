use zed_extension_api as zed;

#[derive(Clone, Debug)]
pub(crate) struct SessionInfo {
    pub(crate) name: String,
    pub(crate) parent: Option<String>,
    pub(crate) origin: SessionOrigin,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SessionOrigin {
    WorktreeRoot,
    OtherRoot,
}

pub(crate) fn auto_logic_from_root(worktree: &zed::Worktree) -> Option<String> {
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

pub(crate) fn pick_auto_logic(sessions: &[SessionInfo]) -> Option<String> {
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

pub(crate) fn parse_session_from_line(line: &str) -> Option<(String, Option<String>)> {
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

pub(crate) fn parse_roots_line(line: &str) -> Option<String> {
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
            for next in chars.by_ref() {
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
