//! Mask metrics and the policy rules evaluated on them.

use cytiq_manifest::{FilterPolicy, QcPolicy, Severity};
use cytiq_mask::EventMask;
use serde::Serialize;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Pass,
    Warn,
    Fail,
}

impl From<Severity> for CheckStatus {
    fn from(severity: Severity) -> Self {
        match severity {
            Severity::Fail => Self::Fail,
            Severity::Warn => Self::Warn,
        }
    }
}

/// One QC finding. `value` and `limit` hold the measured number and the
/// policy bound when the check compares them.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct QcCheck {
    /// `sample S01` or `filter peacoqc_pass`.
    pub scope: String,
    pub check: &'static str,
    pub status: CheckStatus,
    pub message: String,
    pub value: Option<f64>,
    pub limit: Option<f64>,
}

impl QcCheck {
    pub(crate) fn new(
        scope: &str,
        check: &'static str,
        status: CheckStatus,
        message: String,
    ) -> Self {
        Self {
            scope: scope.to_owned(),
            check,
            status,
            message,
            value: None,
            limit: None,
        }
    }

    const fn compared(mut self, value: f64, limit: f64) -> Self {
        self.value = Some(value);
        self.limit = Some(limit);
        self
    }
}

/// What one filter did, over every mask column that carries it: `live`, and
/// `2.live` when `cytiq mask combine` met the name twice.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FilterMetrics {
    pub name: String,
    pub columns: Vec<String>,
    /// Events at least one column assessed (non-null).
    pub assessed: usize,
    /// Events at least one column failed.
    pub removed: usize,
    /// `removed / assessed`; `None` when the filter assessed nothing.
    pub removed_fraction: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct QcMetrics {
    /// Events in the raw sample file.
    pub events: usize,
    /// Raw events the mask keeps.
    pub kept: usize,
    pub removed_fraction: f64,
    pub filters: Vec<FilterMetrics>,
}

impl QcMetrics {
    /// Metrics of `mask` for a raw file of `events` events, `kept` of which
    /// the mask keeps.
    #[must_use]
    pub fn of(mask: &EventMask, events: usize, kept: usize) -> Self {
        Self {
            events,
            kept,
            removed_fraction: fraction(events - kept, events).unwrap_or(0.0),
            filters: filter_metrics(mask),
        }
    }
}

/// `2.margin_pass` → `margin_pass`: the prefix `cytiq mask combine` adds to a
/// repeated filter name.
fn base_name(column: &str) -> &str {
    match column.split_once('.') {
        Some((index, name)) if !index.is_empty() && index.bytes().all(|b| b.is_ascii_digit()) => {
            name
        }
        _ => column,
    }
}

#[allow(clippy::cast_precision_loss)]
fn fraction(part: usize, whole: usize) -> Option<f64> {
    (whole > 0).then(|| part as f64 / whole as f64)
}

fn filter_metrics(mask: &EventMask) -> Vec<FilterMetrics> {
    let mut names = Vec::<&str>::new();
    for (column, _) in &mask.filters {
        let name = base_name(column);
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names
        .into_iter()
        .map(|name| {
            let columns = mask
                .filters
                .iter()
                .filter(|(column, _)| base_name(column) == name)
                .collect::<Vec<_>>();
            let (mut assessed, mut removed) = (0, 0);
            for row in 0..mask.len() {
                let values = columns.iter().map(|(_, values)| values[row]);
                if values.clone().any(|value| value.is_some()) {
                    assessed += 1;
                }
                if values.into_iter().any(|value| value == Some(false)) {
                    removed += 1;
                }
            }
            FilterMetrics {
                name: name.to_owned(),
                columns: columns.iter().map(|(column, _)| column.clone()).collect(),
                assessed,
                removed,
                removed_fraction: fraction(removed, assessed),
            }
        })
        .collect()
}

fn percent(value: f64) -> String {
    format!("{:.1}%", value * 100.0)
}

/// Every rule of `policy` checked against `metrics`.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn evaluate(sample_scope: &str, policy: &QcPolicy, metrics: &QcMetrics) -> Vec<QcCheck> {
    let mut checks = Vec::new();
    let at_least = |check, what: &str, value: usize, limit: u64| {
        let status = if value as u64 >= limit {
            CheckStatus::Pass
        } else {
            CheckStatus::Fail
        };
        QcCheck::new(
            sample_scope,
            check,
            status,
            format!("{value} {what} (minimum {limit})"),
        )
        .compared(value as f64, limit as f64)
    };
    if let Some(limit) = policy.min_events {
        checks.push(at_least(
            "min_events",
            "raw event(s)",
            metrics.events,
            limit,
        ));
    }
    if let Some(limit) = policy.min_kept_events {
        checks.push(at_least(
            "min_kept_events",
            "kept event(s)",
            metrics.kept,
            limit,
        ));
    }
    if let Some(limit) = policy.max_removed_fraction {
        let value = metrics.removed_fraction;
        checks.push(
            QcCheck::new(
                sample_scope,
                "max_removed_fraction",
                if value <= limit {
                    CheckStatus::Pass
                } else {
                    CheckStatus::Fail
                },
                format!(
                    "{} of raw events removed (maximum {})",
                    percent(value),
                    percent(limit)
                ),
            )
            .compared(value, limit),
        );
    }
    for (name, rule) in &policy.filters {
        checks.extend(evaluate_filter(name, rule, metrics));
    }
    checks
}

fn evaluate_filter(name: &str, rule: &FilterPolicy, metrics: &QcMetrics) -> Vec<QcCheck> {
    let mut checks = Vec::new();
    let scope = format!("filter {name}");
    let violated = CheckStatus::from(rule.severity);
    let found = metrics.filters.iter().find(|filter| filter.name == name);
    let Some(removed_fraction) = found.and_then(|filter| filter.removed_fraction) else {
        let what = if found.is_some() {
            "assessed no event"
        } else {
            "is not in the mask"
        };
        checks.push(QcCheck::new(
            &scope,
            "filter_assessed",
            if rule.required {
                violated
            } else {
                CheckStatus::Pass
            },
            format!(
                "`{name}` {what}{}",
                if rule.required { "" } else { " (optional)" }
            ),
        ));
        return checks;
    };
    if rule.required {
        checks.push(QcCheck::new(
            &scope,
            "filter_assessed",
            CheckStatus::Pass,
            format!("`{name}` ran"),
        ));
    }
    if let Some(limit) = rule.max_removed_fraction {
        checks.push(
            QcCheck::new(
                &scope,
                "filter_removed_fraction",
                if removed_fraction <= limit {
                    CheckStatus::Pass
                } else {
                    violated
                },
                format!(
                    "`{name}` removed {} of the events it assessed (maximum {})",
                    percent(removed_fraction),
                    percent(limit)
                ),
            )
            .compared(removed_fraction, limit),
        );
    }
    checks
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    const RAW: &str = "1111111111111111111111111111111111111111111111111111111111111111";

    /// Ten raw events: margins removed 1, `PeacoQC` (as `2.margin_pass` and
    /// `peacoqc_pass`) removed 3 of the 9 it assessed, `live` never ran.
    fn mask() -> EventMask {
        let flags = |removed: &[usize], skipped: &[usize]| {
            (0..10)
                .map(|row| (!skipped.contains(&row)).then_some(!removed.contains(&row)))
                .collect::<Vec<_>>()
        };
        let keep = (0..10).map(|row| ![0, 4, 5, 6].contains(&row)).collect();
        EventMask::new(RAW, keep, None)
            .with_filter("margin_pass", flags(&[0], &[]))
            .with_filter("2.margin_pass", flags(&[], &(0..10).collect::<Vec<_>>()))
            .with_filter("peacoqc_pass", flags(&[4, 5, 6], &[0]))
            .with_filter("live", vec![None; 10])
    }

    fn policy(filters: &[(&str, FilterPolicy)]) -> QcPolicy {
        QcPolicy {
            min_events: Some(10),
            min_kept_events: Some(7),
            max_removed_fraction: Some(0.3),
            filters: filters
                .iter()
                .map(|(name, rule)| ((*name).to_owned(), rule.clone()))
                .collect::<BTreeMap<_, _>>(),
        }
    }

    #[test]
    fn groups_prefixed_columns_into_one_filter() {
        let metrics = QcMetrics::of(&mask(), 10, 6);
        let margin = &metrics.filters[0];
        assert_eq!(margin.name, "margin_pass");
        assert_eq!(margin.columns, ["margin_pass", "2.margin_pass"]);
        assert_eq!((margin.assessed, margin.removed), (10, 1));
        let peacoqc = &metrics.filters[1];
        assert_eq!((peacoqc.assessed, peacoqc.removed), (9, 3));
        assert_eq!(metrics.filters[2].removed_fraction, None);
        assert!((metrics.removed_fraction - 0.4).abs() < 1e-12);
    }

    #[test]
    fn checks_counts_and_overall_loss() {
        let checks = evaluate("sample S01", &policy(&[]), &QcMetrics::of(&mask(), 10, 6));
        let status = |name: &str| {
            checks
                .iter()
                .find(|check| check.check == name)
                .map(|check| check.status)
        };
        assert_eq!(status("min_events"), Some(CheckStatus::Pass));
        assert_eq!(status("min_kept_events"), Some(CheckStatus::Fail));
        assert_eq!(status("max_removed_fraction"), Some(CheckStatus::Fail));
    }

    #[test]
    fn checks_filters_with_their_severity() {
        let rule = |required, max: Option<f64>, severity| FilterPolicy {
            required,
            max_removed_fraction: max,
            severity,
        };
        let checks = evaluate(
            "sample S01",
            &policy(&[
                ("peacoqc_pass", rule(true, Some(0.25), Severity::Warn)),
                ("margin_pass", rule(true, Some(0.2), Severity::Fail)),
                ("live", rule(true, None, Severity::Fail)),
                ("singlets", rule(false, Some(0.1), Severity::Fail)),
            ]),
            &QcMetrics::of(&mask(), 10, 6),
        );
        let find = |scope: &str, name: &str| {
            checks
                .iter()
                .find(|check| check.scope == scope && check.check == name)
                .unwrap_or_else(|| panic!("{scope} {name}: {checks:?}"))
        };
        let peacoqc = find("filter peacoqc_pass", "filter_removed_fraction");
        assert_eq!(peacoqc.status, CheckStatus::Warn, "3 of 9 > 25%, warn only");
        assert_eq!(peacoqc.limit, Some(0.25));
        assert_eq!(
            find("filter margin_pass", "filter_removed_fraction").status,
            CheckStatus::Pass
        );
        let live = find("filter live", "filter_assessed");
        assert_eq!(live.status, CheckStatus::Fail);
        assert!(
            live.message.contains("assessed no event"),
            "{}",
            live.message
        );
        let singlets = find("filter singlets", "filter_assessed");
        assert_eq!(singlets.status, CheckStatus::Pass, "optional and absent");
    }
}
