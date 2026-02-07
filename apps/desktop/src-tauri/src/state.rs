use infimount_core::{config, OperatorRegistry};

pub struct AppState {
    pub registry: OperatorRegistry,
}

impl AppState {
    pub fn new() -> Self {
        let sources = config::load_sources().unwrap_or_default();
        let registry = OperatorRegistry::new(sources);
        Self { registry }
    }
}
