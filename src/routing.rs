//! Routes: each configured route (and each bare target) becomes one long-lived Switchyard
//! algorithm plus the target groups it chooses among. Algorithms are built once so their
//! session state survives across requests.

use std::collections::HashMap;
use std::sync::Arc;

use switchyard_libsy::{
    Algorithm, ClassifyTrigger as LibClassifyTrigger, EscalationJudgeConfig, LlmClassifierConfig,
    LlmTaskClassifier, Passthrough, PickerMode as LibPickerMode, Random, RuntimeModels,
    StageRouter, StageRouterConfig, TaskClassifierConfig,
};
use switchyard_protocol::{Category, ModelId};

use crate::config::{ClassifierMode, ClassifyTrigger, Config, PickerMode, RouteSpec};

/// A servable name: the algorithm and the targets it may choose among.
pub struct Route {
    pub algorithm: Arc<dyn Algorithm>,
    pub models: RuntimeModels,
}

pub struct Routes {
    by_name: HashMap<String, Route>,
}

fn ids(names: &[String]) -> Vec<ModelId> {
    names.iter().map(|n| ModelId::from(n.as_str())).collect()
}

/// `Category::Any` must list every target: algorithms validate their picks against it.
fn with_any(mut groups: HashMap<Category, Vec<ModelId>>) -> RuntimeModels {
    let mut all: Vec<ModelId> = Vec::new();
    for group in groups.values() {
        for id in group {
            if !all.contains(id) {
                all.push(id.clone());
            }
        }
    }
    groups.entry(Category::Any).or_insert(all);
    RuntimeModels::new(groups)
}

fn algorithm_error(route: &str, error: impl std::fmt::Display) -> String {
    format!("route `{route}`: {error}")
}

fn build_route(name: &str, spec: &RouteSpec) -> Result<Route, String> {
    let route = match spec {
        RouteSpec::Passthrough { targets } => Route {
            algorithm: Arc::new(Passthrough),
            models: with_any(HashMap::from([(Category::Any, ids(targets))])),
        },
        RouteSpec::Random {
            targets,
            weights,
            seed,
        } => Route {
            algorithm: Arc::new(
                Random::new(weights.clone(), *seed).map_err(|e| algorithm_error(name, e))?,
            ),
            models: with_any(HashMap::from([(Category::Any, ids(targets))])),
        },
        RouteSpec::StageRouter {
            efficient,
            capable,
            mode,
            confidence_threshold,
        } => {
            let mode = match mode {
                PickerMode::EfficientFirst => LibPickerMode::EfficientFirst,
                PickerMode::CapableFirst => LibPickerMode::CapableFirst,
            };
            Route {
                algorithm: Arc::new(
                    StageRouter::new(StageRouterConfig::new(mode, *confidence_threshold))
                        .map_err(|e| algorithm_error(name, e))?,
                ),
                models: with_any(HashMap::from([
                    (Category::Efficient, ids(efficient)),
                    (Category::Capable, ids(capable)),
                ])),
            }
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
                    contract: Default::default(),
                    config: EscalationJudgeConfig {
                        confirmations: *confirmations,
                        ..EscalationJudgeConfig::default()
                    },
                    max_output_tokens: TaskClassifierConfig::default().max_output_tokens,
                },
            };
            Route {
                algorithm: Arc::new(
                    LlmTaskClassifier::new(config).map_err(|e| algorithm_error(name, e))?,
                ),
                models: with_any(HashMap::from([
                    (Category::Efficient, ids(efficient)),
                    (Category::Capable, ids(capable)),
                    (Category::Judge, ids(judge)),
                ])),
            }
        }
    };
    Ok(route)
}

impl Routes {
    /// Builds every configured route, plus an implicit passthrough route per target.
    pub fn build(config: &Config) -> Result<Self, String> {
        let mut by_name = HashMap::new();
        for target in config.targets.keys() {
            by_name.insert(
                target.clone(),
                Route {
                    algorithm: Arc::new(Passthrough),
                    models: with_any(HashMap::from([(
                        Category::Any,
                        vec![ModelId::from(target.as_str())],
                    )])),
                },
            );
        }
        for (name, spec) in &config.routes {
            by_name.insert(name.clone(), build_route(name, spec)?);
        }
        Ok(Self { by_name })
    }

    pub fn get(&self, name: &str) -> Option<&Route> {
        self.by_name.get(name)
    }
}
