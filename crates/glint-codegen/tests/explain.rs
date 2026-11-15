#![allow(clippy::unwrap_used, clippy::panic)]

//! Every `codegen` example in `glint explain`: a `bad` one makes every backend report its code, a `good` one makes
//! none report it. The example's tags name the root type (`M.T`).

use glint_check::lower_user_type;
use glint_codegen::{Options, backends, generate};
use glint_corpus::EmptyCorpus;
use glint_diag::{examples, explain, explained_codes};

#[test]
fn codegen_examples_behave_as_the_explanation_says() {
    let mut failures = Vec::new();
    let mut tested = 0;
    for code in explained_codes() {
        for example in examples(explain(code).unwrap()) {
            if !example.tags.contains(&"codegen") {
                continue;
            }
            let root = example.tags.iter().find(|t| t.contains('.')).unwrap();
            let (module, ty) = root.split_once('.').unwrap();
            let set = lower_user_type(
                &[example.text.to_owned()],
                &EmptyCorpus::default(),
                module,
                ty,
            )
            .unwrap_or_else(|e| panic!("{code}: {e}"));
            for backend in backends() {
                let output = generate(&set, *backend, &Options::default());
                let raised = output.findings.iter().any(|f| f.code == code);
                let id = backend.meta().id;
                if example.tags.contains(&"bad") && !raised {
                    failures.push(format!(
                        "{code} ({id}): bad example does not raise it:\n{}",
                        example.text
                    ));
                }
                if example.tags.contains(&"good") && raised {
                    failures.push(format!(
                        "{code} ({id}): good example raises it:\n{}",
                        example.text
                    ));
                }
            }
            tested += 1;
        }
    }
    assert!(tested >= 6, "only {tested} codegen examples");
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}
