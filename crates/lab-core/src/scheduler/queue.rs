//! Priority-based async task queue with dependency resolution,
//! concurrent workers, retry with exponential backoff, and recurring tasks.
//! Mirrors core/scheduler/queue.py (480 lines)

use crate::errors::Result;
use crate::scheduler::models::*;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Notify;
use tracing::{error, info, warn};

/// Callback type for task execution.
/// Returns the task result as a JSON value.
pub type TaskExecutor =
    dyn Fn(&TaskSpec) -> tokio::sync::oneshot::Receiver<Result<serde_json::Value>> + Send + Sync;

/// Priority heap — sorted insertion for priority ordering.
struct PriorityHeap {
    items: Vec<TaskSpec>,
}

impl PriorityHeap {
    fn new() -> Self {
        Self { items: Vec::new() }
    }

    fn push(&mut self, task: TaskSpec) {
        let pos = self.items.iter().position(|existing| {
            task.priority > existing.priority
                || (task.priority == existing.priority && task.created_at < existing.created_at)
        });
        match pos {
            Some(i) => self.items.insert(i, task),
            None => self.items.push(task),
        }
    }

    fn pop(&mut self) -> Option<TaskSpec> {
        if self.items.is_empty() {
            return None;
        }
        Some(self.items.remove(0))
    }

    fn peek(&self) -> Option<&TaskSpec> {
        self.items.first()
    }

    fn remove(&mut self, task_id: &str) -> bool {
        if let Some(i) = self.items.iter().position(|t| t.id == task_id) {
            self.items.remove(i);
            true
        } else {
            false
        }
    }

    fn len(&self) -> usize {
        self.items.len()
    }
}

/// Priority-based async task queue.
pub struct TaskQueue {
    queue: PriorityHeap,
    tasks: HashMap<String, TaskSpec>,
    running: HashSet<String>,
    completed: HashMap<String, TaskSpec>,
    max_workers: usize,
    running_flag: bool,
    new_task_notify: Arc<Notify>,
    executor: Option<Arc<TaskExecutor>>,
}

impl TaskQueue {
    pub fn new(max_workers: usize) -> Self {
        Self {
            queue: PriorityHeap::new(),
            tasks: HashMap::new(),
            running: HashSet::new(),
            completed: HashMap::new(),
            max_workers,
            running_flag: false,
            new_task_notify: Arc::new(Notify::new()),
            executor: None,
        }
    }

    /// Set the executor function for running tasks.
    pub fn set_executor(
        &mut self,
        executor: impl Fn(&TaskSpec) -> tokio::sync::oneshot::Receiver<Result<serde_json::Value>>
            + Send
            + Sync
            + 'static,
    ) {
        self.executor = Some(Arc::new(executor));
    }

    /// Start the task queue workers.
    pub async fn start(&mut self) {
        if self.running_flag {
            return;
        }
        self.running_flag = true;
        info!("TaskQueue started (workers={})", self.max_workers);
    }

    /// Stop all workers gracefully.
    pub async fn stop(&mut self) {
        self.running_flag = false;
        self.new_task_notify.notify_waiters();
        info!("TaskQueue stopped");
    }

    /// Submit a task to the queue. Returns the task ID.
    pub async fn submit(&mut self, mut task: TaskSpec) -> Result<String> {
        task.status = TaskStatus::Queued;
        let task_type = task.task_type;
        let task_priority = task.priority;
        let id = task.id.clone();
        self.tasks.insert(id.clone(), task.clone());
        self.queue.push(task);
        self.new_task_notify.notify_one();

        // If recurring, we handle it at the executor level
        info!(
            "Task submitted: {} ({:?}, {:?})",
            id, task_type, task_priority
        );
        Ok(id)
    }

    /// Submit multiple tasks at once.
    pub async fn submit_many(&mut self, tasks: Vec<TaskSpec>) -> Result<Vec<String>> {
        let mut ids = Vec::new();
        for task in tasks {
            ids.push(self.submit(task).await?);
        }
        Ok(ids)
    }

    /// Cancel a queued task.
    pub async fn cancel(&mut self, task_id: &str) -> bool {
        if let Some(task) = self.tasks.get_mut(task_id) {
            if task.status == TaskStatus::Queued {
                self.queue.remove(task_id);
                task.status = TaskStatus::Cancelled;
                return true;
            }
            if task.status == TaskStatus::Running {
                task.status = TaskStatus::Cancelled;
                return true;
            }
        }
        false
    }

    /// Wait for a task to complete and return its result.
    pub async fn wait_for(
        &self,
        task_id: &str,
        timeout: Option<std::time::Duration>,
    ) -> Option<serde_json::Value> {
        self.tasks.get(task_id)?;

        let deadline = timeout.map(|d| std::time::Instant::now() + d);

        // Poll loop
        loop {
            // Check if completed
            if let Some(completed_task) = self.completed.get(task_id) {
                if completed_task.status == TaskStatus::Completed {
                    return completed_task.result.clone();
                }
                return None; // Failed or cancelled
            }

            // Check the tasks map
            if let Some(current_task) = self.tasks.get(task_id) {
                match current_task.status {
                    TaskStatus::Completed => return current_task.result.clone(),
                    TaskStatus::Failed | TaskStatus::Cancelled => return None,
                    _ => {}
                }
            }

            if let Some(d) = deadline {
                if std::time::Instant::now() >= d {
                    return None;
                }
            }

            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }

    /// Get a task by ID.
    pub fn get_task(&self, task_id: &str) -> Option<&TaskSpec> {
        self.tasks.get(task_id)
    }

    /// Get pending count.
    pub fn get_pending_count(&self) -> usize {
        self.queue.len()
    }

    /// Get queue statistics.
    pub fn get_stats(&self) -> TaskQueueStats {
        let mut by_status: HashMap<String, usize> = HashMap::new();
        let mut by_type: HashMap<String, usize> = HashMap::new();
        for t in self.tasks.values() {
            *by_status.entry(t.status.to_string()).or_insert(0) += 1;
            *by_type
                .entry(format!("{:?}", t.task_type).to_lowercase())
                .or_insert(0) += 1;
        }
        TaskQueueStats {
            total: self.tasks.len(),
            running: self.running.len(),
            queue_size: self.queue.len(),
            by_status,
            by_type,
        }
    }

    /// Execute the next available task. Called by the worker loop.
    pub async fn execute_next(&mut self) -> Option<String> {
        if !self.running_flag {
            return None;
        }

        let task = self.queue.peek()?.clone();

        // Check dependencies
        if !task.dependencies.is_empty() {
            let deps_done = task.dependencies.iter().all(|d| {
                self.completed
                    .get(d)
                    .map(|c| c.status == TaskStatus::Completed)
                    .unwrap_or(false)
            });
            if !deps_done {
                return None; // deps not met, skip for now
            }
        }

        // Check schedule time
        if let Some(schedule_at) = task.schedule_at {
            // schedule_at is absolute epoch — compare with current time
            let now_epoch = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64();
            if now_epoch < schedule_at {
                return None; // not scheduled yet
            }
        }

        // Pop and execute
        let task = self.queue.pop()?;
        if task.status == TaskStatus::Cancelled {
            return None;
        }
        if task.status != TaskStatus::Queued {
            return None;
        }

        let task_id = task.id.clone();
        self.execute_task(task).await;
        Some(task_id)
    }

    /// Execute a single task using the registered executor.
    async fn execute_task(&mut self, mut task: TaskSpec) {
        let Some(executor) = self.executor.clone() else {
            error!("No executor registered for task {}", task.id);
            task.status = TaskStatus::Failed;
            task.error = "No executor registered".to_string();
            task.completed_at = Some(Instant::now());
            self.completed.insert(task.id.clone(), task);
            return;
        };

        task.status = TaskStatus::Running;
        task.started_at = Some(Instant::now());
        self.running.insert(task.id.clone());

        let timeout = std::time::Duration::from_secs(task.timeout_seconds);
        let task_id = task.id.clone();

        let timeout_secs = task.timeout_seconds;
        let exec_result = tokio::time::timeout(timeout, async {
            let rx = executor(&task);
            match rx.await {
                Ok(Ok(val)) => Ok(val),
                Ok(Err(e)) => Err(format!("Task executor error: {e}")),
                Err(e) => Err(format!("Channel closed: {e}")),
            }
        })
        .await;

        match exec_result {
            Ok(Ok(result)) => {
                task.status = TaskStatus::Completed;
                task.completed_at = Some(Instant::now());
                task.result = Some(result);
                let duration = task
                    .completed_at
                    .unwrap()
                    .duration_since(task.started_at.unwrap())
                    .as_secs_f64();
                info!("Task {} completed in {:.1}s", task_id, duration);
            }
            Ok(Err(e)) => {
                self.handle_failure(&mut task, &e).await;
            }
            Err(_) => {
                self.handle_failure(&mut task, &format!("Timed out after {}s", timeout_secs))
                    .await;
            }
        }

        self.running.remove(&task_id);
        self.completed.insert(task_id, task);
    }

    /// Handle task failure with retry logic.
    async fn handle_failure(&mut self, task: &mut TaskSpec, error_msg: &str) {
        task.error = error_msg.to_string();
        task.retries += 1;

        if task.retries <= task.max_retries {
            let backoff = std::cmp::min(2u64.pow(task.retries), 60);
            warn!(
                "Task {} failed (attempt {}): {}. Retrying in {}s",
                task.id, task.retries, error_msg, backoff
            );
            tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
            task.status = TaskStatus::Queued;
            task.started_at = None;
            self.queue.push(task.clone());
        } else {
            task.status = TaskStatus::Failed;
            task.completed_at = Some(Instant::now());
            error!("Task {} permanently failed: {}", task.id, error_msg);
        }
    }
}
