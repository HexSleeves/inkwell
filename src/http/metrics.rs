//! In-process Prometheus metrics registry (CYP-46).
//!
//! Deliberately hand-rolled rather than pulling in `metrics` +
//! `metrics-exporter-prometheus`: the surface we need is one counter, one
//! histogram, and a handful of gauges, and a local registry keeps the
//! dependency graph small, avoids a process-global recorder, and makes the
//! exposition text directly unit-testable. See ADR 0012.
//!
//! ## Cardinality
//!
//! Label values are bounded by construction:
//!
//! - `method` is normalised to a fixed set of HTTP verbs, else `OTHER`;
//! - `route` is the **route template** (`/documents/{slug}`), never the raw
//!   path, so a million document ids collapse to one series;
//! - unmatched requests share [`UNMATCHED_ROUTE`];
//! - once [`MAX_SERIES`] distinct series exist, new ones collapse into
//!   [`OVERFLOW_ROUTE`] and `inkwell_http_metrics_series_dropped_total` moves,
//!   so a cardinality leak is visible instead of unbounded.
//!
//! Nothing user-supplied is ever used as a label value, so `/metrics` cannot
//! leak note content, tokens, or query strings.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Histogram bucket upper bounds, in seconds (the Prometheus client default
/// ladder). `LE_LABELS` holds the matching label text so the exposition output
/// is byte-stable regardless of float formatting.
const BUCKETS: [f64; 11] = [
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];
const LE_LABELS: [&str; 11] = [
    "0.005", "0.01", "0.025", "0.05", "0.1", "0.25", "0.5", "1", "2.5", "5", "10",
];

/// Upper bound on distinct `(method, route, status)` series. Well above the
/// route table times plausible status codes, low enough that a cardinality bug
/// can't exhaust memory.
pub const MAX_SERIES: usize = 2000;

/// `route` label for a request that matched no route (404 fallback).
pub const UNMATCHED_ROUTE: &str = "<unmatched>";

/// `route` label for requests recorded after [`MAX_SERIES`] was reached.
pub const OVERFLOW_ROUTE: &str = "<overflow>";

/// The HTTP verbs that get their own label value. Anything else is `OTHER`.
const KNOWN_METHODS: [&str; 9] = [
    "GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS", "TRACE", "CONNECT",
];

/// Collapse an arbitrary request method into a bounded label value.
fn normalize_method(method: &str) -> &'static str {
    KNOWN_METHODS
        .into_iter()
        .find(|known| *known == method)
        .unwrap_or("OTHER")
}

#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct SeriesKey {
    method: &'static str,
    route: String,
    status: u16,
}

#[derive(Default)]
struct Series {
    count: u64,
    /// Sum of observed latencies in seconds (the histogram `_sum`).
    sum_seconds: f64,
    /// Per-bucket (non-cumulative) counts. `render` turns them into the running
    /// totals Prometheus expects.
    buckets: [u64; BUCKETS.len()],
}

/// Label set for the outbound-webhook counters (CYP-53). Both label values come
/// from a fixed set — `event` from [`crate::webhooks::Event::as_str`] and
/// `result` from [`WEBHOOK_RESULTS`] — so this can never leak cardinality.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct WebhookKey {
    event: &'static str,
    result: &'static str,
}

/// The two possible `result` label values for webhook counters.
const WEBHOOK_RESULTS: [&str; 2] = ["success", "failure"];

fn webhook_result(success: bool) -> &'static str {
    if success {
        WEBHOOK_RESULTS[0]
    } else {
        WEBHOOK_RESULTS[1]
    }
}

#[derive(Default)]
struct Inner {
    series: HashMap<SeriesKey, Series>,
    dropped: u64,
    /// Individual delivery attempts, including retries.
    webhook_attempts: HashMap<WebhookKey, u64>,
    /// Terminal outcome per delivery (one per endpoint per event).
    webhook_deliveries: HashMap<WebhookKey, u64>,
}

/// Snapshot of runtime gauges the registry can't observe itself. Supplied by
/// the `/metrics` handler, which owns the DB pool.
#[derive(Clone, Copy, Debug)]
pub struct RuntimeGauges {
    /// Total connections currently held by the pool (idle + in use).
    pub db_pool_connections: u32,
    /// Connections currently idle in the pool.
    pub db_pool_idle: u32,
}

/// The process-wide HTTP metrics registry. Cheap to clone behind an `Arc`;
/// recording takes a short mutex, which is not a bottleneck next to the DB and
/// render work every request already does.
pub struct Metrics {
    started: Instant,
    inner: Mutex<Inner>,
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

impl Metrics {
    pub fn new() -> Self {
        Self {
            started: Instant::now(),
            inner: Mutex::new(Inner::default()),
        }
    }

    /// Record one finished request. `route` MUST be a route template (or one of
    /// the reserved sentinels) — never a raw request path.
    pub fn record(&self, method: &str, route: &str, status: u16, latency: Duration) {
        let seconds = latency.as_secs_f64();
        let mut inner = self.inner.lock().unwrap_or_else(|poisoned| {
            // A panic while holding the lock must not take metrics down for the
            // rest of the process; the counters are advisory.
            poisoned.into_inner()
        });

        let key = SeriesKey {
            method: normalize_method(method),
            route: route.to_string(),
            status,
        };
        let key = if inner.series.contains_key(&key) || inner.series.len() < MAX_SERIES {
            key
        } else {
            inner.dropped += 1;
            SeriesKey {
                route: OVERFLOW_ROUTE.to_string(),
                ..key
            }
        };

        let series = inner.series.entry(key).or_default();
        series.count += 1;
        series.sum_seconds += seconds;
        // Store per-bucket (non-cumulative) counts and cumulate at render time.
        // An observation above the last bound lands in no bucket and shows up
        // only in `le="+Inf"`, which is exactly the Prometheus convention.
        if let Some(index) = BUCKETS.iter().position(|bound| seconds <= *bound) {
            series.buckets[index] += 1;
        }
    }

    /// Record one outbound-webhook delivery *attempt* (retries included).
    pub fn record_webhook_attempt(&self, event: &'static str, success: bool) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *inner
            .webhook_attempts
            .entry(WebhookKey {
                event,
                result: webhook_result(success),
            })
            .or_default() += 1;
    }

    /// Record the *terminal* outcome of one webhook delivery to one endpoint:
    /// success, or failure after the retry cap.
    pub fn record_webhook_delivery(&self, event: &'static str, success: bool) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *inner
            .webhook_deliveries
            .entry(WebhookKey {
                event,
                result: webhook_result(success),
            })
            .or_default() += 1;
    }

    /// Number of distinct series currently tracked.
    pub fn series_count(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .series
            .len()
    }

    /// Render the full Prometheus text exposition (format version 0.0.4).
    ///
    /// Series are emitted in sorted key order so the body is deterministic,
    /// which keeps diffs and tests stable.
    pub fn render(&self, gauges: RuntimeGauges) -> String {
        let inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut keys: Vec<&SeriesKey> = inner.series.keys().collect();
        keys.sort();

        let mut out = String::with_capacity(1024 + keys.len() * 512);

        out.push_str("# HELP inkwell_build_info Build information for the running binary.\n");
        out.push_str("# TYPE inkwell_build_info gauge\n");
        let _ = writeln!(
            out,
            "inkwell_build_info{{version=\"{}\"}} 1",
            escape_label(env!("CARGO_PKG_VERSION"))
        );

        out.push_str(
            "# HELP inkwell_process_uptime_seconds Seconds since this process started serving.\n",
        );
        out.push_str("# TYPE inkwell_process_uptime_seconds gauge\n");
        let _ = writeln!(
            out,
            "inkwell_process_uptime_seconds {:.3}",
            self.started.elapsed().as_secs_f64()
        );

        out.push_str("# HELP inkwell_db_pool_connections Postgres pool connections, by state.\n");
        out.push_str("# TYPE inkwell_db_pool_connections gauge\n");
        let _ = writeln!(
            out,
            "inkwell_db_pool_connections{{state=\"total\"}} {}",
            gauges.db_pool_connections
        );
        let _ = writeln!(
            out,
            "inkwell_db_pool_connections{{state=\"idle\"}} {}",
            gauges.db_pool_idle
        );

        out.push_str("# HELP inkwell_http_metrics_series Distinct HTTP metric label sets tracked in this process.\n");
        out.push_str("# TYPE inkwell_http_metrics_series gauge\n");
        let _ = writeln!(out, "inkwell_http_metrics_series {}", inner.series.len());

        out.push_str("# HELP inkwell_http_metrics_series_dropped_total Requests folded into the overflow series after the cardinality cap was reached.\n");
        out.push_str("# TYPE inkwell_http_metrics_series_dropped_total counter\n");
        let _ = writeln!(
            out,
            "inkwell_http_metrics_series_dropped_total {}",
            inner.dropped
        );

        out.push_str(
            "# HELP inkwell_http_requests_total HTTP requests handled, by method, route template, and status.\n",
        );
        out.push_str("# TYPE inkwell_http_requests_total counter\n");
        for key in &keys {
            let series = &inner.series[*key];
            let _ = writeln!(
                out,
                "inkwell_http_requests_total{{{}}} {}",
                labels(key),
                series.count
            );
        }

        out.push_str(
            "# HELP inkwell_http_request_duration_seconds HTTP request latency, by method, route template, and status.\n",
        );
        out.push_str("# TYPE inkwell_http_request_duration_seconds histogram\n");
        for key in &keys {
            let series = &inner.series[*key];
            let label_set = labels(key);
            let mut cumulative = 0u64;
            for (index, le) in LE_LABELS.iter().enumerate() {
                cumulative += series.buckets[index];
                let _ = writeln!(
                    out,
                    "inkwell_http_request_duration_seconds_bucket{{{label_set},le=\"{le}\"}} {cumulative}"
                );
            }
            let _ = writeln!(
                out,
                "inkwell_http_request_duration_seconds_bucket{{{label_set},le=\"+Inf\"}} {}",
                series.count
            );
            let _ = writeln!(
                out,
                "inkwell_http_request_duration_seconds_sum{{{label_set}}} {:.6}",
                series.sum_seconds
            );
            let _ = writeln!(
                out,
                "inkwell_http_request_duration_seconds_count{{{label_set}}} {}",
                series.count
            );
        }

        // Outbound webhooks (CYP-53). HELP/TYPE are emitted unconditionally so a
        // scrape from a deployment that has never delivered still declares the
        // families; series appear once something has been recorded.
        out.push_str(
            "# HELP inkwell_webhook_attempts_total Outbound webhook delivery attempts, including retries, by event and result.\n",
        );
        out.push_str("# TYPE inkwell_webhook_attempts_total counter\n");
        for (key, count) in sorted_webhook_counters(&inner.webhook_attempts) {
            let _ = writeln!(
                out,
                "inkwell_webhook_attempts_total{{{}}} {count}",
                webhook_labels(key)
            );
        }

        out.push_str(
            "# HELP inkwell_webhook_deliveries_total Outbound webhook deliveries by terminal outcome, one per endpoint per event.\n",
        );
        out.push_str("# TYPE inkwell_webhook_deliveries_total counter\n");
        for (key, count) in sorted_webhook_counters(&inner.webhook_deliveries) {
            let _ = writeln!(
                out,
                "inkwell_webhook_deliveries_total{{{}}} {count}",
                webhook_labels(key)
            );
        }

        out
    }
}

/// Counter entries in sorted key order, so exposition output stays deterministic.
fn sorted_webhook_counters(counters: &HashMap<WebhookKey, u64>) -> Vec<(&WebhookKey, u64)> {
    let mut entries: Vec<(&WebhookKey, u64)> =
        counters.iter().map(|(key, count)| (key, *count)).collect();
    entries.sort_by(|a, b| a.0.cmp(b.0));
    entries
}

/// The `event`/`result` label set for a webhook counter.
fn webhook_labels(key: &WebhookKey) -> String {
    format!(
        "event=\"{}\",result=\"{}\"",
        escape_label(key.event),
        escape_label(key.result)
    )
}

/// The shared `method`/`route`/`status` label set for a series.
fn labels(key: &SeriesKey) -> String {
    format!(
        "method=\"{}\",route=\"{}\",status=\"{}\"",
        escape_label(key.method),
        escape_label(&key.route),
        key.status
    )
}

/// Escape a label value per the Prometheus text format. Route templates never
/// contain these characters today; escaping keeps the output well-formed if a
/// future route (or the overflow sentinel) ever does.
fn escape_label(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gauges() -> RuntimeGauges {
        RuntimeGauges {
            db_pool_connections: 3,
            db_pool_idle: 2,
        }
    }

    #[test]
    fn route_template_collapses_distinct_ids_into_one_series() {
        let metrics = Metrics::new();
        // Two different documents, same template: one series, count 2.
        metrics.record("GET", "/documents/{slug}", 200, Duration::from_millis(1));
        metrics.record("GET", "/documents/{slug}", 200, Duration::from_millis(2));

        assert_eq!(metrics.series_count(), 1);
        let body = metrics.render(gauges());
        assert!(body.contains(
            "inkwell_http_requests_total{method=\"GET\",route=\"/documents/{slug}\",status=\"200\"} 2"
        ));
    }

    #[test]
    fn histogram_buckets_are_cumulative_and_sum_matches() {
        let metrics = Metrics::new();
        metrics.record("GET", "/healthz", 200, Duration::from_millis(2));
        metrics.record("GET", "/healthz", 200, Duration::from_millis(300));

        let body = metrics.render(gauges());
        let labels = "method=\"GET\",route=\"/healthz\",status=\"200\"";
        // 2 ms lands in the first bucket; 300 ms only from le="0.5" upward.
        assert!(body.contains(&format!(
            "inkwell_http_request_duration_seconds_bucket{{{labels},le=\"0.005\"}} 1"
        )));
        assert!(body.contains(&format!(
            "inkwell_http_request_duration_seconds_bucket{{{labels},le=\"0.25\"}} 1"
        )));
        assert!(body.contains(&format!(
            "inkwell_http_request_duration_seconds_bucket{{{labels},le=\"0.5\"}} 2"
        )));
        assert!(body.contains(&format!(
            "inkwell_http_request_duration_seconds_bucket{{{labels},le=\"+Inf\"}} 2"
        )));
        assert!(body.contains(&format!(
            "inkwell_http_request_duration_seconds_count{{{labels}}} 2"
        )));
        assert!(body.contains(&format!(
            "inkwell_http_request_duration_seconds_sum{{{labels}}} 0.302000"
        )));
    }

    #[test]
    fn observations_above_the_top_bound_only_reach_inf() {
        let metrics = Metrics::new();
        metrics.record("GET", "/ask", 200, Duration::from_secs(30));

        let body = metrics.render(gauges());
        let labels = "method=\"GET\",route=\"/ask\",status=\"200\"";
        assert!(body.contains(&format!(
            "inkwell_http_request_duration_seconds_bucket{{{labels},le=\"10\"}} 0"
        )));
        assert!(body.contains(&format!(
            "inkwell_http_request_duration_seconds_bucket{{{labels},le=\"+Inf\"}} 1"
        )));
        assert!(body.contains(&format!(
            "inkwell_http_request_duration_seconds_sum{{{labels}}} 30.000000"
        )));
    }

    #[test]
    fn unknown_methods_collapse_to_other() {
        let metrics = Metrics::new();
        metrics.record("WEIRD", "/", 405, Duration::ZERO);
        let body = metrics.render(gauges());
        assert!(body.contains("method=\"OTHER\""));
        assert!(!body.contains("WEIRD"));
    }

    #[test]
    fn cardinality_cap_folds_new_series_into_overflow() {
        let metrics = Metrics::new();
        for index in 0..MAX_SERIES {
            metrics.record("GET", &format!("/r{index}"), 200, Duration::ZERO);
        }
        assert_eq!(metrics.series_count(), MAX_SERIES);

        // A brand-new label set past the cap is folded, not added...
        metrics.record("GET", "/overflowing", 200, Duration::ZERO);
        assert_eq!(metrics.series_count(), MAX_SERIES + 1);
        let body = metrics.render(gauges());
        assert!(body.contains(&format!("route=\"{OVERFLOW_ROUTE}\"")));
        assert!(!body.contains("/overflowing"));
        assert!(body.contains("inkwell_http_metrics_series_dropped_total 1"));

        // ...while an already-known label set keeps being recorded exactly.
        metrics.record("GET", "/r0", 200, Duration::ZERO);
        let body = metrics.render(gauges());
        assert!(body.contains(
            "inkwell_http_requests_total{method=\"GET\",route=\"/r0\",status=\"200\"} 2"
        ));
    }

    #[test]
    fn render_includes_gauges_and_build_info() {
        let body = Metrics::new().render(gauges());
        assert!(body.contains(&format!(
            "inkwell_build_info{{version=\"{}\"}} 1",
            env!("CARGO_PKG_VERSION")
        )));
        assert!(body.contains("inkwell_db_pool_connections{state=\"total\"} 3"));
        assert!(body.contains("inkwell_db_pool_connections{state=\"idle\"} 2"));
        assert!(body.contains("inkwell_process_uptime_seconds "));
        assert!(body.contains("inkwell_http_metrics_series 0"));
    }

    #[test]
    fn every_metric_family_declares_help_and_type() {
        let metrics = Metrics::new();
        metrics.record("GET", "/", 200, Duration::from_millis(1));
        let body = metrics.render(gauges());
        for family in [
            "inkwell_build_info",
            "inkwell_process_uptime_seconds",
            "inkwell_db_pool_connections",
            "inkwell_http_metrics_series",
            "inkwell_http_metrics_series_dropped_total",
            "inkwell_http_requests_total",
            "inkwell_http_request_duration_seconds",
            "inkwell_webhook_attempts_total",
            "inkwell_webhook_deliveries_total",
        ] {
            assert!(
                body.contains(&format!("# HELP {family} ")),
                "missing HELP for {family}"
            );
            assert!(
                body.contains(&format!("# TYPE {family} ")),
                "missing TYPE for {family}"
            );
        }
    }

    #[test]
    fn webhook_counters_separate_attempts_from_terminal_outcomes() {
        let metrics = Metrics::new();
        // One delivery that failed twice then succeeded: 3 attempts, 1 delivery.
        metrics.record_webhook_attempt("document.published", false);
        metrics.record_webhook_attempt("document.published", false);
        metrics.record_webhook_attempt("document.published", true);
        metrics.record_webhook_delivery("document.published", true);
        // One delivery that exhausted its retries.
        metrics.record_webhook_attempt("document.unpublished", false);
        metrics.record_webhook_delivery("document.unpublished", false);

        let body = metrics.render(gauges());
        assert!(body.contains(
            "inkwell_webhook_attempts_total{event=\"document.published\",result=\"failure\"} 2"
        ));
        assert!(body.contains(
            "inkwell_webhook_attempts_total{event=\"document.published\",result=\"success\"} 1"
        ));
        assert!(body.contains(
            "inkwell_webhook_deliveries_total{event=\"document.published\",result=\"success\"} 1"
        ));
        assert!(body.contains(
            "inkwell_webhook_deliveries_total{event=\"document.unpublished\",result=\"failure\"} 1"
        ));
        // Webhook counters are their own families and never touch HTTP series.
        assert_eq!(metrics.series_count(), 0);
    }

    #[test]
    fn webhook_families_are_declared_before_anything_is_delivered() {
        let body = Metrics::new().render(gauges());
        assert!(body.contains("# TYPE inkwell_webhook_attempts_total counter"));
        assert!(body.contains("# TYPE inkwell_webhook_deliveries_total counter"));
        // ...with no series until a delivery happens.
        assert!(!body.contains("inkwell_webhook_attempts_total{"));
    }

    #[test]
    fn label_values_are_escaped() {
        assert_eq!(escape_label("a\"b\\c\nd"), "a\\\"b\\\\c\\nd");
        assert_eq!(escape_label("/documents/{slug}"), "/documents/{slug}");
    }
}
