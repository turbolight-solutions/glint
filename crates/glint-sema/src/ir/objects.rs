//! Information object sets lowered to tables (X.681 12, X.682 10): one row per object, keyed by the `UNIQUE` field,
//! with the settings of the type fields lowered as types.

use std::collections::{BTreeMap, HashSet};

use glint_ir::{ObjectSetId, ObjectSetTable, Row};
use glint_syntax::{PrefixKind, SyntaxKind, SyntaxNode};

use super::{Actual, Env, Hint, Lowerer};
use crate::sem::toks::{flatten, inner};
use crate::sem::{
    ClassInfo, ClassRes, FieldKind, Kind as SymKind, ObjectInfo, ObjectSetSyntax, Operand,
    Resolved, SymRef, parse_object_set_syntax,
};
use crate::view::child;

/// How deeply object set references may nest.
const MAX_SET_DEPTH: usize = 12;

impl Lowerer<'_, '_, '_> {
    /// Lowers the object set written as `group` in `env`. `class` is the class it is governed by when the context
    /// says so; otherwise it is taken from the set's members.
    pub(super) fn object_set(
        &mut self,
        group: &SyntaxNode,
        env: &Env,
        class: Option<SymRef>,
    ) -> Option<ObjectSetId> {
        let toks = inner(group);
        // `{Dummy}`: the set the caller passed
        if let [only] = toks.as_slice()
            && let Some(actual) = env.actual(&only.text)
        {
            return match actual {
                Actual::ObjectSet(id) => Some(*id),
                _ => None,
            };
        }
        let syntax = parse_object_set_syntax(group);
        let single = match syntax.operands.as_slice() {
            [
                Operand::Named {
                    qualifier,
                    name,
                    actuals: None,
                    path,
                    ..
                },
            ] if path.is_empty() && !syntax.extensible => {
                match self
                    .sem
                    .resolve(env.module, &[], qualifier.as_deref(), name)
                {
                    Resolved::Sym(sym) if self.sem.kind(sym) == SymKind::ObjectSet => {
                        Some((sym, name.clone()))
                    }
                    _ => None,
                }
            }
            _ => None,
        };
        let text = flatten(group)
            .iter()
            .map(|t| t.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let key = match &single {
            Some((sym, _)) => format!("sym:{}:{}", sym.module.0, sym.index),
            None => {
                let dummies: Vec<String> = toks
                    .iter()
                    .filter_map(|t| match env.actual(&t.text) {
                        Some(Actual::ObjectSet(id)) => Some(format!("{}={}", t.text, id.0)),
                        _ => None,
                    })
                    .collect();
                format!(
                    "text:{}:{:?}:{}",
                    env.module.0,
                    group.text_range(),
                    dummies.join(",")
                )
            }
        };
        if let Some(&id) = self.set_memo.get(&key) {
            return Some(id);
        }
        let class = class.or_else(|| self.class_of_set(env, &syntax));
        let class_info = class.and_then(|c| self.sem.class_info(c));
        let id = ObjectSetId(u32::try_from(self.sets.len()).unwrap_or(u32::MAX));
        let (name, module) = match &single {
            Some((sym, name)) => (name.clone(), sym.module),
            None => (text.clone(), env.module),
        };
        self.sets.push(ObjectSetTable {
            name,
            module: self.module_name(module),
            class: class
                .and_then(|c| self.sem.symbol(c))
                .map_or_else(|| "?".to_owned(), |s| s.name.clone()),
            key_field: class_info
                .as_ref()
                .and_then(|info| info.fields.iter().find(|f| f.unique))
                .map(|f| f.name.clone()),
            rows: Vec::new(),
            extensible: false,
        });
        self.set_memo.insert(key, id);
        let Some(info) = class_info else {
            if !syntax.operands.is_empty() {
                self.note(format!(
                    "the class of the object set `{text}` is not available"
                ));
            }
            return Some(id);
        };
        let mut rows = Vec::new();
        let mut extensible = false;
        let mut visited = HashSet::new();
        self.collect(
            env,
            &syntax,
            &info,
            &mut rows,
            &mut extensible,
            &mut visited,
            0,
        );
        if let Some(table) = usize::try_from(id.0)
            .ok()
            .and_then(|i| self.sets.get_mut(i))
        {
            table.rows = rows;
            table.extensible = extensible;
        }
        Some(id)
    }

    /// The class of the first member of `syntax` that has one.
    fn class_of_set(&self, env: &Env, syntax: &ObjectSetSyntax) -> Option<SymRef> {
        syntax.operands.iter().find_map(|operand| {
            let Operand::Named {
                qualifier, name, ..
            } = operand
            else {
                return None;
            };
            let Resolved::Sym(sym) = self
                .sem
                .resolve(env.module, &[], qualifier.as_deref(), name)
            else {
                return None;
            };
            match self.sem.kind(sym) {
                SymKind::ObjectSet => match self.sem.governor_class(sym) {
                    ClassRes::Class(class) => Some(class),
                    _ => None,
                },
                SymKind::Object => self.sem.object_class(sym),
                _ => None,
            }
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn collect(
        &mut self,
        env: &Env,
        syntax: &ObjectSetSyntax,
        class: &ClassInfo,
        rows: &mut Vec<Row>,
        extensible: &mut bool,
        visited: &mut HashSet<SymRef>,
        depth: usize,
    ) {
        if depth > MAX_SET_DEPTH {
            self.note("object sets nest too deeply");
            return;
        }
        *extensible |= syntax.extensible;
        if syntax.arithmetic {
            self.note("object set arithmetic other than union is read as a union");
        }
        for operand in &syntax.operands {
            match operand {
                Operand::Inline { body, range } => {
                    let parsed = self
                        .sem
                        .parse_object_tokens(env.module, class, body, *range);
                    let row = self.row(env.module, None, class, &parsed.info);
                    rows.push(row);
                }
                Operand::Named {
                    qualifier,
                    name,
                    actuals,
                    path,
                    ..
                } => {
                    if qualifier.is_none()
                        && let Some(actual) = env.actual(name)
                    {
                        match actual {
                            Actual::ObjectSet(id) => {
                                if let Some(table) =
                                    usize::try_from(id.0).ok().and_then(|i| self.sets.get(i))
                                {
                                    rows.extend(table.rows.iter().cloned());
                                    *extensible |= table.extensible;
                                }
                            }
                            _ => self.note(format!("the parameter `{name}` is not an object set")),
                        }
                        continue;
                    }
                    if !path.is_empty() {
                        self.note(format!(
                            "an object set taken from the field of `{name}` is not modelled"
                        ));
                        continue;
                    }
                    if actuals.is_some() {
                        self.note(format!(
                            "the parameterized object or object set `{name}` is not expanded"
                        ));
                        continue;
                    }
                    let Resolved::Sym(target) =
                        self.sem
                            .resolve(env.module, &[], qualifier.as_deref(), name)
                    else {
                        self.note(format!("`{name}`: the defining module is not available"));
                        continue;
                    };
                    match self.sem.kind(target) {
                        SymKind::Object => {
                            if let Some(row) = self.object_row(target, class) {
                                rows.push(row);
                            } else {
                                self.note(format!("the object `{name}` cannot be read"));
                            }
                        }
                        SymKind::ObjectSet => {
                            if !visited.insert(target) {
                                continue;
                            }
                            let Some(group) = self
                                .sem
                                .symbol(target)
                                .and_then(|s| s.value_node.as_ref())
                                .and_then(|v| child(v, SyntaxKind::BraceGroup))
                            else {
                                self.note(format!("the object set `{name}` cannot be read"));
                                continue;
                            };
                            let nested = parse_object_set_syntax(&group);
                            self.collect(
                                &Env::new(target.module),
                                &nested,
                                class,
                                rows,
                                extensible,
                                visited,
                                depth.saturating_add(1),
                            );
                        }
                        other => self.note(format!(
                            "`{name}` is not an object or object set ({other:?})"
                        )),
                    }
                }
                Operand::Other { .. } => self.note("an object set element that cannot be read"),
            }
        }
    }

    /// The row of the object assignment `sym`, following `obj ::= other` to the definition.
    fn object_row(&mut self, sym: SymRef, class: &ClassInfo) -> Option<Row> {
        let mut current = sym;
        for _ in 0..MAX_SET_DEPTH {
            let symbol = self.sem.symbol(current)?;
            let value = symbol.value_node.as_ref()?;
            if let Some(group) = child(value, SyntaxKind::BraceGroup) {
                let parsed = self.sem.parse_object(current.module, class, &group);
                let name = self.sem.symbol(sym).map(|s| s.name.clone());
                return Some(self.row(current.module, name, class, &parsed.info));
            }
            let toks = flatten(value);
            let [only] = toks.as_slice() else {
                return None;
            };
            let Resolved::Sym(next) = self.sem.resolve(current.module, &[], None, &only.text)
            else {
                return None;
            };
            current = next;
        }
        None
    }

    /// A row from the settings of one object, read in the module it is written in.
    fn row(
        &mut self,
        module: crate::graph::ModuleId,
        object: Option<String>,
        class: &ClassInfo,
        info: &ObjectInfo,
    ) -> Row {
        let env = Env::new(module);
        let mut row = Row {
            object,
            key: None,
            key_name: None,
            types: BTreeMap::new(),
            values: BTreeMap::new(),
        };
        for setting in &info.settings {
            let Some(field) = class.field(&setting.field) else {
                continue;
            };
            match field.kind {
                FieldKind::Type => {
                    let ty = match self.parse_at(module, setting.range, PrefixKind::Type) {
                        Some(node) => self.ty(&node, &env, 0),
                        None => {
                            self.opaque(format!("the setting of `{}` is not a type", field.name))
                        }
                    };
                    row.types.insert(field.name.clone(), ty);
                }
                FieldKind::FixedValue => {
                    let Some(node) = self.parse_at(module, setting.range, PrefixKind::Value) else {
                        continue;
                    };
                    let hint = match field.governor.as_ref() {
                        Some(governor) => {
                            let governor_env = Env::new(class.sym.module);
                            let ty = self.ty(governor, &governor_env, 0);
                            self.hint(&ty)
                        }
                        None => Hint::None,
                    };
                    let value = self.value(&node, &env, &hint);
                    if field.unique {
                        let toks = flatten(&node);
                        if let [only] = toks.as_slice()
                            && only.kind == SyntaxKind::LowerName
                        {
                            row.key_name = Some(only.text.clone());
                        }
                        row.key = Some(value);
                    } else {
                        row.values.insert(field.name.clone(), value);
                    }
                }
                _ => {}
            }
        }
        row
    }
}
