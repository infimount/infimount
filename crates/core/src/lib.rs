pub mod atomic_file;
pub mod config;
pub mod models;
pub mod operations;
pub mod registry;
pub mod schema;
pub mod secrets;
pub mod util;

pub use crate::models::{CoreError, Entry, Result, Source, SourceKind};
pub use crate::registry::OperatorRegistry;
pub use crate::secrets::{
    discover_secret_field_names, extract_secret_fields, merge_secret_config, strip_secret_fields,
    MemorySecretStore, NativeSecretStore, SecretStore, SecretStoreStatus, UnavailableSecretStore,
    KEYRING_SERVICE,
};
