//! Apply a [`GatingStrategy`] to the events of an FCS dataset.

use std::collections::HashMap;
use std::sync::Arc;

use cytiq_fcs::{FcsDataset, Spillover, compensate, invert_matrix};
use log::debug;
use serde::Serialize;

use crate::error::GatingError;
use crate::geometry::{in_ellipsoid, in_polygon, in_rectangle};
use crate::model::{BooleanOp, CompensationRef, Dimension, Gate, GateKind, GatingStrategy};

/// Event membership and counts for every gate of a strategy.
#[derive(Clone, Debug)]
pub struct GatingResults {
    event_count: usize,
    gates: Vec<GateResult>,
    index: HashMap<String, usize>,
}

#[derive(Clone, Debug)]
struct GateResult {
    id: String,
    parent_id: Option<String>,
    path: Vec<String>,
    gate_type: &'static str,
    membership: Vec<bool>,
    count: usize,
}

/// One row of the population report.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GateCount {
    pub gate_id: String,
    pub gate_path: String,
    pub parent_id: Option<String>,
    pub gate_type: &'static str,
    pub count: usize,
    pub parent_count: usize,
    pub percent_of_parent: f64,
    pub percent_of_total: f64,
}

impl GatingResults {
    /// Total number of events in the gated sample.
    #[must_use]
    pub const fn event_count(&self) -> usize {
        self.event_count
    }

    /// Gate ids in evaluation order.
    pub fn gate_ids(&self) -> impl Iterator<Item = &str> {
        self.gates.iter().map(|gate| gate.id.as_str())
    }

    /// Per-event membership of a gate (already intersected with its parent).
    #[must_use]
    pub fn membership(&self, gate_id: &str) -> Option<&[bool]> {
        self.result(gate_id).map(|gate| gate.membership.as_slice())
    }

    /// Number of events in a gate.
    #[must_use]
    pub fn count(&self, gate_id: &str) -> Option<usize> {
        self.result(gate_id).map(|gate| gate.count)
    }

    #[must_use]
    pub fn parent(&self, gate_id: &str) -> Option<&str> {
        self.result(gate_id)
            .and_then(|gate| gate.parent_id.as_deref())
    }

    /// Population counts and percentages for every gate, in evaluation order.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn report(&self) -> Vec<GateCount> {
        self.gates
            .iter()
            .map(|gate| {
                let parent_count = gate
                    .parent_id
                    .as_deref()
                    .and_then(|parent| self.count(parent))
                    .unwrap_or(self.event_count);
                let percent = |denominator: usize| {
                    if denominator == 0 {
                        0.0
                    } else {
                        gate.count as f64 / denominator as f64 * 100.0
                    }
                };
                GateCount {
                    gate_id: gate.id.clone(),
                    gate_path: gate.path.join("/"),
                    parent_id: gate.parent_id.clone(),
                    gate_type: gate.gate_type,
                    count: gate.count,
                    parent_count,
                    percent_of_parent: percent(parent_count),
                    percent_of_total: percent(self.event_count),
                }
            })
            .collect()
    }

    fn result(&self, gate_id: &str) -> Option<&GateResult> {
        self.index.get(gate_id).map(|index| &self.gates[*index])
    }
}

impl GatingStrategy {
    /// Evaluate every gate on the events of `dataset`.
    ///
    /// Events are pre-processed as `FlowKit` does: `$PnE` log scaling, `$PnG`
    /// gain and `$TIMESTEP` are applied first, then each dimension's
    /// compensation and transformation.
    ///
    /// # Errors
    ///
    /// Returns an error when a dimension is missing from the dataset, a
    /// compensation matrix is invalid or singular, or an ellipsoid covariance
    /// matrix cannot be inverted.
    pub fn gate_sample(&self, dataset: &FcsDataset) -> Result<GatingResults, GatingError> {
        let mut evaluator = Evaluator::new(self, dataset);
        let mut gates: Vec<GateResult> = Vec::with_capacity(self.gates.len());
        let mut index: HashMap<String, usize> = HashMap::with_capacity(self.gates.len());

        for gate in &self.gates {
            debug!("evaluating gate `{}`", gate.id);
            let mut membership = evaluator.evaluate(gate, |id| &gates[index[id]].membership)?;
            if let Some(parent) = &gate.parent_id {
                let parent_membership = &gates[index[parent.as_str()]].membership;
                for (event, parent_event) in membership.iter_mut().zip(parent_membership) {
                    *event &= *parent_event;
                }
            }
            let count = membership.iter().filter(|event| **event).count();
            index.insert(gate.id.clone(), gates.len());
            gates.push(GateResult {
                id: gate.id.clone(),
                parent_id: gate.parent_id.clone(),
                path: self.gate_path(&gate.id),
                gate_type: gate.gate_type(),
                membership,
                count,
            });
        }

        Ok(GatingResults {
            event_count: dataset.event_count(),
            gates,
            index,
        })
    }

    /// Event values of `dimension` exactly as gate evaluation sees them:
    /// raw pre-processing, then the dimension's compensation and
    /// transformation (min/max are ignored).
    ///
    /// # Errors
    ///
    /// Returns an error when the channel is missing, the transformation or
    /// matrix reference is unknown to this strategy, or compensation fails.
    pub fn dimension_values(
        &self,
        dataset: &FcsDataset,
        dimension: &Dimension,
    ) -> Result<Vec<f64>, GatingError> {
        if let Some(reference) = &dimension.transform
            && self.transform(reference).is_none()
        {
            return Err(GatingError::UnknownTransform {
                gate: dimension.name.clone(),
                reference: reference.clone(),
            });
        }
        if let CompensationRef::Matrix(reference) = &dimension.compensation
            && self.matrix(reference).is_none()
        {
            return Err(GatingError::UnknownCompensation {
                gate: dimension.name.clone(),
                reference: reference.clone(),
            });
        }
        let column = Evaluator::new(self, dataset).column(dimension)?;
        Ok(Arc::try_unwrap(column).unwrap_or_else(|shared| shared.as_ref().clone()))
    }
}

type ColumnKey = (usize, CompensationRef, Option<String>);

struct Evaluator<'a> {
    strategy: &'a GatingStrategy,
    dataset: &'a FcsDataset,
    raw: HashMap<usize, Arc<Vec<f64>>>,
    compensated: HashMap<CompensationRef, Arc<HashMap<usize, Vec<f64>>>>,
    prepared: HashMap<ColumnKey, Arc<Vec<f64>>>,
}

impl<'a> Evaluator<'a> {
    fn new(strategy: &'a GatingStrategy, dataset: &'a FcsDataset) -> Self {
        Self {
            strategy,
            dataset,
            raw: HashMap::new(),
            compensated: HashMap::new(),
            prepared: HashMap::new(),
        }
    }

    fn evaluate<'r, F>(&mut self, gate: &Gate, lookup: F) -> Result<Vec<bool>, GatingError>
    where
        F: Fn(&str) -> &'r [bool],
    {
        let event_count = self.dataset.event_count();
        let columns = gate
            .dimensions()
            .iter()
            .map(|dimension| self.column(dimension))
            .collect::<Result<Vec<_>, _>>()?;
        let slices = columns
            .iter()
            .map(|column| column.as_slice())
            .collect::<Vec<_>>();

        Ok(match &gate.kind {
            GateKind::Rectangle { dimensions } => {
                let bounds = dimensions
                    .iter()
                    .map(|dimension| (dimension.min, dimension.max))
                    .collect::<Vec<_>>();
                in_rectangle(&slices, &bounds)
            }
            GateKind::Polygon { vertices, .. } => in_polygon(slices[0], slices[1], vertices),
            GateKind::Ellipsoid {
                mean,
                covariance,
                distance_square,
                ..
            } => {
                let inverse = invert_matrix(covariance, mean.len()).map_err(|_| {
                    GatingError::InvalidGate {
                        id: gate.id.clone(),
                        reason: "covariance matrix is singular".to_owned(),
                    }
                })?;
                in_ellipsoid(&slices, mean, &inverse, *distance_square)
            }
            GateKind::Boolean { op, refs } => {
                let inputs = refs
                    .iter()
                    .map(|reference| (lookup(&reference.id), reference.complement))
                    .collect::<Vec<_>>();
                (0..event_count)
                    .map(|event| {
                        let mut values = inputs
                            .iter()
                            .map(|(membership, complement)| membership[event] != *complement);
                        match op {
                            BooleanOp::And => values.all(|value| value),
                            BooleanOp::Or => values.any(|value| value),
                            BooleanOp::Not => !values.next().unwrap_or(false),
                        }
                    })
                    .collect()
            }
        })
    }

    /// Compensated and transformed values for a dimension.
    fn column(&mut self, dimension: &Dimension) -> Result<Arc<Vec<f64>>, GatingError> {
        let parameter = self.resolve_parameter(dimension)?;
        let key = (
            parameter,
            dimension.compensation.clone(),
            dimension.transform.clone(),
        );
        if let Some(column) = self.prepared.get(&key) {
            return Ok(Arc::clone(column));
        }

        let compensated = self.compensated_set(&dimension.compensation)?;
        let base = match compensated.get(&parameter) {
            Some(column) => Arc::new(column.clone()),
            None => self.raw_column(parameter),
        };
        let column = match &dimension.transform {
            Some(reference) => {
                let transform = self
                    .strategy
                    .transform(reference)
                    .expect("transformation references are validated at parse time");
                Arc::new(transform.apply_column(&base))
            }
            None => base,
        };
        self.prepared.insert(key, Arc::clone(&column));
        Ok(column)
    }

    /// Find the FCS parameter for a dimension: `$PnN`, `$PnS`, the `FlowJo`
    /// `/`→`_` spelling of `$PnN`, then a fluorochrome of the referenced
    /// spectrum matrix.
    fn resolve_parameter(&self, dimension: &Dimension) -> Result<usize, GatingError> {
        let parameters = &self.dataset.parameters;
        self.dataset
            .find_parameter(&dimension.name)
            .or_else(|| {
                parameters
                    .iter()
                    .position(|parameter| parameter.name.replace('/', "_") == dimension.name)
            })
            .or_else(|| {
                let CompensationRef::Matrix(reference) = &dimension.compensation else {
                    return None;
                };
                let matrix = self.strategy.matrix(reference)?;
                let position = matrix
                    .fluorochromes
                    .iter()
                    .position(|fluorochrome| *fluorochrome == dimension.name)?;
                self.dataset.find_parameter(&matrix.detectors[position])
            })
            .ok_or_else(|| GatingError::MissingDimension {
                name: dimension.name.clone(),
            })
    }

    /// Compensated detector columns for a compensation reference, keyed by
    /// parameter index. Parameters outside the matrix are left raw.
    fn compensated_set(
        &mut self,
        reference: &CompensationRef,
    ) -> Result<Arc<HashMap<usize, Vec<f64>>>, GatingError> {
        if let Some(set) = self.compensated.get(reference) {
            return Ok(Arc::clone(set));
        }

        let compensation_error = |source| GatingError::Compensation {
            reference: compensation_name(reference),
            source,
        };
        let spillover = match reference {
            CompensationRef::Uncompensated => None,
            CompensationRef::Fcs => {
                // GatingML 2.0: `FCS` without spillover metadata means uncompensated.
                Spillover::from_dataset(self.dataset).map_err(compensation_error)?
            }
            CompensationRef::Matrix(id) => {
                let matrix = self
                    .strategy
                    .matrix(id)
                    .expect("compensation references are validated at parse time");
                Some(
                    Spillover::from_parts(matrix.detectors.clone(), matrix.values.clone())
                        .map_err(compensation_error)?,
                )
            }
        };

        let mut set = HashMap::new();
        if let Some(spillover) = spillover {
            let indexes = spillover
                .channels()
                .iter()
                .map(|channel| {
                    self.dataset.find_parameter(channel).ok_or_else(|| {
                        GatingError::MissingDimension {
                            name: channel.clone(),
                        }
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            let comp = spillover
                .compensation_matrix()
                .map_err(compensation_error)?;
            let raw = indexes
                .iter()
                .map(|index| self.raw_column(*index).as_ref().clone())
                .collect::<Vec<_>>();
            set.extend(indexes.into_iter().zip(compensate(&raw, &comp)));
        }
        let set = Arc::new(set);
        self.compensated.insert(reference.clone(), Arc::clone(&set));
        Ok(set)
    }

    /// Stored values with `FlowKit`/flowio "raw" pre-processing applied.
    fn raw_column(&mut self, parameter: usize) -> Arc<Vec<f64>> {
        if let Some(column) = self.raw.get(&parameter) {
            return Arc::clone(column);
        }
        let column = Arc::new(preprocess_raw(self.dataset, parameter));
        self.raw.insert(parameter, Arc::clone(&column));
        column
    }
}

fn compensation_name(reference: &CompensationRef) -> String {
    match reference {
        CompensationRef::Uncompensated => "uncompensated".to_owned(),
        CompensationRef::Fcs => "FCS".to_owned(),
        CompensationRef::Matrix(id) => id.clone(),
    }
}

/// Apply `$PnE` anti-log scaling, `$PnG` gain and `$TIMESTEP`, as `flowio` does.
fn preprocess_raw(dataset: &FcsDataset, parameter_index: usize) -> Vec<f64> {
    let parameter = &dataset.parameters[parameter_index];
    let mut values = dataset
        .data
        .column(parameter_index)
        .expect("every parameter has a data column")
        .to_f64_vec();
    let number = parameter.index;
    let keyword_number = |suffix: &str| {
        keyword(dataset, &format!("$P{number}{suffix}"))
            .and_then(|value| value.trim().parse::<f64>().ok())
    };

    let is_time = parameter.name.eq_ignore_ascii_case("time");
    if is_time
        && let Some(step) =
            keyword(dataset, "$TIMESTEP").and_then(|value| value.trim().parse::<f64>().ok())
    {
        for value in &mut values {
            *value *= step;
        }
    }

    let (decades, log_zero) = keyword(dataset, &format!("$P{number}E"))
        .and_then(|value| {
            let (decades, log_zero) = value.split_once(',')?;
            Some((
                decades.trim().parse::<f64>().ok()?,
                log_zero.trim().parse::<f64>().ok()?,
            ))
        })
        .unwrap_or((0.0, 0.0));
    if decades > 0.0
        && let Some(range) = keyword_number("R")
    {
        // The FCS standard says to use 1 for an invalid log(0) value of 0.
        let log_zero = if log_zero > 0.0 { log_zero } else { 1.0 };
        for value in &mut values {
            *value = 10_f64.powf(decades * *value / range) * log_zero;
        }
    }

    // flowio ignores gain on the time channel; a gain of 0 means "no gain".
    let gain = if is_time {
        None
    } else {
        keyword_number("G").filter(|gain| (*gain - 1.0).abs() > 0.0 && gain.abs() > 0.0)
    };
    if let Some(gain) = gain {
        for value in &mut values {
            *value /= gain;
        }
    }
    values
}

fn keyword<'a>(dataset: &'a FcsDataset, key: &str) -> Option<&'a str> {
    dataset.text.get(key).or_else(|| {
        dataset
            .text
            .iter()
            .find_map(|(candidate, value)| candidate.eq_ignore_ascii_case(key).then_some(value))
    })
}
