//! Risk classification for clippy findings.
//!
//! Each finding is classified into one of four tiers:
//!
//! - `AutoFix`      — safe to apply mechanically via `cargo clippy --fix`
//! - `Patch`        — LLM outputs a unified diff, applied via `diffy`
//! - `LLMEdit`      — LLM replaces the compiler-indicated byte range
//! - `HumanReview`  — too risky/complex for automated handling

use super::clippy_runner::{ClippyFinding, SuggestionApplicability};

// ─── RiskLevel ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RiskLevel {
    /// cargo clippy --fix can apply the change with no LLM needed.
    AutoFix,
    /// LLM generates a unified diff; `diffy` applies it to the source file.
    Patch,
    /// LLM replaces the exact byte range the compiler pointed at.
    LLMEdit,
    /// Requires human review — automated changes would be too risky.
    HumanReview,
}

impl std::fmt::Display for RiskLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AutoFix => write!(f, "auto_fix"),
            Self::Patch => write!(f, "patch"),
            Self::LLMEdit => write!(f, "llm_edit"),
            Self::HumanReview => write!(f, "human_review"),
        }
    }
}

// ─── Classification ─────────────────────────────────────────────────

/// Classify a clippy finding into a `RiskLevel`.
///
/// Decision logic (in order of priority):
///
/// 1. If the compiler supplied a `MachineApplicable` replacement and the lint
///    is in the `AUTO_FIX_LINTS` set → `AutoFix`.
/// 2. If the compiler supplied any replacement (even `MaybeIncorrect`) → `Patch`.
/// 3. If there's a known byte range to target → `LLMEdit`.
/// 4. Otherwise → `HumanReview`.
pub fn classify(finding: &ClippyFinding) -> RiskLevel {
    if finding.applicability == SuggestionApplicability::MachineApplicable
        && AUTO_FIX_LINTS.contains(&finding.lint.as_str())
    {
        return RiskLevel::AutoFix;
    }

    if finding.suggested_replacement.is_some()
        && !matches!(finding.applicability, SuggestionApplicability::HasPlaceholders)
    {
        return RiskLevel::Patch;
    }

    if finding.byte_start.is_some() && finding.byte_end.is_some() {
        return RiskLevel::LLMEdit;
    }

    RiskLevel::HumanReview
}

// ─── Auto-fix lint allow-list ────────────────────────────────────────

/// Lints that are safe to apply mechanically via `cargo clippy --fix`.
///
/// These have `MachineApplicable` suggestions that:
/// - change only style/idioms, not semantics
/// - are reverting-safe (can always be undone with git)
const AUTO_FIX_LINTS: &[&str] = &[
    "clippy::derivable_impls",
    "clippy::manual_map",
    "clippy::io_other_error",
    "clippy::iter_overeager_cloned",
    "clippy::question_mark",
    "clippy::useless_format",
    "clippy::collapsible_str_replace",
    "clippy::redundant_closure_for_method_calls",
    "clippy::needless_pass_by_ref_mut",
    "clippy::single_match_else",
    "clippy::match_like_matches_macro",
    "clippy::needless_bool",
    "clippy::bool_assert_comparison",
    "clippy::cloned_instead_of_copied",
    "clippy::map_unwrap_or",
    "clippy::implicit_clone",
    "clippy::uninlined_format_args",
    "clippy::manual_string_new",
    "clippy::manual_let_else",
    "clippy::manual_ok_or",
    "clippy::range_contains",
    "clippy::redundant_field_names",
    "clippy::struct_update_syntax",
];

// ─── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::improvement::clippy_runner::ClippyFinding;
    use std::path::PathBuf;

    fn finding(lint: &str, applicability: SuggestionApplicability, replacement: Option<&str>, byte_start: Option<u32>) -> ClippyFinding {
        ClippyFinding {
            lint: lint.to_string(),
            file: PathBuf::from("src/lib.rs"),
            line_start: 10,
            line_end: 10,
            byte_start,
            byte_end: byte_start.map(|s| s + 20),
            message: "test".to_string(),
            rendered: String::new(),
            applicability,
            suggested_replacement: replacement.map(str::to_string),
        }
    }

    #[test]
    fn machine_applicable_auto_fix_lint_is_auto_fix() {
        let f = finding(
            "clippy::derivable_impls",
            SuggestionApplicability::MachineApplicable,
            Some("#[derive(Default)]"),
            Some(100),
        );
        assert_eq!(classify(&f), RiskLevel::AutoFix);
    }

    #[test]
    fn maybe_incorrect_becomes_patch() {
        let f = finding(
            "clippy::derivable_impls",
            SuggestionApplicability::MaybeIncorrect,
            Some("some replacement"),
            Some(100),
        );
        assert_eq!(classify(&f), RiskLevel::Patch);
    }

    #[test]
    fn machine_applicable_unknown_lint_is_patch() {
        let f = finding(
            "clippy::some_new_lint",
            SuggestionApplicability::MachineApplicable,
            Some("replacement"),
            None,
        );
        assert_eq!(classify(&f), RiskLevel::Patch);
    }

    #[test]
    fn no_replacement_with_byte_range_is_llm_edit() {
        let f = finding(
            "clippy::too_many_arguments",
            SuggestionApplicability::Unspecified,
            None,
            Some(500),
        );
        assert_eq!(classify(&f), RiskLevel::LLMEdit);
    }

    #[test]
    fn no_replacement_no_range_is_human_review() {
        let f = finding(
            "clippy::type_complexity",
            SuggestionApplicability::Unspecified,
            None,
            None,
        );
        assert_eq!(classify(&f), RiskLevel::HumanReview);
    }
}
