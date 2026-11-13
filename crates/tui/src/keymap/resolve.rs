// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! From `tui.keymap` to the keys each action has (Codex's
//! `RuntimeKeymap::resolve`): an action takes `tui.keymap.<context>.<name>`,
//! else `tui.keymap.global.<name>` when that is no global action's, else its
//! defaults. What cannot be used is left out with a warning: an unknown
//! context or action, a key that is not one, a key that types text beside
//! the composer (but for an action read only on an empty composer), and a
//! binding that conflicts with another action's or with a key the
//! interface keeps; the action keeps its defaults then.

use std::collections::BTreeMap;

use super::actions::{Action, Context, PATHS, RESERVED, SHARED_KEYS};
use super::catalogue::ACTIONS;
use super::spec::{Binding, KeyStroke};

/// Each action's bindings, by `Action as usize`.
pub(crate) type Bindings = Vec<Vec<Binding>>;

/// The bindings `config` gives each action over the defaults, which actions
/// it changed, and what was left out.
pub(crate) fn resolve(
    config: &BTreeMap<String, BTreeMap<String, Vec<String>>>,
) -> (Bindings, Vec<bool>, Vec<String>) {
    let defaults = defaults();
    let mut warnings = Vec::new();
    let mut bindings = defaults.clone();
    let mut configured = vec![false; ACTIONS.len()];
    for (action, keys, path) in overrides(config, &mut warnings) {
        match parse_keys(action, &keys) {
            Ok(parsed) => {
                bindings[action as usize] = parsed;
                configured[action as usize] = true;
            }
            Err(error) => warnings.push(format!("{path}: {error}; the default keys stay")),
        }
    }
    // The default leader yields to a key the configuration binds (Codex's
    // `leader.rs`).
    if !configured[Action::Leader as usize] {
        let taken: Vec<KeyStroke> = configured
            .iter()
            .zip(&bindings)
            .filter(|(configured, _)| **configured)
            .flat_map(|(_, bound)| bound.iter().filter_map(prefix))
            .collect();
        bindings[Action::Leader as usize]
            .retain(|binding| prefix(binding).is_none_or(|key| !taken.contains(&key)));
    }
    while let Some(conflict) = conflicts(&bindings).into_iter().next() {
        let Some(action) = [conflict.second, conflict.first]
            .into_iter()
            .flatten()
            .find(|action| configured[*action as usize])
        else {
            // Only the defaults are left in the conflict, which tests rule
            // out; stop rather than loop.
            break;
        };
        warnings.push(format!(
            "tui.keymap.{}: {}; the default keys stay",
            action.path(),
            conflict.message
        ));
        bindings[action as usize] = defaults[action as usize].clone();
        configured[action as usize] = false;
    }
    (bindings, configured, warnings)
}

/// Every action's default bindings.
pub(crate) fn defaults() -> Bindings {
    ACTIONS
        .iter()
        .map(|spec| {
            spec.defaults
                .iter()
                .filter_map(|key| Binding::parse(key).ok())
                .collect()
        })
        .collect()
}

/// The actions `config` names, with their keys and the path they came
/// from: context entries first, then the global fallbacks for actions no
/// context entry named.
fn overrides(
    config: &BTreeMap<String, BTreeMap<String, Vec<String>>>,
    warnings: &mut Vec<String>,
) -> Vec<(Action, Vec<String>, String)> {
    let mut named = Vec::new();
    for (context_name, actions) in config {
        let Some(context) = Context::named(context_name) else {
            warnings.push(format!(
                "tui.keymap.{context_name}: no such context; it is ignored"
            ));
            continue;
        };
        for (name, keys) in actions {
            let path = format!("tui.keymap.{context_name}.{name}");
            match Action::named(context, name) {
                Some(action) => named.push((action, keys.clone(), path)),
                None if context == Context::Global
                    && ACTIONS.iter().any(|spec| spec.name == name) => {}
                None => warnings.push(format!("{path}: no such action; it is ignored")),
            }
        }
    }
    let Some(global) = config.get(Context::Global.name()) else {
        return named;
    };
    let mut fallbacks = Vec::new();
    for spec in ACTIONS
        .iter()
        .filter(|spec| spec.context != Context::Global)
    {
        if let Some(keys) = global.get(spec.name) {
            if !named.iter().any(|(action, _, _)| *action == spec.action) {
                let path = format!("tui.keymap.global.{}", spec.name);
                fallbacks.push((spec.action, keys.clone(), path));
            }
        }
    }
    named.extend(fallbacks);
    named
}

/// `keys` as `action`'s bindings, or why they cannot be.
pub(crate) fn parse_keys(action: Action, keys: &[String]) -> Result<Vec<Binding>, String> {
    let mut parsed: Vec<Binding> = Vec::new();
    for key in keys {
        let binding = Binding::parse(key)?;
        check_binding(action, binding)?;
        if !parsed.contains(&binding) {
            parsed.push(binding);
        }
    }
    Ok(parsed)
}

/// Whether `binding` can be `action`'s at all, whatever else is bound.
fn check_binding(action: Action, binding: Binding) -> Result<(), String> {
    let context = action.spec().context;
    let spec = binding.spec();
    if action == Action::Leader {
        return match binding {
            Binding::Key(key) if !key.is_text() => Ok(()),
            Binding::Key(_) => Err(format!(
                "the leader `{spec}` would type text; use a key with ctrl or alt, or a key \
                 that types none"
            )),
            _ => Err(format!("the leader is one key, not `{spec}`")),
        };
    }
    // An action read only on an empty composer may take a text key, but
    // no chord that starts with one: the chord would swallow the text.
    let text_allowed = action.spec().empty_composer_only && matches!(binding, Binding::Key(_));
    if context.beside_text() && !text_allowed && prefix(&binding).is_some_and(|key| key.is_text()) {
        return Err(format!(
            "`{spec}` types text beside the composer; use a key with ctrl or alt, or one \
             that types none"
        ));
    }
    Ok(())
}

/// The key a binding starts with: the whole binding for a key, the first
/// stroke for a chord; `None` for a leader chord, which starts with the
/// leader (expanded in [`conflicts`]).
fn prefix(binding: &Binding) -> Option<KeyStroke> {
    match binding {
        Binding::Key(key) | Binding::Chord(key, _) => Some(*key),
        Binding::Leader(_) => None,
    }
}

/// Two actions (or an action and a reserved key, `first: None`) that one
/// key would both trigger.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Conflict {
    pub(crate) first: Option<Action>,
    pub(crate) second: Option<Action>,
    pub(crate) message: String,
}

/// Every conflict of `bindings`: on one path of the input, the same key
/// (or chord) for two actions that do not share keys, a key that starts
/// another action's chord, or a key the interface keeps.
pub(crate) fn conflicts(bindings: &Bindings) -> Vec<Conflict> {
    let leaders: Vec<KeyStroke> = bindings[Action::Leader as usize]
        .iter()
        .filter_map(prefix)
        .collect();
    let mut found = Vec::new();
    for path in PATHS {
        let bound: Vec<(Action, Expanded)> = ACTIONS
            .iter()
            .filter(|spec| path.contains(&spec.context))
            .flat_map(|spec| {
                bindings[spec.action as usize]
                    .iter()
                    .flat_map(|binding| expand(binding, &leaders))
                    .map(move |expanded| (spec.action, expanded))
            })
            .collect();
        for (index, (first, one)) in bound.iter().enumerate() {
            for (second, other) in &bound[index + 1..] {
                if first == second || shared(*first, *second) {
                    continue;
                }
                if let Some(message) = clash(*first, one, *second, other) {
                    push(&mut found, Some(*first), Some(*second), message);
                }
            }
        }
    }
    for spec in &ACTIONS {
        for binding in &bindings[spec.action as usize] {
            let Some((key, use_)) = RESERVED.iter().find_map(|(key, use_, contexts)| {
                let key = KeyStroke::parse(key).ok()?;
                (contexts.contains(&spec.context) && prefix(binding) == Some(key))
                    .then_some((key, use_))
            }) else {
                continue;
            };
            let message = format!("`{}` is kept for {use_}", key.spec());
            push(&mut found, None, Some(spec.action), message);
        }
    }
    found
}

/// A binding as the keys it is pressed with: a leader chord once for each
/// leader key.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Expanded {
    Key(KeyStroke),
    Chord(KeyStroke, KeyStroke),
}

fn expand(binding: &Binding, leaders: &[KeyStroke]) -> Vec<Expanded> {
    match *binding {
        Binding::Key(key) => vec![Expanded::Key(key)],
        Binding::Chord(first, second) => vec![Expanded::Chord(first, second)],
        Binding::Leader(second) => leaders
            .iter()
            .map(|leader| Expanded::Chord(*leader, second))
            .collect(),
    }
}

fn shared(first: Action, second: Action) -> bool {
    SHARED_KEYS
        .iter()
        .any(|pair| *pair == (first, second) || *pair == (second, first))
}

/// Why `one` (of `first`) and `other` (of `second`) cannot both be bound.
fn clash(first: Action, one: &Expanded, second: Action, other: &Expanded) -> Option<String> {
    let (a, b) = (first.path(), second.path());
    match (one, other) {
        (left, right) if left == right => {
            Some(format!("`{}` is bound to both {a} and {b}", spec(left)))
        }
        // The leader is the start of every leader chord.
        (Expanded::Key(key), Expanded::Chord(prefix, _))
            if key == prefix && first != Action::Leader =>
        {
            Some(format!(
                "`{}` ({a}) starts {b}'s chord `{}`",
                key.spec(),
                spec(other)
            ))
        }
        (Expanded::Chord(prefix, _), Expanded::Key(key))
            if key == prefix && second != Action::Leader =>
        {
            Some(format!(
                "`{}` ({b}) starts {a}'s chord `{}`",
                key.spec(),
                spec(one)
            ))
        }
        _ => None,
    }
}

fn spec(expanded: &Expanded) -> String {
    match expanded {
        Expanded::Key(key) => key.spec(),
        Expanded::Chord(first, second) => format!("{} {}", first.spec(), second.spec()),
    }
}

fn push(found: &mut Vec<Conflict>, first: Option<Action>, second: Option<Action>, message: String) {
    let conflict = Conflict {
        first,
        second,
        message,
    };
    if !found.contains(&conflict) {
        found.push(conflict);
    }
}
