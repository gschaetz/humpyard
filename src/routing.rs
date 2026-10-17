//! Routes: each configured route (and each bare target) becomes one long-lived Switchyard
//! algorithm plus the target groups it chooses among. Algorithms are built once so their
//! session state survives across requests.

use std::collections::HashMap;
use std::sync::Arc;

use switchyard_libsy::{
    Algorithm, ClassifierContractConfig, ClassifyTrigger as LibClassifyTrigger,
    EscalationJudgeConfig, LlmClassifierConfig, LlmTaskClassifier, Passthrough,
    PickerMode as LibPickerMode, Random, RuntimeModels, StageRouter, StageRouterConfig,
    TaskClassifierConfig,
};
use switchyard_protocol::{Category, ModelId};

use crate::config::{ClassifierMode, ClassifyTrigger, Config, FallbackOn, PickerMode, RouteSpec};
use crate::latency::{Draws, Latency, Ranked, Settings};

/// A servable name: the algorithm and the target groups it chooses among.
pub struct Route {
    algorithm: Arc<dyn Algorithm>,
    /// Target groups by category, `Category::Any` excluded (it is derived).
    groups: HashMap<Category, Vec<ModelId>>,
    /// Random routes keep their weights: they follow target order, so removing a target needs
    /// them re-aligned.
    random: Option<RandomSpec>,
    /// A `fastest` route orders its eligible targets by observed speed for every request.
    fastest: Option<FastestSpec>,
    /// Which failures hand the request to the next target.
    fallback: FallbackOn,
    /// A pool route's guarded tail (ADR 0024): tried after every member, in this order.
    tail: Vec<ModelId>,
    /// Which failures of the last member may reach the tail.
    tail_on: FallbackOn,
}

/// How a `fastest` route weighs its targets, and the draws that decide when it explores.
struct FastestSpec {
    settings: Settings,
    draws: Draws,
}

/// The part of a `random` route that must be re-aligned when targets are removed.
struct RandomSpec {
    weights: Option<Vec<f64>>,
}

/// What to run for one request after the routing policy has had its say.
pub struct Plan {
    pub algorithm: Arc<dyn Algorithm>,
    pub models: RuntimeModels,
    pub fallback: FallbackOn,
    /// For a `fastest` route, the order this request will try its targets in, and why.
    pub ranking: Option<Ranked>,
    /// Set when members and tail are both eligible: only the named last member may hand over to the tail.
    pub tail_guard: Option<TailGuard>,
    /// Every pool member was removed, so only the tail is left. The caller plans again with the members
    /// it set aside (cold ones) before settling for this.
    pub tail_only: bool,
}

/// The guard on a tail (ADR 0024): after the last member fails, the request moves on only for these classes.
pub struct TailGuard {
    pub last_member: String,
    pub on: FallbackOn,
}

pub struct Routes {
    by_name: HashMap<String, Arc<Route>>,
}

fn ids(names: &[String]) -> Vec<ModelId> {
    names.iter().map(|n| ModelId::from(n.as_str())).collect()
}

/// `Category::Any` must list every target: algorithms validate their picks against it.
fn runtime_models(mut groups: HashMap<Category, Vec<ModelId>>) -> RuntimeModels {
    let mut all: Vec<ModelId> = Vec::new();
    let mut ordered: Vec<_> = groups.iter().collect();
    ordered.sort_by_key(|(category, _)| category.as_str().to_string());
    for (_, group) in ordered {
        for id in group {
            if !all.contains(id) {
                all.push(id.clone());
            }
        }
    }
    groups.insert(Category::Any, all);
    RuntimeModels::new(groups)
}

impl Route {
    fn new(algorithm: Arc<dyn Algorithm>, groups: HashMap<Category, Vec<ModelId>>) -> Self {
        Self {
            algorithm,
            groups,
            random: None,
            fastest: None,
            fallback: FallbackOn::default(),
            tail: Vec::new(),
            tail_on: FallbackOn::default(),
        }
    }

    /// The tail targets, in order (empty for a route without one).
    pub fn tail_names(&self) -> Vec<String> {
        self.tail.iter().map(|id| id.as_str().to_string()).collect()
    }

    /// Every target the route can use, in category order.
    fn all_targets(groups: &HashMap<Category, Vec<ModelId>>) -> Vec<ModelId> {
        let mut all = Vec::new();
        let mut ordered: Vec<_> = groups.iter().collect();
        ordered.sort_by_key(|(category, _)| category.as_str().to_string());
        for (_, group) in ordered {
            for id in group {
                if !all.contains(id) {
                    all.push(id.clone());
                }
            }
        }
        all
    }

    /// The algorithm and target groups for a request, keeping only targets `eligible` accepts.
    ///
    /// Removed targets disappear from every group, `Any` is rebuilt from what remains, and a
    /// tier left empty is served by the other tier so limits degrade routing instead of failing
    /// it. Returns `None` when no target is eligible.
    pub fn plan(&self, route: &str, eligible: impl Fn(&str) -> bool) -> Option<Plan> {
        self.plan_ranked(route, eligible, None, &|_| None)
    }

    /// Like [`Route::plan`]. For a `fastest` route, `latency` orders the eligible targets by observed
    /// speed (ADR 0020); ordering ranks what is left after removal and never adds a target back.
    /// Other routes ignore it.
    pub fn plan_ranked(
        &self,
        route: &str,
        eligible: impl Fn(&str) -> bool,
        latency: Option<&Latency>,
        quality: &dyn Fn(&str) -> Option<f64>,
    ) -> Option<Plan> {
        let keep = |ids: &[ModelId]| -> Vec<ModelId> {
            ids.iter()
                .filter(|id| eligible(id.as_str()))
                .cloned()
                .collect()
        };
        let mut groups: HashMap<Category, Vec<ModelId>> = self
            .groups
            .iter()
            .map(|(category, ids)| (category.clone(), keep(ids)))
            .collect();
        let tail = keep(&self.tail);
        if Self::all_targets(&groups).is_empty() && tail.is_empty() {
            return None;
        }
        // Degrade: an emptied tier borrows from the tiers that survive (capable <-> efficient;
        // a judge falls back to whichever tier is left).
        let remaining = |groups: &HashMap<Category, Vec<ModelId>>, order: &[Category]| {
            order
                .iter()
                .find_map(|c| groups.get(c).filter(|g| !g.is_empty()).cloned())
        };
        for (tier, fallback_order) in [
            (Category::Capable, [Category::Efficient, Category::Judge]),
            (Category::Efficient, [Category::Capable, Category::Judge]),
            (Category::Judge, [Category::Efficient, Category::Capable]),
        ] {
            let was_configured = self.groups.get(&tier).is_some_and(|g| !g.is_empty());
            let now_empty = groups.get(&tier).is_none_or(Vec::is_empty);
            if was_configured
                && now_empty
                && let Some(substitute) = remaining(&groups, &fallback_order)
            {
                tracing::info!(
                    route,
                    tier = tier.as_str(),
                    "tier has no eligible target; substituting"
                );
                groups.insert(tier, substitute);
            }
        }
        let algorithm = match &self.random {
            Some(spec)
                if groups
                    .get(&Category::Any)
                    .is_some_and(|g| g.len() != self.groups[&Category::Any].len()) =>
            {
                self.rebuilt_random(
                    spec.weights.as_deref(),
                    groups.get(&Category::Any).map(Vec::as_slice),
                )
            }
            _ => self.algorithm.clone(),
        };
        let ranking = match (&self.fastest, latency) {
            (Some(fastest), Some(latency)) => {
                let names: Vec<String> = groups
                    .get(&Category::Any)
                    .map(|any| any.iter().map(|id| id.as_str().to_string()).collect())
                    .unwrap_or_default();
                let ranked =
                    latency.rank(&names, &fastest.settings, quality, || fastest.draws.next());
                groups.insert(Category::Any, ids(&ranked.order));
                Some(ranked)
            }
            _ => None,
        };
        let members_left = !Self::all_targets(&groups).is_empty();
        let last_member = groups
            .get(&Category::Any)
            .and_then(|g| g.last())
            .map(|id| id.as_str().to_string());
        if !tail.is_empty() {
            groups
                .entry(Category::Any)
                .or_default()
                .extend(tail.clone());
        }
        let tail_guard = match (&last_member, tail.is_empty()) {
            (Some(last), false) => Some(TailGuard {
                last_member: last.clone(),
                on: self.tail_on.clone(),
            }),
            _ => None,
        };
        Some(Plan {
            algorithm,
            models: runtime_models(groups),
            fallback: self.fallback.clone(),
            ranking,
            tail_guard,
            tail_only: !tail.is_empty() && !members_left,
        })
    }

    /// For a `fastest` route, the order its eligible targets would be tried in right now by speed alone
    /// (no exploration, no draw consumed), for reports. `None` for other routes.
    pub fn speed_order(
        &self,
        latency: &Latency,
        names: &[String],
        quality: &dyn Fn(&str) -> Option<f64>,
    ) -> Option<Ranked> {
        let fastest = self.fastest.as_ref()?;
        let settings = Settings {
            explore: 0.0,
            ..fastest.settings
        };
        Some(latency.rank(names, &settings, quality, || 0.0))
    }

    /// For a `fastest` route, how much declared quality counts against speed, for reports.
    pub fn quality_weight(&self) -> Option<f64> {
        self.fastest.as_ref().map(|f| f.settings.quality_weight)
    }

    /// The probability a `fastest` route explores, for reports.
    pub fn explore(&self) -> Option<f64> {
        self.fastest.as_ref().map(|f| f.settings.explore)
    }

    fn rebuilt_random(
        &self,
        weights: Option<&[f64]>,
        kept: Option<&[ModelId]>,
    ) -> Arc<dyn Algorithm> {
        let original = &self.groups[&Category::Any];
        let kept = kept.unwrap_or(&[]);
        let aligned: Option<Vec<f64>> = weights.map(|w| {
            original
                .iter()
                .zip(w)
                .filter(|(id, _)| kept.contains(id))
                .map(|(_, weight)| *weight)
                .collect()
        });
        // Only zero-weight targets left: serve from them rather than fail.
        let aligned = aligned.filter(|w| w.iter().any(|x| *x > 0.0));
        match Random::new(aligned, None) {
            Ok(random) => Arc::new(random),
            Err(_) => self.algorithm.clone(),
        }
    }
}

fn algorithm_error(route: &str, error: impl std::fmt::Display) -> String {
    format!("route `{route}`: {error}")
}

/// A `fastest` route: a passthrough over its targets whose eligible set is ordered by speed per request.
fn fastest_route(targets: &[String], settings: Settings, seed: Option<u64>) -> Route {
    let mut route = Route::new(
        Arc::new(Passthrough),
        HashMap::from([(Category::Any, ids(targets))]),
    );
    route.fastest = Some(FastestSpec {
        settings,
        draws: Draws::new(seed),
    });
    route
}

/// The ordering settings a `fastest` route spec describes.
fn fastest_settings(spec: &RouteSpec) -> Option<Settings> {
    let RouteSpec::Fastest {
        explore,
        min_samples,
        max_age_secs,
        quality_weight,
        latency_target_ms,
        unknown_quality,
        ..
    } = spec
    else {
        return None;
    };
    Some(Settings {
        explore: *explore,
        min_samples: *min_samples,
        max_age_ms: i64::try_from(max_age_secs.saturating_mul(1000)).unwrap_or(i64::MAX),
        quality_weight: *quality_weight,
        latency_target_ms: crate::num::f64_from_u64(*latency_target_ms),
        unknown_quality: *unknown_quality,
    })
}

fn build_route(name: &str, spec: &RouteSpec) -> Result<Route, String> {
    let mut route = match spec {
        RouteSpec::Passthrough { targets, .. } => Route::new(
            Arc::new(Passthrough),
            HashMap::from([(Category::Any, ids(targets))]),
        ),
        RouteSpec::Fastest { targets, seed, .. } => {
            let settings = fastest_settings(spec)
                .ok_or_else(|| algorithm_error(name, "not a fastest route"))?;
            let mut route = fastest_route(targets, settings, *seed);
            if let Some(tail) = spec.tail() {
                route.tail = ids(&tail.targets);
                route.tail_on = tail.guard();
            }
            route
        }
        RouteSpec::Random {
            targets,
            weights,
            seed,
            ..
        } => {
            let mut route = Route::new(
                Arc::new(
                    Random::new(weights.clone(), *seed).map_err(|e| algorithm_error(name, e))?,
                ),
                HashMap::from([(Category::Any, ids(targets))]),
            );
            route.random = Some(RandomSpec {
                weights: weights.clone(),
            });
            route
        }
        RouteSpec::StageRouter {
            efficient,
            capable,
            mode,
            confidence_threshold,
            ..
        } => {
            let mode = match mode {
                PickerMode::EfficientFirst => LibPickerMode::EfficientFirst,
                PickerMode::CapableFirst => LibPickerMode::CapableFirst,
            };
            Route::new(
                Arc::new(
                    StageRouter::new(StageRouterConfig::new(mode, *confidence_threshold))
                        .map_err(|e| algorithm_error(name, e))?,
                ),
                HashMap::from([
                    (Category::Efficient, ids(efficient)),
                    (Category::Capable, ids(capable)),
                ]),
            )
        }
        RouteSpec::LlmClassifier {
            mode,
            efficient,
            capable,
            judge,
            base_threshold,
            threshold_step,
            classify_trigger,
            confirmations,
            ..
        } => {
            let config = match mode {
                ClassifierMode::Capability => LlmClassifierConfig::Capability {
                    config: TaskClassifierConfig {
                        base_threshold: *base_threshold,
                        threshold_step: *threshold_step,
                        classify_trigger: match classify_trigger {
                            ClassifyTrigger::EveryRequest => LibClassifyTrigger::EveryRequest,
                            ClassifyTrigger::UserTurn => LibClassifyTrigger::UserTurn,
                            ClassifyTrigger::NewSession => LibClassifyTrigger::NewSession,
                        },
                        ..TaskClassifierConfig::default()
                    },
                },
                ClassifierMode::Escalation => LlmClassifierConfig::Escalation {
                    contract: ClassifierContractConfig::default(),
                    config: EscalationJudgeConfig {
                        confirmations: *confirmations,
                        ..EscalationJudgeConfig::default()
                    },
                    max_output_tokens: TaskClassifierConfig::default().max_output_tokens,
                },
            };
            Route::new(
                Arc::new(LlmTaskClassifier::new(config).map_err(|e| algorithm_error(name, e))?),
                HashMap::from([
                    (Category::Efficient, ids(efficient)),
                    (Category::Capable, ids(capable)),
                    (Category::Judge, ids(judge)),
                ]),
            )
        }
    };
    route.fallback = spec.fallback_on();
    Ok(route)
}

impl Routes {
    /// Builds every configured route, plus an implicit passthrough route per target.
    /// Builds every route. With `previous` (the old config and its routes, on a reload), a route
    /// whose definition did not change is reused as is, which keeps its per-session algorithm
    /// state (escalation latches and the like).
    pub fn build(config: &Config, previous: Option<(&Config, &Routes)>) -> Result<Self, String> {
        let mut by_name = HashMap::new();
        for target in config.targets.keys() {
            by_name.insert(
                target.clone(),
                Arc::new(Route::new(
                    Arc::new(Passthrough),
                    HashMap::from([(Category::Any, vec![ModelId::from(target.as_str())])]),
                )),
            );
        }
        for (name, spec) in &config.routes {
            let reused = previous
                .filter(|(old, _)| old.routes.get(name) == Some(spec))
                .and_then(|(_, routes)| routes.by_name.get(name).cloned());
            let route = match reused {
                Some(route) => route,
                None => Arc::new(build_route(name, spec)?),
            };
            by_name.insert(name.clone(), route);
        }
        Ok(Self { by_name })
    }

    pub fn get(&self, name: &str) -> Option<&Route> {
        self.by_name.get(name).map(AsRef::as_ref)
    }
}

#[cfg(test)]
mod reuse_tests {
    use super::*;

    fn config(extra: &str) -> Config {
        let toml = format!(
            r#"listen = "127.0.0.1:0"
[providers.p]
base_url = "http://127.0.0.1:1"
api_key_env = "K"
[targets.a]
endpoints = [{{ provider = "p", model = "m" }}]
[targets.b]
endpoints = [{{ provider = "p", model = "n" }}]
[routes.steady]
type = "stage_router"
efficient = ["a"]
capable = ["b"]
[routes.moving]
type = "passthrough"
targets = ["a"]
{extra}"#
        );
        Config::from_toml(&toml, |_| Some("k".into())).unwrap()
    }

    #[test]
    fn a_reload_keeps_routes_whose_definition_did_not_change() {
        let old_config = config("");
        let old = Routes::build(&old_config, None).unwrap();

        // `moving` now lists another target; `steady` is untouched.
        let mut new_config = config("");
        new_config.routes.insert(
            "moving".into(),
            crate::config::RouteSpec::Passthrough {
                targets: vec!["b".into()],
                pool: None,
                fallback_on: None,
                allowed_classes: None,
            },
        );
        let new = Routes::build(&new_config, Some((&old_config, &old))).unwrap();

        let same = |name: &str| Arc::ptr_eq(&old.by_name[name], &new.by_name[name]);
        assert!(
            same("steady"),
            "unchanged route keeps its instance (and its session state)"
        );
        assert!(!same("moving"), "a changed route is rebuilt");
        // Bare-target routes are stateless and always rebuilt; they must still exist.
        assert!(new.get("a").is_some() && new.get("b").is_some());
    }
}
