//! `ValueTransform` rebuilt from a written `CYTIQ_TRANSFORM` keyword must
//! reproduce the values `cytiq transform` stored, for every method.

use std::path::{Path, PathBuf};

use cytiq_fcs::{FcsDataset, FcsReader, TransformContract, linear_values, transform_state};
use cytiq_transform::{TransformConfig, TransformMethod, TransformRunner, ValueTransform};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/fixtures/gating/sample.fcs")
}

fn read(path: &Path) -> FcsDataset {
    FcsReader::open(path)
        .and_then(FcsReader::read)
        .expect("readable FCS")
}

#[test]
fn value_transform_reproduces_stored_values_for_every_method() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let input = read(&fixture());
    for method in [
        TransformMethod::Arcsinh,
        TransformMethod::Logicle,
        TransformMethod::Hyperlog,
        TransformMethod::Biexponential,
    ] {
        let output = directory.path().join(format!("{}.fcs", method.as_str()));
        let mut config = TransformConfig::new(vec![fixture()], output.clone());
        config.method = method;
        config.allow_uncompensated = true;
        TransformRunner::run(&config).expect("transform succeeds");

        let transformed = read(&output);
        let state = transform_state(&transformed.text)
            .expect("valid keyword")
            .expect("transformed file carries CYTIQ_TRANSFORM");
        assert_ne!(state.channels.len(), 0, "transformed channels are listed");
        let contract =
            TransformContract::of(&transformed.text, &state.channels).expect("valid contract");
        let value_transform = ValueTransform::from_contract(&contract).expect("supported method");
        assert_eq!(value_transform.method(), method);

        for channel in &state.channels {
            let source = linear_values(&input, input.find_parameter(channel).expect("channel"));
            let stored = linear_values(
                &transformed,
                transformed.find_parameter(channel).expect("channel"),
            );
            for (raw, stored) in source.iter().zip(&stored) {
                let expected = value_transform.apply(*raw);
                let tolerance = 1e-5 * expected.abs().max(1.0);
                assert!(
                    (expected - stored).abs() <= tolerance,
                    "{} {channel}: apply({raw}) = {expected}, stored {stored}",
                    method.as_str()
                );
            }
        }
    }
}

#[test]
fn value_transform_rejects_unknown_methods_and_bad_cofactors() {
    let unknown = TransformContract {
        method: "fancy".to_owned(),
        parameters: serde_json::json!({}),
    };
    assert!(ValueTransform::from_contract(&unknown).is_err());
    let zero = TransformContract {
        method: "arcsinh".to_owned(),
        parameters: serde_json::json!({"cofactor": 0.0}),
    };
    assert!(ValueTransform::from_contract(&zero).is_err());
}
