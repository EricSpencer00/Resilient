//! Declaration-owned verification attributes, separate from executable guards and source proofs.
use crate::{Node, loop_bound::LoopBoundSpec, span::Span};
use std::{cell::RefCell, collections::HashMap};

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub(crate) struct Declaration {
    pub name: String,
    span: Span,
    kind: ScopeKind,
}
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
enum ScopeKind {
    Function,
    Module,
}
pub(crate) type DeclarationPath = Vec<Declaration>;
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct Attributes {
    pub loop_bound: Option<LoopBoundSpec>,
    pub overflow_checked: bool,
    pub refinement: Option<Refinement>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Refinement {
    pub spec: String,
    pub action: String,
    pub line: usize,
}
pub(crate) type Bounds = HashMap<DeclarationPath, Attributes>;
type Captured = HashMap<(usize, Declaration), Attributes>;

thread_local! {
    static CURRENT: RefCell<Option<Bounds>> = const { RefCell::new(None) };
    static CAPTURE: RefCell<Option<Captured>> = const { RefCell::new(None) };
}

pub(crate) fn declaration(node: &Node) -> Option<Declaration> {
    match node {
        Node::Function { name, span, .. } => Some(Declaration {
            name: name.clone(),
            span: *span,
            kind: ScopeKind::Function,
        }),
        _ => None,
    }
}

pub(crate) fn scope(node: &Node) -> Option<Declaration> {
    match node {
        Node::ModuleDecl { name, span, .. } => Some(Declaration {
            name: name.clone(),
            span: *span,
            kind: ScopeKind::Module,
        }),
        _ => declaration(node),
    }
}

pub(crate) struct Invocation(Option<Bounds>);
impl Invocation {
    pub(crate) fn enter() -> Self {
        Self(CURRENT.with(|slot| slot.replace(Some(Bounds::new()))))
    }
}
impl Drop for Invocation {
    fn drop(&mut self) {
        CURRENT.with(|slot| {
            slot.replace(self.0.take());
        });
    }
}

pub(crate) fn snapshot() -> Option<Bounds> {
    CURRENT.with(|slot| slot.borrow().clone())
}
pub(crate) fn install(bounds: Bounds) {
    CURRENT.with(|slot| {
        if let Some(current) = slot.borrow_mut().as_mut() {
            *current = bounds;
        }
    });
}

pub(crate) struct ParseCapture(Option<Captured>);
impl ParseCapture {
    pub(crate) fn enter() -> Self {
        Self(CAPTURE.with(|slot| slot.replace(Some(Captured::new()))))
    }
    pub(crate) fn finish(self, program: &Node) {
        let mut bounds = Bounds::new();
        CAPTURE.with(|slot| {
            let captured = slot.borrow();
            walk(program, &mut Vec::new(), &mut |node, path| {
                if let Some(key) = capture_key(node) {
                    let bound = captured
                        .as_ref()
                        .and_then(|items| items.get(&key))
                        .cloned()
                        .unwrap_or_default();
                    bounds.insert(path.clone(), bound);
                }
            });
        });
        install(bounds);
    }
}
impl Drop for ParseCapture {
    fn drop(&mut self) {
        CAPTURE.with(|slot| {
            slot.replace(self.0.take());
        });
    }
}

fn capture_key(node: &Node) -> Option<(usize, Declaration)> {
    let Node::Function { body, .. } = node else {
        return None;
    };
    // Addresses are compared only while this parser owns the still-live AST.
    // The persisted key uses lexical paths and spans, so clones need no pointers.
    Some((body.as_ref() as *const Node as usize, declaration(node)?))
}

pub(crate) fn record(node: &Node, bound: u32) -> bool {
    let Some(key) = capture_key(node) else {
        return false;
    };
    CAPTURE.with(|slot| {
        if let Some(captured) = slot.borrow_mut().as_mut() {
            let attributes = captured.entry(key).or_default();
            if let Some(previous) = attributes.loop_bound {
                return previous.bound == bound;
            }
            attributes.loop_bound = Some(LoopBoundSpec { bound });
        }
        true
    })
}

pub(crate) fn record_overflow(node: &Node) {
    let Some(key) = capture_key(node) else { return };
    CAPTURE.with(|slot| {
        if let Some(captured) = slot.borrow_mut().as_mut() {
            captured.entry(key).or_default().overflow_checked = true;
        }
    });
}

pub(crate) fn record_refinement(node: &Node, spec: &str, action: &str, line: usize) {
    let Some(key) = capture_key(node) else { return };
    CAPTURE.with(|slot| {
        if let Some(captured) = slot.borrow_mut().as_mut() {
            captured.entry(key).or_default().refinement = Some(Refinement {
                spec: spec.to_string(),
                action: action.to_string(),
                line,
            });
        }
    });
}

pub(crate) fn scoped_refinements() -> Option<Vec<(bool, String, Refinement)>> {
    CURRENT.with(|slot| {
        slot.borrow().as_ref().map(|items| {
            let mut records: Vec<_> = items
                .iter()
                .filter_map(|(path, attributes)| {
                    attributes.refinement.clone().map(|mapping| {
                        (
                            path.len() == 1,
                            path.iter()
                                .map(|part| part.name.as_str())
                                .collect::<Vec<_>>()
                                .join("::"),
                            mapping,
                        )
                    })
                })
                .collect();
            records.sort_by(|a, b| (a.2.line, &a.1, &a.2.spec).cmp(&(b.2.line, &b.1, &b.2.spec)));
            records
        })
    })
}

pub(crate) fn resolve_refinement_paths(base_dir: &std::path::Path) -> Result<(), String> {
    CURRENT.with(|slot| {
        let mut current = slot.borrow_mut();
        let Some(items) = current.as_mut() else {
            return Ok(());
        };
        for attributes in items.values_mut() {
            if let Some(mapping) = attributes.refinement.as_mut()
                && !std::path::Path::new(&mapping.spec).is_absolute()
            {
                let base = if base_dir.is_absolute() {
                    base_dir.to_path_buf()
                } else {
                    std::env::current_dir()
                        .map_err(|error| {
                            format!("cannot resolve @refines source directory: {error}")
                        })?
                        .join(base_dir)
                };
                mapping.spec = base
                    .join(&mapping.spec)
                    .to_str()
                    .ok_or("@refines source directory is not valid UTF-8")?
                    .to_string();
            }
        }
        Ok(())
    })
}

#[cfg(feature = "z3")]
pub(crate) fn overflow_checked(path: &DeclarationPath) -> Option<bool> {
    CURRENT.with(|slot| {
        slot.borrow().as_ref().map(|items| {
            items
                .get(path)
                .is_some_and(|attributes| attributes.overflow_checked)
        })
    })
}

pub(crate) fn walk(
    node: &Node,
    path: &mut DeclarationPath,
    visit: &mut impl FnMut(&Node, &DeclarationPath),
) {
    let own = scope(node);
    if let Some(own) = &own {
        path.push(own.clone());
    }
    visit(node, path);
    crate::uniqueness_walk::children(node, &mut |child| walk(child, path, visit));
    if own.is_some() {
        path.pop();
    }
}

pub(crate) fn top_level() -> Option<HashMap<String, LoopBoundSpec>> {
    snapshot().map(|bounds| {
        bounds
            .into_iter()
            .filter_map(|(path, bound)| {
                if path.len() == 1 {
                    bound.loop_bound.map(|bound| (path[0].name.clone(), bound))
                } else {
                    None
                }
            })
            .collect()
    })
}

pub(crate) fn for_function(
    node: &Node,
    legacy: &HashMap<String, LoopBoundSpec>,
) -> Option<LoopBoundSpec> {
    let key = declaration(node)?;
    CURRENT.with(|slot| match slot.borrow().as_ref() {
        Some(bounds) => bounds
            .get(&vec![key])
            .and_then(|attributes| attributes.loop_bound),
        None => legacy.get(&key.name).copied(),
    })
}

/// Inline globs clone declarations out of their module scope before checking.
pub(crate) fn project_inline_glob(module: &str, items: &[&Node]) -> Result<(), String> {
    let Some(source) = snapshot() else {
        return Ok(());
    };
    let modules: Vec<_> = module.split("::").collect();
    let mut exported = std::collections::HashSet::new();
    for item in items {
        walk(item, &mut Vec::new(), &mut |node, path| {
            if declaration(node).is_some() {
                exported.insert(path.clone());
            }
        });
    }
    let mut additions = Vec::new();
    for (path, attributes) in &source {
        if path.len() <= modules.len()
            || !path
                .iter()
                .zip(&modules)
                .all(|(scope, name)| scope.kind == ScopeKind::Module && scope.name == *name)
        {
            continue;
        }
        let destination = path[modules.len()..].to_vec();
        if exported.contains(&destination) {
            additions.push((destination, attributes.clone()));
        }
    }
    CURRENT.with(|slot| {
        let mut current = slot.borrow_mut();
        let Some(target) = current.as_mut() else {
            return Ok(());
        };
        for (path, attributes) in additions {
            if target
                .get(&path)
                .is_some_and(|previous| *previous != attributes)
            {
                return Err(format!(
                    "ambiguous verification attributes in inline glob `{module}`"
                ));
            }
            target.insert(path, attributes);
        }
        Ok(())
    })
}

/// Transfer only declarations retained by the import's visibility/selector rules.
pub(crate) fn transfer(
    node: &Node,
    source: &Bounds,
    target: &mut Bounds,
    namespace: Option<&str>,
) -> Result<(), String> {
    let renamed = declaration(node).zip(namespace);
    let mut error = None;
    walk(node, &mut Vec::new(), &mut |candidate, path| {
        if declaration(candidate).is_none() {
            return;
        }
        let Some(bound) = source.get(path) else {
            return;
        };
        let mut destination = path.clone();
        if let Some((root, namespace)) = &renamed
            && destination.first() == Some(root)
        {
            destination[0].name = format!("{namespace}::{}", root.name);
        }
        if let Some(previous) = target.get(&destination)
            && previous != bound
        {
            let kind = if previous.loop_bound != bound.loop_bound {
                "loop-bound"
            } else if previous.overflow_checked != bound.overflow_checked {
                "overflow-check"
            } else {
                "refinement"
            };
            error = Some(format!(
                "ambiguous {kind} ownership for declaration `{}`",
                destination[0].name
            ));
            return;
        }
        target.insert(destination, bound.clone());
    });
    match error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

#[cfg(all(test, feature = "z3"))]
mod tests {
    #[test]
    fn static_bound_check_uses_the_actual_function_body() {
        let _lock = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        let _invocation = super::Invocation::enter();
        let (program, errors) = crate::parse(
            "@require_contracts(strict)\n#[loop_bound(1)] fn f(int n)->int requires n>=0 ensures result>=0 {let i=0;while i<1 {i=i+1;}return i;}\n#[loop_bound(1)] fn f(int n)->int requires n>=0 ensures result>=0 {let i=0;while i<2 {i=i+1;}return i;}",
        );
        assert!(errors.is_empty(), "{errors:?}");
        let result = crate::loop_bound::check(&program, "<declaration-test>");
        crate::feature_attrs::reset();
        assert!(
            result.is_err(),
            "the second declaration exceeds its own bound"
        );
        assert!(
            result
                .unwrap_err()
                .contains("may exceed its declared bound")
        );
    }
}
