//! Actors (ADR 0037, `add-actors`): named owners of keys, a person (`human`) or an agent (`agent`) that acts for a
//! person (`owner`). They are labels for attribution and grant nothing: a key's routes, budgets and limits come only
//! from the key and its profile. The checks run once, when the configuration loads or reloads, so a request never
//! resolves anything.

use std::collections::BTreeMap;

use super::schema::RawConfig;
use super::{ActorConfig, ActorKind, ConfigError, invalid};

/// Checks every actor and every key's `actor`, and returns the actors as defined.
pub(super) fn resolve(raw: &RawConfig) -> Result<BTreeMap<String, ActorConfig>, ConfigError> {
    let mut defined = BTreeMap::new();
    for (id, actor) in &raw.actors {
        let kind = match actor.kind.as_deref() {
            Some("human") => ActorKind::Human,
            Some("agent") => ActorKind::Agent,
            Some(other) => {
                return Err(invalid(format!(
                    "actors.{id}.kind must be `human` or `agent`, not `{other}`"
                )));
            }
            None => {
                return Err(invalid(format!(
                    "actors.{id}.kind is required: `human` or `agent`"
                )));
            }
        };
        defined.insert(
            id.clone(),
            ActorConfig {
                kind,
                display_name: actor.display_name.clone(),
                owner: actor.owner.clone(),
            },
        );
    }
    for (id, actor) in &defined {
        let Some(owner) = &actor.owner else { continue };
        if actor.kind == ActorKind::Human {
            return Err(invalid(format!(
                "actors.{id}.owner is only for an agent; a human has no owner"
            )));
        }
        match defined.get(owner) {
            None => {
                return Err(invalid(format!(
                    "actors.{id}.owner names unknown actor `{owner}`; define it as [actors.{owner}]"
                )));
            }
            Some(found) if found.kind != ActorKind::Human => {
                return Err(invalid(format!(
                    "actors.{id}.owner must name a human actor, and `{owner}` is an agent"
                )));
            }
            Some(_) => {}
        }
    }
    for (id, key) in &raw.keys {
        if let Some(name) = &key.actor
            && !defined.contains_key(name)
        {
            return Err(invalid(format!(
                "keys.{id}.actor names unknown actor `{name}`; define it as [actors.{name}]"
            )));
        }
    }
    Ok(defined)
}

/// The person a key's usage rolls up to: the actor's owner for an agent, the actor itself for a human, and nobody
/// for a key with no actor or an agent with no owner.
pub(super) fn owner_of(
    actors: &BTreeMap<String, ActorConfig>,
    actor: Option<&str>,
) -> Option<String> {
    let id = actor?;
    let found = actors.get(id)?;
    match found.kind {
        ActorKind::Human => Some(id.to_string()),
        ActorKind::Agent => found.owner.clone(),
    }
}
