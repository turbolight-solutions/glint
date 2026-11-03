//! The differences between the normal form of a snippet and that of a module definition.

use glint_syntax::TextRange;

use super::norm::{DEFAULT_TAGGING, Norm};

const MAX_DIFFERENCES: usize = 12;
const MAX_TEXT: usize = 72;

/// One difference, with a place in each text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Difference {
    pub snippet: TextRange,
    pub module: TextRange,
    pub reason: String,
    /// The difference may only be a name the module cannot resolve.
    pub uncertain: bool,
}

/// Every difference between `snippet` and `module`, most important first (text order); empty when they are equal.
pub(crate) fn differences(snippet: &Norm, module: &Norm) -> Vec<Difference> {
    let mut walker = Walker { out: Vec::new() };
    walker.pair(snippet, module, None);
    walker.out
}

fn shorten(text: &str) -> String {
    if text.chars().count() <= MAX_TEXT {
        return text.to_owned();
    }
    let cut: String = text.chars().take(MAX_TEXT).collect();
    format!("{cut}…")
}

/// Two texts for a message; when they are long, both start a little before the first character where they differ.
fn excerpts(a: &str, b: &str) -> (String, String) {
    if a.chars().count() <= MAX_TEXT && b.chars().count() <= MAX_TEXT {
        return (a.to_owned(), b.to_owned());
    }
    let first = a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count();
    let start = first.saturating_sub(24);
    let window = |text: &str| {
        let total = text.chars().count();
        let part: String = text.chars().skip(start).take(MAX_TEXT).collect();
        let head = if start > 0 { "…" } else { "" };
        let tail = if start.saturating_add(MAX_TEXT) < total {
            "…"
        } else {
            ""
        };
        format!("{head}{part}{tail}")
    };
    (window(a), window(b))
}

fn noun(label: &str) -> &str {
    match label {
        "tagged" => "tag",
        "other" => "construct",
        other => other,
    }
}

struct Walker {
    out: Vec<Difference>,
}

impl Walker {
    fn push(&mut self, snippet: TextRange, module: TextRange, reason: String) {
        if self.out.len() < MAX_DIFFERENCES {
            self.out.push(Difference {
                snippet,
                module,
                reason,
                uncertain: false,
            });
        }
    }

    fn with_scope(reason: String, scope: Option<&str>) -> String {
        match scope {
            Some(scope) => format!("{reason} (in {scope})"),
            None => reason,
        }
    }

    fn pair(&mut self, a: &Norm, m: &Norm, scope: Option<&str>) {
        let before = self.out.len();
        self.pair_inner(a, m, scope);
        if a.uncertain || m.uncertain {
            for difference in self.out.iter_mut().skip(before) {
                difference.uncertain = true;
            }
        }
    }

    fn pair_inner(&mut self, a: &Norm, m: &Norm, scope: Option<&str>) {
        if a.label != m.label {
            let (here, there) = excerpts(&a.text(), &m.text());
            let reason = format!(
                "{} is `{here}` here but `{there}` in the module",
                noun(a.label)
            );
            self.push(a.range, m.range, Self::with_scope(reason, scope));
            return;
        }
        if a.atom != m.atom {
            if a.key.is_some() && m.key.is_some() {
                let reason = format!(
                    "{} `{}` here is `{}` in the module",
                    noun(a.label),
                    a.atom,
                    m.atom
                );
                self.push(a.range, m.range, Self::with_scope(reason, scope));
            } else if a.label == "tagging"
                && (a.atom == DEFAULT_TAGGING || m.atom == DEFAULT_TAGGING)
            {
                let reason = if m.atom == DEFAULT_TAGGING {
                    format!(
                        "`{}` is written out here; the module leaves the tagging to the module default",
                        a.atom
                    )
                } else {
                    format!(
                        "`{}` is written out in the module; the snippet leaves the tagging to the module default",
                        m.atom
                    )
                };
                self.push(a.range, m.range, Self::with_scope(reason, scope));
                return;
            } else if a.label == "presence" {
                let reason = format!(
                    "{} is `{}` here but `{}` in the module",
                    scope.unwrap_or("component"),
                    a.atom,
                    m.atom
                );
                self.push(a.range, m.range, reason);
                return;
            } else {
                let (here, there) = excerpts(&a.atom, &m.atom);
                let reason = format!(
                    "{} is `{here}` here but `{there}` in the module",
                    noun(a.label)
                );
                self.push(a.range, m.range, Self::with_scope(reason, scope));
                return;
            }
        }
        let own_scope = a
            .key
            .as_ref()
            .filter(|_| matches!(a.label, "component" | "alternative" | "addition group"))
            .map(|k| format!("{} `{k}`", a.label));
        let scope = own_scope.as_deref().or(scope);
        self.children(a, m, scope);
    }

    fn children(&mut self, a: &Norm, m: &Norm, scope: Option<&str>) {
        let keyed = a.children.iter().all(|c| c.key.is_some())
            && m.children.iter().all(|c| c.key.is_some())
            && (!a.children.is_empty() || !m.children.is_empty());
        if keyed && a.partial {
            self.partial(a, m, scope);
        } else if keyed {
            self.keyed(a, m, scope);
        } else {
            self.positional(a, m, scope);
        }
    }

    fn positional(&mut self, a: &Norm, m: &Norm, scope: Option<&str>) {
        let skip_a = usize::from(parameters(a).is_some());
        let skip_m = usize::from(parameters(m).is_some());
        if let (Some(p), None) = (parameters(a), parameters(m)) {
            let reason = format!(
                "the snippet has parameters `{}` that the module's definition does not have",
                p.atom
            );
            self.push(p.range, m.range, reason);
        }
        if let (None, Some(p)) = (parameters(a), parameters(m)) {
            let reason = format!(
                "the module's definition has parameters `{}` that the snippet does not have",
                p.atom
            );
            self.push(a.range, p.range, reason);
        }
        let mine = a.children.get(skip_a..).unwrap_or_default();
        let theirs = m.children.get(skip_m..).unwrap_or_default();
        for (x, y) in mine.iter().zip(theirs) {
            self.pair(x, y, scope);
        }
        for extra in mine.iter().skip(theirs.len()) {
            let reason = format!(
                "extra {} `{}` here that the module does not have",
                noun(extra.label),
                shorten(&extra.text())
            );
            self.push(extra.range, m.range, Self::with_scope(reason, scope));
        }
        for missing in theirs.iter().skip(mine.len()) {
            let reason = format!(
                "{} `{}` of the module is missing here",
                noun(missing.label),
                shorten(&missing.text())
            );
            self.push(a.range, missing.range, Self::with_scope(reason, scope));
        }
    }

    fn keyed(&mut self, a: &Norm, m: &Norm, scope: Option<&str>) {
        let key_in = |list: &[Norm], key: &Option<String>| list.iter().any(|c| &c.key == key);
        let mut i = 0usize;
        let mut j = 0usize;
        while let (Some(x), Some(y)) = (a.children.get(i), m.children.get(j)) {
            if x.key == y.key {
                self.pair(x, y, scope);
                i = i.saturating_add(1);
                j = j.saturating_add(1);
                continue;
            }
            let x_known = key_in(&m.children, &x.key);
            let y_known = key_in(&a.children, &y.key);
            match (x_known, y_known) {
                (false, false) if x.label == y.label => {
                    self.pair(x, y, scope);
                    i = i.saturating_add(1);
                    j = j.saturating_add(1);
                }
                (false, false) => {
                    let reason = format!("{} `{}` is not in the module", noun(x.label), x.atom);
                    self.push(x.range, m.range, Self::with_scope(reason, scope));
                    i = i.saturating_add(1);
                }
                (true, true) => {
                    let reason = format!(
                        "{}s are in a different order: `{}` here where the module has `{}`",
                        noun(x.label),
                        x.atom,
                        y.atom
                    );
                    self.push(x.range, y.range, Self::with_scope(reason, scope));
                    i = i.saturating_add(1);
                    j = j.saturating_add(1);
                }
                (false, true) => {
                    let reason = format!("{} `{}` is not in the module", noun(x.label), x.atom);
                    self.push(x.range, m.range, Self::with_scope(reason, scope));
                    i = i.saturating_add(1);
                }
                (true, false) => {
                    let reason = format!(
                        "{} `{}` of the module is missing here",
                        noun(y.label),
                        y.atom
                    );
                    self.push(a.range, y.range, Self::with_scope(reason, scope));
                    j = j.saturating_add(1);
                }
            }
        }
        for extra in a.children.iter().skip(i) {
            let reason = format!(
                "{} `{}` is not in the module",
                noun(extra.label),
                extra.atom
            );
            self.push(extra.range, m.range, Self::with_scope(reason, scope));
        }
        for missing in m.children.iter().skip(j) {
            let reason = format!(
                "{} `{}` of the module is missing here",
                noun(missing.label),
                missing.atom
            );
            self.push(a.range, missing.range, Self::with_scope(reason, scope));
        }
    }

    // The snippet's items must appear in the module in the same order; the module may have more.
    fn partial(&mut self, a: &Norm, m: &Norm, scope: Option<&str>) {
        let mut from = 0usize;
        for x in &a.children {
            let found = m
                .children
                .iter()
                .enumerate()
                .skip(from)
                .find(|(_, y)| y.key == x.key);
            match found {
                Some((position, y)) => {
                    self.pair(x, y, scope);
                    from = position.saturating_add(1);
                }
                None => {
                    let reason = if m.children.iter().any(|y| y.key == x.key) {
                        format!(
                            "{} `{}` is out of order: the module has it earlier",
                            noun(x.label),
                            x.atom
                        )
                    } else {
                        format!("{} `{}` is not in the module", noun(x.label), x.atom)
                    };
                    self.push(x.range, m.range, Self::with_scope(reason, scope));
                }
            }
        }
    }
}

fn parameters(norm: &Norm) -> Option<&Norm> {
    norm.children.first().filter(|c| c.label == "parameters")
}
