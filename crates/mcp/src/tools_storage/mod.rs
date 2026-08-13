mod add_storage;
mod common;
mod edit_storage;
mod export_config;
mod import_config;
mod list_storages;
mod remove_storage;
#[cfg(test)]
mod tests;
mod validate_storage;

pub use add_storage::{add_storage, AddStorageInput, AddStorageOutput};
pub use edit_storage::{edit_storage, EditStorageInput, EditStorageOutput, EditStoragePatch};
pub use export_config::{export_config, ExportConfigOutput};
pub use import_config::{
    apply_storage_import, apply_storage_import_with_validator, preview_storage_import,
    ApplyStorageImportInput, ApplyStorageImportResult, PreviewStorageImportInput,
    StorageImportPreview,
};
pub use list_storages::{list_storages, ListStoragesInput, ListStoragesOutput};
pub use remove_storage::{remove_storage, RemoveStorageInput, RemoveStorageOutput};
pub use validate_storage::{
    validate_storage, validate_storage_record, StorageCapabilities, ValidateStorageInput,
    ValidateStorageOutput,
};
