//! Industry-level improvement architecture.
//!
//! Replaces the regex-based `ReviewerAgent` + exact-string `SelfEditAgent`
//! with a compiler-backed, risk-classified edit pipeline:
//!
//! ```text
//! cargo clippy --message-format json
//!         │
//!         ▼
//!   clippy_runner  →  Vec<ClippyFinding>  (file, lint, byte offsets, suggestion)
//!         │
//!         ▼
//!      risk       →  RiskLevel  (AutoFix | Patch | LLMEdit | HumanReview)
//!         │
//!         ▼
//!   edit_strategy  →  EditResult  (ClippyFixStrategy | DiffPatch | LLMEdit)
//!         │
//!         ▼
//!   verification   →  VerificationResult  (rustfmt → clippy Δ → check → test)
//! ```

pub mod clippy_runner;
pub mod edit_strategy;
pub mod risk;
pub mod verification;

pub use clippy_runner::{run_clippy, ClippyFinding, SuggestionApplicability};
pub use edit_strategy::{dispatch, ClippyFixStrategy, DiffPatchStrategy, EditResult, EditStrategy, LLMEditStrategy};
pub use risk::{classify, RiskLevel};
pub use verification::{VerificationChain, VerificationResult};
