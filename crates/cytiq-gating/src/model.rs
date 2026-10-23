//! In-memory representation of a `GatingML` 2.0 gating strategy.

use std::collections::HashMap;

use crate::transforms::Transform;

/// How a dimension's events are compensated before transformation.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum CompensationRef {
    /// Raw (uncompensated) events.
    Uncompensated,
    /// The spillover matrix stored in the FCS `$SPILLOVER`/`$SPILL` keyword.
    Fcs,
    /// A `transforms:spectrumMatrix` declared in the `GatingML` document.
    Matrix(String),
}

/// A gate dimension: an FCS channel plus its compensation, transformation and
/// optional rectangle bounds (in transformed space).
#[derive(Clone, Debug, PartialEq)]
pub struct Dimension {
    pub name: String,
    pub compensation: CompensationRef,
    pub transform: Option<String>,
    pub min: Option<f64>,
    pub max: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BooleanOp {
    And,
    Or,
    Not,
}

/// A reference from a Boolean gate to another gate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GateRef {
    pub id: String,
    pub complement: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum GateKind {
    /// Range (1-D), rectangle (2-D) or hyper-rectangle gate: `min <= x < max`.
    Rectangle { dimensions: Vec<Dimension> },
    /// 2-D polygon, boundary inclusive, crossing-number (odd winding) interior.
    Polygon {
        dimensions: [Dimension; 2],
        vertices: Vec<[f64; 2]>,
    },
    /// n-D ellipsoid: `(x-μ)ᵀ Σ⁻¹ (x-μ) <= distance_square`.
    Ellipsoid {
        dimensions: Vec<Dimension>,
        mean: Vec<f64>,
        /// Row-major `n × n` covariance matrix.
        covariance: Vec<f64>,
        distance_square: f64,
    },
    /// Boolean combination of other gates.
    Boolean { op: BooleanOp, refs: Vec<GateRef> },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Gate {
    pub id: String,
    pub parent_id: Option<String>,
    pub kind: GateKind,
}

impl Gate {
    /// Dimensions read by the gate (empty for Boolean gates).
    #[must_use]
    pub fn dimensions(&self) -> &[Dimension] {
        match &self.kind {
            GateKind::Rectangle { dimensions } | GateKind::Ellipsoid { dimensions, .. } => {
                dimensions
            }
            GateKind::Polygon { dimensions, .. } => dimensions,
            GateKind::Boolean { .. } => &[],
        }
    }

    /// Short gate type name, matching the `GatingML` element name.
    #[must_use]
    pub const fn gate_type(&self) -> &'static str {
        match self.kind {
            GateKind::Rectangle { .. } => "RectangleGate",
            GateKind::Polygon { .. } => "PolygonGate",
            GateKind::Ellipsoid { .. } => "EllipsoidGate",
            GateKind::Boolean { .. } => "BooleanGate",
        }
    }
}

/// A `transforms:spectrumMatrix`: rows are fluorochromes, columns detectors.
#[derive(Clone, Debug, PartialEq)]
pub struct SpectrumMatrix {
    pub fluorochromes: Vec<String>,
    pub detectors: Vec<String>,
    /// Row-major `fluorochromes × detectors` coefficients.
    pub values: Vec<f64>,
}

/// A parsed gating strategy. Gates are stored in evaluation order: every
/// gate appears after its parent and after every gate it references.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GatingStrategy {
    pub(crate) gates: Vec<Gate>,
    pub(crate) index: HashMap<String, usize>,
    pub(crate) transforms: HashMap<String, Transform>,
    pub(crate) matrices: HashMap<String, SpectrumMatrix>,
}

impl GatingStrategy {
    /// Gates in evaluation order.
    #[must_use]
    pub fn gates(&self) -> &[Gate] {
        &self.gates
    }

    #[must_use]
    pub fn gate(&self, id: &str) -> Option<&Gate> {
        self.index.get(id).map(|index| &self.gates[*index])
    }

    #[must_use]
    pub fn transform(&self, id: &str) -> Option<&Transform> {
        self.transforms.get(id)
    }

    #[must_use]
    pub fn matrix(&self, id: &str) -> Option<&SpectrumMatrix> {
        self.matrices.get(id)
    }

    /// Gate ids from the root down to (and including) `id`.
    #[must_use]
    pub fn gate_path(&self, id: &str) -> Vec<String> {
        let mut path = Vec::new();
        let mut current = self.gate(id);
        while let Some(gate) = current {
            path.push(gate.id.clone());
            current = gate
                .parent_id
                .as_deref()
                .and_then(|parent| self.gate(parent));
        }
        path.reverse();
        path
    }
}
