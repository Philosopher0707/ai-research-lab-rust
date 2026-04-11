//! Full lab configuration: LabConfig, AgentProfile, PipelineConfig, PermissionPolicyConfig.
//! Mirrors core/lab/config.py (274 lines) with env override + TOML/YAML load/save.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Once;

// ─── Defaults ──────────────────────────────────────────────────────

fn default_workspace() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}
fn def_sessions() -> PathBuf {
    PathBuf::from("lab-sessions")
}
fn def_memory() -> PathBuf {
    PathBuf::from("lab-memory")
}
fn def_outputs() -> PathBuf {
    PathBuf::from("lab-outputs")
}
fn def_skills() -> PathBuf {
    PathBuf::from("lab-skills")
}
fn def_audits() -> PathBuf {
    PathBuf::from("lab-audits")
}
fn def_cache() -> PathBuf {
    PathBuf::from("lab-cache")
}
fn def_max_agents() -> usize {
    20
}
fn def_max_tasks() -> usize {
    10
}
fn def_timeout() -> u64 {
    300
}
fn def_provider() -> String {
    "openrouter".into()
}
fn def_base_url() -> String {
    "https://openrouter.ai/api/v1".into()
}
fn def_model() -> String {
    "anthropic/claude-sonnet-4-20250514".into()
}
fn def_memory_backend() -> String {
    "filesystem".into()
}
fn def_memory_max() -> usize {
    10000
}
fn def_memory_ttl() -> u64 {
    86400 * 7
}
fn def_web_engine() -> String {
    "default".into()
}
fn def_web_rate() -> usize {
    10
}
fn def_web_timeout() -> u64 {
    30
}

fn resolve_initial_workspace() -> PathBuf {
    env_non_empty("LAB_WORKSPACE")
        .map(PathBuf::from)
        .unwrap_or_else(default_workspace)
}

static ENV_INIT: Once = Once::new();

fn env_non_empty(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn provider_api_env_key(provider: &str) -> Option<&'static str> {
    match provider {
        "anthropic" => Some("ANTHROPIC_API_KEY"),
        "openai" => Some("OPENAI_API_KEY"),
        "openrouter" => Some("OPENROUTER_API_KEY"),
        "deepseek" => Some("DEEPSEEK_API_KEY"),
        "zhipu" => Some("ZHIPU_API_KEY"),
        "minimax" => Some("MINIMAX_API_KEY"),
        "xai" => Some("XAI_API_KEY"),
        _ => None,
    }
}

fn provider_default_base_url(provider: &str) -> &'static str {
    match provider {
        "anthropic" => "https://api.anthropic.com",
        "openai" => "https://api.openai.com/v1",
        "openrouter" => "https://openrouter.ai/api/v1",
        "deepseek" => "https://api.deepseek.com/v1",
        "zhipu" => "https://open.bigmodel.cn/api/paas/v4",
        "minimax" => "https://api.minimax.chat/v1",
        "xai" => "https://api.x.ai/v1",
        "local" => "http://localhost:11434/v1",
        _ => "https://openrouter.ai/api/v1",
    }
}

fn provider_default_model(provider: &str) -> &'static str {
    match provider {
        "anthropic" => "claude-sonnet-4-6",
        "openai" => "gpt-4o",
        "openrouter" => "anthropic/claude-sonnet-4-5",
        "deepseek" => "deepseek-chat",
        "zhipu" => "glm-4-flash",
        "minimax" => "MiniMax-Text-01",
        "xai" => "grok-2",
        "local" => "llama3.2",
        _ => "anthropic/claude-sonnet-4-20250514",
    }
}

fn provider_supports_keyless_access(provider: &str) -> bool {
    provider == "local"
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeProfile {
    #[default]
    Development,
    Production,
    Test,
    Benchmark,
}

impl RuntimeProfile {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Development => "development",
            Self::Production => "production",
            Self::Test => "test",
            Self::Benchmark => "benchmark",
        }
    }

    pub fn from_env() -> Option<Self> {
        env_non_empty("LAB_PROFILE").and_then(|value| value.parse().ok())
    }
}

impl FromStr for RuntimeProfile {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match value.trim().to_lowercase().as_str() {
            "development" | "dev" => Ok(Self::Development),
            "production" | "prod" => Ok(Self::Production),
            "test" => Ok(Self::Test),
            "benchmark" | "bench" => Ok(Self::Benchmark),
            other => Err(format!("unsupported runtime profile: {other}")),
        }
    }
}

fn load_env_file(path: &Path) {
    if path.exists() {
        let _ = dotenvy::from_path_override(path);
    }
}

fn load_env_stack(workspace: &Path) {
    load_env_file(&workspace.join(".env"));

    let profile = env_non_empty("LAB_PROFILE").unwrap_or_else(|| "development".to_string());
    load_env_file(&workspace.join(format!(".env.{profile}")));
    load_env_file(&workspace.join(".env.local"));
    load_env_file(&workspace.join(format!(".env.{profile}.local")));
}

fn detect_provider_from_env() -> Option<&'static str> {
    for provider in [
        "anthropic",
        "openai",
        "openrouter",
        "deepseek",
        "zhipu",
        "minimax",
        "xai",
    ] {
        if let Some(env_key) = provider_api_env_key(provider) {
            if env_non_empty(env_key).is_some() {
                return Some(provider);
            }
        }
    }

    if env_non_empty("LAB_BASE_URL").is_some() {
        return Some("local");
    }

    None
}

// ─── AgentProfile ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentProfile {
    pub name: String,
    pub role: String,
    #[serde(default = "def_max_tokens")]
    pub max_context_tokens: usize,
    #[serde(default = "def_temperature")]
    pub temperature: f64,
    #[serde(default = "def_top_p")]
    pub top_p: f64,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub forbidden_tools: Vec<String>,
    #[serde(default = "def_max_iter")]
    pub max_iterations: usize,
    #[serde(default)]
    pub auto_approve: bool,
    #[serde(default = "def_perm_level")]
    pub permission_level: String,
    #[serde(default = "def_output_fmt")]
    pub output_format: String,
    #[serde(default = "def_verbosity")]
    pub verbosity: String,
    #[serde(default = "def_agent_timeout")]
    pub timeout_seconds: u64,
    #[serde(default = "def_concurrent")]
    pub concurrent_tasks: usize,
    #[serde(default = "def_retry")]
    pub retry_limit: usize,
    #[serde(default = "def_mem_scope")]
    pub memory_scope: String,
    #[serde(default)]
    pub custom_instructions: String,
}

fn def_max_tokens() -> usize {
    32000
}
fn def_temperature() -> f64 {
    0.3
}
fn def_top_p() -> f64 {
    0.9
}
fn def_max_iter() -> usize {
    50
}
fn def_perm_level() -> String {
    "standard".into()
}
fn def_output_fmt() -> String {
    "markdown".into()
}
fn def_verbosity() -> String {
    "normal".into()
}
fn def_agent_timeout() -> u64 {
    300
}
fn def_concurrent() -> usize {
    3
}
fn def_retry() -> usize {
    3
}
fn def_mem_scope() -> String {
    "session".into()
}

impl Default for AgentProfile {
    fn default() -> Self {
        Self {
            name: String::new(),
            role: String::new(),
            max_context_tokens: def_max_tokens(),
            temperature: def_temperature(),
            top_p: def_top_p(),
            allowed_tools: Vec::new(),
            forbidden_tools: Vec::new(),
            max_iterations: def_max_iter(),
            auto_approve: false,
            permission_level: def_perm_level(),
            output_format: def_output_fmt(),
            verbosity: def_verbosity(),
            timeout_seconds: def_agent_timeout(),
            concurrent_tasks: def_concurrent(),
            retry_limit: def_retry(),
            memory_scope: def_mem_scope(),
            custom_instructions: String::new(),
        }
    }
}

// ─── PipelineConfig ────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineConfig {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub stages: Vec<String>,
    #[serde(default = "def_max_concurrent_stages")]
    pub max_concurrent_stages: usize,
    #[serde(default = "def_true")]
    pub fail_fast: bool,
    #[serde(default)]
    pub retry_on_failure: bool,
    #[serde(default = "def_stage_timeout")]
    pub timeout_per_stage_secs: u64,
    pub output_path: Option<PathBuf>,
    #[serde(default)]
    pub input_targets: Vec<String>,
    #[serde(default)]
    pub exclude_patterns: Vec<String>,
    #[serde(default)]
    pub custom_params: HashMap<String, serde_json::Value>,
}

fn def_max_concurrent_stages() -> usize {
    5
}
fn def_stage_timeout() -> u64 {
    1800
}
fn def_true() -> bool {
    true
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            stages: Vec::new(),
            max_concurrent_stages: def_max_concurrent_stages(),
            fail_fast: def_true(),
            retry_on_failure: false,
            timeout_per_stage_secs: def_stage_timeout(),
            output_path: None,
            input_targets: Vec::new(),
            exclude_patterns: Vec::new(),
            custom_params: HashMap::new(),
        }
    }
}

impl PipelineConfig {
    pub fn copy(&self) -> Self {
        self.clone()
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    pub fn with_stages(mut self, stages: Vec<String>) -> Self {
        self.stages = stages;
        self
    }
}

// ─── PermissionPolicyConfig ────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionPolicyConfig {
    #[serde(default = "def_restrict_level")]
    pub default_restriction_level: String,
    pub require_approval_tools: Vec<String>,
    pub auto_approve_tools: Vec<String>,
    pub forbidden_tools: Vec<String>,
    #[serde(default = "def_max_concurrent_ops")]
    pub max_concurrent_operations: usize,
    #[serde(default = "def_true")]
    pub workspace_boundary_enforcement: bool,
    #[serde(default = "def_true")]
    pub allow_network: bool,
    #[serde(default)]
    pub allow_file_deletion: bool,
    #[serde(default)]
    pub allow_symlink_creation: bool,
    #[serde(default = "def_true")]
    pub audit_all_operations: bool,
    #[serde(default = "def_rate_limit")]
    pub rate_limit_per_minute: usize,
    #[serde(default = "def_escalation")]
    pub escalation_policy: String,
    #[serde(default = "def_true")]
    pub admin_override: bool,
    #[serde(default)]
    pub emergency_stop_tools: Vec<String>,
}

fn def_restrict_level() -> String {
    "standard".into()
}
fn def_max_concurrent_ops() -> usize {
    10
}
fn def_rate_limit() -> usize {
    60
}
fn def_escalation() -> String {
    "ask-user".into()
}

impl Default for PermissionPolicyConfig {
    fn default() -> Self {
        Self {
            default_restriction_level: def_restrict_level(),
            require_approval_tools: vec!["bash".into(), "write_file".into(), "edit_file".into()],
            auto_approve_tools: vec![
                "read_file".into(),
                "glob_search".into(),
                "grep_search".into(),
            ],
            forbidden_tools: vec![],
            max_concurrent_operations: def_max_concurrent_ops(),
            workspace_boundary_enforcement: def_true(),
            allow_network: def_true(),
            allow_file_deletion: false,
            allow_symlink_creation: false,
            audit_all_operations: def_true(),
            rate_limit_per_minute: def_rate_limit(),
            escalation_policy: def_escalation(),
            admin_override: def_true(),
            emergency_stop_tools: Vec::new(),
        }
    }
}

// ─── TOML file config (all optional) ────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
struct LabConfigFile {
    workspace: Option<String>,
    runtime_profile: Option<RuntimeProfile>,
    sessions_dir: Option<String>,
    memory_dir: Option<String>,
    outputs_dir: Option<String>,
    skills_dir: Option<String>,
    audits_dir: Option<String>,
    cache_dir: Option<String>,
    max_agents: Option<usize>,
    max_concurrent_tasks: Option<usize>,
    default_timeout: Option<u64>,
    debug_mode: Option<bool>,
    verbose_logging: Option<bool>,
    enable_profiling: Option<bool>,
    provider: Option<String>,
    base_url: Option<String>,
    model: Option<String>,
    api_key: Option<String>,
    use_ai: Option<bool>,
    memory_persistence: Option<bool>,
    memory_backend: Option<String>,
    memory_max_entries: Option<usize>,
    memory_ttl_seconds: Option<u64>,
    web_search_engine: Option<String>,
    web_rate_limit: Option<usize>,
    web_timeout_seconds: Option<u64>,
}

// ─── LabConfig ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LabConfig {
    // Workspace paths
    #[serde(default = "default_workspace")]
    pub workspace: PathBuf,
    #[serde(default)]
    pub runtime_profile: RuntimeProfile,
    #[serde(default = "def_sessions")]
    pub sessions_dir: PathBuf,
    #[serde(default = "def_memory")]
    pub memory_dir: PathBuf,
    #[serde(default = "def_outputs")]
    pub outputs_dir: PathBuf,
    #[serde(default = "def_skills")]
    pub skills_dir: PathBuf,
    #[serde(default = "def_audits")]
    pub audits_dir: PathBuf,
    #[serde(default = "def_cache")]
    pub cache_dir: PathBuf,

    // Runtime settings
    #[serde(default = "def_max_agents")]
    pub max_agents: usize,
    #[serde(default = "def_max_tasks")]
    pub max_concurrent_tasks: usize,
    #[serde(default = "def_timeout")]
    pub default_timeout: u64,
    #[serde(default)]
    pub debug_mode: bool,
    #[serde(default)]
    pub verbose_logging: bool,
    #[serde(default)]
    pub enable_profiling: bool,

    // Permission
    #[serde(default)]
    pub permission_policy: PermissionPolicyConfig,

    // Agents
    #[serde(default)]
    pub default_agent_profile: AgentProfile,
    #[serde(default)]
    pub agent_profiles: HashMap<String, AgentProfile>,

    // Pipelines
    #[serde(default)]
    pub pipeline_defaults: PipelineConfig,

    // API / Provider
    #[serde(default = "def_provider")]
    pub provider: String,
    #[serde(default = "def_base_url")]
    pub base_url: String,
    #[serde(default = "def_model")]
    pub model: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub fallback_models: Vec<String>,

    // AI Pipeline toggle
    #[serde(default)]
    pub use_ai: bool,

    // Memory
    #[serde(default = "def_true")]
    pub memory_persistence: bool,
    #[serde(default = "def_memory_backend")]
    pub memory_backend: String,
    #[serde(default = "def_memory_max")]
    pub memory_max_entries: usize,
    #[serde(default = "def_memory_ttl")]
    pub memory_ttl_seconds: u64,

    // Web
    #[serde(default = "def_web_engine")]
    pub web_search_engine: String,
    #[serde(default = "def_web_rate")]
    pub web_rate_limit: usize,
    #[serde(default = "def_web_timeout")]
    pub web_timeout_secs: u64,
}

impl Default for LabConfig {
    fn default() -> Self {
        Self::initialize_process_env();
        Self::build_config(
            resolve_initial_workspace(),
            RuntimeProfile::from_env().unwrap_or_default(),
            false,
            false,
        )
    }
}

impl LabConfig {
    fn build_config(
        workspace: PathBuf,
        runtime_profile: RuntimeProfile,
        lock_workspace: bool,
        lock_profile: bool,
    ) -> Self {
        let mut cfg = Self {
            workspace: workspace.clone(),
            runtime_profile,
            sessions_dir: def_sessions(),
            memory_dir: def_memory(),
            outputs_dir: def_outputs(),
            skills_dir: def_skills(),
            audits_dir: def_audits(),
            cache_dir: def_cache(),
            max_agents: def_max_agents(),
            max_concurrent_tasks: def_max_tasks(),
            default_timeout: def_timeout(),
            debug_mode: false,
            verbose_logging: false,
            enable_profiling: false,
            permission_policy: PermissionPolicyConfig::default(),
            default_agent_profile: AgentProfile::default(),
            agent_profiles: HashMap::new(),
            pipeline_defaults: PipelineConfig::default(),
            provider: def_provider(),
            base_url: def_base_url(),
            model: def_model(),
            api_key: String::new(),
            fallback_models: Vec::new(),
            use_ai: false,
            memory_persistence: def_true(),
            memory_backend: def_memory_backend(),
            memory_max_entries: def_memory_max(),
            memory_ttl_seconds: def_memory_ttl(),
            web_search_engine: def_web_engine(),
            web_rate_limit: def_web_rate(),
            web_timeout_secs: def_web_timeout(),
        };

        cfg.load_layered_files();
        cfg.apply_env_overrides();

        if lock_workspace {
            cfg.workspace = workspace;
        }
        if lock_profile {
            cfg.runtime_profile = runtime_profile;
        }

        cfg.ensure_directories();
        cfg
    }

    pub fn initialize_process_env() {
        ENV_INIT.call_once(|| {
            let cwd = default_workspace();
            load_env_stack(&cwd);

            if let Some(workspace) = env_non_empty("LAB_WORKSPACE") {
                let workspace = PathBuf::from(workspace);
                if workspace != cwd {
                    load_env_stack(&workspace);
                }
            }
        });
    }

    /// Create config with a specific workspace.
    pub fn with_workspace(path: PathBuf) -> Self {
        Self::initialize_process_env();
        Self::build_config(
            path,
            RuntimeProfile::from_env().unwrap_or_default(),
            true,
            false,
        )
    }

    /// Create config with a specific runtime profile.
    pub fn with_profile(profile: RuntimeProfile) -> Self {
        Self::initialize_process_env();
        Self::build_config(resolve_initial_workspace(), profile, false, true)
    }

    /// Create config with an explicit workspace and runtime profile.
    pub fn with_workspace_and_profile(path: PathBuf, profile: RuntimeProfile) -> Self {
        Self::initialize_process_env();
        Self::build_config(path, profile, true, true)
    }

    /// Apply LAB_* environment variable overrides.
    /// Priority: LAB_PROVIDER (explicit) > auto-detect from API key env vars.
    fn apply_env_overrides(&mut self) {
        if let Some(profile) = RuntimeProfile::from_env() {
            self.runtime_profile = profile;
        }

        let explicit_provider = env_non_empty("LAB_PROVIDER");
        let explicit_model = env_non_empty("LAB_MODEL");
        let explicit_base_url = env_non_empty("LAB_BASE_URL");

        if let Some(provider) = explicit_provider {
            self.provider = provider;
        } else if let Some(provider) = detect_provider_from_env() {
            self.provider = provider.to_string();
        }

        // Only apply env / provider defaults when the value hasn't been set by a
        // config file.  File-loaded values take priority over derived defaults;
        // explicit env vars (LAB_MODEL / LAB_BASE_URL) always win.
        if let Some(model) = explicit_model {
            self.model = model;
        } else if self.model == def_model() {
            self.model = provider_default_model(&self.provider).to_string();
        }

        if let Some(base_url) = explicit_base_url {
            self.base_url = base_url;
        } else if self.base_url == def_base_url() {
            self.base_url = provider_default_base_url(&self.provider).to_string();
        }
        self.api_key = env_non_empty("LAB_API_KEY")
            .or_else(|| provider_api_env_key(&self.provider).and_then(env_non_empty))
            .unwrap_or_default();

        if let Ok(v) = std::env::var("LAB_DEBUG") {
            self.debug_mode = v == "true" || v == "1";
        }
        if let Ok(v) = std::env::var("LAB_VERBOSE") {
            self.verbose_logging = v == "true" || v == "1";
        }
        if let Ok(v) = std::env::var("LAB_WORKSPACE") {
            self.workspace = PathBuf::from(v);
        }
        if let Ok(v) = std::env::var("LAB_MAX_AGENTS") {
            self.max_agents = v.parse().unwrap_or(self.max_agents);
        }
    }

    fn load_layered_files(&mut self) {
        let base_path = self.workspace.join("lab-config.toml");
        self.load_optional_file(&base_path);

        let profile = self.runtime_profile.as_str().to_string();
        let profile_path = self.workspace.join(format!("lab-config.{profile}.toml"));
        self.load_optional_file(&profile_path);

        let local_path = self.workspace.join("lab-config.local.toml");
        self.load_optional_file(&local_path);

        let profile_local_path = self
            .workspace
            .join(format!("lab-config.{profile}.local.toml"));
        self.load_optional_file(&profile_local_path);
    }

    fn load_optional_file(&mut self, path: &Path) {
        if !path.exists() {
            return;
        }

        let Ok(contents) = std::fs::read_to_string(path) else {
            return;
        };
        let Ok(file_cfg) = toml::from_str::<LabConfigFile>(&contents) else {
            return;
        };
        self.apply_from_file(&file_cfg);
    }

    pub fn requires_api_key(&self) -> bool {
        !provider_supports_keyless_access(&self.provider)
    }

    pub fn llm_configured(&self) -> bool {
        provider_supports_keyless_access(&self.provider) || !self.api_key.is_empty()
    }

    pub fn expected_api_key_env(&self) -> Option<&'static str> {
        provider_api_env_key(&self.provider)
    }

    /// Ensure all required directories exist.
    pub fn ensure_directories(&self) {
        for dir in &[
            &self.sessions_dir,
            &self.memory_dir,
            &self.outputs_dir,
            &self.skills_dir,
            &self.audits_dir,
            &self.cache_dir,
        ] {
            let full: PathBuf = if dir.is_absolute() {
                (*dir).clone()
            } else {
                self.workspace.join(*dir)
            };
            let _ = std::fs::create_dir_all(&full);
        }
    }

    /// Get the full path for a relative directory.
    pub fn full_path(&self, dir: &PathBuf) -> PathBuf {
        if dir.is_absolute() {
            dir.clone()
        } else {
            self.workspace.join(dir)
        }
    }

    /// Get an agent profile by name, falling back to default.
    pub fn get_agent_profile(&self, name: &str) -> &AgentProfile {
        self.agent_profiles
            .get(name)
            .unwrap_or(&self.default_agent_profile)
    }

    /// Save configuration to a TOML file.
    pub fn save(&self, path: Option<&PathBuf>) -> std::io::Result<()> {
        let save_path = path
            .cloned()
            .unwrap_or_else(|| self.workspace.join("lab-config.toml"));
        let content = toml::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        if let Some(parent) = save_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&save_path, content)
    }

    /// Load configuration from a TOML file.
    pub fn load(&mut self, path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
        if !path.exists() {
            return Err(format!("Config file not found: {}", path.display()).into());
        }
        let contents = std::fs::read_to_string(path)?;
        let file_cfg: LabConfigFile = toml::from_str(&contents)?;
        self.apply_from_file(&file_cfg);
        self.ensure_directories();
        Ok(())
    }

    fn apply_from_file(&mut self, file: &LabConfigFile) {
        if let Some(v) = &file.workspace {
            self.workspace = PathBuf::from(v);
        }
        if let Some(v) = file.runtime_profile {
            self.runtime_profile = v;
        }
        if let Some(v) = &file.sessions_dir {
            self.sessions_dir = PathBuf::from(v);
        }
        if let Some(v) = &file.memory_dir {
            self.memory_dir = PathBuf::from(v);
        }
        if let Some(v) = &file.outputs_dir {
            self.outputs_dir = PathBuf::from(v);
        }
        if let Some(v) = &file.skills_dir {
            self.skills_dir = PathBuf::from(v);
        }
        if let Some(v) = &file.audits_dir {
            self.audits_dir = PathBuf::from(v);
        }
        if let Some(v) = &file.cache_dir {
            self.cache_dir = PathBuf::from(v);
        }
        if let Some(v) = file.max_agents {
            self.max_agents = v;
        }
        if let Some(v) = file.max_concurrent_tasks {
            self.max_concurrent_tasks = v;
        }
        if let Some(v) = file.default_timeout {
            self.default_timeout = v;
        }
        if let Some(v) = file.debug_mode {
            self.debug_mode = v;
        }
        if let Some(v) = file.verbose_logging {
            self.verbose_logging = v;
        }
        if let Some(v) = file.enable_profiling {
            self.enable_profiling = v;
        }
        if let Some(ref v) = file.provider {
            self.provider = v.clone();
        }
        if let Some(ref v) = file.base_url {
            self.base_url = v.clone();
        }
        if let Some(ref v) = file.model {
            self.model = v.clone();
        }
        if let Some(ref v) = file.api_key {
            self.api_key = v.clone();
        }
        if let Some(v) = file.use_ai {
            self.use_ai = v;
        }
        if let Some(v) = file.memory_persistence {
            self.memory_persistence = v;
        }
        if let Some(ref v) = file.memory_backend {
            self.memory_backend = v.clone();
        }
        if let Some(v) = file.memory_max_entries {
            self.memory_max_entries = v;
        }
        if let Some(v) = file.memory_ttl_seconds {
            self.memory_ttl_seconds = v;
        }
        if let Some(ref v) = file.web_search_engine {
            self.web_search_engine = v.clone();
        }
        if let Some(v) = file.web_rate_limit {
            self.web_rate_limit = v;
        }
        if let Some(v) = file.web_timeout_seconds {
            self.web_timeout_secs = v;
        }
    }

    /// Convert to a serializable dict (for JSON output).
    pub fn to_dict(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

/// Create a default config.
pub fn create_default_config(workspace: Option<PathBuf>) -> LabConfig {
    match workspace {
        Some(ws) => LabConfig::with_workspace(ws),
        None => LabConfig::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Temporarily blanks a LAB_* env var for the duration of the guard.
    /// Setting to "" is enough because `env_non_empty` treats empty as unset.
    struct EnvBlankGuard {
        key: &'static str,
        prev: Option<String>,
    }

    impl EnvBlankGuard {
        fn blank(key: &'static str) -> Self {
            let prev = std::env::var(key).ok();
            std::env::set_var(key, "");
            Self { key, prev }
        }
    }

    impl Drop for EnvBlankGuard {
        fn drop(&mut self) {
            match &self.prev {
                Some(v) => std::env::set_var(self.key, v),
                None => std::env::remove_var(self.key),
            }
        }
    }

    /// Mutex that serialises every test touching global env vars.
    /// Unit tests run in parallel by default; without serialisation the blank
    /// guards from one test can race with the restore of another.
    static ENV_TEST_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Blank every env var that `apply_env_overrides` or `detect_provider_from_env`
    /// inspects. Call at the start of any test that asserts on provider/model/base_url
    /// default values to prevent ambient env state from interfering.
    fn blank_provider_env() -> Vec<EnvBlankGuard> {
        [
            "LAB_PROVIDER",
            "LAB_MODEL",
            "LAB_BASE_URL",
            "LAB_API_KEY",
            "ANTHROPIC_API_KEY",
            "OPENAI_API_KEY",
            "OPENROUTER_API_KEY",
            "DEEPSEEK_API_KEY",
            "ZHIPU_API_KEY",
            "MINIMAX_API_KEY",
            "XAI_API_KEY",
        ]
        .iter()
        .map(|k| EnvBlankGuard::blank(k))
        .collect()
    }

    #[test]
    fn default_config_has_sane_values() {
        let _lock = ENV_TEST_MUTEX.lock().unwrap();
        LabConfig::initialize_process_env();
        let _guards = blank_provider_env();
        let cfg = LabConfig::default();
        assert_eq!(cfg.max_agents, 20);
        assert_eq!(cfg.max_concurrent_tasks, 10);
        assert_eq!(cfg.default_timeout, 300);
        assert_eq!(cfg.provider, "openrouter");
        assert!(cfg.memory_persistence);
        assert_eq!(cfg.memory_backend, "filesystem");
    }

    #[test]
    fn explicit_profile_loads_profile_specific_file() {
        let _lock = ENV_TEST_MUTEX.lock().unwrap();
        // Pre-fire ENV_INIT so .env is already loaded before we blank vars.
        // The constructor's initialize_process_env() will then be a no-op.
        LabConfig::initialize_process_env();
        let _guards = blank_provider_env();

        let temp_dir = tempfile::tempdir().unwrap();
        std::fs::write(
            temp_dir.path().join("lab-config.toml"),
            "model = 'base-model'\nverbose_logging = false\n",
        )
        .unwrap();
        std::fs::write(
            temp_dir.path().join("lab-config.production.toml"),
            "model = 'prod-model'\nverbose_logging = true\n",
        )
        .unwrap();

        let cfg = LabConfig::with_workspace_and_profile(
            temp_dir.path().to_path_buf(),
            RuntimeProfile::Production,
        );

        assert_eq!(cfg.runtime_profile, RuntimeProfile::Production);
        assert_eq!(cfg.model, "prod-model");
        assert!(cfg.verbose_logging);
    }

    #[test]
    fn agent_profile_defaults() {
        let p = AgentProfile::default();
        assert_eq!(p.max_context_tokens, 32000);
        assert!((p.temperature - 0.3).abs() < f64::EPSILON);
        assert_eq!(p.permission_level, "standard");
    }

    #[test]
    fn pipeline_config_defaults() {
        let p = PipelineConfig::default();
        assert_eq!(p.max_concurrent_stages, 5);
        assert!(p.fail_fast);
        assert!(!p.retry_on_failure);
    }
}
