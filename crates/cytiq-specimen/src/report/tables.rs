use super::{ReportData, SpecimenReport};
use crate::{
    Result, SpecimenComparison, SpecimenResult,
    compare::{ComparisonKey, index_rows},
};
use cytiq_population::PopulationObservation;
use serde::Serialize;
use serde_json::Value;
use std::path::Path;

pub(super) struct Table {
    pub filename: String,
    pub title: &'static str,
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

impl Table {
    fn new(filename: &str, title: &'static str, headers: &[&str]) -> Self {
        Self {
            filename: filename.into(),
            title,
            headers: headers.iter().map(|s| (*s).into()).collect(),
            rows: Vec::new(),
        }
    }

    pub fn write_csv(&self, path: &Path) -> Result<()> {
        let mut writer = csv::WriterBuilder::new()
            .terminator(csv::Terminator::Any(b'\n'))
            .from_path(path)?;
        writer.write_record(&self.headers)?;
        for row in &self.rows {
            writer.write_record(row)?;
        }
        writer.flush()?;
        Ok(())
    }
}

pub(super) fn cell(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        _ => value.to_string(),
    }
}

fn enum_text(value: &impl Serialize) -> Result<String> {
    Ok(cell(&serde_json::to_value(value)?))
}

const OBSERVATION_COLUMNS: &[&str] = &[
    "schema_version",
    "observation_id",
    "specimen_id",
    "panel_id",
    "acquisition_id",
    "source_id",
    "target_id",
    "target_kind",
    "target_scope",
    "target_state",
    "measure",
    "unit",
    "status",
    "value_u64",
    "value_f64",
    "n_events",
    "supporting_count",
    "reason",
    "target",
    "denominator",
    "method",
    "lineage",
];

fn observation_cells(row: Option<&PopulationObservation>) -> Result<Vec<String>> {
    let Some(row) = row else {
        return Ok(vec![String::new(); OBSERVATION_COLUMNS.len()]);
    };
    let value = serde_json::to_value(row)?;
    Ok(OBSERVATION_COLUMNS
        .iter()
        .map(|name| match *name {
            "target_id" => cell(&value["target"]["id"]),
            "target_kind" => cell(&value["target"]["kind"]),
            "target_scope" => cell(&value["target"]["scope"]),
            "target_state" => cell(&value["target"]["state"]),
            _ => cell(&value[*name]),
        })
        .collect())
}

fn observations(
    filename: &str,
    title: &'static str,
    rows: &[PopulationObservation],
) -> Result<Table> {
    let mut table = Table::new(filename, title, OBSERVATION_COLUMNS);
    for row in rows {
        table.rows.push(observation_cells(Some(row))?);
    }
    Ok(table)
}

fn specimen_tables(result: &SpecimenResult) -> Result<Vec<Table>> {
    let mut summary = Table::new(
        "summary.csv",
        "Specimen summary",
        &[
            "specimen_id",
            "protocol_id",
            "complete",
            "status",
            "observations",
            "results",
            "checks",
        ],
    );
    summary.rows.push(vec![
        result.specimen_id.clone(),
        result.protocol_id.clone(),
        result.complete.to_string(),
        enum_text(&result.status)?,
        result.observations.len().to_string(),
        result.results.len().to_string(),
        result.checks.len().to_string(),
    ]);
    let mut checks = Table::new(
        "checks.csv",
        "QC checks",
        &["category", "scope", "check", "status", "message"],
    );
    for check in &result.checks {
        checks.rows.push(vec![
            check.category.clone(),
            check.scope.clone(),
            check.check.clone(),
            enum_text(&check.status)?,
            check.message.clone(),
        ]);
    }
    Ok(vec![
        summary,
        observations(
            "observations.csv",
            "Original observations",
            &result.observations,
        )?,
        observations("results.csv", "Reconciled results", &result.results)?,
        checks,
    ])
}

fn key_cells(key: &ComparisonKey) -> Result<Vec<String>> {
    Ok(vec![
        enum_text(&key.collection)?,
        key.panel_id.clone(),
        key.source_id.clone(),
        key.target.id.clone(),
        enum_text(&key.target.kind)?,
        key.target.scope.clone(),
        key.target.state.map_or_else(String::new, |b| b.to_string()),
        enum_text(&key.measure)?,
    ])
}

const KEY_COLUMNS: &[&str] = &[
    "collection",
    "panel_id",
    "source_id",
    "target_id",
    "target_kind",
    "target_scope",
    "target_state",
    "measure",
];

fn comparison_tables(diff: &SpecimenComparison) -> Result<Vec<Table>> {
    let mut summary = Table::new(
        "summary.csv",
        "Comparison summary",
        &[
            "specimen_id",
            "verdict",
            "baseline_protocol",
            "candidate_protocol",
            "baseline_complete",
            "candidate_complete",
            "baseline_qc",
            "candidate_qc",
            "rows",
            "numeric_pairs",
            "failed_rows",
            "failed_checks",
            "specimen_changed",
            "context_changes",
        ],
    );
    summary.rows.push(vec![
        diff.baseline.specimen_id.clone(),
        enum_text(&diff.outcome_reason())?,
        diff.baseline.protocol_id.clone(),
        diff.candidate.protocol_id.clone(),
        diff.baseline.complete.to_string(),
        diff.candidate.complete.to_string(),
        enum_text(&diff.baseline.status)?,
        enum_text(&diff.candidate.status)?,
        diff.summary.rows.to_string(),
        diff.summary.numeric_pairs.to_string(),
        diff.summary.failed_rows.to_string(),
        diff.summary.failed_checks.to_string(),
        diff.summary.specimen_changed.to_string(),
        diff.summary.context_changes.to_string(),
    ]);
    let rows = comparison_rows(diff)?;
    let mut checks = Table::new(
        "checks.csv",
        "QC comparison",
        &[
            "category",
            "scope",
            "check",
            "passed",
            "baseline",
            "candidate",
        ],
    );
    for c in &diff.checks {
        checks.rows.push(vec![
            c.category.clone(),
            c.scope.clone(),
            c.check.clone(),
            c.passed.to_string(),
            serde_json::to_string(&c.baseline)?,
            serde_json::to_string(&c.candidate)?,
        ]);
    }
    let mut context = Table::new("context.csv", "Context changes", KEY_COLUMNS);
    context
        .headers
        .extend(["field", "baseline", "candidate"].map(str::to_owned));
    for c in &diff.context {
        let mut cells = if let Some(key) = &c.key {
            key_cells(key)?
        } else {
            vec![String::new(); KEY_COLUMNS.len()]
        };
        cells.extend([c.field.clone(), cell(&c.baseline), cell(&c.candidate)]);
        context.rows.push(cells);
    }
    Ok(vec![summary, rows, checks, context])
}

fn comparison_rows(diff: &SpecimenComparison) -> Result<Table> {
    let mut table = Table::new("rows.csv", "Measurement comparison", KEY_COLUMNS);
    table.headers.extend(
        [
            "verdict",
            "reasons",
            "absolute_delta_u64",
            "direction",
            "absolute_delta_f64",
            "signed_delta_f64",
            "tolerance_count",
            "tolerance_float",
        ]
        .map(str::to_owned),
    );
    for prefix in ["baseline", "candidate"] {
        table.headers.extend(
            OBSERVATION_COLUMNS
                .iter()
                .map(|name| format!("{prefix}_{name}")),
        );
    }
    let baseline = index_rows(&diff.baseline)?;
    let candidate = index_rows(&diff.candidate)?;
    for row in &diff.rows {
        let mut cells = key_cells(&row.key)?;
        let mut delta = vec![String::new(); 4];
        if let Some(d) = &row.delta {
            match d {
                crate::compare::Delta::Count {
                    absolute,
                    direction,
                } => {
                    delta[0] = absolute.to_string();
                    delta[1] = enum_text(direction)?;
                }
                crate::compare::Delta::Float { absolute, signed } => {
                    delta[2] = absolute.to_string();
                    delta[3] = signed.to_string();
                }
            }
        }
        cells.extend([
            enum_text(&row.verdict)?,
            serde_json::to_string(&row.reasons)?,
        ]);
        cells.extend(delta);
        let limits = match &row.tolerance {
            crate::compare::Tolerance::Count(n) => [n.to_string(), String::new()],
            crate::compare::Tolerance::Float(n) => [String::new(), n.to_string()],
        };
        cells.extend(limits);
        let token = row.key.token()?;
        cells.extend(observation_cells(baseline.get(&token).map(|(_, o)| *o))?);
        cells.extend(observation_cells(candidate.get(&token).map(|(_, o)| *o))?);
        table.rows.push(cells);
    }
    Ok(table)
}

pub(super) fn build(report: &SpecimenReport) -> Result<Vec<Table>> {
    match &report.content {
        ReportData::Specimen(result) => specimen_tables(result),
        ReportData::Comparison(diff) => comparison_tables(diff),
    }
}
