pub mod atomic_file;
pub mod backup;
pub mod config;
pub mod models;
pub mod operations;
pub mod registry;
pub mod runtime;
pub mod schema;
pub mod secrets;
pub mod util;
pub mod workspaces;

pub use crate::models::{CoreError, Entry, Result, Source, SourceKind};
pub use crate::registry::OperatorRegistry;
pub use crate::runtime::{get_or_create_operator, invalidate_source, CacheKey, OperatorCache};
pub use crate::secrets::{
    contains_plaintext_secrets, discover_secret_field_names, extract_secret_fields,
    merge_secret_config, strip_secret_fields, MemorySecretStore, NativeSecretStore, SecretStore,
    SecretStoreStatus, UnavailableSecretStore, KEYRING_SERVICE,
};
