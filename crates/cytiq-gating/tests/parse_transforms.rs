use cytiq_gating::{GatingError, Transform, parse_gatingml};

const GATING_NS: &str = "http://www.isac-net.org/std/Gating-ML/v2.0/gating";
const TRANSFORMS_NS: &str = "http://www.isac-net.org/std/Gating-ML/v2.0/transformations";
const DATA_TYPE_NS: &str = "http://www.isac-net.org/std/Gating-ML/v2.0/datatypes";

fn xml(body: &str) -> String {
    format!(
        r#"<g:Gating-ML xmlns:g="{GATING_NS}" xmlns:t="{TRANSFORMS_NS}" xmlns:d="{DATA_TYPE_NS}">{body}</g:Gating-ML>"#
    )
}
fn transformation(body: &str) -> String {
    xml(&format!(
        r#"<t:transformation t:id="x">{body}</t:transformation>"#
    ))
}

#[test]
fn transformations_keep_parameters_and_validate_each_boundary() {
    for (body, expected) in [
        (
            r#"<t:flin t:T="4" t:A="2"/>"#,
            Transform::Linear { t: 4.0, a: 2.0 },
        ),
        (
            r#"<t:flin t:T="-4" t:A="2"/>"#,
            Transform::Linear { t: -4.0, a: 2.0 },
        ),
        (
            r#"<t:flog t:T="100" t:M="3"/>"#,
            Transform::Log { t: 100.0, m: 3.0 },
        ),
        (
            r#"<t:fasinh t:T="100" t:M="3" t:A="2"/>"#,
            Transform::Asinh {
                t: 100.0,
                m: 3.0,
                a: 2.0,
            },
        ),
    ] {
        assert_eq!(
            parse_gatingml(&transformation(body))
                .unwrap()
                .transform("x"),
            Some(&expected)
        );
    }
    for body in [
        r#"<t:flin t:T="4" t:A="-4"/>"#,
        r#"<t:flog t:T="0" t:M="3"/>"#,
        r#"<t:flog t:T="-1" t:M="3"/>"#,
        r#"<t:flog t:T="10" t:M="0"/>"#,
        r#"<t:flog t:T="10" t:M="-3"/>"#,
        r#"<t:fasinh t:T="0" t:M="3" t:A="2"/>"#,
        r#"<t:fasinh t:T="-1" t:M="3" t:A="2"/>"#,
        r#"<t:fasinh t:T="10" t:M="0" t:A="2"/>"#,
        r#"<t:fasinh t:T="10" t:M="-1" t:A="2"/>"#,
        r#"<t:fasinh t:T="10" t:M="3" t:A="-3"/>"#,
        r#"<t:logicle t:T="-1" t:W="0.5" t:M="4.5" t:A="0"/>"#,
        r#"<t:hyperlog t:T="-1" t:W="0.5" t:M="4.5" t:A="0"/>"#,
        "",
        r#"<t:flin t:T="2" t:A="0"/><t:flin t:T="2" t:A="0"/>"#,
        r#"<g:flin t:T="2" t:A="0"/>"#,
    ] {
        assert!(
            matches!(parse_gatingml(&transformation(body)), Err(GatingError::InvalidTransform { id, .. }) if id == "x"),
            "{body}"
        );
    }
    for body in [
        r#"<t:flog t:T="10"/>"#,
        r#"<t:flin t:T="NaN" t:A="0"/>"#,
        r#"<t:fasinh t:T="1" t:M="2"/>"#,
    ] {
        assert!(matches!(
            parse_gatingml(&transformation(body)),
            Err(GatingError::InvalidGatingMl(_))
        ));
    }
    assert!(
        matches!(parse_gatingml(&transformation("<t:fratio/>")), Err(GatingError::UnsupportedElement { element }) if element == "transforms:fratio")
    );
    let hyper = parse_gatingml(&transformation(
        r#"<t:hyperlog t:T="262144" t:W="0.5" t:M="4.5" t:A="0"/>"#,
    ))
    .unwrap();
    let Some(Transform::Hyperlog(scale)) = hyper.transform("x") else {
        panic!("hyperlog required");
    };
    assert!((scale.scale(262_144.0) - 1.0).abs() < 1e-6);
}

#[test]
fn document_rejects_wrong_root_and_duplicate_or_reserved_ids() {
    assert!(matches!(
        parse_gatingml("<wrong/>"),
        Err(GatingError::InvalidGatingMl(_))
    ));
    let transform = r#"<t:transformation t:id="x"><t:flin t:T="1" t:A="0"/></t:transformation>"#;
    assert!(
        matches!(parse_gatingml(&xml(&transform.repeat(2))), Err(GatingError::InvalidGatingMl(reason)) if reason == "duplicate transformation id `x`")
    );
    let matrix = |id| {
        format!(
            r#"<t:spectrumMatrix t:id="{id}"><t:fluorochromes><d:fcs-dimension d:name="A"/></t:fluorochromes><t:detectors><d:fcs-dimension d:name="A"/></t:detectors><t:spectrum><t:coefficient t:value="1"/></t:spectrum></t:spectrumMatrix>"#
        )
    };
    for id in ["uncompensated", "fcs", "FcS"] {
        assert!(
            matches!(parse_gatingml(&xml(&matrix(id))), Err(GatingError::InvalidGatingMl(reason)) if reason.contains("reserved"))
        );
    }
    assert!(
        matches!(parse_gatingml(&xml(&matrix("x").repeat(2))), Err(GatingError::InvalidGatingMl(reason)) if reason == "duplicate spectrumMatrix id `x`")
    );
    assert_eq!(parse_gatingml(&xml("<unknown/>")).unwrap().gates().len(), 0);
}

#[test]
fn non_gate_elements_in_the_gating_namespace_are_ignored() {
    assert_eq!(parse_gatingml(&xml("<g:metadata/>")).unwrap().gates(), []);
}
