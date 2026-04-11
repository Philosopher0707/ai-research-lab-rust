//! Workflow Engine — DAG-based workflow orchestration with conditional
//! branching, parallel execution, and templates.
//! Mirrors core/workflows/engine.py (730 lines — partial implementation)

use crate::errors::{LabError, Result};
use crate::scheduler::models::{TaskPriority, TaskType};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ─── StepStatus ────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    #[default]
    Pending,
    Queued,
    Running,
    Completed,
    Failed,
    Skipped,
    Cancelled,
    Timeout,
}

// ─── StepOutcome ───────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepOutcome {
    Success,
    Failure,
    ConditionFalse,
    Error,
}

// ─── Condition ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Condition {
    #[serde(rename = "type")]
    pub cond_type: String, // "step_success" | "output_contains" | "custom"
    pub step_id: Option<String>,
    pub value: Option<String>,
}

impl Condition {
    pub fn step_success(step_id: impl Into<String>) -> Self {
        Self {
            cond_type: "step_success".into(),
            step_id: Some(step_id.into()),
            value: None,
        }
    }

    pub fn output_contains(step_id: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            cond_type: "output_contains".into(),
            step_id: Some(step_id.into()),
            value: Some(value.into()),
        }
    }
}

// ─── WorkflowStep ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowStep {
    pub id: String,
    pub name: String,
    pub task_type: TaskType,
    pub task_string: String,
    pub depends_on: Vec<String>,
    pub condition: Option<Condition>,
    #[serde(default)]
    pub skip_on_failure: bool,
    pub timeout_secs: Option<u64>,
    pub priority: TaskPriority,
    pub session_id: Option<String>,
    #[serde(skip)]
    pub status: StepStatus,
    #[serde(skip)]
    pub outcome: Option<StepOutcome>,
    #[serde(skip)]
    pub result: Option<serde_json::Value>,
    #[serde(skip)]
    pub error: Option<String>,
}

// ─── Workflow ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workflow {
    pub id: String,
    pub name: String,
    pub steps: HashMap<String, WorkflowStep>,
    pub timeout_secs: Option<u64>,
    pub context: HashMap<String, serde_json::Value>,
}

impl Workflow {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string()[..8].to_string(),
            name: name.into(),
            steps: HashMap::new(),
            timeout_secs: None,
            context: HashMap::new(),
        }
    }

    /// Add a step to the workflow.
    pub fn add_step(
        &mut self,
        id: impl Into<String>,
        task_type: TaskType,
        task_string: impl Into<String>,
        depends_on: Vec<String>,
    ) {
        let step_id = id.into();
        self.steps.insert(
            step_id.clone(),
            WorkflowStep {
                id: step_id.clone(),
                name: step_id.clone(),
                task_type,
                task_string: task_string.into(),
                depends_on,
                condition: None,
                skip_on_failure: false,
                timeout_secs: None,
                priority: TaskPriority::Normal,
                session_id: None,
                status: StepStatus::Pending,
                outcome: None,
                result: None,
                error: None,
            },
        );
    }

    /// Add condition to a step.
    pub fn add_condition(&mut self, step_id: &str, condition: Condition) {
        if let Some(step) = self.steps.get_mut(step_id) {
            step.condition = Some(condition);
        }
    }

    /// Validate the workflow DAG (no cycles, all deps exist).
    pub fn validate(&self) -> Result<()> {
        // Check all dependencies exist
        for (step_id, step) in &self.steps {
            for dep in &step.depends_on {
                if !self.steps.contains_key(dep) {
                    return Err(LabError::WorkflowError(format!(
                        "Step '{}' depends on unknown step '{}'",
                        step_id, dep
                    )));
                }
            }
        }
        // Check for cycles (topological sort)
        self.topological_sort()?;
        Ok(())
    }

    /// Topological sort for cycle detection.
    fn topological_sort(&self) -> Result<Vec<String>> {
        // Standard Kahn's algorithm:
        //   in_degree[id] = number of direct prerequisites of `id`.
        //   Queue starts with nodes that have no prerequisites (sources).
        let mut in_degree: HashMap<&str, usize> = HashMap::new();
        for (id, step) in &self.steps {
            // Ensure every node appears in the map, even with degree 0.
            let entry = in_degree.entry(id.as_str()).or_insert(0);
            *entry += step.depends_on.len();
            // Ensure dependency nodes are in the map too.
            for dep in &step.depends_on {
                in_degree.entry(dep.as_str()).or_insert(0);
            }
        }

        let mut queue: Vec<&str> = in_degree
            .iter()
            .filter(|(_, &d)| d == 0)
            .map(|(&k, _)| k)
            .collect();

        let mut sorted = Vec::new();

        while !queue.is_empty() {
            let node = queue.remove(0);
            sorted.push(node.to_string());
            // Find steps that depend on this node
            for (id, step) in &self.steps {
                if step.depends_on.iter().any(|d| d == node) {
                    if let Some(d) = in_degree.get_mut(id.as_str()) {
                        *d -= 1;
                        if *d == 0 {
                            queue.push(id.as_str());
                        }
                    }
                }
            }
        }

        if sorted.len() < self.steps.len() {
            Err(LabError::WorkflowError("Workflow contains a cycle".into()))
        } else {
            Ok(sorted)
        }
    }
}

// ─── StepResult ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct StepResult {
    pub step_id: String,
    pub status: StepStatus,
    pub outcome: Option<StepOutcome>,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
    pub duration_secs: f64,
}

// ─── WorkflowExecution ─────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct WorkflowExecution {
    pub workflow_id: String,
    pub workflow_name: String,
    pub execution_id: String,
    pub status: String,
    pub step_results: Vec<StepResult>,
    pub total_duration_secs: f64,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub error: Option<String>,
}

impl WorkflowExecution {
    pub fn to_dict(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

// ─── WorkflowTemplate ───────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct WorkflowTemplate {
    pub name: String,
    pub description: String,
    pub steps: Vec<TemplateStep>,
}

#[derive(Debug, Clone)]
pub struct TemplateStep {
    pub id: String,
    pub task_type: TaskType,
    pub task_string_template: String,
    pub depends_on: Vec<String>,
    pub condition: Option<Condition>,
}

impl WorkflowTemplate {
    pub fn instantiate(
        &self,
        name: &str,
        params: Option<&HashMap<String, serde_json::Value>>,
    ) -> Workflow {
        let mut workflow = Workflow::new(name);
        let params = params.cloned().unwrap_or_default();

        for tstep in &self.steps {
            // Simple string interpolation for task templates
            let task_string = tstep.task_string_template.replace(
                "{topic}",
                params.get("topic").and_then(|v| v.as_str()).unwrap_or(""),
            );

            workflow.add_step(
                &tstep.id,
                tstep.task_type,
                task_string,
                tstep.depends_on.clone(),
            );
            if let Some(ref cond) = tstep.condition {
                workflow.add_condition(&tstep.id, cond.clone());
            }
        }

        workflow
    }
}

// ─── TemplateRegistry ──────────────────────────────────────────────

pub struct TemplateRegistry {
    templates: HashMap<String, WorkflowTemplate>,
}

impl Default for TemplateRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl TemplateRegistry {
    pub fn new() -> Self {
        Self {
            templates: HashMap::new(),
        }
    }

    pub fn register(&mut self, template: WorkflowTemplate) {
        self.templates.insert(template.name.clone(), template);
    }

    pub fn get(&self, name: &str) -> Option<&WorkflowTemplate> {
        self.templates.get(name)
    }

    pub fn list_templates(&self) -> Vec<&WorkflowTemplate> {
        self.templates.values().collect()
    }
}

/// Register built-in workflow templates.
pub fn register_builtin_templates(registry: &mut TemplateRegistry) {
    // Research Pipeline Template
    registry.register(WorkflowTemplate {
        name: "research-pipeline".into(),
        description: "Full research pipeline: discover → analyze → report".into(),
        steps: vec![
            TemplateStep {
                id: "discover".into(),
                task_type: TaskType::Research,
                task_string_template: "Discover and map the codebase for: {topic}".into(),
                depends_on: vec![],
                condition: None,
            },
            TemplateStep {
                id: "analyze".into(),
                task_type: TaskType::Research,
                task_string_template: "Analyze architecture patterns for: {topic}".into(),
                depends_on: vec!["discover".into()],
                condition: None,
            },
            TemplateStep {
                id: "review".into(),
                task_type: TaskType::Review,
                task_string_template: "Review code quality".into(),
                depends_on: vec!["analyze".into()],
                condition: None,
            },
            TemplateStep {
                id: "report".into(),
                task_type: TaskType::Summary,
                task_string_template: "Generate research report for: {topic}".into(),
                depends_on: vec!["review".into()],
                condition: None,
            },
        ],
    });

    // Code Review Template
    registry.register(WorkflowTemplate {
        name: "code-review".into(),
        description: "Code review workflow: scan → analyze → report".into(),
        steps: vec![
            TemplateStep {
                id: "scan".into(),
                task_type: TaskType::Research,
                task_string_template: "Scan codebase for review".into(),
                depends_on: vec![],
                condition: None,
            },
            TemplateStep {
                id: "analyze".into(),
                task_type: TaskType::Review,
                task_string_template: "Analyze code quality and patterns".into(),
                depends_on: vec!["scan".into()],
                condition: None,
            },
            TemplateStep {
                id: "report".into(),
                task_type: TaskType::Summary,
                task_string_template: "Generate review report".into(),
                depends_on: vec!["analyze".into()],
                condition: None,
            },
        ],
    });

    // Analysis Template
    registry.register(WorkflowTemplate {
        name: "analysis".into(),
        description: "Deep analysis workflow".into(),
        steps: vec![
            TemplateStep {
                id: "gather".into(),
                task_type: TaskType::Research,
                task_string_template: "Gather data about: {topic}".into(),
                depends_on: vec![],
                condition: None,
            },
            TemplateStep {
                id: "analyze".into(),
                task_type: TaskType::Research,
                task_string_template: "Analyze findings for: {topic}".into(),
                depends_on: vec!["gather".into()],
                condition: None,
            },
            TemplateStep {
                id: "summarize".into(),
                task_type: TaskType::Summary,
                task_string_template: "Summarize analysis of: {topic}".into(),
                depends_on: vec!["analyze".into()],
                condition: None,
            },
        ],
    });
}

// ─── WorkflowEngine ────────────────────────────────────────────────

/// Workflow executor that runs steps in dependency order.
/// For full async parallel execution with task queue integration,
/// the engine is called from ResearchLab.run_workflow().
pub struct WorkflowEngine;

impl Default for WorkflowEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkflowEngine {
    pub fn new() -> Self {
        Self
    }

    /// Execute a workflow step-by-step (sequential by default).
    /// Parallel execution requires integration with TaskQueue.
    pub async fn execute(
        &self,
        workflow: Workflow,
        executor: impl Fn(
                &WorkflowStep,
                &HashMap<String, serde_json::Value>,
            ) -> std::pin::Pin<
                Box<dyn std::future::Future<Output = Result<serde_json::Value>> + Send>,
            > + Send
            + Sync,
        _session_id: Option<&str>,
        _params: HashMap<String, serde_json::Value>,
    ) -> Result<WorkflowExecution> {
        workflow.validate()?;

        let exec_id = uuid::Uuid::new_v4().to_string()[..8].to_string();
        let step_order = workflow.topological_sort()?;

        let execution = WorkflowExecution {
            workflow_id: workflow.id.clone(),
            workflow_name: workflow.name.clone(),
            execution_id: exec_id,
            status: "running".into(),
            step_results: Vec::new(),
            total_duration_secs: 0.0,
            started_at: chrono::Utc::now().to_rfc3339(),
            completed_at: None,
            error: None,
        };

        let mut results: HashMap<String, StepResult> = HashMap::new();
        let mut context = HashMap::new();

        for step_id in &step_order {
            let step = workflow.steps.get(step_id).unwrap();

            // Check condition
            if let Some(ref condition) = step.condition {
                if !self.evaluate_condition(condition, &results) {
                    let skipped = StepResult {
                        step_id: step_id.clone(),
                        status: StepStatus::Skipped,
                        outcome: Some(StepOutcome::ConditionFalse),
                        result: None,
                        error: None,
                        duration_secs: 0.0,
                    };
                    results.insert(step_id.clone(), skipped);
                    continue;
                }
            }

            // Check if any dependency failed
            let deps_failed = step.depends_on.iter().any(|d| {
                results
                    .get(d)
                    .map(|r| r.status == StepStatus::Failed || r.status == StepStatus::Skipped)
                    .unwrap_or(false)
            });

            if deps_failed && step.skip_on_failure {
                let skipped = StepResult {
                    step_id: step_id.clone(),
                    status: StepStatus::Skipped,
                    outcome: Some(StepOutcome::Failure),
                    result: None,
                    error: Some("Dependency failed".into()),
                    duration_secs: 0.0,
                };
                results.insert(step_id.clone(), skipped);
                continue;
            }

            // Execute step
            let start = std::time::Instant::now();
            let exec_fn = executor(step, &context);
            match exec_fn.await {
                Ok(result) => {
                    results.insert(
                        step_id.clone(),
                        StepResult {
                            step_id: step_id.clone(),
                            status: StepStatus::Completed,
                            outcome: Some(StepOutcome::Success),
                            result: Some(result.clone()),
                            error: None,
                            duration_secs: start.elapsed().as_secs_f64(),
                        },
                    );
                    context.insert(step_id.clone(), result);
                }
                Err(e) => {
                    results.insert(
                        step_id.clone(),
                        StepResult {
                            step_id: step_id.clone(),
                            status: StepStatus::Failed,
                            outcome: Some(StepOutcome::Error),
                            result: None,
                            error: Some(e.to_string()),
                            duration_secs: start.elapsed().as_secs_f64(),
                        },
                    );
                    return Err(LabError::WorkflowError(format!(
                        "Step '{}' failed: {}",
                        step_id, e
                    )));
                }
            }
        }

        let step_results: Vec<StepResult> = step_order
            .iter()
            .filter_map(|id| results.get(id).cloned())
            .collect();

        let all_success = step_results
            .iter()
            .all(|r| r.status == StepStatus::Completed || r.status == StepStatus::Skipped);

        Ok(WorkflowExecution {
            status: if all_success {
                "completed".into()
            } else {
                "failed".into()
            },
            step_results,
            completed_at: Some(chrono::Utc::now().to_rfc3339()),
            total_duration_secs: chrono::Utc::now()
                .signed_duration_since(
                    chrono::DateTime::parse_from_rfc3339(&execution.started_at).unwrap_or_else(
                        |_| chrono::DateTime::<chrono::FixedOffset>::from(chrono::Utc::now()),
                    ),
                )
                .num_milliseconds() as f64
                / 1000.0,
            ..execution
        })
    }

    fn evaluate_condition(
        &self,
        condition: &Condition,
        results: &HashMap<String, StepResult>,
    ) -> bool {
        match condition.cond_type.as_str() {
            "step_success" => {
                if let Some(ref step_id) = condition.step_id {
                    results
                        .get(step_id)
                        .map(|r| r.status == StepStatus::Completed)
                        .unwrap_or(false)
                } else {
                    false
                }
            }
            "output_contains" => {
                if let Some(ref step_id) = condition.step_id {
                    if let Some(ref value) = condition.value {
                        if let Some(step_result) = results.get(step_id) {
                            if let Some(ref result) = step_result.result {
                                return result.to_string().contains(value);
                            }
                        }
                    }
                    false
                } else {
                    false
                }
            }
            _ => true,
        }
    }
}
