//! Local mock Datadog (T29): a DogStatsD receiver + web viewer.
//!
//! Captures DogStatsD datagrams (the wire format the worker's `DogStatsdSink`
//! produces) and
//! serves them for viewing, so local/test runs never send to real Datadog (V39).
//! It parses the line protocol back into structured metrics purely for display.

use std::collections::VecDeque;
use std::sync::Mutex;

use axum::extract::State;
use axum::response::Html;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;

/// A single parsed DogStatsD metric.
#[derive(Debug, Clone, Serialize)]
pub struct Metric {
    /// Metric name (e.g. `synthetics.check.up`).
    pub name: String,
    /// Numeric value.
    pub value: f64,
    /// DogStatsD type suffix (`g`, `c`, `ms`, ...).
    pub kind: String,
    /// Tags (without the leading `#`).
    pub tags: Vec<String>,
    /// The original line, for reference.
    pub raw: String,
}

/// Parse one DogStatsD line (`name:value|type|#tag1,tag2`). Returns `None` for a
/// malformed line.
pub fn parse_line(line: &str) -> Option<Metric> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let mut parts = line.split('|');
    let name_value = parts.next()?;
    let (name, value_str) = name_value.rsplit_once(':')?;
    let value: f64 = value_str.parse().ok()?;
    let kind = parts.next()?.to_string();

    let mut tags = Vec::new();
    for part in parts {
        if let Some(tag_list) = part.strip_prefix('#') {
            tags.extend(
                tag_list
                    .split(',')
                    .filter(|t| !t.is_empty())
                    .map(str::to_string),
            );
        }
    }

    Some(Metric {
        name: name.to_string(),
        value,
        kind,
        tags,
        raw: line.to_string(),
    })
}

/// A bounded, in-memory capture of received metrics (most recent last).
#[derive(Debug)]
pub struct Capture {
    inner: Mutex<VecDeque<Metric>>,
    capacity: usize,
}

impl Capture {
    /// Create a capture retaining up to `capacity` most-recent metrics.
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(VecDeque::new()),
            capacity: capacity.max(1),
        }
    }

    /// Ingest one datagram (which may contain several newline-separated lines).
    pub fn ingest(&self, datagram: &str) {
        let mut guard = self.lock();
        for line in datagram.split('\n') {
            if let Some(metric) = parse_line(line) {
                if guard.len() == self.capacity {
                    guard.pop_front();
                }
                guard.push_back(metric);
            }
        }
    }

    /// A snapshot of all captured metrics, oldest first.
    pub fn snapshot(&self) -> Vec<Metric> {
        self.lock().iter().cloned().collect()
    }

    /// Number of captured metrics.
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    /// Whether nothing has been captured.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, VecDeque<Metric>> {
        match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

/// Build the viewer HTTP router: `GET /metrics` (JSON) and `GET /` (HTML table).
pub fn router(capture: std::sync::Arc<Capture>) -> Router {
    Router::new()
        .route("/metrics", get(metrics_json))
        .route("/", get(viewer))
        .with_state(capture)
}

async fn metrics_json(State(capture): State<std::sync::Arc<Capture>>) -> Json<Vec<Metric>> {
    Json(capture.snapshot())
}

async fn viewer() -> Html<&'static str> {
    Html(VIEWER_HTML)
}

const VIEWER_HTML: &str = r#"<!doctype html><html><head><meta charset="utf-8">
<title>mock datadog</title>
<style>body{font-family:system-ui,sans-serif;margin:1.5rem}
table{border-collapse:collapse;width:100%}th,td{border:1px solid #ddd;padding:4px 8px;font-size:13px;text-align:left}
th{background:#f4f6f9}code{font-size:12px;color:#555}</style></head>
<body><h2>mock datadog — captured DogStatsD</h2>
<p id="count"></p>
<table><thead><tr><th>metric</th><th>value</th><th>type</th><th>tags</th></tr></thead><tbody id="rows"></tbody></table>
<script>
async function refresh(){
  const m = await (await fetch('/metrics')).json();
  document.getElementById('count').textContent = m.length + ' metrics captured';
  document.getElementById('rows').innerHTML = m.slice().reverse().map(x =>
    `<tr><td><code>${x.name}</code></td><td>${x.value}</td><td>${x.kind}</td><td><code>${x.tags.join(', ')}</code></td></tr>`
  ).join('');
}
refresh(); setInterval(refresh, 2000);
</script></body></html>"#;
