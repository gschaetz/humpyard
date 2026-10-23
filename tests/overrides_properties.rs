//! Runtime overrides only ever remove (spec: `request-routing`, ADR 0025): whatever set of overrides is in force,
//! a route's plan never gains a target, every target that goes is explained by an override that is in force, and
//! naming something that does not exist changes nothing. Checked over random override sets against real routes.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding

use std::collections::BTreeSet;

use humpyard::config::Config;
use humpyard::overrides::{Kind, Override, removed_by};
use humpyard::routing::Routes;
use proptest::prelude::*;
use switchyard_protocol::Category;

const TOML: &str = r#"
listen = "127.0.0.1:0"
[providers.p0]
base_url = "http://127.0.0.1:1"
api_key_env = "K"
[providers.p1]
base_url = "http://127.0.0.1:1"
api_key_env = "K"
[providers.p2]
base_url = "http://127.0.0.1:1"
api_key_env = "K"
[targets.t0]
endpoints = [{ provider = "p0", model = "m" }]
[targets.t1]
endpoints = [{ provider = "p1", model = "m" }]
[targets.t2]
endpoints = [{ provider = "p0", model = "m" }, { provider = "p1", model = "m" }]
[targets.t3]
endpoints = [{ provider = "p2", model = "m" }]
[targets.t4]
endpoints = [{ provider = "p1", model = "m" }, { provider = "p2", model = "m" }]
[targets.t5]
endpoints = [{ provider = "p0", model = "m" }]
[routes.passes]
type = "passthrough"
targets = ["t0", "t1", "t2"]
[routes.tiers]
type = "stage_router"
efficient = ["t3"]
capable = ["t4", "t5"]
[routes.dice]
type = "random"
targets = ["t1", "t3", "t5"]
"#;

/// Every name an override might use: real targets, providers and routes (including the implicit per-target
/// routes), and one that exists nowhere.
const SUBJECTS: [&str; 16] = [
    "t0", "t1", "t2", "t3", "t4", "t5", "p0", "p1", "p2", "passes", "tiers", "dice", "ghost", "t0",
    "p1", "t4",
];
const ROUTES: [&str; 11] = [
    "passes", "tiers", "dice", "t0", "t1", "t2", "t3", "t4", "t5", "ghost", "passes",
];

struct World {
    config: Config,
    routes: Routes,
    route_names: Vec<String>,
}

fn world() -> World {
    let config = Config::from_toml(TOML, |_| Some("k".into())).unwrap();
    let routes = Routes::build(&config, None).unwrap();
    let mut route_names: Vec<String> = config.routes.keys().cloned().collect();
    route_names.extend(config.targets.keys().cloned());
    World {
        config,
        routes,
        route_names,
    }
}

fn providers<'a>(world: &'a World, target: &str) -> Vec<&'a str> {
    world
        .config
        .targets
        .get(target)
        .map(|endpoints| endpoints.iter().map(|e| e.provider.as_str()).collect())
        .unwrap_or_default()
}

fn arb_override() -> impl Strategy<Value = Override> {
    (0..3_usize, 0..SUBJECTS.len(), 0..ROUTES.len()).prop_map(|(kind, subject, route)| {
        let kind = [Kind::DisableTarget, Kind::DrainProvider, Kind::PinRoute][kind];
        Override {
            id: 0,
            kind,
            subject: SUBJECTS[subject].to_string(),
            route: (kind == Kind::PinRoute).then(|| ROUTES[route].to_string()),
            reason: "r".into(),
            created_ms: 0,
            expires_ms: i64::MAX,
            created_by: "ops".into(),
        }
    })
}

fn arb_active() -> impl Strategy<Value = Vec<Override>> {
    prop::collection::vec(arb_override(), 0..8).prop_map(|mut list| {
        for (i, o) in list.iter_mut().enumerate() {
            o.id = i64::try_from(i).unwrap() + 1;
        }
        list
    })
}

/// The targets a route would try, with `eligible` deciding which may be used.
fn plan_targets(world: &World, route: &str, eligible: impl Fn(&str) -> bool) -> BTreeSet<String> {
    world
        .routes
        .get(route)
        .and_then(|r| r.plan(route, eligible))
        .map(|plan| {
            plan.models
                .models_for(&Category::Any)
                .iter()
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default()
}

proptest! {
    /// No set of overrides adds a target to any route's plan, and the plan with none is the plan without them.
    #[test]
    fn an_override_never_adds_a_target(active in arb_active()) {
        let w = world();
        for route in &w.route_names {
            let before = plan_targets(&w, route, |_| true);
            let after = plan_targets(&w, route, |t| {
                removed_by(&active, route, t, &providers(&w, t)).is_none()
            });
            prop_assert!(
                after.is_subset(&before),
                "route {route}: {after:?} is not within {before:?} with {active:?}"
            );
        }
    }

    /// Every target that goes is explained by an override that is in force, and nothing goes without one.
    #[test]
    fn every_removal_names_the_override_responsible(active in arb_active()) {
        let w = world();
        for route in &w.route_names {
            let before = plan_targets(&w, route, |_| true);
            let after = plan_targets(&w, route, |t| {
                removed_by(&active, route, t, &providers(&w, t)).is_none()
            });
            for gone in before.difference(&after) {
                let by = removed_by(&active, route, gone, &providers(&w, gone));
                prop_assert!(by.is_some(), "route {route}: `{gone}` went with no override to blame");
                prop_assert!(
                    active.iter().any(|o| Some(o.id) == by.map(|b| b.id)),
                    "the blamed override must be one in force"
                );
            }
        }
    }

    /// With no overrides in force every plan is exactly what it was.
    #[test]
    fn no_overrides_changes_nothing(_x in 0..1_u8) {
        let w = world();
        for route in &w.route_names {
            let before = plan_targets(&w, route, |_| true);
            let after = plan_targets(&w, route, |t| removed_by(&[], route, t, &providers(&w, t)).is_none());
            prop_assert_eq!(before, after);
        }
    }

    /// Overrides that name only things that do not exist change nothing for any route.
    #[test]
    fn naming_something_that_does_not_exist_changes_nothing(count in 1..6_usize, kind in 0..3_usize) {
        let w = world();
        let kind = [Kind::DisableTarget, Kind::DrainProvider, Kind::PinRoute][kind];
        let active: Vec<Override> = (0..count)
            .map(|i| Override {
                id: i64::try_from(i).unwrap() + 1,
                kind,
                subject: "ghost".into(),
                route: (kind == Kind::PinRoute).then(|| "ghost".to_string()),
                reason: "r".into(),
                created_ms: 0,
                expires_ms: i64::MAX,
                created_by: "ops".into(),
            })
            .collect();
        for route in &w.route_names {
            let before = plan_targets(&w, route, |_| true);
            let after = plan_targets(&w, route, |t| removed_by(&active, route, t, &providers(&w, t)).is_none());
            prop_assert_eq!(&before, &after, "route {}", route);
        }
    }

    /// A pinned route keeps only targets a pin names; the other routes are untouched by that pin.
    #[test]
    fn a_pin_only_narrows_its_own_route(target in 0..6_usize) {
        let w = world();
        let pinned = format!("t{target}");
        let active = vec![Override {
            id: 1,
            kind: Kind::PinRoute,
            subject: pinned.clone(),
            route: Some("passes".into()),
            reason: "r".into(),
            created_ms: 0,
            expires_ms: i64::MAX,
            created_by: "ops".into(),
        }];
        for route in &w.route_names {
            let before = plan_targets(&w, route, |_| true);
            let after = plan_targets(&w, route, |t| removed_by(&active, route, t, &providers(&w, t)).is_none());
            if route == "passes" {
                prop_assert!(after.iter().all(|t| *t == pinned), "{after:?} is not only {pinned}");
            } else {
                prop_assert_eq!(before, after, "a pin on `passes` must not touch `{}`", route);
            }
        }
    }
}
