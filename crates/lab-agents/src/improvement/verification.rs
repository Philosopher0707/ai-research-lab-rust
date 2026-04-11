//! Verification pipeline run after each automated edit.
//!
//! Stages (in order):
//!
//! 1. **`rustfmt` (corrective)** — formats the changed file in-place.
//!    Runs `rustfmt --edition 2021 <file>`. A formatting failure is a warning,
//!    not a hard error — the file is still checked by later stages.
//!
//! 2. **`cargo clippy` (vs baseline)** — counts new warnings introduced by the
//!    edit. If the edit made things worse, the run fails.
//!
//! 3. **`cargo check`** — confirms the workspace still compiles.
//!
//! 4. **`cargo test` (optional)** — only run when `run_tests` is set.

use std::path::{Path, PathBuf};

// ─── VerificationResult ─────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct VerificationResult {
    pub passed: bool,
    pub rustfmt_ok: bool,
    pub new_warnings: i32,
    pub cargo_check_ok: bool,
    pub tests_ok: Option<bool>,
    pub details: Vec<String>,
}

impl VerificationResult {
    pub fn failure(detail: impl Into<String>) -> Self {
        Self {
            passed: false,
            details: vec![detail.into()],
            ..Default::default()
        }
    }
}

impl Default for VerificationResult {
    fn default() -> Self {
        Self {
            passed: true,
            rustfmt_ok: true,
            new_warnings: 0,
            cargo_check_ok: true,
            tests_ok: None,
            details: Vec::new(),
        }
    }
}

// ─── VerificationChain ──────────────────────────────────────────────

/// Run rustfmt → clippy → cargo check → (optionally) cargo test.
pub struct VerificationChain {
    workspace: PathBuf,
    run_tests: bool,
    /// Baseline warning count from before the edit — used to detect regressions.
    baseline_warnings: usize,
}

impl VerificationChain {
    pub fn new(workspace: PathBuf, run_tests: bool, baseline_warnings: usize) -> Self {
        Self {
            workspace,
            run_tests,
            baseline_warnings,
        }
    }

    /// Snapshot the current clippy warning count to use as baseline.
    pub async fn snapshot_baseline(workspace: &Path) -> usize {
        let out = tokio::process::Command::new("cargo")
            .args([
                "clippy",
                "--message-format",
                "json",
                "--workspace",
                "--quiet",
            ])
            .current_dir(workspace)
            .output()
            .await
            .unwrap_or_else(|_| std::process::Output {
                status: std::process::ExitStatus::default(),
                stdout: Vec::new(),
                stderr: Vec::new(),
            });

        super::clippy_runner::parse_output(&out.stdout).len()
    }

    /// Run the full verification chain for a single edited file.
    ///
    /// `edited_file` is the workspace-relative path that was modified.
    pub async fn run(&self, edited_file: &Path) -> VerificationResult {
        let mut result = VerificationResult::default();

        // ── 1. rustfmt ──────────────────────────────────────────────
        let abs_file = self.workspace.join(edited_file);
        let fmt_ok = self.run_rustfmt(&abs_file).await;
        result.rustfmt_ok = fmt_ok;
        if !fmt_ok {
            result
                .details
                .push(format!("rustfmt: failed on {}", edited_file.display()));
            // Not a hard failure — formatting issues don't break compilation.
        }

        // ── 2. clippy (vs baseline) ──────────────────────────────────
        let current_warnings = self.count_clippy_warnings().await;
        let new_warnings = current_warnings as i32 - self.baseline_warnings as i32;
        result.new_warnings = new_warnings;
        if new_warnings > 0 {
            result.passed = false;
            result
                .details
                .push(format!("clippy: {new_warnings} new warning(s) introduced"));
        }

        // ── 3. cargo check ──────────────────────────────────────────
        let check_ok = self.run_cargo_check().await;
        result.cargo_check_ok = check_ok;
        if !check_ok {
            result.passed = false;
            result
                .details
                .push("cargo check: compilation failed".to_string());
            // If the code doesn't compile, skip tests.
            return result;
        }

        // ── 4. cargo test (optional) ────────────────────────────────
        if self.run_tests {
            let tests_ok = self.run_cargo_test().await;
            result.tests_ok = Some(tests_ok);
            if !tests_ok {
                result.passed = false;
                result
                    .details
                    .push("cargo test: test suite failed".to_string());
            }
        }

        result
    }

    // ─── Stage helpers ───────────────────────────────────────────────

    async fn run_rustfmt(&self, file: &Path) -> bool {
        tokio::process::Command::new("rustfmt")
            .args(["--edition", "2021", "--"])
            .arg(file)
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false)
    }

    async fn count_clippy_warnings(&self) -> usize {
        let out = tokio::process::Command::new("cargo")
            .args([
                "clippy",
                "--message-format",
                "json",
                "--workspace",
                "--quiet",
            ])
            .current_dir(&self.workspace)
            .output()
            .await
            .unwrap_or_else(|_| std::process::Output {
                status: std::process::ExitStatus::default(),
                stdout: Vec::new(),
                stderr: Vec::new(),
            });

        super::clippy_runner::parse_output(&out.stdout).len()
    }

    async fn run_cargo_check(&self) -> bool {
        tokio::process::Command::new("cargo")
            .args(["check", "--workspace", "--quiet"])
            .current_dir(&self.workspace)
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false)
    }

    async fn run_cargo_test(&self) -> bool {
        tokio::process::Command::new("cargo")
            .args(["test", "--workspace", "--quiet"])
            .current_dir(&self.workspace)
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

// ─── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_result_is_passing() {
        let r = VerificationResult::default();
        assert!(r.passed);
        assert!(r.rustfmt_ok);
        assert_eq!(r.new_warnings, 0);
        assert!(r.cargo_check_ok);
        assert!(r.tests_ok.is_none());
    }

    #[test]
    fn failure_constructor_sets_passed_false() {
        let r = VerificationResult::failure("something went wrong");
        assert!(!r.passed);
        assert_eq!(r.details.len(), 1);
    }
}
