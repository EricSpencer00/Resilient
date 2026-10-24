//! RES-4986: module function identities without leaking local aliases.
use super::{ContractInfo, Type, TypeChecker, TypeEnvironment, render_duplicate_fn_error};
use crate::{Node, span::Span};
use std::rc::Rc;

struct FunctionExport {
    name: String,
    ty: Type,
    contract: Rc<ContractInfo>,
    span: Span,
    parameters: Option<Vec<String>>,
    bounds: Option<Vec<Vec<String>>>,
}

impl TypeChecker {
    fn bind_module_function(
        &mut self,
        node: &Node,
        binding: &str,
        reject_duplicate: bool,
    ) -> Result<(), String> {
        let Node::Function {
            parameters,
            requires,
            ensures,
            fails,
            return_type,
            type_params,
            type_param_bounds,
            span,
            ..
        } = node
        else {
            return Ok(());
        };
        if reject_duplicate && let Some(previous) = self.fn_decl_spans.get(binding) {
            return Err(render_duplicate_fn_error(
                &self.source_path,
                *span,
                binding,
                *previous,
            ));
        }
        self.fn_decl_spans.insert(binding.to_owned(), *span);
        self.contract_table.insert(
            binding.to_owned(),
            Rc::new(ContractInfo {
                parameters: parameters.clone(),
                requires: requires.clone(),
                ensures: ensures.clone(),
                fails: fails.clone(),
            }),
        );
        if !type_params.is_empty() {
            self.fn_type_params
                .insert(binding.to_owned(), type_params.clone());
            self.fn_type_param_bounds
                .insert(binding.to_owned(), type_param_bounds.clone());
        } else {
            self.fn_type_params.remove(binding);
            self.fn_type_param_bounds.remove(binding);
        }
        let params = parameters
            .iter()
            .map(|(ty, _)| self.parse_type_name(ty).unwrap_or(Type::Any))
            .collect();
        let result = return_type
            .as_ref()
            .map(|ty| self.parse_type_name(ty).unwrap_or(Type::Any))
            .unwrap_or(Type::Any);
        self.env.set(
            binding.to_owned(),
            Type::Function {
                params,
                return_type: Box::new(result),
            },
        );
        Ok(())
    }

    pub(super) fn hoist_module_functions(
        &mut self,
        module: &str,
        body: &[Node],
    ) -> Result<(), String> {
        // Match compiler pass1: only directly nested functions are callable here.
        for node in body {
            if let Node::Function { name, .. } = node {
                self.bind_module_function(node, &format!("{module}::{name}"), true)?;
            }
        }
        Ok(())
    }

    fn with_module_bindings<T>(
        &mut self,
        check: impl FnOnce(&mut Self) -> Result<T, String>,
    ) -> Result<T, String> {
        // These are owned maps with immutable Rc/Arc values; cloning does not
        // grant the child mutation access to the parent's binding tables.
        // Preserve all lexical/callable metadata, including error exits.
        let saved = (
            self.env.clone(),
            self.contract_table.clone(),
            self.fn_decl_spans.clone(),
            self.const_bindings.clone(),
            self.struct_fields.clone(),
            self.declared_structs.clone(),
            self.repr_c_structs.clone(),
            self.generic_struct_info.clone(),
            self.enum_decls.clone(),
            self.enum_type_params.clone(),
            self.type_aliases.clone(),
            self.current_fn_fails.clone(),
            self.current_fn_return_type.clone(),
            self.current_self_assoc_types.clone(),
            self.loop_depth,
            self.loop_label_stack.clone(),
            self.fn_type_params.clone(),
            self.fn_type_param_bounds.clone(),
            self.assoc_type_bindings.clone(),
            self.trait_impls.clone(),
            self.trait_supers.clone(),
            self.trait_default_methods.clone(),
            self.trait_default_method_defs.clone(),
        );
        self.env = TypeEnvironment::new_enclosed(self.env.clone());
        let result = check(self);
        (
            self.env,
            self.contract_table,
            self.fn_decl_spans,
            self.const_bindings,
            self.struct_fields,
            self.declared_structs,
            self.repr_c_structs,
            self.generic_struct_info,
            self.enum_decls,
            self.enum_type_params,
            self.type_aliases,
            self.current_fn_fails,
            self.current_fn_return_type,
            self.current_self_assoc_types,
            self.loop_depth,
            self.loop_label_stack,
            self.fn_type_params,
            self.fn_type_param_bounds,
            self.assoc_type_bindings,
            self.trait_impls,
            self.trait_supers,
            self.trait_default_methods,
            self.trait_default_method_defs,
        ) = saved;
        result
    }

    pub(super) fn check_module_bindings(
        &mut self,
        module: &str,
        body: &[Node],
    ) -> Result<Type, String> {
        for node in body {
            if let Node::Function { name, .. } = node {
                let qualified = format!("{module}::{name}");
                let export = self.with_module_bindings(|checker| {
                    for sibling in body {
                        if let Node::Function { name, .. } = sibling {
                            checker.bind_module_function(sibling, name, false)?;
                            checker.bind_module_function(
                                sibling,
                                &format!("{module}::{name}"),
                                false,
                            )?;
                        }
                    }
                    // Rename only the declaration identity. Parameters/body bindings
                    // stay local; proof caches/captured sites use exactly module::fn.
                    let mut declaration = node.clone();
                    if let Node::Function { name, .. } = &mut declaration {
                        *name = qualified.clone();
                    }
                    checker.check_node(&declaration)?;
                    Ok(FunctionExport {
                        name: qualified.clone(),
                        ty: checker
                            .env
                            .get(&qualified)
                            .expect("checked function binding"),
                        contract: checker.contract_table[&qualified].clone(),
                        span: checker.fn_decl_spans[&qualified],
                        parameters: checker.fn_type_params.get(&qualified).cloned(),
                        bounds: checker.fn_type_param_bounds.get(&qualified).cloned(),
                    })
                })?;
                self.env.set(export.name.clone(), export.ty);
                self.contract_table
                    .insert(export.name.clone(), export.contract);
                self.fn_decl_spans.insert(export.name.clone(), export.span);
                if let Some(parameters) = export.parameters {
                    self.fn_type_params.insert(export.name.clone(), parameters);
                }
                if let Some(bounds) = export.bounds {
                    self.fn_type_param_bounds.insert(export.name, bounds);
                }
            } else if matches!(
                node,
                Node::ImplBlock { .. } | Node::StructDecl { .. } | Node::EnumDecl { .. }
            ) {
                // Preserve legacy declaration checking. In particular runtime
                // impl methods deliberately use outer Struct$method identities,
                // with no module prefix, and must remain visible after the mod.
                // This does not introduce qualified struct/enum type support.
                self.check_node(node)?;
            } else {
                // Runtime ordinary module statements have temporary environments;
                // restore constant/type/callable metadata as well as variables.
                self.with_module_bindings(|checker| checker.check_node(node))?;
            }
        }
        Ok(Type::Void)
    }
}
