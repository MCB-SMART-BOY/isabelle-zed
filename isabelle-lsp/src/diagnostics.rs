use bridge::protocol::{Diagnostic as BridgeDiagnostic, Message, MessageType, Severity};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::RwLock;
use tower_lsp::Client;
use tower_lsp::lsp_types::{
    Diagnostic, DiagnosticSeverity, Position, PublishDiagnosticsParams, Range, Url,
};
use tracing::warn;

pub(crate) type PublishedDiagnosticTargets = Arc<RwLock<HashMap<Url, Vec<Url>>>>;

pub(crate) fn bridge_diagnostic_to_lsp(diagnostic: &BridgeDiagnostic) -> Diagnostic {
    Diagnostic {
        range: bridge_range_to_lsp(&diagnostic.range),
        severity: Some(bridge_severity_to_lsp(diagnostic.severity)),
        message: diagnostic.message.clone(),
        source: Some("isabelle".to_string()),
        ..Diagnostic::default()
    }
}

fn bridge_range_to_lsp(range: &bridge::protocol::Range) -> Range {
    Range {
        start: Position {
            line: bridge_index_to_lsp(range.start.line),
            character: bridge_index_to_lsp(range.start.col),
        },
        end: Position {
            line: bridge_index_to_lsp(range.end.line),
            character: bridge_index_to_lsp(range.end.col),
        },
    }
}

fn bridge_severity_to_lsp(severity: Severity) -> DiagnosticSeverity {
    match severity {
        Severity::Error => DiagnosticSeverity::ERROR,
        Severity::Warning => DiagnosticSeverity::WARNING,
        Severity::Info => DiagnosticSeverity::INFORMATION,
    }
}

fn bridge_index_to_lsp(value: i64) -> u32 {
    if value <= 1 {
        return 0;
    }

    u32::try_from(value - 1).unwrap_or(u32::MAX)
}

pub(crate) async fn publish_diagnostics_for(
    client: &Client,
    published_diagnostic_targets: &PublishedDiagnosticTargets,
    uri: Url,
    version: i64,
    response: Message,
) -> Result<(), String> {
    if response.msg_type != MessageType::Diagnostics {
        return Err(format!(
            "unexpected response type from bridge: {:?}",
            response.msg_type
        ));
    }

    let payload = response
        .diagnostics_payload()
        .map_err(|err| err.to_string())?;
    let mut grouped: HashMap<Url, Vec<Diagnostic>> = HashMap::new();

    for diagnostic in &payload {
        let target_uri = match Url::parse(&diagnostic.uri) {
            Ok(parsed) => parsed,
            Err(err) => {
                warn!(
                    "bridge diagnostic had invalid uri '{}': {err}; publishing to request uri",
                    diagnostic.uri
                );
                uri.clone()
            }
        };

        grouped
            .entry(target_uri)
            .or_default()
            .push(bridge_diagnostic_to_lsp(diagnostic));
    }

    grouped.entry(uri.clone()).or_default();

    let current_targets = grouped.keys().cloned().collect::<Vec<_>>();
    let stale_targets = {
        let mut state = published_diagnostic_targets.write().await;
        let previous = state.get(&uri).cloned().unwrap_or_default();
        state.insert(uri.clone(), current_targets.clone());
        previous
    };

    let current_target_set = current_targets.into_iter().collect::<HashSet<_>>();
    let request_version = Some(i32::try_from(version).unwrap_or(i32::MAX));

    for stale_uri in stale_targets {
        if !current_target_set.contains(&stale_uri) {
            let publish_version = if stale_uri == uri {
                request_version
            } else {
                None
            };
            client
                .publish_diagnostics(stale_uri, Vec::new(), publish_version)
                .await;
        }
    }

    for (target_uri, diagnostics) in grouped {
        let publish_version = if target_uri == uri {
            request_version
        } else {
            None
        };
        let params = PublishDiagnosticsParams {
            uri: target_uri,
            diagnostics,
            version: publish_version,
        };
        client
            .publish_diagnostics(params.uri, params.diagnostics, params.version)
            .await;
    }

    Ok(())
}
