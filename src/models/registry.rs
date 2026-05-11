use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use tracing::{error, info};

use super::manager::ResolvedModel;
use super::{Manager, ModelFamily};

#[derive(Debug, Clone)]
pub enum ModelStatus {
    Pulling,
    Ready(ResolvedModel),
    Failed(String),
}

impl ModelStatus {
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready(_))
    }
}

type Inner = HashMap<String, ModelStatus>;

/// Shared, thread-safe registry of model readiness states.
///
/// Created at startup; background pull threads update it; the queue driver
/// reads it before acquiring the model guard.
#[derive(Clone)]
pub struct ModelRegistry {
    state: Arc<RwLock<Inner>>,
}

impl ModelRegistry {
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn get(&self, name: &str) -> Option<ModelStatus> {
        self.state.read().unwrap().get(name).cloned()
    }

    pub fn set(&self, name: String, status: ModelStatus) {
        self.state.write().unwrap().insert(name, status);
    }

    /// Try to resolve each requested model. Models already on disk become
    /// `Ready`; missing ones become `Pulling` and a background thread is
    /// spawned to download them.
    pub fn init_models(
        &self,
        manager: Arc<Manager>,
        models: Vec<(String, Option<ModelFamily>)>,
    ) {
        for (name, family) in models {
            match manager.resolve(&name, family) {
                Ok(handle) => {
                    info!(model = %name, "model present on disk");
                    self.set(name, ModelStatus::Ready(handle));
                }
                Err(_) => {
                    if manager.catalog().find(&name).is_some() {
                        info!(model = %name, "model missing, starting background pull");
                        self.set(name.clone(), ModelStatus::Pulling);
                        let reg = self.clone();
                        let mgr = Arc::clone(&manager);
                        let model_name = name.clone();
                        let family_hint = family;
                        std::thread::spawn(move || {
                            pull_background(&mgr, &reg, &model_name, family_hint);
                        });
                    } else {
                        let msg = format!("model `{name}` not in catalog and not on disk");
                        error!(%msg);
                        self.set(name, ModelStatus::Failed(msg));
                    }
                }
            }
        }
    }
}

fn pull_background(
    manager: &Manager,
    registry: &ModelRegistry,
    name: &str,
    family: Option<ModelFamily>,
) {
    info!(model = %name, "background pull started");
    if let Err(e) = manager.pull(name) {
        let msg = format!("background pull failed: {e}");
        error!(model = %name, %msg);
        registry.set(name.to_string(), ModelStatus::Failed(msg));
        return;
    }
    match manager.resolve(name, family) {
        Ok(handle) => {
            info!(model = %name, "background pull complete, model ready");
            registry.set(name.to_string(), ModelStatus::Ready(handle));
        }
        Err(e) => {
            let msg = format!("resolve after pull failed: {e}");
            error!(model = %name, %msg);
            registry.set(name.to_string(), ModelStatus::Failed(msg));
        }
    }
}
