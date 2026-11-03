pub(crate) mod components;
mod constraints;
mod imports;
mod oids;
mod policy;
mod references;
pub(crate) mod tags;
mod values;
mod x681;
mod x682;
mod x683;

use glint_syntax::TextRange;

use crate::eval::Evaluator;
use crate::finding::{Finding, Loc};
use crate::graph::{LoadedModule, ModuleGraph, ModuleId};
use crate::sem::Sem;

/// What every check needs to read and report.
pub(crate) struct Ctx<'a, 'g, 'c> {
    pub graph: &'g ModuleGraph<'c>,
    pub module: ModuleId,
    pub loaded: &'g LoadedModule,
    pub eval: &'a Evaluator<'g, 'c>,
    pub sem: &'a Sem<'g, 'c>,
    pub findings: &'a mut Vec<Finding>,
}

impl Ctx<'_, '_, '_> {
    pub fn loc(&self, range: TextRange) -> Loc {
        Loc {
            source: self.loaded.source,
            range,
        }
    }

    pub fn report(&mut self, finding: Finding) {
        self.findings.push(finding);
    }
}

pub(crate) fn check_module(graph: &ModuleGraph<'_>, module: ModuleId, findings: &mut Vec<Finding>) {
    let Some(loaded) = graph.module(module) else {
        return;
    };
    let eval = Evaluator::new(graph);
    let sem = Sem::new(graph);
    let mut ctx = Ctx {
        graph,
        module,
        loaded,
        eval: &eval,
        sem: &sem,
        findings,
    };
    imports::check(&mut ctx);
    references::check(&mut ctx);
    components::check(&mut ctx);
    tags::check(&mut ctx);
    oids::check(&mut ctx);
    values::check(&mut ctx);
    constraints::check(&mut ctx);
    x681::check(&mut ctx);
    x682::check(&mut ctx);
    x683::check(&mut ctx);
    policy::check(&mut ctx);
}
