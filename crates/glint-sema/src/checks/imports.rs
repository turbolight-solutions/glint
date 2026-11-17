//! Import checks: W100, E101, W102, I103, E104, I105, W106, E107, I109, E116 (X.680 13.7-13.16).

use std::collections::HashSet;

use glint_corpus::{IanaVerdict, ModuleRecord, Oid, SourceKind, iana_conflict, iana_verdict};

use super::Ctx;
use crate::finding::{Finding, Loc};
use crate::graph::{Found, Matched, ModuleGraph, ModuleId, Origin};
use crate::model::{AssignedId, ExportsModel, SymbolKind};

pub(super) fn check(ctx: &mut Ctx<'_, '_, '_>) {
    let loaded = ctx.loaded;
    let mut seen_names: HashSet<&str> = HashSet::new();
    let mut seen_oids = Vec::new();
    let mut opaque_reported: HashSet<&str> = HashSet::new();
    let mut curated_reported: HashSet<usize> = HashSet::new();
    for (index, clause) in loaded.model.imports.iter().enumerate() {
        let Some(resolved) = loaded.imports.get(index) else {
            continue;
        };
        // X.680 13.16 e
        let duplicate_name = !seen_names.insert(clause.module_name.as_str());
        let duplicate_oid = resolved
            .assigned_oid
            .as_ref()
            .is_some_and(|oid| seen_oids.contains(oid));
        if let Some(oid) = &resolved.assigned_oid {
            seen_oids.push(oid.clone());
        }
        if duplicate_name || duplicate_oid || resolved.matched == Matched::SelfImport {
            let what = if resolved.matched == Matched::SelfImport {
                format!("module `{}` imports itself", clause.module_name)
            } else {
                format!("module `{}` is imported more than once", clause.module_name)
            };
            let finding = Finding::new("E104", what, ctx.loc(clause.module_range));
            ctx.report(finding);
            if resolved.matched == Matched::SelfImport {
                continue;
            }
        }

        // X.680 13.9
        if let Some((AssignedId::Defined { module, name, .. }, range)) = &clause.assigned
            && !satisfies_13_9(ctx, module.as_deref(), name, &mut Vec::new())
        {
            let finding = Finding::new(
                "E107",
                format!("`{name}` cannot be used in an assigned identifier"),
                ctx.loc(*range),
            )
            .with_note(
                "a value reference there must be defined in this module (using only such references), \
                 or imported from a module whose own identifier uses no references",
            );
            ctx.report(finding);
        }

        let Some(target_id) = resolved.target else {
            if opaque_reported.insert(clause.module_name.as_str()) {
                let finding = Finding::new(
                    "W100",
                    format!(
                        "module `{}` is not in the draft, the RFC corpus or the ITU-T set; its symbols are treated as opaque",
                        clause.module_name
                    ),
                    ctx.loc(clause.module_range),
                );
                ctx.report(finding);
            }
            continue;
        };
        let Some(target) = ctx.graph.module(target_id) else {
            continue;
        };

        let record = match target.origin {
            Origin::Corpus { record } => ctx
                .graph
                .corpus()
                .modules()
                .get(record)
                .map(|r| (record, r)),
            Origin::Draft | Origin::Builtin => None,
        };

        // X.680 13.10, 13.11
        if resolved.oid_mismatch
            && resolved.matched == Matched::NameOnly
            && let (Some(given), Some(actual)) = (&resolved.assigned_oid, &target.oid)
            && given != actual
        {
            let mut finding = Finding::new(
                "W102",
                format!(
                    "module `{}` was matched by name, but its identifier is {actual}, not {given}",
                    clause.module_name
                ),
                ctx.loc(clause.module_range),
            )
            .with_note("the object identifier is definitive (X.680 13.11)");
            if let Some(note) = registry_note(ctx, &clause.module_name, record, given, actual) {
                finding = finding.with_note(note);
            }
            ctx.report(finding);
        }

        // X.680 13.12
        if clause.assigned.is_none()
            && let Some(oid) = &target.oid
        {
            let finding = Finding::new(
                "W106",
                format!(
                    "`{}` has the identifier {oid}, but IMPORTS gives none",
                    clause.module_name
                ),
                ctx.loc(clause.module_range),
            )
            .with_help(format!(
                "write `{} {{ {} }}`",
                clause.module_name,
                oid.to_string().replace('.', " ")
            ));
            ctx.report(finding);
        }

        if ctx.graph.options().resolution_notes
            && let Some((rec_index, rec)) = record
        {
            if resolved.candidates.len() > 1 && resolved.matched == Matched::NameOnly {
                let finding = Finding::new(
                    "I103",
                    format!(
                        "`{}` resolved to {} ({} candidates)",
                        clause.module_name,
                        rec.source.label(),
                        resolved.candidates.len()
                    ),
                    ctx.loc(clause.module_range),
                );
                ctx.report(finding);
            }
            if rec.source.is_draft() {
                let finding = Finding::new(
                    "I109",
                    format!(
                        "`{}` resolved to the Internet-Draft {} ({}), which may still change",
                        clause.module_name,
                        rec.source.document,
                        rec.source.edition.as_deref().unwrap_or("undated")
                    ),
                    ctx.loc(clause.module_range),
                )
                .with_help("pin it in glint.toml once the module is published, or import the RFC");
                ctx.report(finding);
            }
            if let Some(message) = curated_note(rec)
                && curated_reported.insert(rec_index)
            {
                ctx.report(Finding::new("I105", message, ctx.loc(clause.module_range)));
            }
        }

        // X.680 13.16 a, b
        for symbol in &clause.symbols {
            match ctx.graph.find_in_module(target_id, &symbol.name, 16) {
                Found::Symbol { .. } => {
                    if !exported(target, &symbol.name) {
                        let finding = Finding::new(
                            "E101",
                            format!(
                                "`{}` is not exported by `{}`",
                                symbol.name, clause.module_name
                            ),
                            ctx.loc(symbol.range),
                        );
                        ctx.report(finding);
                    }
                }
                Found::MissingInTarget => {
                    let mut finding = Finding::new(
                        "E101",
                        format!(
                            "`{}` is not defined in module `{}`",
                            symbol.name, clause.module_name
                        ),
                        ctx.loc(symbol.range),
                    );
                    if resolved.candidates.len() > 1 {
                        finding = finding.with_note(format!(
                            "the module name matches {} modules in the corpus; add its OID to IMPORTS to choose one",
                            resolved.candidates.len()
                        ));
                    }
                    if let Some((_, rec)) = record
                        && matches!(
                            rec.parse_status,
                            glint_corpus::ParseStatus::Errors
                                | glint_corpus::ParseStatus::Unparseable
                        )
                    {
                        finding = finding.with_note(format!(
                            "{} does not check cleanly in the corpus, so symbols may be missing",
                            rec.label()
                        ));
                    }
                    ctx.report(finding);
                }
                Found::Opaque => {}
            }
        }
    }
}

fn exported(target: &crate::graph::LoadedModule, name: &str) -> bool {
    match &target.model.exports {
        ExportsModel::Absent | ExportsModel::All => true,
        ExportsModel::List(list) => list.iter().any(|(n, _)| n == name),
    }
}

// X.680 13.9 a, b
fn satisfies_13_9(
    ctx: &Ctx<'_, '_, '_>,
    qualifier: Option<&str>,
    name: &str,
    visiting: &mut Vec<String>,
) -> bool {
    let model = &ctx.loaded.model;
    if let Some(q) = qualifier {
        return model
            .clause_by_module_name(q)
            .and_then(|i| model.imports.get(i))
            .is_some_and(|clause| {
                clause.symbols.iter().any(|s| s.name == name)
                    && clause_uses_no_references(ctx, clause)
            });
    }
    if let Some(symbol) = model.first_local(name) {
        if symbol.kind != SymbolKind::Value || visiting.iter().any(|v| v == name) {
            return false;
        }
        visiting.push(name.to_owned());
        let ok = symbol.value_node.as_ref().is_some_and(|value| {
            let Some(group) = crate::view::child(value, glint_syntax::SyntaxKind::BraceGroup)
            else {
                return false;
            };
            let result = ctx.eval.eval_oid_group(ctx.module, &group);
            result.references.iter().all(|(q, n, _)| {
                crate::names::is_registered_anywhere(n)
                    || satisfies_13_9(ctx, q.as_deref(), n, visiting)
            })
        });
        visiting.pop();
        return ok;
    }
    model.imported(name).iter().any(|&(clause_index, _)| {
        model
            .imports
            .get(clause_index)
            .is_some_and(|clause| clause_uses_no_references(ctx, clause))
    })
}

fn clause_uses_no_references(ctx: &Ctx<'_, '_, '_>, clause: &crate::model::ImportClause) -> bool {
    match &clause.assigned {
        None => true,
        Some((AssignedId::Defined { .. }, _)) => false,
        Some((AssignedId::Oid { group }, _)) => ctx
            .eval
            .eval_oid_group(ctx.module, group)
            .references
            .is_empty(),
    }
}

/// For `W102`: when the IANA module identifier registries side with the identifier the import gives, against the one
/// the corpus module declares (as for RFC 5912's `AttributeCertificateVersion1-2009`, erratum 9255).
fn registry_note(
    ctx: &Ctx<'_, '_, '_>,
    name: &str,
    record: Option<(usize, &ModuleRecord)>,
    given: &Oid,
    actual: &Oid,
) -> Option<String> {
    let corpus = ctx.graph.corpus();
    let document = record.map(|(_, r)| r.source.document.as_str());
    let against = iana_conflict(corpus, name, document, actual, None)?;
    let Some(IanaVerdict::Registered(entry)) = iana_verdict(corpus, name, None, given, None) else {
        return None;
    };
    if &entry.oid != given {
        return None;
    }
    let source = record.map_or_else(String::new, |(_, r)| format!(" in {}", r.source.title()));
    Some(format!(
        "IANA registers {given} as `{}` for {}; the identifier {actual} that the module declares{source} is \
         registered as `{}` for {} (W146)",
        entry.description,
        entry.registrant(),
        against.description,
        against.registrant()
    ))
}

/// The `I105` note for a corpus module whose text is not the published one: corrected by errata patches, or replaced
/// by a `correction` override. `None` for a module as published (or repaired by an `extraction` override).
fn curated_note(rec: &glint_corpus::ModuleRecord) -> Option<String> {
    let document = glint_corpus::document_title(&rec.source.document);
    if let SourceKind::Overridden { tier, errata_ids } = &rec.source.kind
        && *tier == glint_corpus::OverrideTier::Correction
    {
        let errata = if errata_ids.is_empty() {
            String::new()
        } else {
            format!(", errata {}", errata_ids.join(", "))
        };
        return Some(format!(
            "using curated version of {} from {document} (correction{errata})",
            rec.name
        ));
    }
    if rec.source.errata.is_empty() {
        return None;
    }
    let ids: Vec<String> = rec.source.errata.iter().map(u32::to_string).collect();
    Some(format!(
        "using {} from {document} as corrected by errata {}",
        rec.name,
        ids.join(", ")
    ))
}

// X.680 13.7 (a module name is used once), 13.8 (a module identifier identifies one module) and the NOTE to 13.10
// (editions of a module share its name and differ in their identifiers): among `modules`, the modules an import
// resolves to before the corpus (the draft's and those of the other documents checked with it, not snippets that
// quote a module header), no two may share an identifier, nor a name unless both have identifiers and they differ. An
// import could not tell them apart. Within one document (`own`), a module shown twice with the same name and
// identifier is one module, as the corpus build takes it: a document may quote a module's header with a stub body
// before the full module (RFC 4120 Section 5 and Appendix A).
pub(crate) fn unique_identity(
    graph: &ModuleGraph<'_>,
    module: ModuleId,
    own: &[ModuleId],
    others: &[ModuleId],
    findings: &mut Vec<Finding>,
) {
    let Some(loaded) = graph.module(module) else {
        return;
    };
    let candidates = own
        .iter()
        .map(|&id| (id, true))
        .chain(others.iter().map(|&id| (id, false)));
    for (id, same_document) in candidates {
        let Some(other) = graph.module(id).filter(|_| id != module) else {
            continue;
        };
        let same_oid = loaded.oid.is_some() && loaded.oid == other.oid;
        let editions = loaded.oid.is_some() && other.oid.is_some() && loaded.oid != other.oid;
        let same_name = loaded.model.name == other.model.name;
        let shown_twice = same_document && same_name && loaded.oid == other.oid;
        if shown_twice || !(same_oid || (same_name && !editions)) {
            continue;
        }
        let what = if same_name {
            format!("module `{}` is defined twice", loaded.model.name)
        } else {
            format!(
                "module `{}` has the identifier of module `{}`",
                loaded.model.name, other.model.name
            )
        };
        let at = |m: &crate::graph::LoadedModule| Loc {
            source: m.source,
            range: m.model.name_range,
        };
        findings.push(
            Finding::new("E116", what, at(loaded))
                .with_secondary(at(other), "also defined here")
                .with_note("an import of the module cannot tell the two apart"),
        );
    }
}
