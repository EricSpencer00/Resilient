//! Loop obligations use the same signed width and overflow policy as execution.
use super::*;
use crate::vm::OverflowMode;

struct Term<'c> {
    value: Value<'c>,
    defined: Bool<'c>,
}
enum Value<'c> {
    Integer(BV<'c>),
    Boolean(Bool<'c>),
}
impl<'c> Term<'c> {
    fn integer(&self) -> Option<&BV<'c>> {
        if let Value::Integer(value) = &self.value {
            Some(value)
        } else {
            None
        }
    }
    fn boolean(&self) -> Option<&Bool<'c>> {
        if let Value::Boolean(value) = &self.value {
            Some(value)
        } else {
            None
        }
    }
}
struct Model<'c, 'b> {
    ctx: &'c z3::Context,
    bindings: &'b HashMap<String, i64>,
    mode: OverflowMode,
    integers: &'b std::collections::HashSet<String>,
}
impl<'c> Model<'c, '_> {
    fn all(&self, values: &[&Bool<'c>]) -> Bool<'c> {
        Bool::and(self.ctx, values)
    }
    fn narrow(&self, wide: BV<'c>, defined: Bool<'c>) -> Term<'c> {
        let min = BV::from_i64(self.ctx, i64::MIN, 128);
        let max = BV::from_i64(self.ctx, i64::MAX, 128);
        let below = wide.bvslt(&min);
        let above = wide.bvsgt(&max);
        let value = wide.extract(63, 0);
        let (value, defined) = match self.mode {
            OverflowMode::Wrap => (value, defined),
            OverflowMode::Saturate => (
                below.ite(
                    &BV::from_i64(self.ctx, i64::MIN, 64),
                    &above.ite(&BV::from_i64(self.ctx, i64::MAX, 64), &value),
                ),
                defined,
            ),
            OverflowMode::Trap => (value, self.all(&[&defined, &below.not(), &above.not()])),
        };
        Term {
            value: Value::Integer(value),
            defined,
        }
    }
    fn expression(&self, node: &Node, depth: usize, remaining: &mut usize) -> Option<Term<'c>> {
        if depth > 128 || *remaining == 0 {
            return None;
        }
        *remaining -= 1;
        let yes = Bool::from_bool(self.ctx, true);
        match node {
            Node::IntegerLiteral { value, .. } => Some(Term {
                value: Value::Integer(BV::from_i64(self.ctx, *value, 64)),
                defined: yes,
            }),
            Node::BooleanLiteral { value, .. } => Some(Term {
                value: Value::Boolean(Bool::from_bool(self.ctx, *value)),
                defined: yes,
            }),
            Node::Identifier { name, .. } => Some(Term {
                value: Value::Integer(match self.bindings.get(name) {
                    Some(value) => BV::from_i64(self.ctx, *value, 64),
                    None => BV::new_const(self.ctx, name.as_str(), 64),
                }),
                defined: Bool::from_bool(
                    self.ctx,
                    self.bindings.contains_key(name) || self.integers.contains(name),
                ),
            }),
            Node::PrefixExpression {
                operator, right, ..
            } => {
                let right = self.expression(right, depth + 1, remaining)?;
                match *operator {
                    "-" => Some(self.narrow(right.integer()?.sign_ext(64).bvneg(), right.defined)),
                    "!" => Some(Term {
                        value: Value::Boolean(right.boolean()?.not()),
                        defined: right.defined,
                    }),
                    _ => None,
                }
            }
            Node::InfixExpression {
                left,
                operator,
                right,
                ..
            } => {
                let left = self.expression(left, depth + 1, remaining)?;
                let right = self.expression(right, depth + 1, remaining)?;
                if matches!(*operator, "&&" | "||") {
                    let l = left.boolean()?;
                    let r = right.boolean()?;
                    let (value, rhs_needed) = if *operator == "&&" {
                        (self.all(&[l, r]), l.clone())
                    } else {
                        (Bool::or(self.ctx, &[l, r]), l.not())
                    };
                    return Some(Term {
                        value: Value::Boolean(value),
                        defined: self.all(&[&left.defined, &rhs_needed.implies(&right.defined)]),
                    });
                }
                let l = left.integer()?;
                let r = right.integer()?;
                let defined = self.all(&[&left.defined, &right.defined]);
                if matches!(*operator, "+" | "-" | "*") {
                    let l = l.sign_ext(64);
                    let r = r.sign_ext(64);
                    let wide = match *operator {
                        "+" => l.bvadd(&r),
                        "-" => l.bvsub(&r),
                        _ => l.bvmul(&r),
                    };
                    return Some(self.narrow(wide, defined));
                }
                let value = match *operator {
                    "==" => Value::Boolean(l._eq(r)),
                    "!=" => Value::Boolean(l._eq(r).not()),
                    "<" => Value::Boolean(l.bvslt(r)),
                    "<=" => Value::Boolean(l.bvsle(r)),
                    ">" => Value::Boolean(l.bvsgt(r)),
                    ">=" => Value::Boolean(l.bvsge(r)),
                    "&" => Value::Integer(l.bvand(r)),
                    "|" => Value::Integer(l.bvor(r)),
                    "^" => Value::Integer(l.bvxor(r)),
                    "/" | "%" => {
                        let zero = BV::from_i64(self.ctx, 0, 64);
                        let overflow = self.all(&[
                            &l._eq(&BV::from_i64(self.ctx, i64::MIN, 64)),
                            &r._eq(&BV::from_i64(self.ctx, -1, 64)),
                        ]);
                        let mut defined = self.all(&[&defined, &r._eq(&zero).not()]);
                        if self.mode == OverflowMode::Trap {
                            defined = self.all(&[&defined, &overflow.not()]);
                        }
                        let mut value = if *operator == "/" {
                            l.bvsdiv(r)
                        } else {
                            l.bvsrem(r)
                        };
                        if *operator == "/" && self.mode == OverflowMode::Saturate {
                            value = overflow.ite(&BV::from_i64(self.ctx, i64::MAX, 64), &value);
                        }
                        return Some(Term {
                            value: Value::Integer(value),
                            defined,
                        });
                    }
                    _ => return None,
                };
                Some(Term { value, defined })
            }
            _ => None,
        }
    }
    fn term(&self, node: &Node) -> Option<Term<'c>> {
        self.expression(node, 0, &mut 32768)
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn prove(
    ctx: &z3::Context,
    goal: &Node,
    bindings: &HashMap<String, i64>,
    integers: &std::collections::HashSet<String>,
    entry: Option<(&Node, &Node)>,
    evaluated: &[Node],
    timeout_ms: u32,
) -> Option<String> {
    if evaluated.len() > 32768 {
        return None;
    }
    let mode = OverflowMode::from_env();
    let model = Model {
        ctx,
        bindings,
        mode,
        integers,
    };
    let goal = model.term(goal)?;
    let mut conditions = vec![goal.defined.clone(), goal.boolean()?.clone()];
    for expression in evaluated {
        conditions.push(model.term(expression)?.defined);
    }
    let post = model.all(&conditions.iter().collect::<Vec<_>>());
    let claim = if let Some((invariant, condition)) = entry {
        let invariant = model.term(invariant)?;
        let condition = model.term(condition)?;
        // Reachable entries already satisfy a defined invariant. A false
        // condition skips every body operation, including arithmetic faults.
        model
            .all(&[&invariant.defined, invariant.boolean()?])
            .implies(&model.all(&[&condition.defined, &condition.boolean()?.implies(&post)]))
    } else {
        post
    };
    let solver = z3::Solver::new(ctx);
    let mut params = z3::Params::new(ctx);
    params.set_u32("timeout", timeout_ms.clamp(1, 5000));
    solver.set_params(&params);
    solver.assert(&claim.not());
    if solver.check() != z3::SatResult::Unsat {
        return None;
    }
    let policy = match mode {
        OverflowMode::Wrap => "wrap",
        OverflowMode::Saturate => "saturate",
        OverflowMode::Trap => "trap",
    };
    Some(format!(
        "; loop-arithmetic: signed-i64-v1\n; overflow-policy: {policy}\n; obligation: defined execution and invariant preservation\n(set-logic QF_BV)\n{}\n(check-sat)\n",
        solver
    ))
}
