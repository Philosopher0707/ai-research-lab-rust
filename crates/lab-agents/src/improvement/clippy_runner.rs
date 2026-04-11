//! Parse `cargo clippy --message-format json` output into structured findings.
//!
//! Each compiler message that carries a diagnostic code is converted to a
//! `ClippyFinding`. Primary spans expose byte offsets so downstream edit
//! strategies can replace exact source ranges without string matching.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// ─── SuggestionApplicability ────────────────────────────────────────

/// How confidently the compiler's suggested replacement can be applied.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuggestionApplicability {
    /// The replacement is exact and machine-safe.
    MachineApplicable,
    /// The replacement might be wrong in some edge cases.
    MaybeIncorrect,
    /// The replacement contains `$VAR` placeholders that need filling.
    HasPlaceholders,
    /// No applicability information.
    #[default]
    Unspecified,
}

// ─── ClippyFinding ──────────────────────────────────────────────────

/// One clippy warning/error parsed from the JSON compiler output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClippyFinding {
    /// Lint name, e.g. `"clippy::unwrap_used"`.
    pub lint: String,
    /// Workspace-relative (or absolute) source path.
    pub file: PathBuf,
    /// 1-based line numbers of the primary span.
    pub line_start: u32,
    pub line_end: u32,
    /// Byte offsets into the source file (present when the compiler emits them).
    pub byte_start: Option<u32>,
    pub byte_end: Option<u32>,
    /// Short message text.
    pub message: String,
    /// Full rendered diagnostic (with context lines and suggestion).
    pub rendered: String,
    /// How safe the compiler's suggested replacement is to apply mechanically.
    pub applicability: SuggestionApplicability,
    /// The suggested replacement string, if any.
    pub suggested_replacement: Option<String>,
}

// ─── Raw JSON shapes ────────────────────────────────────────────────

#[derive(Deserialize)]
struct CompilerMessage {
    reason: String,
    message: Option<DiagnosticMessage>,
}

#[derive(Deserialize)]
struct DiagnosticMessage {
    code: Option<DiagnosticCode>,
    message: String,
    level: String,
    rendered: Option<String>,
    spans: Vec<DiagnosticSpan>,
    children: Vec<DiagnosticChild>,
}

#[derive(Deserialize)]
struct DiagnosticCode {
    code: String,
}

#[derive(Deserialize)]
struct DiagnosticSpan {
    file_name: String,
    is_primary: bool,
    line_start: u32,
    line_end: u32,
    byte_start: Option<u32>,
    byte_end: Option<u32>,
    suggested_replacement: Option<String>,
    suggestion_applicability: Option<String>,
}

#[derive(Deserialize)]
struct DiagnosticChild {
    spans: Vec<DiagnosticSpan>,
}

// ─── Runner ─────────────────────────────────────────────────────────

/// Run `cargo clippy --message-format json` in `workspace` and return all findings.
///
/// Only `warning`-level diagnostics with a `clippy::` lint code are returned.
/// Hard errors and `#[allow]`-suppressed lints are excluded.
pub async fn run_clippy(workspace: &Path) -> Vec<ClippyFinding> {
    let output = tokio::process::Command::new("cargo")
        .args(["clippy", "--message-format", "json", "--workspace", "--quiet"])
        .current_dir(workspace)
        .output()
        .await;

    let output = match output {
        Ok(o) => o,
        Err(e) => {
            tracing::warn!("clippy_runner: failed to spawn cargo: {e}");
            return Vec::new();
        }
    };

    parse_output(&output.stdout)
}

/// Parse raw bytes from `cargo clippy --message-format json` output.
pub fn parse_output(bytes: &[u8]) -> Vec<ClippyFinding> {
    let text = match std::str::from_utf8(bytes) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };

    let mut findings = Vec::new();

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let msg: CompilerMessage = match serde_json::from_str(line) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if msg.reason != "compiler-message" {
            continue;
        }
        let diag = match msg.message {
            Some(d) => d,
            None => continue,
        };
        if diag.level != "warning" {
            continue;
        }
        let code = match &diag.code {
            Some(c) if c.code.starts_with("clippy::") => c.code.clone(),
            _ => continue,
        };

        // Find the primary span
        let primary = match diag.spans.iter().find(|s| s.is_primary) {
            Some(s) => s,
            None => continue,
        };

        // Look for a suggested replacement — check the primary span first,
        // then the first child span.
        let (suggested_replacement, applicability) = if primary.suggested_replacement.is_some() {
            (
                primary.suggested_replacement.clone(),
                parse_applicability(primary.suggestion_applicability.as_deref()),
            )
        } else {
            let child_span = diag
                .children
                .iter()
                .flat_map(|c| c.spans.iter())
                .find(|s| s.suggested_replacement.is_some());
            (
                child_span.and_then(|s| s.suggested_replacement.clone()),
                child_span
                    .map(|s| parse_applicability(s.suggestion_applicability.as_deref()))
                    .unwrap_or_default(),
            )
        };

        findings.push(ClippyFinding {
            lint: code,
            file: PathBuf::from(&primary.file_name),
            line_start: primary.line_start,
            line_end: primary.line_end,
            byte_start: primary.byte_start,
            byte_end: primary.byte_end,
            message: diag.message,
            rendered: diag.rendered.unwrap_or_default(),
            applicability,
            suggested_replacement,
        });
    }

    findings
}

fn parse_applicability(raw: Option<&str>) -> SuggestionApplicability {
    match raw {
        Some("MachineApplicable") => SuggestionApplicability::MachineApplicable,
        Some("MaybeIncorrect") => SuggestionApplicability::MaybeIncorrect,
        Some("HasPlaceholders") => SuggestionApplicability::HasPlaceholders,
        _ => SuggestionApplicability::Unspecified,
    }
}

// ─── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"reason":"compiler-message","package_id":"lab-core","manifest_path":"/ws/Cargo.toml","target":{"kind":["lib"],"name":"lab-core"},"message":{"$message_type":"diagnostic","message":"this can be `std::io::Error::other(_)`","code":{"code":"clippy::io_other_error","explanation":null},"level":"warning","spans":[{"file_name":"src/config.rs","is_primary":true,"line_start":786,"line_end":786,"byte_start":24200,"byte_end":24248,"suggested_replacement":".map_err(std::io::Error::other)?","suggestion_applicability":"MachineApplicable"}],"children":[],"rendered":"warning: this can be...\n"}}"#;

    #[test]
    fn parses_machine_applicable_finding() {
        let findings = parse_output(SAMPLE.as_bytes());
        assert_eq!(findings.len(), 1);
        let f = &findings[0];
        assert_eq!(f.lint, "clippy::io_other_error");
        assert_eq!(f.file, PathBuf::from("src/config.rs"));
        assert_eq!(f.line_start, 786);
        assert_eq!(f.applicability, SuggestionApplicability::MachineApplicable);
        assert!(f.suggested_replacement.is_some());
    }

    #[test]
    fn skips_non_clippy_codes() {
        let non_clippy = r#"{"reason":"compiler-message","message":{"$message_type":"diagnostic","message":"unused variable","code":{"code":"unused_variables","explanation":null},"level":"warning","spans":[{"file_name":"src/main.rs","is_primary":true,"line_start":10,"line_end":10,"byte_start":null,"byte_end":null}],"children":[],"rendered":""}}"#;
        assert!(parse_output(non_clippy.as_bytes()).is_empty());
    }

    #[test]
    fn skips_errors() {
        let err = r#"{"reason":"compiler-message","message":{"$message_type":"diagnostic","message":"cannot find","code":{"code":"clippy::some_lint","explanation":null},"level":"error","spans":[{"file_name":"src/main.rs","is_primary":true,"line_start":1,"line_end":1,"byte_start":null,"byte_end":null}],"children":[],"rendered":""}}"#;
        assert!(parse_output(err.as_bytes()).is_empty());
    }
}
