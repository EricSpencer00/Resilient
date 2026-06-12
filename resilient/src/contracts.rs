use crate::{Node, span::Span, uniqueness_walk::visit};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, PartialEq, Eq)]
enum Shape {
    String,
    Number,
    Struct,
    List(Option<Box<Shape>>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExpectedShape {
    String,
    Number,
    ListAny,
    ListString,
    ListNumber,
    ListStruct,
}

struct BuiltinContract {
    arity: usize,
    args: &'static [ExpectedShape],
    return_shape: fn(&[Option<Shape>]) -> Option<Shape>,
}

const STRING_FROM_BYTES: &[ExpectedShape] = &[ExpectedShape::ListNumber];
const ARRAY_COUNT_RUNS: &[ExpectedShape] = &[ExpectedShape::ListAny];
const ARRAY_DEDUP: &[ExpectedShape] = &[ExpectedShape::ListAny];
const ARRAY_FOLD_INT: &[ExpectedShape] = &[
    ExpectedShape::ListNumber,
    ExpectedShape::Number,
    ExpectedShape::String,
];
const ARRAY_SCAN_INT: &[ExpectedShape] = &[
    ExpectedShape::ListNumber,
    ExpectedShape::Number,
    ExpectedShape::String,
];
const ARRAY_ZIP_WITH_INT: &[ExpectedShape] = &[
    ExpectedShape::ListNumber,
    ExpectedShape::ListNumber,
    ExpectedShape::String,
];
const STRING_JOIN_LINES: &[ExpectedShape] = &[ExpectedShape::ListString];
const STRING_UNWORDS: &[ExpectedShape] = &[ExpectedShape::ListString];
const ARRAY_SORT_BY_FIELD: &[ExpectedShape] = &[ExpectedShape::ListStruct, ExpectedShape::String];
const ARRAY_DEDUP_BY: &[ExpectedShape] = &[ExpectedShape::ListStruct, ExpectedShape::String];

pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let const_shapes = collect_const_shapes(program);
    let mut result = Ok(());
    visit(program, &mut |node| {
        if result.is_err() {
            return;
        }
        if let Node::CallExpression {
            function,
            arguments,
            ..
        } = node
            && let Err(err) = check_call(node, function, arguments, source_path, &const_shapes)
        {
            result = Err(err);
        }
    });
    result
}

fn contract_for(callee: &str) -> Option<BuiltinContract> {
    let contract = match callee {
        "string_from_bytes" => BuiltinContract {
            arity: 1,
            args: STRING_FROM_BYTES,
            return_shape: |_| Some(Shape::String),
        },
        "array_count_runs" => BuiltinContract {
            arity: 1,
            args: ARRAY_COUNT_RUNS,
            return_shape: |_| Some(Shape::Number),
        },
        "array_dedup" => BuiltinContract {
            arity: 1,
            args: ARRAY_DEDUP,
            return_shape: |args| args.first().cloned().flatten(),
        },
        "array_fold_int" => BuiltinContract {
            arity: 3,
            args: ARRAY_FOLD_INT,
            return_shape: |_| Some(Shape::Number),
        },
        "array_scan_int" => BuiltinContract {
            arity: 3,
            args: ARRAY_SCAN_INT,
            return_shape: |_| Some(Shape::List(Some(Box::new(Shape::Number)))),
        },
        "array_zip_with_int" => BuiltinContract {
            arity: 3,
            args: ARRAY_ZIP_WITH_INT,
            return_shape: |_| Some(Shape::List(Some(Box::new(Shape::Number)))),
        },
        "string_join_lines" => BuiltinContract {
            arity: 1,
            args: STRING_JOIN_LINES,
            return_shape: |_| Some(Shape::String),
        },
        "string_unwords" => BuiltinContract {
            arity: 1,
            args: STRING_UNWORDS,
            return_shape: |_| Some(Shape::String),
        },
        "array_sort_by_field" => BuiltinContract {
            arity: 2,
            args: ARRAY_SORT_BY_FIELD,
            return_shape: |args| args.first().cloned().flatten(),
        },
        "array_sort_by_field_desc" => BuiltinContract {
            arity: 2,
            args: ARRAY_SORT_BY_FIELD,
            return_shape: |args| args.first().cloned().flatten(),
        },
        "array_dedup_by" => BuiltinContract {
            arity: 2,
            args: ARRAY_DEDUP_BY,
            return_shape: |args| args.first().cloned().flatten(),
        },
        _ => return None,
    };

    Some(contract)
}

fn diagnostic(source_path: &str, span: Span, message: &str) -> String {
    format!(
        "{}:{}:{}: error: {}",
        source_path, span.start.line, span.start.column, message
    )
}

fn expected_label(expected: ExpectedShape) -> &'static str {
    match expected {
        ExpectedShape::String => "string",
        ExpectedShape::Number => "number",
        ExpectedShape::ListAny => "list",
        ExpectedShape::ListString => "list<string>",
        ExpectedShape::ListNumber => "list<number>",
        ExpectedShape::ListStruct => "list<struct>",
    }
}

fn shape_label(shape: &Shape) -> String {
    match shape {
        Shape::String => "string".to_string(),
        Shape::Number => "number".to_string(),
        Shape::Struct => "struct".to_string(),
        Shape::List(Some(inner)) => format!("list<{}>", shape_label(inner)),
        Shape::List(None) => "list".to_string(),
    }
}

fn shape_matches(actual: &Shape, expected: ExpectedShape) -> bool {
    match (actual, expected) {
        (Shape::String, ExpectedShape::String) => true,
        (Shape::Number, ExpectedShape::Number) => true,
        (Shape::List(_), ExpectedShape::ListAny) => true,
        (Shape::List(Some(inner)), ExpectedShape::ListString) => matches!(**inner, Shape::String),
        (Shape::List(Some(inner)), ExpectedShape::ListNumber) => matches!(**inner, Shape::Number),
        (Shape::List(Some(inner)), ExpectedShape::ListStruct) => matches!(**inner, Shape::Struct),
        (Shape::List(None), ExpectedShape::ListString)
        | (Shape::List(None), ExpectedShape::ListNumber)
        | (Shape::List(None), ExpectedShape::ListStruct) => true,
        _ => false,
    }
}

fn infer_shape(node: &Node, const_shapes: &HashMap<String, Shape>) -> Option<Shape> {
    match node {
        Node::StringLiteral { .. } | Node::StringInternLiteral { .. } => Some(Shape::String),
        Node::IntegerLiteral { .. } | Node::FloatLiteral { .. } => Some(Shape::Number),
        Node::StructLiteral { .. } | Node::MapLiteral { .. } | Node::NewtypeConstruct { .. } => {
            Some(Shape::Struct)
        }
        Node::ArrayLiteral { items, .. } => {
            let mut item_shape: Option<Shape> = None;
            for item in items {
                let shape = infer_shape(item, const_shapes)?;
                item_shape = match item_shape {
                    None => Some(shape),
                    Some(existing) if existing == shape => Some(existing),
                    Some(_) => Some(Shape::List(None)),
                };
            }
            Some(Shape::List(item_shape.map(Box::new)))
        }
        Node::ExpressionStatement { expr, .. } | Node::TryExpression { expr, .. } => {
            infer_shape(expr, const_shapes)
        }
        Node::Block { stmts, .. } if stmts.len() == 1 => match &stmts[0] {
            Node::ExpressionStatement { expr, .. } => infer_shape(expr, const_shapes),
            other => infer_shape(other, const_shapes),
        },
        Node::IfStatement {
            consequence,
            alternative,
            ..
        } => {
            let cons = infer_shape(consequence, const_shapes)?;
            let alt = alternative
                .as_deref()
                .and_then(|alt| infer_shape(alt, const_shapes))?;
            if cons == alt { Some(cons) } else { None }
        }
        Node::PrefixExpression {
            operator, right, ..
        } if *operator == "-" => match infer_shape(right, const_shapes)? {
            Shape::Number => Some(Shape::Number),
            _ => None,
        },
        Node::InfixExpression {
            left,
            operator,
            right,
            ..
        } => {
            let left_shape = infer_shape(left, const_shapes)?;
            let right_shape = infer_shape(right, const_shapes)?;
            match (*operator, left_shape, right_shape) {
                ("+", Shape::String, Shape::String) => Some(Shape::String),
                ("+", Shape::Number, Shape::Number)
                | ("-", Shape::Number, Shape::Number)
                | ("*", Shape::Number, Shape::Number)
                | ("/", Shape::Number, Shape::Number)
                | ("%", Shape::Number, Shape::Number)
                | ("<<", Shape::Number, Shape::Number)
                | (">>", Shape::Number, Shape::Number)
                | ("&", Shape::Number, Shape::Number)
                | ("|", Shape::Number, Shape::Number)
                | ("^", Shape::Number, Shape::Number) => Some(Shape::Number),
                _ => None,
            }
        }
        Node::NamedArg { value, .. } => infer_shape(value, const_shapes),
        Node::Identifier { name, .. } => const_shapes.get(name).cloned(),
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            let Node::Identifier { name, .. } = function.as_ref() else {
                return None;
            };
            let contract = contract_for(name.as_str())?;
            let arg_shapes: Vec<Option<Shape>> = arguments
                .iter()
                .map(|arg| infer_shape(arg, const_shapes))
                .collect();
            (contract.return_shape)(&arg_shapes)
        }
        _ => None,
    }
}

fn collect_const_shapes(program: &Node) -> HashMap<String, Shape> {
    let mut const_shapes = HashMap::new();
    let Node::Program(statements) = program else {
        return const_shapes;
    };
    let mut evaluating = HashSet::new();
    for stmt in statements {
        let Node::Const { name, value, .. } = &stmt.node else {
            continue;
        };
        if !evaluating.insert(name.clone()) {
            continue;
        }
        if let Some(shape) = infer_shape(value, &const_shapes) {
            const_shapes.insert(name.clone(), shape);
        }
        evaluating.remove(name);
    }
    const_shapes
}

fn check_call(
    call: &Node,
    function: &Node,
    arguments: &[Node],
    source_path: &str,
    const_shapes: &HashMap<String, Shape>,
) -> Result<(), String> {
    let Node::Identifier { name, .. } = function else {
        return Ok(());
    };
    let Some(contract) = contract_for(name.as_str()) else {
        return Ok(());
    };

    let Node::CallExpression { span, .. } = call else {
        return Ok(());
    };

    if arguments.len() != contract.arity {
        return Err(diagnostic(
            source_path,
            *span,
            &format!(
                "{}: expected {} arguments, got {}",
                name,
                contract.arity,
                arguments.len()
            ),
        ));
    }

    for (idx, (arg, expected)) in arguments.iter().zip(contract.args.iter()).enumerate() {
        let Some(actual) = infer_shape(arg, const_shapes) else {
            continue;
        };
        if !shape_matches(&actual, *expected) {
            return Err(diagnostic(
                source_path,
                *span,
                &format!(
                    "{}: argument {} must be {}, got {}",
                    name,
                    idx + 1,
                    expected_label(*expected),
                    shape_label(&actual)
                ),
            ));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::typechecker::TypeChecker;

    fn typecheck_ok(src: &str) {
        let (prog, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program(&prog)
            .unwrap_or_else(|e| panic!("unexpected type error: {e}"));
    }

    fn typecheck_err(src: &str) -> String {
        let (prog, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program(&prog)
            .expect_err("expected typecheck failure")
    }

    #[test]
    fn accepts_valid_const_eval_ext_call_shapes() {
        typecheck_ok(
            r#"
struct Person { string name, int age }
const BYTES = [72, 101, 108, 108, 111];
const TEXT = string_from_bytes(BYTES);
const LINES = string_join_lines(["hello", "world"]);
const PEOPLE = [new Person { name: "Ada", age: 32 }, new Person { name: "Grace", age: 48 }];
const SORTED = array_sort_by_field(PEOPLE, "age");
"#,
        );
    }

    #[test]
    fn rejects_string_from_bytes_with_string_argument() {
        let err = typecheck_err(
            r#"
const BYTES = "hello";
const TEXT = string_from_bytes(BYTES);
"#,
        );
        assert!(
            err.contains("string_from_bytes") && err.contains("list<number>"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_array_fold_int_with_non_numeric_list_items() {
        let err = typecheck_err(
            r#"
const TOTAL = array_fold_int(["a", "b"], 0, "sum");
"#,
        );
        assert!(
            err.contains("array_fold_int") && err.contains("list<number>"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_array_sort_by_field_with_non_struct_list_items() {
        let err = typecheck_err(
            r#"
const SORTED = array_sort_by_field([1, 2], "age");
"#,
        );
        assert!(
            err.contains("array_sort_by_field") && err.contains("list<struct>"),
            "unexpected error: {err}"
        );
    }
}
