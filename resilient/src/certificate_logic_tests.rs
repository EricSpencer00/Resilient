//! New constructor-coverage requirements frozen before carried-summary changes.
use super::*;
fn int(value: i64) -> Node {
    Node::IntegerLiteral {
        value,
        span: Default::default(),
    }
}
fn var(name: &str) -> Node {
    Node::Identifier {
        name: name.into(),
        span: Default::default(),
    }
}
fn op(left: Node, operator: &'static str, right: Node) -> Node {
    Node::InfixExpression {
        left: Box::new(left),
        operator,
        right: Box::new(right),
        span: Default::default(),
    }
}
fn forall(lo: Node, hi: Node, body: Node) -> Node {
    Node::Quantifier {
        kind: crate::quantifiers::QuantifierKind::Forall,
        var: "i".into(),
        range: crate::quantifiers::QuantRange::Range {
            lo: Box::new(lo),
            hi: Box::new(hi),
        },
        body: Box::new(body),
        span: Default::default(),
    }
}
fn fragment(node: &Node, bindings: &HashMap<String, i64>, expected: CertificateFragment) {
    Z3_CTX.with(|ctx| {
        assert_eq!(
            translate_bool_encoded(ctx, node, bindings)
                .unwrap()
                .fragment,
            expected
        )
    });
}
#[test]
fn actual_int_constructors_preserve_linear_coefficients_and_bound_constants() {
    fragment(
        &op(op(int(2), "*", var("x")), "==", op(var("x"), "+", var("x"))),
        &HashMap::new(),
        CertificateFragment::Linear,
    );
    fragment(
        &op(
            op(var("factor"), "*", var("x")),
            "==",
            op(int(2), "*", var("x")),
        ),
        &HashMap::from([("factor".into(), 2)]),
        CertificateFragment::Linear,
    );
    fragment(
        &op(op(var("x"), "*", var("x")), ">=", int(0)),
        &HashMap::new(),
        CertificateFragment::Nonlinear,
    );
    for operator in ["/", "%"] {
        fragment(
            &op(op(var("x"), operator, int(2)), "==", int(0)),
            &HashMap::new(),
            CertificateFragment::Nonlinear,
        );
    }
}
#[test]
fn nonlinear_index_match_and_quantifier_children_are_not_hidden() {
    let index = Node::IndexExpression {
        target: Box::new(var("a")),
        index: Box::new(op(var("x"), "*", var("x"))),
        span: Default::default(),
    };
    fragment(
        &op(index.clone(), "==", index),
        &HashMap::new(),
        CertificateFragment::Nonlinear,
    );
    let value = Node::Match {
        scrutinee: Box::new(var("x")),
        arms: vec![
            (Pattern::Literal(int(0)), None, op(var("x"), "*", var("x"))),
            (Pattern::Wildcard, None, int(0)),
        ],
        span: Default::default(),
    };
    fragment(
        &op(value.clone(), "==", value),
        &HashMap::new(),
        CertificateFragment::Nonlinear,
    );
    fragment(
        &forall(int(0), int(5), op(var("i"), ">=", int(0))),
        &HashMap::new(),
        CertificateFragment::Linear,
    );
    fragment(
        &forall(
            int(0),
            int(5),
            op(op(var("i"), "*", var("i")), ">=", int(0)),
        ),
        &HashMap::from([("i".into(), 3)]),
        CertificateFragment::Nonlinear,
    );
    fragment(
        &forall(
            op(var("x"), "*", var("x")),
            int(5),
            op(var("i"), "==", var("i")),
        ),
        &HashMap::new(),
        CertificateFragment::Nonlinear,
    );
}
#[test]
fn both_serializers_include_nonlinear_axioms_in_logic_join() {
    let goal = op(var("x"), "==", var("x"));
    let axiom = op(op(var("x"), "*", var("x")), ">=", int(0));
    let bindings = HashMap::new();
    let (verdict, cert, _, _) =
        prove_with_axioms_and_timeout(&goal, &bindings, std::slice::from_ref(&axiom), 2000);
    assert_eq!(verdict, Some(true));
    assert!(cert.unwrap().smt2.contains("(set-logic AUFNIRA)"));
    let (verdict, cert, _) = prove_tautology_with_certificate(&goal, &bindings, &[axiom], 2000);
    assert!(verdict);
    assert!(cert.unwrap().smt2.contains("(set-logic AUFNIRA)"));
}

#[test]
fn formal_coefficient_shapes_and_existential_children_are_covered() {
    let negative = Node::PrefixExpression {
        operator: "-",
        right: Box::new(int(3)),
        span: Default::default(),
    };
    fragment(
        &op(
            op(negative, "*", var("x")),
            "==",
            op(var("x"), "*", int(-3)),
        ),
        &HashMap::new(),
        CertificateFragment::Linear,
    );
    // The standard's coefficient extension requires a non-Ints top symbol on
    // the other term. Broader but mathematically linear syntax uses AUFNIRA.
    let compound = op(var("x"), "+", int(1));
    fragment(
        &op(op(int(2), "*", compound.clone()), ">=", int(0)),
        &HashMap::new(),
        CertificateFragment::Nonlinear,
    );
    fragment(
        &op(op(compound.clone(), "*", compound), ">=", int(0)),
        &HashMap::new(),
        CertificateFragment::Nonlinear,
    );
    let existential = Node::Quantifier {
        kind: crate::quantifiers::QuantifierKind::Exists,
        var: "i".into(),
        range: crate::quantifiers::QuantRange::Range {
            lo: Box::new(int(0)),
            hi: Box::new(int(2)),
        },
        body: Box::new(op(op(var("i"), "*", var("i")), ">=", int(0))),
        span: Default::default(),
    };
    fragment(
        &existential,
        &HashMap::new(),
        CertificateFragment::Nonlinear,
    );
}

#[test]
fn both_serializers_preserve_linear_headers_with_binding_equalities() {
    let goal = op(
        op(var("factor"), "*", var("x")),
        "==",
        op(var("x"), "+", var("x")),
    );
    let bindings = HashMap::from([("factor".into(), 2)]);
    let (verdict, cert, _, _) = prove_with_axioms_and_timeout(&goal, &bindings, &[], 2000);
    assert_eq!(verdict, Some(true));
    let first = cert.unwrap().smt2;
    assert!(first.contains("(set-logic AUFLIA)"));
    assert!(first.contains("(assert (= factor 2))"));
    let (verdict, cert, _) = prove_tautology_with_certificate(&goal, &bindings, &[], 2000);
    assert!(verdict);
    let second = cert.unwrap().smt2;
    assert!(second.contains("(set-logic AUFLIA)"));
    assert!(second.contains("(assert (= factor 2))"));
    assert_eq!(first, second, "both paths share one serialized inventory");
}
