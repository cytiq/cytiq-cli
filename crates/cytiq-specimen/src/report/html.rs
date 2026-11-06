use super::{ReportData, SpecimenReport, tables::Table};
use crate::Result;
use std::io::Write;

const STYLE: &str = "body{margin:0;background:#f5f7fa;color:#172a3a;font:15px/1.5 system-ui,sans-serif}main{max-width:1440px;margin:auto;padding:36px 24px}h1{font-size:30px;margin:8px 0}h2{font-size:20px}header,section{background:white;border:1px solid #dce3eb;border-radius:12px;padding:22px;margin-bottom:20px}nav{display:flex;gap:18px;flex-wrap:wrap;margin-top:18px}a{color:#165c9b}p{max-width:100ch}.eyebrow{color:#4b647b;font-size:12px;font-weight:700;letter-spacing:.12em;text-transform:uppercase}.verdict{font-weight:700;font-size:18px}table{border-collapse:collapse;width:100%;font-size:13px}th,td{text-align:left;vertical-align:top;padding:10px 12px;border-bottom:1px solid #e2e8ef}th{background:#edf2f7;white-space:nowrap}td{max-width:340px;overflow-wrap:anywhere}tr:hover{background:#f7fafc}.scroll{overflow:auto;max-height:680px}pre{white-space:pre-wrap;overflow-wrap:anywhere;margin:8px 0;font-size:12px}summary{cursor:pointer;color:#165c9b}details{min-width:120px}.empty{color:#526779}footer{color:#526779;font-size:12px}@media print{body{background:white}main{padding:0}.scroll{overflow:visible;max-height:none}section{break-inside:avoid}nav{display:none}}";

fn escape(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&#39;"),
            _ => output.push(c),
        }
    }
    output
}

pub(super) fn write(mut out: impl Write, report: &SpecimenReport, tables: &[Table]) -> Result<()> {
    let (title, specimen, verdict) = match &report.content {
        ReportData::Specimen(r) => (
            "Specimen result",
            r.specimen_id.as_str(),
            serde_json::to_string(&r.outcome_reason())?,
        ),
        ReportData::Comparison(c) => (
            "Specimen comparison",
            c.baseline.specimen_id.as_str(),
            serde_json::to_string(&c.outcome_reason())?,
        ),
    };
    write!(
        out,
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{} · {}</title><style>{STYLE}</style></head><body><main><header><div class=\"eyebrow\">CYTIQ · {title}</div><h1>{}</h1><p class=\"verdict\">Source verdict: {}</p><p>Observed counts and derived estimates are shown separately. Fractions use units 0–1. Empty numeric cells mean unavailable; measured zero remains 0.</p><nav>",
        escape(specimen),
        title,
        escape(specimen),
        escape(verdict.trim_matches('"'))
    )?;
    for (index, table) in tables.iter().enumerate() {
        write!(
            out,
            "<a href=\"#table-{index}\">{}</a>",
            escape(table.title)
        )?;
    }
    write!(
        out,
        "<a href=\"#provenance\">Provenance &amp; definitions</a></nav></header>"
    )?;
    if let ReportData::Comparison(c) = &report.content {
        write!(
            out,
            "<section><strong>{} failed measurement comparisons</strong> out of {} rows · {} numeric pairs · {} failed QC comparisons</section>",
            c.summary.failed_rows, c.summary.rows, c.summary.numeric_pairs, c.summary.failed_checks
        )?;
    }
    for (index, table) in tables.iter().enumerate() {
        write_table(&mut out, index, table)?;
    }
    write!(
        out,
        "<section id=\"provenance\"><h2>Provenance &amp; definitions</h2><details><summary>Source package identity and completion</summary><pre>{}</pre></details>",
        escape(&serde_json::to_string_pretty(&(
            &report.source,
            &report.source_outcome
        ))?)
    )?;
    if let ReportData::Comparison(c) = &report.content {
        write!(
            out,
            "<details><summary>Effective comparison policy</summary><pre>{}</pre></details><details><summary>Baseline and candidate package identities</summary><pre>{}</pre></details>",
            escape(&serde_json::to_string_pretty(&c.policy)?),
            escape(&serde_json::to_string_pretty(&(
                &c.baseline_package,
                &c.candidate_package
            ))?)
        )?;
    }
    write!(
        out,
        "</section><footer>Summary-only export. Report generation success is independent of source QC and comparison verdict. Package digests are recorded in provenance.json and outcome.json.</footer></main></body></html>"
    )?;
    out.flush()?;
    Ok(())
}

fn write_table(out: &mut impl Write, index: usize, table: &Table) -> Result<()> {
    let compact;
    let table = if table.filename == "rows.csv" {
        compact = comparison_view(table)?;
        &compact
    } else {
        table
    };
    write!(
        out,
        "<section id=\"table-{index}\"><h2>{}</h2>",
        escape(table.title)
    )?;
    if table.rows.is_empty() {
        write!(out, "<p class=\"empty\">No rows.</p></section>")?;
        return Ok(());
    }
    // Keep the main table readable; all remaining columns stay available per row.
    let primary: Vec<_> = table
        .headers
        .iter()
        .enumerate()
        .filter(|(_, h)| primary_column(h))
        .map(|(i, _)| i)
        .collect();
    let secondary: Vec<_> = (0..table.headers.len())
        .filter(|i| !primary.contains(i))
        .collect();
    write!(out, "<div class=\"scroll\"><table><thead><tr>")?;
    for &i in &primary {
        write!(out, "<th scope=\"col\">{}</th>", escape(&table.headers[i]))?;
    }
    if !secondary.is_empty() {
        write!(out, "<th scope=\"col\">Details</th>")?;
    }
    write!(out, "</tr></thead><tbody>")?;
    for row in &table.rows {
        write!(out, "<tr>")?;
        for &i in &primary {
            write!(out, "<td>{}</td>", escape(&row[i]))?;
        }
        if !secondary.is_empty() {
            write!(out, "<td><details><summary>View details</summary>")?;
            for &i in &secondary {
                write!(
                    out,
                    "<strong>{}</strong><pre>{}</pre>",
                    escape(&table.headers[i]),
                    escape(&row[i])
                )?;
            }
            write!(out, "</details></td>")?;
        }
        write!(out, "</tr>")?;
    }
    write!(out, "</tbody></table></div></section>")?;
    Ok(())
}

fn comparison_view(table: &Table) -> Result<Table> {
    let mut view = Table {
        filename: table.filename.clone(),
        title: table.title,
        headers: [
            "layer",
            "source",
            "population",
            "measure",
            "baseline",
            "candidate",
            "change",
            "limit",
            "verdict",
            "full_row",
        ]
        .map(str::to_owned)
        .to_vec(),
        rows: Vec::with_capacity(table.rows.len()),
    };
    for row in &table.rows {
        let fields: std::collections::BTreeMap<_, _> = table
            .headers
            .iter()
            .map(String::as_str)
            .zip(row.iter().map(String::as_str))
            .collect();
        let get = |name: &str| fields.get(name).copied().unwrap_or("");
        let side = |prefix: &str| {
            let integer = get(&format!("{prefix}_value_u64"));
            let float = get(&format!("{prefix}_value_f64"));
            let status = get(&format!("{prefix}_status"));
            let value = if !integer.is_empty() {
                integer
            } else if !float.is_empty() {
                float
            } else {
                "—"
            };
            if status.is_empty() {
                "absent".to_owned()
            } else {
                format!("{value} ({status})")
            }
        };
        let count_delta = get("absolute_delta_u64");
        let change = if count_delta.is_empty() {
            get("signed_delta_f64").to_owned()
        } else {
            format!(
                "{}{count_delta}",
                match get("direction") {
                    "increased" => "+",
                    "decreased" => "−",
                    _ => "",
                }
            )
        };
        let limit = if get("tolerance_count").is_empty() {
            get("tolerance_float")
        } else {
            get("tolerance_count")
        };
        view.rows.push(vec![
            get("collection").into(),
            format!("{} / {}", get("panel_id"), get("source_id")),
            get("target_id").into(),
            get("measure").into(),
            side("baseline"),
            side("candidate"),
            change,
            limit.into(),
            get("verdict").into(),
            serde_json::to_string_pretty(&fields)?,
        ]);
    }
    Ok(view)
}

fn primary_column(name: &str) -> bool {
    matches!(
        name,
        "layer"
            | "source"
            | "population"
            | "change"
            | "limit"
            | "specimen_id"
            | "protocol_id"
            | "complete"
            | "status"
            | "collection"
            | "panel_id"
            | "source_id"
            | "target_id"
            | "measure"
            | "unit"
            | "value_u64"
            | "value_f64"
            | "verdict"
            | "category"
            | "scope"
            | "check"
            | "message"
            | "passed"
            | "field"
            | "baseline"
            | "candidate"
            | "absolute_delta_u64"
            | "direction"
            | "absolute_delta_f64"
            | "baseline_status"
            | "candidate_status"
            | "baseline_value_u64"
            | "candidate_value_u64"
            | "baseline_value_f64"
            | "candidate_value_f64"
            | "baseline_qc"
            | "candidate_qc"
    )
}
