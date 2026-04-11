//! HTML Report Generator for workflow and pipeline execution.
//! Mirrors reports/workflow_report.py from Python project.

use lab_core::workflows::{StepResult, StepStatus, WorkflowExecution};

// ─── Research Pipeline Report ────────────────────────────────────

const CSS: &str = "
* { box-sizing: border-box; margin: 0; padding: 0; }
body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
       background: #f0f2f5; color: #1a1a2e; line-height: 1.5; }
.header { background: linear-gradient(135deg, #b71c1c, #c62828); color: white;
          padding: 1.5rem 2rem; }
.header-inner { display: flex; align-items: flex-start; justify-content: space-between;
                max-width: 1100px; margin: 0 auto; }
.lab-logo { font-size: 0.72rem; font-weight: 800; letter-spacing: 0.22em;
            background: rgba(255,255,255,0.2); padding: 2px 7px; border-radius: 3px;
            margin-right: 0.6rem; vertical-align: middle; }
.header h1 { font-size: 1.4rem; font-weight: 600; display: inline; }
.meta { font-size: 0.82rem; opacity: 0.8; margin-top: 0.5rem; }
.meta code { background: rgba(255,255,255,0.15); padding: 1px 5px; border-radius: 3px;
             font-size: 0.88em; }
.badge { display: inline-block; padding: 4px 12px; border-radius: 20px;
         font-size: 0.78rem; font-weight: 700; letter-spacing: 0.04em; margin-top: 4px; }
.badge-completed { background: #1b5e20; color: #a5d6a7; border: 1px solid #2e7d32; }
.badge-failed    { background: #4a1010; color: #ef9a9a; border: 1px solid #c62828; }
.badge-partial   { background: #bf360c; color: #ffccbc; border: 1px solid #e64a19; }
.container { max-width: 1100px; margin: 0 auto; padding: 2rem; }
.cards { display: grid; grid-template-columns: repeat(auto-fit, minmax(155px, 1fr));
         gap: 1rem; margin-bottom: 1.5rem; }
.card { background: white; border-radius: 8px; padding: 1.2rem;
        box-shadow: 0 1px 4px rgba(0,0,0,0.08); border-top: 3px solid #e0e0e0; }
.card-label { font-size: 0.7rem; text-transform: uppercase; letter-spacing: 0.07em;
              color: #999; font-weight: 600; }
.card-value { font-size: 2.1rem; font-weight: 700; color: #c62828; margin-top: 0.2rem; }
.value-warn { color: #e65100; }
.value-ok   { color: #388e3c; }
.two-col { display: grid; grid-template-columns: 1fr 1fr; gap: 1.5rem; margin-bottom: 1.5rem; }
@media (max-width: 700px) { .two-col { grid-template-columns: 1fr; } }
.section { background: white; border-radius: 8px; padding: 1.5rem;
           box-shadow: 0 1px 4px rgba(0,0,0,0.08); margin-bottom: 1.5rem; }
.section h2 { font-size: 0.82rem; font-weight: 700; text-transform: uppercase;
              letter-spacing: 0.07em; color: #666; margin-bottom: 1rem;
              padding-bottom: 0.6rem; border-bottom: 1px solid #f0f2f5; }
.stages { display: flex; flex-direction: column; gap: 0.55rem; }
.stage  { display: flex; align-items: center; gap: 0.75rem; }
.stage-icon { width: 20px; height: 20px; border-radius: 50%; display: flex;
              align-items: center; justify-content: center; font-size: 0.68rem;
              font-weight: 800; flex-shrink: 0; }
.icon-ok   { background: #e8f5e9; color: #2e7d32; }
.icon-fail { background: #fce4ec; color: #c62828; }
.stage-name { font-size: 0.85rem; font-weight: 600; width: 88px; flex-shrink: 0; color: #333; }
.stage-bar-wrap { flex: 1; background: #f0f2f5; border-radius: 3px; height: 5px; }
.stage-bar        { height: 5px; border-radius: 3px; background: #c62828; min-width: 3px; }
.stage-bar-failed { background: #ef9a9a; }
.stage-duration { font-size: 0.78rem; color: #999; width: 50px;
                  text-align: right; flex-shrink: 0; }
.bar-chart { display: flex; flex-direction: column; gap: 0.5rem; }
.bar-row   { display: flex; align-items: center; gap: 0.75rem; }
.bar-label { font-family: 'SF Mono', monospace; font-size: 0.78rem; color: #666;
             width: 42px; text-align: right; flex-shrink: 0; }
.bar-track { flex: 1; background: #f0f2f5; border-radius: 3px; height: 18px; overflow: hidden; }
.bar-fill  { height: 18px; border-radius: 3px; min-width: 28px; display: flex;
             align-items: center; padding-left: 7px; font-size: 0.72rem;
             font-weight: 700; color: rgba(255,255,255,0.9); }
.tags { display: flex; flex-wrap: wrap; gap: 0.35rem; }
.tag  { background: #f0f2f5; border-radius: 4px; padding: 3px 8px;
        font-size: 0.8rem; font-family: monospace; color: #555; }
table { width: 100%; border-collapse: collapse; font-size: 0.88rem; }
thead th { background: #fafafa; padding: 0.5rem 0.75rem; text-align: left;
           font-weight: 700; color: #666; font-size: 0.72rem;
           text-transform: uppercase; letter-spacing: 0.06em;
           border-bottom: 2px solid #f0f2f5; }
td { padding: 0.42rem 0.75rem; border-bottom: 1px solid #f8f8f8; vertical-align: top; }
tr:hover td { background: #fafafa; }
tr:last-child td { border-bottom: none; }
.code   { font-family: 'SF Mono', 'Fira Code', monospace; font-size: 0.8em; color: #333; }
.num    { text-align: right; font-variant-numeric: tabular-nums; color: #666; }
.dimmed { color: #bbb; font-size: 0.85rem; }
.severity { display: inline-block; padding: 1px 7px; border-radius: 3px;
            font-size: 0.72rem; font-weight: 700; }
.sev-error   { background: #fce4ec; color: #b71c1c; }
.sev-warning { background: #fff3e0; color: #bf360c; }
.sev-info    { background: #e3f2fd; color: #1565c0; }
.insight-item { margin-bottom: 1.2rem; padding-bottom: 1.2rem;
                border-bottom: 1px solid #f0f2f5; }
.insight-item:last-child { border-bottom: none; margin-bottom: 0; padding-bottom: 0; }
.insight-path { font-family: monospace; font-size: 0.8rem; color: #c62828;
                font-weight: 700; margin-bottom: 0.4rem; }
.insight-body { font-size: 0.87rem; line-height: 1.65; white-space: pre-wrap; color: #333; }
.insights     { white-space: pre-wrap; font-size: 0.9rem; line-height: 1.75; color: #333; }
footer { text-align: center; padding: 2rem; color: #ccc; font-size: 0.8rem; }
footer strong { color: #aaa; }
";

/// One pipeline stage for the HTML report.
pub struct StageInput {
    pub name: String,
    pub completed: bool,
    pub duration_secs: f64,
    pub error: Option<String>,
}

/// All data needed to render a research pipeline HTML report.
pub struct ResearchReportContext {
    pub pipeline_name: String,
    pub session_id: String,
    pub status: String,
    pub total_duration_secs: f64,
    pub stages: Vec<StageInput>,
    /// JSON from the `discover` memory key
    pub discover: Option<serde_json::Value>,
    /// JSON from the `researcher_map` memory key
    pub researcher_map: Option<serde_json::Value>,
    /// JSON from the `review_results` memory key
    pub review_results: Option<serde_json::Value>,
    /// Optional LLM-generated executive summary
    pub ai_summary: Option<String>,
}

/// Generate a self-contained HTML research report from pipeline data.
pub fn render_research_html(ctx: &ResearchReportContext) -> String {
    let now = chrono::Utc::now().format("%Y-%m-%d %H:%M UTC").to_string();

    // ── Discover ─────────────────────────────────────────────────
    let (total_files, file_type_counts, top_dirs) = extract_discover(&ctx.discover);

    // ── Researcher map ───────────────────────────────────────────
    let (files_analysed, total_classes, total_functions, file_rows, llm_analyses) =
        extract_researcher(&ctx.researcher_map);

    // ── Review results ───────────────────────────────────────────
    let (files_reviewed, total_issues, issues_by_rule, llm_insights) =
        extract_review(&ctx.review_results);

    // ── Badge ────────────────────────────────────────────────────
    let (badge_class, badge_text) = match ctx.status.as_str() {
        "completed" => ("badge-completed", "Completed"),
        "failed" => ("badge-failed", "Failed"),
        _ => ("badge-partial", "Partial"),
    };

    // ── Stages HTML ──────────────────────────────────────────────
    let max_dur = ctx
        .stages
        .iter()
        .map(|s| s.duration_secs)
        .fold(0.0_f64, f64::max)
        .max(0.001);

    let stages_html: String = ctx
        .stages
        .iter()
        .map(|s| {
            let pct = ((s.duration_secs / max_dur) * 100.0).min(100.0) as u32;
            let (icon, icon_cls, bar_cls) = if s.completed {
                ("✓", "icon-ok", "stage-bar")
            } else {
                ("✗", "icon-fail", "stage-bar stage-bar-failed")
            };
            let err_html = s
                .error
                .as_deref()
                .map(|e| {
                    format!(
                        "<div style='font-size:0.72rem;color:#c62828;margin-top:2px'>{}</div>",
                        he(e)
                    )
                })
                .unwrap_or_default();
            format!(
                "<div class='stage'>\
                   <div class='stage-icon {icon_cls}'>{icon}</div>\
                   <div class='stage-name'>{name}</div>\
                   <div class='stage-bar-wrap'>\
                     <div class='{bar_cls}' style='width:{pct}%'></div>\
                   </div>\
                   <div class='stage-duration'>{dur:.2}s</div>\
                   {err_html}\
                 </div>",
                name = he(&s.name),
                dur = s.duration_secs,
            )
        })
        .collect();

    // ── File-type chart HTML ─────────────────────────────────────
    let max_count = file_type_counts.iter().map(|(_, c)| *c).max().unwrap_or(1) as f64;

    let type_chart_html: String = file_type_counts
        .iter()
        .map(|(ext, count)| {
            let pct = (*count as f64 / max_count * 100.0).max(4.0) as u32;
            let color = ext_color(ext);
            format!(
                "<div class='bar-row'>\
                   <div class='bar-label'>.{ext}</div>\
                   <div class='bar-track'>\
                     <div class='bar-fill' style='width:{pct}%;background:{color}'>{count}</div>\
                   </div>\
                 </div>",
                ext = he(ext),
            )
        })
        .collect();

    let dirs_html: String = top_dirs
        .iter()
        .map(|d| format!("<span class='tag'>{}</span>", he(d)))
        .collect();

    let file_type_section = if !file_type_counts.is_empty() {
        format!("<div class='bar-chart'>{type_chart_html}</div>")
    } else if !top_dirs.is_empty() {
        format!("<p style='font-size:0.8rem;color:#999;margin-bottom:0.6rem'>Top-level directories</p><div class='tags'>{dirs_html}</div>")
    } else {
        "<p class='dimmed'>No discovery data</p>".to_string()
    };

    // ── Files table ──────────────────────────────────────────────
    let files_section = if !file_rows.is_empty() {
        let rows: String = file_rows
            .iter()
            .enumerate()
            .map(|(i, (path, lines, classes, fns))| {
                let disp = if path.len() > 58 {
                    format!("…{}", &path[path.len().saturating_sub(56)..])
                } else {
                    path.clone()
                };
                format!(
                    "<tr>\
                       <td class='num dimmed'>{}</td>\
                       <td class='code'>{}</td>\
                       <td class='num'>{lines}</td>\
                       <td class='num'>{classes}</td>\
                       <td class='num'>{fns}</td>\
                     </tr>",
                    i + 1,
                    he(&disp),
                )
            })
            .collect();
        format!(
            "<div class='section'>\
               <h2>Files Analysed &nbsp;<span style='font-weight:400;color:#bbb'>{} files</span></h2>\
               <table><thead><tr>\
                 <th class='num'>#</th>\
                 <th>Path</th>\
                 <th class='num'>Lines</th>\
                 <th class='num'>Classes</th>\
                 <th class='num'>Functions</th>\
               </tr></thead><tbody>{rows}</tbody></table>\
             </div>",
            file_rows.len(),
        )
    } else {
        String::new()
    };

    // ── Issues table ─────────────────────────────────────────────
    let issue_rows: String = if issues_by_rule.is_empty() {
        "<tr><td colspan='3' class='dimmed' style='text-align:center;padding:1rem'>No issues found</td></tr>".to_string()
    } else {
        issues_by_rule
            .iter()
            .map(|(rule, count)| {
                let sev = rule_severity(rule);
                format!(
                    "<tr>\
                       <td class='code'>{}</td>\
                       <td class='num'>{count}</td>\
                       <td><span class='severity sev-{sev}'>{sev}</span></td>\
                     </tr>",
                    he(rule),
                )
            })
            .collect()
    };
    let issues_section = format!(
        "<div class='section'>\
           <h2>Code Quality Issues</h2>\
           <table><thead><tr>\
             <th>Rule</th><th class='num'>Count</th><th>Severity</th>\
           </tr></thead><tbody>{issue_rows}</tbody></table>\
         </div>"
    );

    // ── LLM file analyses ────────────────────────────────────────
    let llm_analyses_section: String = if llm_analyses.is_empty() {
        String::new()
    } else {
        let items: String = llm_analyses
            .iter()
            .map(|(path, text)| {
                format!(
                    "<div class='insight-item'>\
                       <div class='insight-path'>{}</div>\
                       <div class='insight-body'>{}</div>\
                     </div>",
                    he(path),
                    he(text),
                )
            })
            .collect();
        format!("<div class='section'><h2>LLM File Analysis</h2>{items}</div>")
    };

    // ── LLM review insights ──────────────────────────────────────
    let llm_review_section: String = if llm_insights.is_empty() {
        String::new()
    } else {
        let items: String = llm_insights
            .iter()
            .map(|(path, text)| {
                format!(
                    "<div class='insight-item'>\
                       <div class='insight-path'>{}</div>\
                       <div class='insight-body'>{}</div>\
                     </div>",
                    he(path),
                    he(text),
                )
            })
            .collect();
        format!("<div class='section'><h2>LLM Code Review</h2>{items}</div>")
    };

    // ── AI summary ───────────────────────────────────────────────
    let ai_section: String = ctx
        .ai_summary
        .as_deref()
        .map(|s| {
            format!(
                "<div class='section'>\
                   <h2>AI Insights</h2>\
                   <div class='insights'>{}</div>\
                 </div>",
                he(s)
            )
        })
        .unwrap_or_default();

    // ── Assemble ─────────────────────────────────────────────────
    let issues_card_class = if total_issues > 0 {
        "value-warn"
    } else {
        "value-ok"
    };

    let mut html = String::with_capacity(64_000);

    html.push_str("<!DOCTYPE html><html lang=\"en\"><head>");
    html.push_str("<meta charset=\"UTF-8\">");
    html.push_str("<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">");
    html.push_str(&format!(
        "<title>{} — AI Research Lab Report</title>",
        he(&ctx.pipeline_name)
    ));
    html.push_str("<style>");
    html.push_str(CSS);
    html.push_str("</style></head><body>");

    // Header
    html.push_str(&format!(
        "<div class='header'>\
           <div class='header-inner'>\
             <div>\
               <span class='lab-logo'>LAB</span><h1>{name}</h1>\
               <div class='meta'>Session: <code>{session}</code> &nbsp;·&nbsp; \
               Generated: {now} &nbsp;·&nbsp; Duration: {dur:.1}s</div>\
             </div>\
             <span class='badge {badge_class}'>{badge_text}</span>\
           </div>\
         </div>",
        name = he(&ctx.pipeline_name),
        session = he(&ctx.session_id),
        dur = ctx.total_duration_secs,
    ));

    // Container open
    html.push_str("<div class='container'>");

    // Summary cards
    html.push_str(&format!(
        "<div class='cards'>\
           <div class='card'><div class='card-label'>Files Discovered</div>\
             <div class='card-value'>{total_files}</div></div>\
           <div class='card'><div class='card-label'>Files Analysed</div>\
             <div class='card-value'>{files_analysed}</div></div>\
           <div class='card'><div class='card-label'>Classes / Structs</div>\
             <div class='card-value'>{total_classes}</div></div>\
           <div class='card'><div class='card-label'>Functions</div>\
             <div class='card-value'>{total_functions}</div></div>\
           <div class='card'><div class='card-label'>Files Reviewed</div>\
             <div class='card-value'>{files_reviewed}</div></div>\
           <div class='card'><div class='card-label'>Issues Found</div>\
             <div class='card-value {issues_card_class}'>{total_issues}</div></div>\
         </div>",
    ));

    // Stages + file types (two-column)
    html.push_str(&format!(
        "<div class='two-col'>\
           <div class='section'><h2>Pipeline Stages</h2>\
             <div class='stages'>{stages_html}</div></div>\
           <div class='section'><h2>File Types</h2>\
             {file_type_section}</div>\
         </div>",
    ));

    html.push_str(&files_section);
    html.push_str(&issues_section);
    html.push_str(&llm_analyses_section);
    html.push_str(&llm_review_section);
    html.push_str(&ai_section);

    html.push_str("</div>"); // container
    html.push_str(&format!(
        "<footer>Generated by <strong>AI Research Lab</strong> &nbsp;·&nbsp; {now}</footer>"
    ));
    html.push_str("</body></html>");

    html
}

// ─── Report Data Extractors ──────────────────────────────────────

fn extract_discover(v: &Option<serde_json::Value>) -> (u64, Vec<(String, usize)>, Vec<String>) {
    let Some(disc) = v else {
        return (0, Vec::new(), Vec::new());
    };
    let total = disc
        .get("total_files")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    if let Some(arr) = disc.get("file_types").and_then(|v| v.as_array()) {
        for t in arr {
            if let Some(ext) = t.as_str() {
                *counts.entry(ext.to_string()).or_insert(0) += 1;
            }
        }
    }
    let mut types: Vec<(String, usize)> = counts.into_iter().collect();
    types.sort_by(|a, b| b.1.cmp(&a.1));
    types.truncate(10);

    let dirs: Vec<String> = disc
        .get("top_level_dirs")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    (total, types, dirs)
}

type ResearcherStats = (
    u64,
    u64,
    u64,
    Vec<(String, u64, u64, u64)>,
    Vec<(String, String)>,
);

fn extract_researcher(v: &Option<serde_json::Value>) -> ResearcherStats {
    let Some(rm) = v else {
        return (0, 0, 0, Vec::new(), Vec::new());
    };
    let fa = rm
        .get("files_analysed")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let tc = rm
        .get("total_classes")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let tf = rm
        .get("total_functions")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let files: Vec<(String, u64, u64, u64)> = rm
        .get("files")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .take(30)
                .filter_map(|f| {
                    let path = f.get("path").and_then(|v| v.as_str())?.to_string();
                    let lines = f.get("total_lines").and_then(|v| v.as_u64()).unwrap_or(0);
                    let classes = f
                        .get("class_names")
                        .and_then(|v| v.as_array())
                        .map(|a| a.len() as u64)
                        .unwrap_or(0);
                    let fns = f
                        .get("async_functions")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0)
                        + f.get("sync_functions")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0);
                    Some((path, lines, classes, fns))
                })
                .collect()
        })
        .unwrap_or_default();

    let llm: Vec<(String, String)> = rm
        .get("llm_analysis")
        .and_then(|v| v.get("llm_file_analyses"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|a| {
                    let path = a.get("path").and_then(|v| v.as_str())?.to_string();
                    let analysis = a.get("llm_analysis").and_then(|v| v.as_str())?.to_string();
                    Some((path, analysis))
                })
                .collect()
        })
        .unwrap_or_default();

    (fa, tc, tf, files, llm)
}

type ReviewStats = (u64, u64, Vec<(String, u64)>, Vec<(String, String)>);

fn extract_review(v: &Option<serde_json::Value>) -> ReviewStats {
    let Some(rv) = v else {
        return (0, 0, Vec::new(), Vec::new());
    };
    let summary = rv.get("summary");
    let fr = summary
        .and_then(|s| s.get("files_reviewed"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let ti = summary
        .and_then(|s| s.get("total_issues"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let mut rules: Vec<(String, u64)> = summary
        .and_then(|s| s.get("by_rule"))
        .and_then(|v| v.as_object())
        .map(|obj| {
            obj.iter()
                .map(|(k, v)| (k.clone(), v.as_u64().unwrap_or(0)))
                .collect()
        })
        .unwrap_or_default();
    rules.sort_by(|a, b| b.1.cmp(&a.1));

    let insights: Vec<(String, String)> = rv
        .get("llm_insights")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|i| {
                    let path = i.get("path").and_then(|v| v.as_str())?.to_string();
                    let review = i
                        .get("review")
                        .and_then(|v| v.as_str())
                        .or_else(|| i.get("llm_review").and_then(|v| v.as_str()))?
                        .to_string();
                    Some((path, review))
                })
                .collect()
        })
        .unwrap_or_default();

    (fr, ti, rules, insights)
}

// ─── HTML Helpers ────────────────────────────────────────────────

fn he(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn ext_color(ext: &str) -> &'static str {
    match ext {
        "rs" => "#e65100",
        "py" => "#1565c0",
        "js" | "mjs" | "cjs" => "#f9a825",
        "ts" | "tsx" | "jsx" => "#0277bd",
        "toml" | "yaml" | "yml" => "#388e3c",
        "json" => "#6a1b9a",
        "md" | "mdx" => "#00838f",
        "go" => "#00796b",
        "java" | "kt" => "#ad1457",
        "html" | "css" => "#0097a7",
        _ => "#546e7a",
    }
}

fn rule_severity(rule: &str) -> &'static str {
    let r = rule.to_ascii_lowercase();
    if r.contains("unsafe") || r.contains("unwrap") || r.contains("panic") || r.contains("error") {
        "error"
    } else if r.contains("todo")
        || r.contains("fixme")
        || r.contains("hack")
        || r.contains("bare_except")
        || r.contains("bare_catch")
    {
        "warning"
    } else {
        "info"
    }
}

/// Generates self-contained HTML reports with SVG charts and status badges.
pub struct ReportGenerator;

impl ReportGenerator {
    pub fn new() -> Self {
        Self
    }

    /// Generate an HTML report for a single workflow execution.
    pub fn generate_workflow_report(
        &self,
        execution: &WorkflowExecution,
        title: Option<&str>,
    ) -> String {
        let title = title.unwrap_or(&execution.workflow_name);
        let status_badge = match execution.status.as_str() {
            "completed" => self.badge("Completed", "4CAF50", "white"),
            "failed" => self.badge("Failed", "f44336", "white"),
            "partial" => self.badge("Partial", "ff9800", "white"),
            _ => self.badge(&execution.status, "607D8B", "white"),
        };

        let steps_html: String = execution
            .step_results
            .iter()
            .map(|step| self.step_row(step))
            .collect();

        let total_duration = execution.total_duration_secs;

        format!(
            r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<title>{title} — AI Research Lab Report</title>
<style>
body {{ font-family: system-ui, -apple-system, sans-serif; max-width: 900px; margin: 2rem auto; padding: 0 1rem; background: #f5f5f5; }}
h1 {{ color: #333; border-bottom: 2px solid #1976D2; padding-bottom: 0.5rem; }}
.summary {{ display: flex; gap: 1rem; margin: 1rem 0; flex-wrap: wrap; }}
.summary-item {{ background: white; padding: 1rem; border-radius: 8px; box-shadow: 0 2px 4px rgba(0,0,0,0.1); flex: 1; min-width: 150px; }}
.summary-item label {{ font-size: 0.85rem; color: #666; display: block; }}
.summary-item span {{ font-size: 1.4rem; font-weight: 600; color: #333; }}
table {{ width: 100%; border-collapse: collapse; margin-top: 1rem; background: white; border-radius: 8px; overflow: hidden; box-shadow: 0 2px 4px rgba(0,0,0,0.1); }}
th {{ background: #1976D2; color: white; padding: 0.75rem 1rem; text-align: left; }}
td {{ padding: 0.75rem 1rem; border-bottom: 1px solid #eee; }}
tr:last-child td {{ border-bottom: none; }}
.badge {{ display: inline-block; padding: 2px 8px; border-radius: 4px; color: white; font-size: 0.85rem; }}
.error-text {{ color: #f44336; font-family: monospace; font-size: 0.85rem; }}
</style>
</head>
<body>
<h1>📊 {title}</h1>
<div class="summary">
  <div class="summary-item"><label>Status</label>{status_badge}</div>
  <div class="summary-item"><label>Duration</label><span>{total_duration:.1}s</span></div>
  <div class="summary-item"><label>Steps</label><span>{steps_count}</span></div>
</div>
<h2>Step Results</h2>
<table>
  <tr><th>Step</th><th>Status</th><th>Duration</th><th>Error</th></tr>
  {steps_html}
</table>
<p style="color: #999; font-size: 0.8rem; margin-top: 2rem;">Generated by AI Research Lab Report Generator</p>
</body>
</html>"#,
            title = title,
            status_badge = status_badge,
            total_duration = total_duration,
            steps_count = execution.step_results.len(),
            steps_html = steps_html,
        )
    }

    /// Generate an HTML summary for multiple workflow executions.
    pub fn generate_batch_report(
        &self,
        executions: &[WorkflowExecution],
        title: Option<&str>,
    ) -> String {
        let title = title.unwrap_or("Workflow Batch Report");
        let rows: String = executions
            .iter()
            .map(|e| {
                let status_badge = match e.status.as_str() {
                    "completed" => self.badge("Completed", "4CAF50", "white"),
                    "failed" => self.badge("Failed", "f44336", "white"),
                    _ => self.badge(&e.status, "607D8B", "white"),
                };
                format!(
                    r#"<tr><td>{}</td><td>{}</td><td>{:.1}s</td><td>{}</td></tr>"#,
                    e.workflow_name,
                    status_badge,
                    e.total_duration_secs,
                    e.step_results.len(),
                )
            })
            .collect();

        format!(
            r#"<!DOCTYPE html>
<html lang="en">
<head><meta charset="UTF-8"><title>{title}</title>
<style>
body {{ font-family: system-ui, sans-serif; max-width: 1000px; margin: 2rem auto; }}
h1 {{ color: #333; }}
table {{ width: 100%; border-collapse: collapse; margin-top: 1rem; }}
th {{ background: #1565C0; color: white; padding: 0.75rem 1rem; text-align: left; }}
td {{ padding: 0.75rem 1rem; border-bottom: 1px solid #eee; }}
</style></head><body>
<h1>{title}</h1>
<table><tr><th>Workflow</th><th>Status</th><th>Duration</th><th>Steps</th></tr>{rows}</table>
</body></html>"#,
            title = title,
            rows = rows,
        )
    }

    // ─── Helpers ──────────────────────────────────────────────

    fn badge(&self, text: &str, color: &str, text_color: &str) -> String {
        format!(
            r#"<span class="badge" style="background-color: {color}; color: {text_color};">{text}</span>"#,
            color = color,
            text_color = text_color,
            text = text,
        )
    }

    fn step_row(&self, step: &StepResult) -> String {
        let status_badge = match step.status {
            StepStatus::Completed => self.badge("Passed", "4CAF50", "white"),
            StepStatus::Failed => self.badge("Failed", "f44336", "white"),
            StepStatus::Skipped => self.badge("Skipped", "ff9800", "white"),
            StepStatus::Timeout => self.badge("Timeout", "ff9800", "white"),
            StepStatus::Cancelled => self.badge("Cancelled", "9e9e9e", "white"),
            _ => self.badge("Pending", "b0bec5", "white"),
        };
        let error_html = match &step.error {
            Some(err) => format!(
                r#"<span class="error-text">{}</span>"#,
                self.escape_html(err)
            ),
            None => String::new(),
        };
        format!(
            r#"<tr><td>{}</td><td>{}</td><td>{:.2}s</td><td>{}</td></tr>"#,
            step.step_id, status_badge, step.duration_secs, error_html,
        )
    }

    fn escape_html(&self, text: &str) -> String {
        text.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    }
}

impl Default for ReportGenerator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_valid_html() {
        let gen = ReportGenerator::new();
        let execution = WorkflowExecution {
            workflow_id: "wf-1".into(),
            workflow_name: "Test Workflow".into(),
            execution_id: "exec-1".into(),
            status: "completed".into(),
            step_results: vec![StepResult {
                step_id: "step-1".into(),
                status: StepStatus::Completed,
                outcome: Some(lab_core::workflows::StepOutcome::Success),
                result: None,
                error: None,
                duration_secs: 1.5,
            }],
            total_duration_secs: 2.3,
            started_at: chrono::Utc::now().to_rfc3339(),
            completed_at: Some(chrono::Utc::now().to_rfc3339()),
            error: None,
        };
        let html = gen.generate_workflow_report(&execution, None);
        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("Test Workflow"));
        assert!(html.contains("Completed"));
        assert!(html.contains("2.3s"));
    }

    #[test]
    fn fails_with_error() {
        let gen = ReportGenerator::new();
        let execution = WorkflowExecution {
            workflow_id: "wf-1".into(),
            workflow_name: "Failing Workflow".into(),
            execution_id: "exec-1".into(),
            status: "failed".into(),
            step_results: vec![StepResult {
                step_id: "step-1".into(),
                status: StepStatus::Failed,
                outcome: Some(lab_core::workflows::StepOutcome::Error),
                result: None,
                error: Some("Connection refused".into()),
                duration_secs: 0.3,
            }],
            total_duration_secs: 0.3,
            started_at: chrono::Utc::now().to_rfc3339(),
            completed_at: Some(chrono::Utc::now().to_rfc3339()),
            error: Some("step-1 failed".into()),
        };
        let html = gen.generate_workflow_report(&execution, None);
        assert!(html.contains("Failed"));
        assert!(html.contains("Connection refused"));
    }

    #[test]
    fn escape_html_works() {
        let gen = ReportGenerator::new();
        assert_eq!(
            gen.escape_html("<script>alert('xss')</script>"),
            "&lt;script&gt;alert('xss')&lt;/script&gt;"
        );
    }
}
