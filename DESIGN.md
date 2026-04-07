# AI Research Lab — Rust Implementation Design

> Full parity design document mapping the Python AI Research Lab (`~/research/ai-research-lab`) to its Rust port (`~/archived/ai-research-lab-rust`).

---

## 1. Dependency Graph Analysis

The entire system forms a DAG with clear layers. Understanding this is critical because no crate can depend on a sibling crate that in turn depends back on it.

```
layer 6: lab-api (REST)           → lab-core, lab-memory, lab-tools
layer 5: lab-api (REST, optional)  → lab-core, lab-memory, lab-tools, lab-agents
layer 4: lab-cli                   → lab-core, lab-memory, lab-tools, lab-permissions
layer 4: lab-reports               → lab-core (execution types only)
layer 4: lab-pipelines             → lab-core (types, tasks), lab-agents, lab-memory
layer 4: lab-agents                → lab-core (types only), lab-tools, lab-memory, lab-permissions
       │
layer 3: lab-core                  → lab-memory, lab-tools, lab-permissions
layer 3: lab-tools                 → (std + tokio, walkdir, globset, regex)
layer 3: lab-memory                → (std + tokio, serde, serde_json)
layer 3: lab-permissions           → (std, serde, serde_json)
       │
layer 2: Cargo.toml workspace      → all crates share workspace deps
       │
layer 1: Rust stdlib + crate.io deps
```

**Key insight from dependency graph:** The Python version has a circular dependency problem that it solves via runtime imports and `TYPE_CHECKING` guards — for example, `agents/framework/base.py` imports from `core/lab/engine.py` and vice versa. In Rust we MUST break this cycle.

**The cycle break strategy:** `lab-core` defines all shared types (AgentSpec, SessionSpec, TaskSpec, etc.) as plain data structs. `lab-agents` depends on `lab-core` but `lab-core` does NOT depend on `lab-agents`. The `ResearchEngine` in `lab-core` accepts agents as trait objects (`Box<dyn Agent>`) rather than concrete types.

---

## 2. Current State Assessment

### What already exists (Rust stubs)

| Crate | Status | Completeness | Notes |
|-------|--------|-------------|-------|
| `lab-core` | Partial scaffolding | 15% | `config.rs` (basic only), `types.rs` (simple), `engine.rs` (3-phase research loop exists but is naive — no session management, no event bus, no task queue, no workflows) |
| `lab-memory` | Partial scaffolding | 10% | `MemoryStore` is just `Vec<Finding>` with keyword search — needs full `MemoryWorkspace` with TTL, scopes, vector search, FS persistence, pluggable backends |
| `lab-tools` | Partial scaffolding | 60% | ToolRegistry + 5 built-in tools (Read/Write/Bash/Glob/Grep) implemented well — but needs tool schema, validation |
| `lab-permissions` | Partial scaffolding | 45% | RBAC with 4 restriction levels, audit log, tool categories — works but missing rate limiting, workspace boundary checks, escalation policy |
| `lab-cli` | Working prototype | 40% | 7 CLI commands (init/run/memory/tools/list/clear/status) functional but only uses the naive engine |
| `lab-agents` | MISSING | 0% | Needs new crate |
| `lab-pipelines` | MISSING | 0% | Needs new crate |
| `lab-reports` | MISSING | 0% | Needs new crate |
| `lab-api` | MISSING | 0% | Needs new crate (optional, future) |

### Python source inventory (what needs to be ported)

| Python Module | Lines | Key Concepts |
|--------------|-------|-------------|
| `core/lab/config.py` | 274 | LabConfig, AgentProfile, PipelineConfig, PermissionPolicy, env overrides, YAML/JSON save/load, `__post_init__` directory creation |
| `core/lab/engine.py` | 460 | ResearchLab — session management, agent lifecycle, tool execution with RBAC, pipeline orchestration, workflow execution, memory access, event bus integration, stats |
| `core/memory/workspace.py` | 554 | MemoryWorkspace — KV store with 3 scopes (session/project/global), TTL, TF-IDF vector search (256-dim word-hash embeddings), cosine similarity, optional sentence-transformers embedder, FS persistence, capacity eviction |
| `core/memory/store.py` | ~80 | Simple KV JSON store (subsumed by workspace) |
| `core/memory/backends.py` | 234 | MemoryBackend trait + FilesystemBackend, InMemoryBackend, RedisBackend, factory function |
| `core/events/bus.py` | 200 | EventBus — async pub/sub, pattern matching (`agent.*`, `*.*`), priority ordering, error isolation, event history, `wait_for` with timeout |
| `core/scheduler/models.py` | 111 | TaskSpec, TaskPriority (4 levels), TaskStatus (7 states), TaskType (6 types) |
| `core/scheduler/queue.py` | 480 | TaskQueue — priority heap, dependency resolution, concurrent workers, retry with exponential backoff, delayed scheduling, recurring tasks, session-aware |
| `core/workflows/engine.py` | 730 | WorkflowEngine — DAG-based workflow, conditional branching (step_success, output_contains, custom fn), parallel execution, WorkflowTemplates (research-pipeline, code-review, analysis), WorkflowExecution history |
| `core/llm/client.py` | 301 | LLM client factory — OpenAI-compatible (OpenRouter/local), Anthropic native, retry with exponential backoff, ChatResponse |
| `core/sessions/store.py` | 425 | SessionStore — disk-backed session persistence, index.json for fast lookup, query with filters, restore from disk, purge, aggregate statistics |
| `agents/framework/base.py` | 292 | BaseAgent — lifecycle (start/pause/resume/cancel/cleanup), permission-aware tool access, memory read/write, event emission, metrics tracking, LLM integration (ask_llm with heuristic fallback) |
| `agents/framework/concrete.py` | 337 | ResearcherAgent, CoderAgent, ReviewerAgent, SummarizerAgent — each discovers files, uses tools, writes memory, runs analysis, generates output |
| `agents/framework/llm_agents.py` | ~200 | LLM-enhanced versions of above agents with LLM → heuristic fallback |
| `agents/communication.py` | 247 | AgentCommunicator — AgentMessage types (TASK/RESULT/QUERY/RESPONSE/NOTIFY/ERROR), per-agent mailbox (async queue with history), session-scoped pub/sub with direct/routed/broadcast delivery |
| `agents/collaborator.py` | 252 | MultiAgentCollaborator — runs Research => Review => Code => Summarize pipeline, configurable phases, memory-passing between agents |
| `tools/registry.py` | 359 | ToolRegistry — BaseTool abstract class, ToolSchema, ToolResult, 5 built-ins (Read/Write/Bash/Glob/Grep with path traversal protection), execution log |
| `permissions/rbac.py` | 250 | PermissionEngine — 5 restriction levels, agent-specific policies, audit trail, tool categorization |
| `pipelines/research/engine.py` | 594 | ResearchPipeline — 7-stage pipeline (discover → research → analyze → review → code → summarize → report), parallel execution, LLM enhancement, retry, result aggregation |
| `reports/workflow_report.py` | ~200 | HTML report generator with SVG charts, DAG visualization, status badges |

---

## 3. Crate Architecture (Final)

### 3.1 Workspace Cargo.toml

```toml
[workspace]
members = [
    "crates/lab-core",
    "crates/lab-memory",
    "crates/lab-tools",
    "crates/lab-permissions",
    "crates/lab-agents",
    "crates/lab-pipelines",
    "crates/lab-reports",
    "crates/lab-cli",
    # "crates/lab-api",  # future
]
resolver = "2"

[workspace.package]
version = "0.1.0"
edition = "2021"
rust-version = "1.75"

[workspace.dependencies]
lab-core = { path = "crates/lab-core" }
lab-memory = { path = "crates/lab-memory" }
lab-tools = { path = "crates/lab-tools" }
lab-permissions = { path = "crates/lab-permissions" }
lab-agents = { path = "crates/lab-agents" }
lab-pipelines = { path = "crates/lab-pipelines" }
lab-reports = { path = "crates/lab-reports" }
anyhow = "1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1", features = ["full"] }
clap = { version = "4", features = ["derive"] }
uuid = { version = "1", features = ["v4"] }
tracing = "0.1"
tracing-subscriber = "0.3"
thiserror = "2"
globset = "0.4"
regex = "1"
chrono = { version = "0.4", features = ["serde"] }
async-trait = "0.1"
colored = "2"
walkdir = "2"
futures = "0.3"
toml = "0.8"
serde_yaml = "0.9"
# Future LLM deps
reqwest = { version = "0.12", features = ["json"] }
# Future API deps
# axum = "0.7"
# tower = "0.5"
```

### 3.2 Crate Details

---

#### CRATE: lab-core (THE HUB)

**Dependencies:** lab-memory, lab-tools, lab-permissions, tokio, serde, serde_json, uuid, thiserror, tracing, async-trait, futures, chrono, regex

**Module layout:**
```
crates/lab-core/src/
├── lib.rs           — re-exports
├── config.rs        — REWRITE: Full LabConfig, AgentProfile, PipelineConfig, PermissionPolicy
├── engine.rs        — REWRITE: Full ResearchLab orchestrator (replace current naive ResearchEngine)
├── types.rs         — EXTEND: LabSession, AgentSpec, enums (AgentState, TaskPriority, TaskStatus, TaskType)
├── events/
│   ├── mod.rs
│   └── bus.rs       — NEW: EventBus (async pub/sub, pattern matching, history)
├── scheduler/
│   ├── mod.rs
│   ├── models.rs    — NEW: TaskSpec, TaskPriority, TaskStatus, TaskType
│   └── queue.rs     — NEW: TaskQueue (priority heap, workers, deps, retry, recurring)
├── workflows/
│   ├── mod.rs
│   └── engine.rs    — NEW: Workflow, WorkflowEngine, conditions, templates
├── sessions/
│   ├── mod.rs
│   └── store.rs     — NEW: SessionStore (disk persistence, query, restore)
├── llm/
│   ├── mod.rs
│   └── client.rs    — NEW: LLM client (OpenAI-compatible + Anthropic, retry)
└── errors.rs        — NEW: LabError enum (all error types)
```

**Key types to add to `config.rs`:**

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LabConfig {
    // workspace paths
    pub workspace: PathBuf,
    pub sessions_dir: PathBuf,       // DEFAULT: "lab-sessions"
    pub memory_dir: PathBuf,         // DEFAULT: "lab-memory"
    pub outputs_dir: PathBuf,        // DEFAULT: "lab-outputs"
    pub skills_dir: PathBuf,         // DEFAULT: "lab-skills"
    pub audits_dir: PathBuf,         // DEFAULT: "lab-audits"
    pub cache_dir: PathBuf,          // DEFAULT: "lab-cache"
    // runtime
    pub max_agents: usize,           // DEFAULT: 20
    pub max_concurrent_tasks: usize, // DEFAULT: 10
    pub default_timeout_secs: u64,   // DEFAULT: 300
    pub debug_mode: bool,
    pub verbose_logging: bool,
    pub enable_profiling: bool,
    // permission
    pub permission_policy: PermissionPolicyConfig,
    // agent
    pub default_agent_profile: AgentProfile,
    pub agent_profiles: HashMap<String, AgentProfile>,
    // pipeline
    pub pipeline_defaults: PipelineConfig,
    // LLM
    pub provider: String,            // "openrouter" | "anthropic" | "local"
    pub base_url: String,
    pub model: String,
    pub api_key: String,
    pub fallback_models: Vec<String>,
    // AI toggle
    pub use_ai: bool,
    // memory
    pub memory_persistence: bool,
    pub memory_backend: String,      // "filesystem" | "memory" | "redis"
    pub memory_max_entries: usize,
    pub memory_ttl_seconds: u64,
    // web
    pub web_search_engine: String,
    pub web_rate_limit: usize,
    pub web_timeout_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentProfile {
    pub name: String,
    pub role: String,
    pub max_context_tokens: usize,
    pub temperature: f64,
    pub top_p: f64,
    pub allowed_tools: Vec<String>,
    pub forbidden_tools: Vec<String>,
    pub max_iterations: usize,
    pub auto_approve: bool,
    pub permission_level: String,    // "read" | "standard" | "write" | "admin" | "super-admin"
    pub output_format: String,       // "markdown" | "json" | "text"
    pub verbosity: String,           // "quiet" | "normal" | "verbose"
    pub timeout_seconds: u64,
    pub concurrent_tasks: usize,
    pub retry_limit: usize,
    pub memory_scope: String,        // "session" | "project" | "global"
    pub custom_instructions: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineConfig {
    pub name: String,
    pub stages: Vec<String>,
    pub max_concurrent_stages: usize,
    pub fail_fast: bool,
    pub retry_on_failure: bool,
    pub timeout_per_stage_secs: u64,
    pub output_path: Option<PathBuf>,
    pub input_targets: Vec<String>,
    pub exclude_patterns: Vec<String>,
    pub custom_params: HashMap<String, serde_json::Value>,
}
```

**Key types to add to `types.rs`:**

```rust
// LabSession — mirrors LabSession from Python engine.py
pub struct LabSession {
    pub id: String,
    pub name: String,
    pub started_at: DateTime<Utc>,
    pub agents_active: usize,
    pub tasks_completed: usize,
    pub tasks_failed: usize,
    pub total_tokens_used: usize,
    pub status: SessionStatus,  // Active | Paused | Completed | Failed
    pub artifacts: Vec<String>,
    pub metadata: HashMap<String, serde_json::Value>,
}

// Agent trait — replaces concrete dependency cycle
#[async_trait]
pub trait Agent: Send + Sync {
    fn id(&self) -> &str;
    fn name(&self) -> &str;
    fn session_id(&self) -> &str;
    fn profile(&self) -> &AgentProfile;
    async fn start(&self) -> Result<AgentResult>;
    async fn run_task(&self, task: &str, kwargs: HashMap<String, serde_json::Value>) -> Result<AgentResult>;
    async fn cancel(&self);
    async fn cleanup(&self);
    fn state(&self) -> AgentState;  // Initialized | Running | Paused | Waiting | Completed | Failed | Cancelled
}

// AgentResult — mirrors AgentResult from Python base.py
pub struct AgentResult {
    pub success: bool,
    pub data: serde_json::Value,
    pub error: Option<String>,
    pub metrics: AgentMetrics,
    pub artifacts: Vec<String>,
    pub reasoning_trace: Vec<String>,
}

pub struct AgentMetrics {
    pub tool_calls: usize,
    pub tool_errors: usize,
    pub permission_checks: usize,
    pub permission_denials: usize,
    pub memory_reads: usize,
    pub memory_writes: usize,
    pub tokens_used: usize,
    pub iteration_count: usize,
    pub start_time: Option<Instant>,
    pub end_time: Option<Instant>,
}
```

**ResearchLab (engine.rs) — the master orchestrator:**

```rust
pub struct ResearchLab {
    pub config: LabConfig,
    // sessions
    sessions: HashMap<String, LabSession>,
    // agents — stored as trait objects to break the dependency cycle
    agent_registry: HashMap<String, HashMap<String, Box<dyn Agent>>>,  // session_id -> agent_id -> agent
    // subsystems
    tool_registry: ToolRegistry,
    permission_engine: PermissionEngine,
    memory: MemoryWorkspace,
    event_bus: EventBus,
    session_store: SessionStore,
    // optional
    task_queue: Option<Arc<Mutex<TaskQueue>>>,
    llm_client: Option<Box<dyn LLMClient>>,
    // config collections
    pipeline_configs: HashMap<String, PipelineConfig>,
    workflow_history: Vec<WorkflowExecution>,
    operation_counts: OperationCounts,
    running: bool,
    start_time: Option<Instant>,
}

impl ResearchLab {
    // Session management
    pub async fn create_session(&mut self, name: &str) -> Result<&LabSession>;
    pub async fn close_session(&mut self, session_id: &str) -> Result<()>;
    pub fn list_sessions(&self) -> Vec<&LabSession>;
    pub fn get_session(&self, session_id: &str) -> Option<&LabSession>;

    // Agent lifecycle
    pub fn register_agent(&mut self, session_id: &str, agent: Box<dyn Agent>);
    pub fn get_agent(&self, session_id: &str, agent_id: &str) -> Option<&dyn Agent>;
    pub fn get_session_agents(&self, session_id: &str) -> Vec<&dyn Agent>;
    pub async fn remove_agent(&mut self, session_id: &str, agent_id: &str) -> Result<()>;

    // Tool access (permission-aware)
    pub async fn check_permission(&mut self, agent_id: &str, tool_name: &str, params: &ToolParams) -> PermissionResult;
    pub async fn execute_tool(&mut self, tool_name: &str, params: ToolParams, agent_id: Option<&str>) -> ToolResult;

    // Pipeline management
    pub fn register_pipeline(&mut self, config: PipelineConfig);
    pub async fn run_pipeline(&mut self, pipeline_name: &str, targets: Vec<String>) -> Result<PipelineResult>;

    // Workflow execution
    pub async fn run_workflow(&mut self, workflow: Workflow, session_id: Option<&str>) -> Result<WorkflowExecution>;
    pub async fn run_workflow_from_template(&mut self, template: &str, name: &str, session_id: Option<&str>) -> Result<WorkflowExecution>;

    // Lifecycle
    pub async fn start(&mut self) -> Result<()>;
    pub async fn shutdown(&mut self) -> Result<()>;
    pub fn get_stats(&self) -> LabStats;
}
```

**EventBus (events/bus.rs):**

```rust
pub struct LabEvent {
    pub event_type: String,         // e.g., "agent.started", "tool.executed"
    pub data: HashMap<String, Value>,
    pub timestamp: Instant,
    pub source: String,
    pub priority: EventPriority,    // Low | Normal | High | Critical
}

pub struct EventBus {
    // pattern -> list of (handler, once, priority)
    subscribers: HashMap<String, Vec<Subscriber>>,
    history: VecDeque<LabEvent>,    // capped at max_history
    max_history: usize,
}

impl EventBus {
    pub fn subscribe(&mut self, pattern: &str, handler: Box<dyn Fn(LabEvent) + Send>);
    pub fn subscribe_all(&mut self, handler: Box<dyn Fn(LabEvent) + Send>);
    pub async fn emit(&mut self, event: LabEvent) -> Result<()>;
    pub async fn wait_for(&mut self, pattern: &str, timeout: Duration) -> Option<LabEvent>;
    pub fn get_history(&self, filter: Option<&str>, limit: usize) -> Vec<LabEvent>;
}
```

**TaskQueue (scheduler/queue.rs):**

```rust
pub struct TaskQueue {
    lab: Arc<ResearchLab>,
    queue: PriorityHeap,
    tasks: HashMap<String, TaskSpec>,
    running: HashSet<String>,
    completed: HashMap<String, TaskSpec>,
    max_workers: usize,
    semaphore: Semaphore,
}

impl TaskQueue {
    pub fn new(lab: Arc<ResearchLab>, max_workers: usize) -> Self;
    pub fn start(&mut self);  // spawn workers
    pub async fn submit(&mut self, task: TaskSpec) -> Result<String>;  // returns task_id
    pub async fn submit_many(&mut self, tasks: Vec<TaskSpec>) -> Vec<String>;
    pub async fn cancel(&mut self, task_id: &str) -> bool;
    pub async fn wait_for(&self, task_id: &str, timeout: Option<Duration>) -> Option<Value>;
    pub fn get_task(&self, task_id: &str) -> Option<&TaskSpec>;
    pub fn list_tasks(&self, filters: TaskFilter) -> Vec<&TaskSpec>;
    pub fn get_pending_count(&self) -> usize;
    pub fn get_stats(&self) -> TaskQueueStats;
    pub async fn stop(&mut self);
}
```

**WorkflowEngine (workflows/engine.rs):**

```rust
pub struct WorkflowStep {
    pub id: String,
    pub name: String,
    pub task_type: TaskType,
    pub task_string: String,
    pub depends_on: Vec<String>,
    pub condition: Option<Condition>,
    pub skip_on: Vec<StepOutcome>,
}

pub struct Workflow {
    pub id: String,
    pub name: String,
    pub steps: HashMap<String, WorkflowStep>,
    pub timeout_secs: u64,
}

pub struct WorkflowEngine {
    queue: Arc<Mutex<TaskQueue>>,
}

impl WorkflowEngine {
    pub fn new(queue: Arc<Mutex<TaskQueue>>) -> Self;
    pub async fn execute(&self, workflow: &Workflow, session_id: Option<&str>) -> Result<WorkflowExecution>;
}

// Templates like "research-pipeline", "code-review", "analysis"
pub struct TemplateRegistry {
    templates: HashMap<String, WorkflowTemplate>,
}
```

---

#### CRATE: lab-memory

**Dependencies:** serde, serde_json, tokio, thiserror, tracing, parking_lot (or tokio::sync::Mutex)

**Module layout:**
```
crates/lab-memory/src/
├── lib.rs
├── workspace.rs       — MemoryWorkspace (main API: put/get/delete/search/similarity_search)
├── entry.rs           — MemoryEntry dataclass equivalent
├── vector.rs          — TF-IDF + word-hash embedding (256-dim), cosine similarity
├── backends.rs        — MemoryBackend trait + FilesystemBackend + InMemoryBackend
└── persistence.rs     — FS persistence logic
```

**Key design:**

```rust
pub struct MemoryEntry {
    pub key: String,
    pub value: serde_json::Value,
    pub scope: MemoryScope,      // Session | Project | Global
    pub created_at: Instant,
    pub updated_at: Instant,
    pub ttl: Option<Duration>,
    pub tags: Vec<String>,
    pub metadata: HashMap<String, serde_json::Value>,
    pub source: String,          // agent_id or "system"
    pub vector: Option<Vec<f32>>, // TF-IDF embedding
}

pub struct MemoryWorkspace {
    workspace: PathBuf,
    stores: HashMap<String, HashMap<String, MemoryEntry>>,  // session_id -> {key: entry}
    global_store: HashMap<String, MemoryEntry>,
    vector_index: HashMap<String, MemoryEntry>,  // hash -> entry
    idf_cache: HashMap<String, f32>,
    vocabulary_dirty: bool,
    max_entries: usize,
    default_ttl: Duration,
    backend: Box<dyn MemoryBackend>,
}
```

---

#### CRATE: lab-tools (ALREADY 60% DONE)

**Dependencies:** tokio, globset, regex, thiserror, serde, serde_json, walkdir

**Module layout:**
```
crates/lab-tools/src/
├── lib.rs           — ToolRegistry + all builtin tools
├── registry.rs      — ToolRegistry (already done)
├── traits.rs        — Tool trait (already done)
├── result.rs        — ToolResult (already done)
├── read.rs          — ReadTool (already done)
├── write.rs         — WriteTool (already done)
├── bash.rs          — BashTool (already done)
├── glob.rs          — GlobTool (already done)
└── grep.rs          — GrepTool (already done)
```

**What's missing in lab-tools:**
- ToolSchema for introspection (Python has it)
- Parameter validation (Python inspects function signatures)
- Workspace boundary enforcement for write operations

---

#### CRATE: lab-permissions (ALREADY 45% DONE)

**Dependencies:** serde, serde_json, thiserror, tracing, chrono

**Module layout:**
```
crates/lab-permissions/src/
├── lib.rs
├── engine.rs        — PermissionEngine (already done)
├── policy.rs        — PermissionPolicy (already done)
├── levels.rs        — RestrictionLevel enum (already done)
├── categories.rs    — tool_category() mapping (already done)
├── audit.rs         — PermissionCheck records (already done)
└── rate_limiter.rs  — NEW: Rate limiting (60 ops/min from Python)
```

**What's missing in lab-permissions:**
- Rate limiting per agent
- Workspace boundary enforcement
- Escalation policy (ask-user/deny/fail-safe)
- Emergency stop tools

---

#### CRATE: lab-agents (NEW)

**Dependencies:** lab-core, lab-tools, lab-memory, lab-permissions, tokio, serde, thiserror, tracing, async-trait

**Module layout:**
```
crates/lab-agents/src/
├── lib.rs
├── base.rs          — Agent trait impl, BaseAgent concrete struct (lifecycle, tools, memory, metrics)
├── researcher.rs    — ResearcherAgent (finds files, reads content, builds structure map)
├── coder.rs         — CoderAgent (writes files, generates boilerplate)
├── reviewer.rs      — ReviewerAgent (static analysis, code quality checks)
├── summarizer.rs    — SummarizerAgent (reads memory, generates consolidated reports)
├── llm_agents.rs    — LLM-enhanced versions with fallback
├── communication.rs — AgentMailbox, AgentCommunicator (inter-agent messaging)
├── collaborator.rs  — MultiAgentCollaborator (orchestrates multi-phase workflows)
└── metrics.rs       — AgentMetrics tracking
```

**Agent trait (bridges lab-core and lab-agents):**

```rust
// In lab-core: trait definition
#[async_trait]
pub trait Agent: Send + Sync {
    fn id(&self) -> &str;
    fn session_id(&self) -> &str;
    fn profile(&self) -> &AgentProfile;
    async fn start(&mut self) -> Result<AgentResult>;
    async fn run_task(&mut self, task: &str, kwargs: HashMap<String, Value>) -> Result<AgentResult>;
    async fn cancel(&mut self);
    async fn cleanup(&mut self);
    fn state(&self) -> AgentState;
}

// In lab-agents: concrete implementation
pub struct ConcreteAgent {
    id: String,
    session_id: String,
    profile: AgentProfile,
    state: AgentState,
    metrics: AgentMetrics,
    lab: Arc<Mutex<ResearchLab>>,
    cancel_token: CancellationToken,
    name: String,
    role: String,
    task_fn: Box<dyn Fn(...) -> ...>,  // the actual task logic
}

impl Agent for ConcreteAgent { ... }
```

---

#### CRATE: lab-pipelines (NEW)

**Dependencies:** lab-core, lab-agents, tokio, serde, thiserror, tracing, futures, chrono

**Module layout:**
```
crates/lab-pipelines/src/
├── lib.rs
├── engine.rs        — ResearchPipeline (7-stage: discover → research → analyze → review → code → summarize → report)
├── stage.rs         — StageResult, StageStatus, stage execution
├── config.rs        — PipelineConfig (mostly in lab-core, but pipeline-specific extras here)
└── result.rs        — PipelineResult, summary
```

---

#### CRATE: lab-reports (NEW)

**Dependencies:** serde, serde_json, askama (or minijinja for HTML templating)

**Module layout:**
```
crates/lab-reports/src/
├── lib.rs
├── workflow.rs      — WorkflowReportGenerator (HTML + SVG DAG visualization)
├── pipeline.rs      — PipelineReportGenerator
└── templates/       — HTML report templates
```

---

#### CRATE: lab-cli (NEEDS REWRITE)

**Dependencies:** lab-core, lab-memory, lab-tools, lab-permissions, lab-agents, lab-pipelines, tokio, clap, colored, tracing-subscriber

The current CLI uses the naive `ResearchEngine`. Needs to be updated to use the full `ResearchLab` once all crates are complete. The 7 commands (init/run/memory/tools/list/clear/status) should be extended to include:

```
lab session create --name "code-review"
lab session list
lab session close <id>
lab agent spawn --name "researcher" --session "xyz"
lab task submit --type research --priority high --session "xyz"
lab workflow run --template "research-pipeline" --session "xyz"
lab pipeline run --name "code-review" --targets src/ tests/
lab permission set --agent "abc" --level "read"
lab report generate --workflow-id "xyz"
```

---

## 4. Implementation Phases

### Phase 1: Foundation (Weeks 1-2)

**Goal:** Complete core data types, config, memory, and tools

| Task | Crate | Effort | Description |
|------|-------|--------|-------------|
| 1.1 Rewrite `config.rs` | lab-core | 2 days | Full LabConfig, AgentProfile, PipelineConfig, PermissionPolicy with env overrides and file load/save |
| 1.2 Extend `types.rs` | lab-core | 1 day | LabSession, AgentState, OperationCounts, LabStats, Agent trait |
| 1.3 Rewrite `engine.rs` | lab-core | 3 days | Full ResearchLab orchestrator (sessions, agents, tools, memory, events, RBAC integration) |
| 1.4 Rewrite `lab-memory` | lab-memory | 3 days | MemoryWorkspace with scopes, TTL, vector search, FS persistence, capacity eviction |
| 1.5 Add vector.rs | lab-memory | 1 day | TF-IDF word-hash embedding (256-dim), cosine similarity — optional sentence-transformers via optional dependency |
| 1.6 Add backends.rs | lab-memory | 1 day | MemoryBackend trait + FilesystemBackend + InMemoryBackend |
| 1.7 Split lab-tools | lab-tools | 1 day | Refactor single lib.rs into separate modules per tool (already 60% done, just organization) |
| 1.8 Complete lab-permissions | lab-permissions | 1 day | Add rate limiting, workspace boundary, escalation policy |

### Phase 2: Async Systems (Weeks 3-4)

**Goal:** Event bus, task queue, scheduler, session persistence

| Task | Crate | Effort | Description |
|------|-------|--------|-------------|
| 2.1 EventBus | lab-core | 2 days | Async pub/sub, pattern matching, priority, history, wait_for |
| 2.2 TaskQueue | lab-core | 3 days | Priority heap, concurrent workers, dependency resolution, retry, recurring |
| 2.3 SessionStore | lab-core | 1 day | Disk persistence, index.json, query with filters, restore, purge |
| 2.4 LLM Client | lab-core | 2 days | LLMClient trait + OpenAI-compatible + Anthropic + retry |
| 2.5 Integrate EventBus | lab-core | 1 day | Wire EventBus into ResearchLab (emit events on session/agent/tool/pipeline actions) |

### Phase 3: Agent Framework (Weeks 5-6)

**Goal:** Concrete agents, communication, collaborator

| Task | Crate | Effort | Description |
|------|-------|--------|-------------|
| 3.1 New crate: lab-agents | lab-agents | 0.5 days | Cargo.toml, lib.rs, module structure |
| 3.2 BaseAgent | lab-agents | 2 days | ConcreteAgent implementing Agent trait — lifecycle, tool access (permission-aware), memory, metrics, LLM integration |
| 3.3 ResearcherAgent | lab-agents | 1 day | File discovery, code structure analysis |
| 3.4 CoderAgent | lab-agents | 1 day | File writing, boilerplate generation |
| 3.5 ReviewerAgent | lab-agents | 1 day | Static analysis, code quality checks |
| 3.6 SummarizerAgent | lab-agents | 1 day | Memory consolidation, report generation |
| 3.7 AgentMailbox + Communicator | lab-agents | 2 days | Inter-agent async messaging, direct/routed/broadcast delivery |
| 3.8 MultiAgentCollaborator | lab-agents | 2 days | Orchestrates Research => Review => Code => Summarize pipeline |
| 3.9 LLM-enhanced agents | lab-agents | 2 days | Agents with LLM primary + heuristic fallback |
| 3.10 Wire agents into ResearchLab | lab-core + lab-agents | 1 day | ResearchLab.create_agent() returns concrete agents |

### Phase 4: Orchestrators (Weeks 7-8)

**Goal:** Workflow engine, pipelines, reports

| Task | Crate | Effort | Description |
|------|-------|--------|-------------|
| 4.1 New crate: lab-workflows (part of lab-core or separate) | lab-core | 0.5 days | Workflow, WorkflowStep, WorkflowEngine, conditions, DAG execution |
| 4.2 Workflow templates | lab-core | 1 day | research-pipeline, code-review, analysis templates |
| 4.3 New crate: lab-pipelines | lab-pipelines | 0.5 days | Cargo.toml, lib.rs |
| 4.4 ResearchPipeline | lab-pipelines | 3 days | 7-stage pipeline with parallel stages, LLM enhancement, retry, aggregation |
| 4.5 New crate: lab-reports | lab-reports | 0.5 days | Cargo.toml, lib.rs, HTML templates |
| 4.6 WorkflowReportGenerator | lab-reports | 2 days | HTML report with SVG DAG visualization, status badges |
| 4.7 Wire pipelines into ResearchLab | lab-core + lab-pipelines | 1 day | ResearchLab.run_pipeline() delegates to ResearchPipeline |
| 4.8 Wire workflows into ResearchLab | lab-core | 1 day | ResearchLab.run_workflow() delegates to WorkflowEngine |

### Phase 5: Interfaces (Weeks 9-10)

**Goal:** CLI rewrite, optional REST API

| Task | Crate | Effort | Description |
|------|-------|--------|-------------|
| 5.1 Rewrite lab-cli | lab-cli | 3 days | Full CLI with all lab operations, extended commands |
| 5.2 Integration tests | all | 3 days | End-to-end tests across crates |
| 5.3 lab-api (optional) | lab-api | 5 days | Axum REST API + WebSocket event streaming |
| 5.4 Documentation | all | 2 days | rustdoc, examples, README |

---

## 5. Rust-Specific Design Decisions

### 5.1 Breaking the Cycle: The Agent Trait

Python solves circular dependencies at import time with lazy imports. Rust has no such escape. The solution:

```
lab-core defines:    trait Agent { ... }
lab-agents defines:  struct ConcreteAgent { ... } impl Agent for ConcreteAgent
lab-core uses:       Box<dyn Agent> (no knowledge of ConcreteAgent)
```

This way the dependency arrow only goes one direction: `lab-agents → lab-core`, never back.

### 5.2 Async vs Sync

Python is fully async (`asyncio`). The Rust port must also be async (tokio) because:
- Tools execute external commands (bash needs async process)
- LLM calls are network I/O
- EventBus needs async fan-out
- TaskQueue needs async workers

The only exception: config loading and simple data structure manipulation can be sync.

### 5.3 Error Handling

Python uses `Result<...>` with string errors and exception catching. Rust should use `thiserror`:

```rust
#[derive(Debug, thiserror::Error)]
pub enum LabError {
    #[error("Session not found: {0}")]
    SessionNotFound(String),
    #[error("Permission denied: {reason}")]
    PermissionDenied { reason: String },
    #[error("Tool execution failed: {tool}: {error}")]
    ToolError { tool: String, error: String },
    #[error("Agent error: {0}")]
    AgentError(String),
    #[error("Task timeout after {0}s")]
    TaskTimeout(u64),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}
```

### 5.4 Memory Safety for EventBus Subscribers

Python uses `Callable` that can be garbage collected. In Rust, subscriber lifetimes must be carefully managed. Options:
- Use `Arc<dyn Fn + Send>` for shared ownership
- Use `tokio::sync::broadcast` for simpler pub/sub (loses pattern matching)
- Use weak references for subscriber cleanup

Recommended: `Arc<dyn Fn(LabEvent) + Send + Sync>` for subscriber handlers.

### 5.5 Vector Search in Rust

Python uses TF-IDF with word-hash embeddings (256-dim). In Rust:
- Use `md5` crate for word hashing (same as Python's `hashlib.md5`)
- Pre-compute IDF scores lazily
- Cosine similarity is just dot product since vectors are L2-normalized
- NO heavy ML dependencies — keep it pure Rust math

For production-grade semantic search, add optional `candle-transformers` or `ort` dependency for sentence-transformers.

---

## 6. Testing Strategy

| Layer | Approach | Coverage Target |
|-------|----------|----------------|
| Unit | `cargo test` per crate | 80%+ per module |
| Integration | Tests that wire multiple crates together | Core orchestrator flows |
| Property-based | `proptest` for vector math, pattern matching | Edge cases |
| Benchmark | `criterion` for search, queue ops | Python parity or better |
| E2E | End-to-end test via CLI with temp workspace | Full pipeline run |

---

## 7. What NOT to Port (Yet)

These Python features are deferred:
1. **Redis memory backend** — optional, add later
2. **Gradio UI** (`lab/gradio_ui.py`) — the Python has a Gradio dashboard, but Rust will use REST API instead
3. **FastAPI REST API** — deferred to Phase 5 (optional)
4. **Docker setup** — not needed for CLI-first approach
5. **CI/CD workflows** — set up separately
6. **Sentence-transformers optional dependency** — TF-IDF is sufficient for v1

---

## 8. Migration Parity Matrix

| Python Feature | Rust Equivalent | Status | Effort |
|----------------|----------------|--------|--------|
| LabConfig + env overrides | LabConfig + env | NOT DONE | 2 days |
| AgentProfile | AgentProfile | NOT DONE | 0.5 days |
| PipelineConfig | PipelineConfig | NOT DONE | 0.5 days |
| LabSession | LabSession | PARTIAL | 1 day |
| ResearchLab orchestrator | ResearchLab | NOT DONE | 3 days |
| EventBus | EventBus | NOT DONE | 2 days |
| TaskQueue | TaskQueue | NOT DONE | 3 days |
| MemoryWorkspace (TF-IDF) | MemoryWorkspace | NOT DONE | 3 days |
| Memory backends | MemoryBackend trait | NOT DONE | 1 day |
| Agent trait + lifecycle | Agent trait | NOT DONE | 2 days |
| ResearcherAgent | ResearcherAgent | NOT DONE | 1 day |
| CoderAgent | CoderAgent | NOT DONE | 1 day |
| ReviewerAgent | ReviewerAgent | NOT DONE | 1 day |
| SummarizerAgent | SummarizerAgent | NOT DONE | 1 day |
| LLM agents (fallback) | LLMAgent | NOT DONE | 2 days |
| AgentMailbox/Communicator | AgentMailbox | NOT DONE | 2 days |
| MultiAgentCollaborator | MultiAgentCollaborator | NOT DONE | 2 days |
| ToolRegistry + 5 tools | ToolRegistry | 60% DONE | 1 day |
| RBAC PermissionEngine | PermissionEngine | 45% DONE | 1 day |
| ResearchPipeline (7-stage) | ResearchPipeline | NOT DONE | 3 days |
| WorkflowEngine (DAG) | WorkflowEngine | NOT DONE | 3 days |
| WorkflowTemplates | TemplateRegistry | NOT DONE | 1 day |
| HTML Reports | ReportGenerator | NOT DONE | 2 days |
| SessionStore (disk) | SessionStore | NOT DONE | 1 day |
| LLM Client | LLMClient | NOT DONE | 2 days |
| CLI (7 commands) | CLI | 40% DONE | 3 days |

---

## 9. File Count Estimate

| Crate | Estimated Files | Estimated Lines |
|-------|----------------|-----------------|
| lab-core | 12 files | ~2,500 lines |
| lab-memory | 5 files | ~1,200 lines |
| lab-tools | 8 files (split from 1) | ~600 lines (mostly done) |
| lab-permissions | 5 files (split from 1) | ~350 lines |
| lab-agents | 10 files | ~2,000 lines |
| lab-pipelines | 4 files | ~1,000 lines |
| lab-reports | 3 files | ~600 lines |
| lab-cli | 2 files | ~500 lines |
| **Total** | **~49 files** | **~8,750 lines** |

This compares to Python's ~64 files and ~8,000+ lines. Rust will be slightly more verbose due to type annotations and error handling but achieves the same functionality with compile-time safety.
