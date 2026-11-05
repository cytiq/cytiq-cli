use std::collections::BTreeMap;

use cytiq_cytonorm::{
    CytoNormModel, Goal, Mapping, Marker, MarkerContract, Mode, ModelDocument, MonotoneSpline,
    SomSettings, SparsePolicy, Target,
};
use cytiq_flowsom::{FlowSomModel, Metaclustering};
use serde_json::json;

fn model() -> CytoNormModel {
    let markers: MarkerContract = serde_json::from_value(json!({"panel":{"id":"panel", "markers":[{"marker":"CD3","channel":"A"},{"marker":"CD4","channel":"B"}]},"transform":{"method":"arcsinh","parameters":{"cofactor":150}}})).unwrap();
    let som: FlowSomModel = serde_json::from_value(json!({"schema_version":1,"channels":["A","B"],"channel_labels":[["CD3"],["CD4"]],"xdim":2,"ydim":1,"seed":42,"scale_input":false,"scale":[{"channel":"A","mean":0.0,"standard_deviation":1.0},{"channel":"B","mean":0.0,"standard_deviation":1.0}],"codebook":[[0.0,0.0],[1.0,1.0]],"node_metaclusters":[1,2],"metaclustering":"consensus","training_events":10,"transform":markers.transform})).unwrap();
    let goals = (1..=2)
        .flat_map(|metacluster| {
            ["CD3", "CD4"].map(|marker| Goal {
                metacluster,
                marker: marker.into(),
                quantiles: Some(vec![1.0, 2.0]),
            })
        })
        .collect();
    let document = ModelDocument {
        schema_version: 1,
        algorithm: "cytonorm-2.0".into(),
        mode: Mode::Controls,
        target: Target::Mean,
        batches: BTreeMap::from([("a".into(), None), ("b".into(), None)]),
        normalization_markers: vec!["CD3".into(), "CD4".into()],
        probabilities: vec![0.0, 1.0],
        sparse_policy: SparsePolicy::Error,
        min_cluster_events: 2,
        som: SomSettings {
            xdim: 2,
            ydim: 1,
            metaclusters: 2,
            scale: false,
            seed: 42,
        },
        training_events: 10,
        training_inputs: BTreeMap::new(),
        files: BTreeMap::new(),
        goals,
    };
    let mut mappings = Vec::new();
    for batch in ["a", "b"] {
        for metacluster in 1..=2 {
            for marker in ["CD3", "CD4"] {
                mappings.push(Mapping {
                    batch: batch.into(),
                    metacluster,
                    marker: marker.into(),
                    events: 2,
                    spline: Some(MonotoneSpline::fit(&[0.0, 1.0], &[1.0, 2.0]).unwrap()),
                    identity_reason: None,
                });
            }
        }
    }
    CytoNormModel {
        document,
        markers,
        som,
        mappings,
        model_sha256: String::new(),
        panel_sha256: String::new(),
    }
}

type BreakModel = fn(&mut CytoNormModel);

#[test]
fn model_checks_an_asymmetric_complete_batch_cluster_marker_grid() {
    let mut m = model();
    m.markers.panel.markers.push(Marker {
        marker: "CD8".into(),
        channel: "C".into(),
        labels: vec![],
    });
    m.document.normalization_markers.push("CD8".into());
    for cluster in 1..=2 {
        m.document.goals.push(Goal {
            metacluster: cluster,
            marker: "CD8".into(),
            quantiles: Some(vec![1.0, 2.0]),
        });
        for batch in ["a", "b"] {
            m.mappings.push(Mapping {
                batch: batch.into(),
                metacluster: cluster,
                marker: "CD8".into(),
                ..m.mappings[0].clone()
            });
        }
    }
    m.document.batches.insert("c".into(), None);
    let third_batch = m
        .mappings
        .iter()
        .filter(|mapping| mapping.batch == "a")
        .map(|mapping| Mapping {
            batch: "c".into(),
            ..mapping.clone()
        })
        .collect::<Vec<_>>();
    m.mappings.extend(third_batch);
    assert_eq!(m.document.goals.len(), 6);
    assert_eq!(m.mappings.len(), 18);
    assert!(m.validate().is_ok());
    let mut missing_goal = m.clone();
    missing_goal.document.goals.pop();
    assert!(
        missing_goal
            .validate()
            .unwrap_err()
            .to_string()
            .contains("missing goal distribution")
    );
    m.mappings.pop();
    assert!(
        m.validate()
            .unwrap_err()
            .to_string()
            .contains("missing batch/cluster/marker mapping")
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn model_rejects_each_inconsistent_panel_som_and_training_contract() {
    assert!(model().validate().is_ok());
    let cases: &[(&str, BreakModel)] = &[
        ("version", |m| m.document.schema_version = 2),
        ("algorithm", |m| m.document.algorithm = "other".into()),
        ("panel id", |m| m.markers.panel.id = " ".into()),
        ("empty panel", |m| m.markers.panel.markers.clear()),
        ("duplicate marker", |m| {
            m.markers.panel.markers[1].marker = "CD3".into();
        }),
        ("duplicate channel", |m| {
            m.markers.panel.markers[1].channel = "A".into();
        }),
        ("unused duplicate marker", |m| {
            m.markers.panel.markers.push(Marker {
                marker: "CD3".into(),
                channel: "C".into(),
                labels: vec![],
            });
        }),
        ("unused duplicate channel", |m| {
            m.markers.panel.markers.push(Marker {
                marker: "CD8".into(),
                channel: "A".into(),
                labels: vec![],
            });
        }),
        ("unused empty marker", |m| {
            m.markers.panel.markers.push(Marker {
                marker: String::new(),
                channel: "C".into(),
                labels: vec![],
            });
        }),
        ("unused empty channel", |m| {
            m.markers.panel.markers.push(Marker {
                marker: "CD8".into(),
                channel: String::new(),
                labels: vec![],
            });
        }),
        ("empty marker", |m| {
            m.markers.panel.markers[0].marker.clear();
        }),
        ("empty channel", |m| {
            m.markers.panel.markers[0].channel.clear();
        }),
        ("empty normalization", |m| {
            m.document.normalization_markers.clear();
        }),
        ("duplicate normalization", |m| {
            m.document.normalization_markers.push("CD3".into());
        }),
        ("unknown normalization", |m| {
            m.document.normalization_markers[0] = "unknown".into();
        }),
        ("som channel", |m| {
            m.som.channels[0] = "unknown".into();
            m.som.scale[0].channel = "unknown".into();
        }),
        ("transform", |m| m.som.transform = None),
        ("nested normalization", |m| {
            m.som.normalization_model = Some("other".into());
        }),
        ("xdim", |m| m.document.som.xdim = 3),
        ("ydim", |m| m.document.som.ydim = 2),
        ("clusters", |m| m.document.som.metaclusters = 1),
        ("seed", |m| m.document.som.seed = 43),
        ("scale", |m| m.document.som.scale = true),
        ("training events", |m| m.document.training_events = 11),
        ("consensus", |m| {
            m.som.metaclustering = Metaclustering::Average;
        }),
        ("one batch", |m| {
            m.document.batches.remove("b");
        }),
        ("empty batch", |m| {
            m.document.batches.insert(" ".into(), None);
        }),
        ("minimum", |m| m.document.min_cluster_events = 1),
        ("no training events", |m| {
            m.document.training_events = 0;
            m.som.training_events = 0;
        }),
        ("short probabilities", |m| {
            m.document.probabilities = vec![0.0];
        }),
        ("nan", |m| m.document.probabilities[0] = f64::NAN),
        ("infinity", |m| m.document.probabilities[1] = f64::INFINITY),
        ("negative probability", |m| {
            m.document.probabilities[0] = -0.1;
        }),
        ("large probability", |m| m.document.probabilities[1] = 1.1),
        ("equal probabilities", |m| {
            m.document.probabilities = vec![0.0, 0.0];
        }),
        ("reversed probabilities", |m| {
            m.document.probabilities = vec![1.0, 0.0];
        }),
        ("unknown target", |m| {
            m.document.target = Target::Batch {
                batch: "missing".into(),
            }
        }),
        ("reagent labels", |m| {
            m.som.channel_labels = Some(vec![vec!["other".into()], vec!["CD4".into()]]);
        }),
    ];
    for (name, change) in cases {
        let mut m = model();
        change(&mut m);
        let error = m.validate().unwrap_err().to_string();
        let expected = match *name {
            "no training events" => "invalid SOM model dimensions, scale or labels",
            "reagent labels" => "SOM reagent labels differ from the panel contract",
            _ => "inconsistent model panel, SOM, batch or quantile contract",
        };
        assert!(error.contains(expected), "{name}: {error}");
    }
    for channel in ["Time", "Original_ID", "FSC-A", "SSC-A"] {
        let mut m = model();
        m.markers.panel.markers.push(Marker {
            marker: "forbidden".into(),
            channel: channel.into(),
            labels: vec![],
        });
        m.document.normalization_markers.push("forbidden".into());
        let error = m.validate().unwrap_err().to_string();
        assert!(
            error.contains("inconsistent model panel, SOM, batch or quantile contract"),
            "{channel}: {error}"
        );
    }
    let mut m = model();
    m.markers.panel.markers.push(Marker {
        marker: "clock".into(),
        channel: "Time".into(),
        labels: vec![],
    });
    assert!(m.validate().is_ok());
    let mut m = model();
    m.document.target = Target::Batch { batch: "a".into() };
    assert!(m.validate().is_ok());
    m.som.channel_labels = None;
    assert!(m.validate().is_ok());
    m.markers.panel.markers[0].labels = vec!["label".into()];
    m.som.channel_labels = Some(vec![vec!["label".into()], vec!["CD4".into()]]);
    assert!(m.validate().is_ok());
}

#[test]
fn model_requires_complete_valid_goals_and_mappings() {
    let cases: &[(&str, BreakModel)] = &[
        ("zero goal cluster", |m| m.document.goals[0].metacluster = 0),
        ("large goal cluster", |m| {
            m.document.goals[0].metacluster = 3;
        }),
        ("unknown goal marker", |m| {
            m.document.goals[0].marker = "unknown".into();
        }),
        ("duplicate goal", |m| {
            m.document.goals.push(m.document.goals[0].clone());
        }),
        ("missing goal", |m| {
            m.document.goals.pop();
        }),
        ("short quantiles", |m| {
            m.document.goals[0].quantiles = Some(vec![1.0]);
        }),
        ("nan quantiles", |m| {
            m.document.goals[0].quantiles = Some(vec![f64::NAN, 2.0]);
        }),
        ("reversed quantiles", |m| {
            m.document.goals[0].quantiles = Some(vec![2.0, 1.0]);
        }),
        ("missing mapping", |m| {
            m.mappings.pop();
        }),
        ("duplicate mapping", |m| {
            m.mappings.push(m.mappings[0].clone());
        }),
        ("unknown batch", |m| m.mappings[0].batch = "unknown".into()),
        ("zero mapping cluster", |m| m.mappings[0].metacluster = 0),
        ("large mapping cluster", |m| m.mappings[0].metacluster = 3),
        ("unknown mapping marker", |m| {
            m.mappings[0].marker = "unknown".into();
        }),
        ("too few events", |m| m.mappings[0].events = 1),
        ("missing spline", |m| m.mappings[0].spline = None),
        ("missing target", |m| m.document.goals[0].quantiles = None),
        ("spline and reason", |m| {
            m.mappings[0].identity_reason = Some("why".into());
        }),
        ("invalid spline", |m| {
            m.mappings[0].spline.as_mut().unwrap().x = vec![0.0, 0.0];
        }),
    ];
    for (name, change) in cases {
        let mut m = model();
        change(&mut m);
        let error = m.validate().unwrap_err().to_string();
        let expected = match *name {
            "missing goal" => "missing goal distribution",
            "zero goal cluster"
            | "large goal cluster"
            | "unknown goal marker"
            | "duplicate goal"
            | "short quantiles"
            | "nan quantiles"
            | "reversed quantiles" => "invalid or duplicate goal quantiles",
            "missing mapping" => "missing batch/cluster/marker mapping",
            "duplicate mapping"
            | "unknown batch"
            | "zero mapping cluster"
            | "large mapping cluster"
            | "unknown mapping marker" => "invalid or duplicate normalization mapping",
            "too few events" | "missing target" => "spline without sufficient events or target",
            "missing spline" | "spline and reason" => "invalid identity mapping policy",
            _ => "spline",
        };
        assert!(error.contains(expected), "{name}: {error}");
    }
    let mut m = model();
    m.document.goals[0].quantiles = Some(vec![1.0, 1.0]);
    assert!(m.validate().is_ok());
    let mut m = model();
    m.mappings[0].spline = None;
    m.mappings[0].identity_reason = Some("sparse".into());
    assert!(m.validate().is_err());
    m.document.sparse_policy = SparsePolicy::Identity;
    assert!(m.validate().is_ok());
    m.mappings[0].identity_reason = Some(String::new());
    assert!(m.validate().is_err());
}
