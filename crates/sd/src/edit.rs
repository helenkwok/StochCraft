//! Changes to a model that reach beyond one variable: renaming one (and every place that names
//! it) and removing one (refused while something uses it).

use crate::layout::references;
use crate::model::{SdModel, VarKind, canonical};
use crate::{ErrorKind, SdError};

fn model_err(message: String) -> SdError {
    SdError {
        kind: ErrorKind::Model,
        message,
    }
}

/// The position of the variable called `name` (by canonical name).
pub fn find_variable(model: &SdModel, name: &str) -> Option<usize> {
    let want = canonical(name);
    model.vars.iter().position(|v| canonical(&v.name) == want)
}

/// What uses a variable.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Users {
    /// The variables whose equations name it, in the model's order.
    pub equations: Vec<String>,
    /// The stocks that list it as an inflow or an outflow, in the model's order.
    pub stocks: Vec<String>,
}

/// Who uses the variable `name`: the equations that name it and the stocks that list it as a
/// flow. A variable's own equation does not count.
pub fn users_of(model: &SdModel, name: &str) -> Users {
    let want = canonical(name);
    let mut out = Users::default();
    for v in &model.vars {
        if canonical(&v.name) != want
            && v.eqn
                .as_deref()
                .is_some_and(|e| references(e).contains(&want))
        {
            out.equations.push(v.name.clone());
        }
        if let VarKind::Stock {
            inflows, outflows, ..
        } = &v.kind
            && inflows
                .iter()
                .chain(outflows.iter())
                .any(|f| canonical(f) == want)
        {
            out.stocks.push(v.name.clone());
        }
    }
    out
}

/// How an equation writes the name `name`: as it stands if it is one word, else with
/// underscores for spaces, else in double quotes.
fn written(name: &str) -> String {
    let words: Vec<&str> = name.split_whitespace().collect();
    let joined = words.join("_");
    let simple = joined
        .chars()
        .next()
        .is_some_and(|c| c.is_alphabetic() || c == '_' || c == '$')
        && joined
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '$');
    if simple {
        joined
    } else {
        let mut q = String::from("\"");
        for c in name.trim().chars() {
            match c {
                '"' | '\\' => {
                    q.push('\\');
                    q.push(c);
                }
                c => q.push(c),
            }
        }
        q.push('"');
        q
    }
}

/// `text` with each use of the variable `old` (a canonical name) written as `new` (a name as
/// given). Everything else, spacing included, is kept.
pub(crate) fn rename_in_equation(text: &str, old: &str, new: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        if c == '"' {
            let mut j = i + 1;
            let mut name = String::new();
            while let Some(&e) = chars.get(j) {
                if e == '"' {
                    break;
                }
                if e == '\\' {
                    match chars.get(j + 1) {
                        Some('n') => name.push(' '),
                        Some(&x) => name.push(x),
                        None => {}
                    }
                    j += 2;
                } else {
                    name.push(e);
                    j += 1;
                }
            }
            let end = (j + 1).min(chars.len());
            if canonical(&name) == old {
                out.push_str(&written(new));
            } else {
                out.extend(chars.get(i..end).unwrap_or_default());
            }
            i = end;
        } else if c.is_ascii_digit()
            || (c == '.' && chars.get(i + 1).is_some_and(char::is_ascii_digit))
        {
            let mut j = i;
            while chars
                .get(j)
                .is_some_and(|d| d.is_ascii_digit() || *d == '.')
            {
                j += 1;
            }
            if chars.get(j).is_some_and(|e| *e == 'e' || *e == 'E') {
                let mut k = j + 1;
                if chars.get(k).is_some_and(|s| *s == '+' || *s == '-') {
                    k += 1;
                }
                if chars.get(k).is_some_and(char::is_ascii_digit) {
                    while chars.get(k).is_some_and(char::is_ascii_digit) {
                        k += 1;
                    }
                    j = k;
                }
            }
            out.extend(chars.get(i..j).unwrap_or_default());
            i = j;
        } else if c.is_alphabetic() || c == '_' || c == '$' {
            let mut j = i;
            while chars
                .get(j)
                .is_some_and(|d| d.is_alphanumeric() || *d == '_' || *d == '$')
            {
                j += 1;
            }
            let word: String = chars.get(i..j).unwrap_or_default().iter().collect();
            if canonical(&word) == old {
                out.push_str(&written(new));
            } else {
                out.push_str(&word);
            }
            i = j;
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

/// Renames the variable `old` to `new`, in its own entry, in every equation that names it, in
/// the flow lists of the stocks and in the diagram. Names are compared by their canonical
/// form, so a change of case or of spaces for underscores is a rename too.
pub fn rename_variable(model: &mut SdModel, old: &str, new: &str) -> Result<(), SdError> {
    let Some(idx) = find_variable(model, old) else {
        return Err(model_err(format!("no variable `{old}`")));
    };
    let new = new.trim();
    let (oc, nc) = (canonical(old), canonical(new));
    if nc.is_empty() {
        return Err(model_err("a variable needs a name".to_owned()));
    }
    if nc != oc && find_variable(model, new).is_some() {
        return Err(model_err(format!(
            "there is already a variable called `{new}` (names are not case-sensitive, and spaces and underscores are the same)"
        )));
    }
    if let Some(v) = model.vars.get_mut(idx) {
        v.name = new.to_owned();
    }
    if nc != oc {
        for v in &mut model.vars {
            if let Some(e) = &v.eqn {
                v.eqn = Some(rename_in_equation(e, &oc, new));
            }
            if let VarKind::Stock {
                inflows, outflows, ..
            } = &mut v.kind
            {
                for f in inflows.iter_mut().chain(outflows.iter_mut()) {
                    if canonical(f) == oc {
                        *f = new.to_owned();
                    }
                }
            }
        }
        model.layout.rename(&oc, &nc);
    }
    Ok(())
}

/// Removes the variable `name`. It is refused while another variable's equation names it, and
/// while a stock lists it as a flow, unless `force`, which takes it out of the stocks' lists
/// (never out of an equation). The refusal names who uses it.
pub fn remove_variable(model: &mut SdModel, name: &str, force: bool) -> Result<(), SdError> {
    let Some(idx) = find_variable(model, name) else {
        return Err(model_err(format!("no variable `{name}`")));
    };
    let users = users_of(model, name);
    let list = |v: &[String]| {
        v.iter()
            .map(|n| format!("`{n}`"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    if !users.equations.is_empty() {
        return Err(model_err(format!(
            "`{name}` is used in the equation of {}: change them first",
            list(&users.equations)
        )));
    }
    if !users.stocks.is_empty() && !force {
        return Err(model_err(format!(
            "`{name}` is a flow of the stock {}: take it out of their flows first, or remove it with `force`",
            list(&users.stocks)
        )));
    }
    let want = canonical(name);
    for v in &mut model.vars {
        if let VarKind::Stock {
            inflows, outflows, ..
        } = &mut v.kind
        {
            inflows.retain(|f| canonical(f) != want);
            outflows.retain(|f| canonical(f) != want);
        }
    }
    model.vars.remove(idx);
    model.layout.remove(&want);
    Ok(())
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::model::Variable;

    #[test]
    fn a_rename_rewrites_names_and_leaves_the_rest() {
        assert_eq!(
            rename_in_equation(
                "0.1 * Pop + pop1 - \"Pop\" + POP(2) + 1e5",
                "pop",
                "Fish stock"
            ),
            "0.1 * Fish_stock + pop1 - Fish_stock + Fish_stock(2) + 1e5"
        );
        assert_eq!(
            rename_in_equation("IF a < 1 THEN \"my var\" ELSE b", "my_var", "R&D"),
            "IF a < 1 THEN \"R&D\" ELSE b"
        );
    }

    #[test]
    fn rename_and_remove_follow_the_references() {
        let mut m = SdModel {
            vars: vec![
                Variable::stock("Pop", "100", &["births"], &[]),
                Variable::flow("births", "rate * Pop"),
                Variable::aux("rate", "0.1"),
                Variable::aux("twice", "2 * rate"),
            ],
            ..SdModel::default()
        };
        m.layout = crate::Layout::auto(&m);
        rename_variable(&mut m, "rate", "growth rate").unwrap();
        assert_eq!(m.vars[1].eqn.as_deref(), Some("growth_rate * Pop"));
        assert_eq!(m.vars[3].eqn.as_deref(), Some("2 * growth_rate"));
        assert!(m.layout.shapes.contains_key("growth_rate"));
        assert!(rename_variable(&mut m, "births", "POP").is_err());
        let e = remove_variable(&mut m, "growth rate", false).unwrap_err();
        assert!(
            e.message.contains("`births`") && e.message.contains("`twice`"),
            "{e}"
        );
        let e = remove_variable(&mut m, "twice", false);
        assert!(e.is_ok());
        let e = remove_variable(&mut m, "births", false).unwrap_err();
        assert!(e.message.contains("`Pop`"), "{e}");
    }
}
