#![allow(non_snake_case)]

use infimount_core::{operations, CoreError};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::state::AppState;

fn reject_namespace_conflicts(
    state: &State<'_, AppState>,
    from_source_id: &str,
    to_source_id: &str,
    paths: &[String],
    target_dir: &str,
) -> Result<(), CoreError> {
    if from_source_id == to_source_id {
        return Ok(());
    }
    let from_storage = state
        .find_storage_by_id(from_source_id)
        .map_err(crate::state::mcp_error_to_core_error)?;
    let to_storage = state
        .find_storage_by_id(to_source_id)
        .map_err(crate::state::mcp_error_to_core_error)?;
    for path in paths {
        let relation = infimount_mcp::storage_namespace::transfer_namespace_relation(
            &from_storage,
            path,
            &to_storage,
            target_dir,
        )
        .map_err(|error| CoreError::Config(error.to_string()))?;
        if infimount_mcp::storage_namespace::transfer_has_namespace_conflict(&relation) {
            return Err(CoreError::Config(format!(
                "transfer destination overlaps the source namespace: {path}"
            )));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TransferProgressPayload {
    job_id: String,
    completed_items: u64,
    total_items: u64,
    bytes_transferred: u64,
    total_bytes: u64,
    current_path: String,
}

fn parse_transfer_operation(operation: &str) -> Result<operations::TransferOperation, CoreError> {
    match operation {
        "copy" => Ok(operations::TransferOperation::Copy),
        "move" => Ok(operations::TransferOperation::Move),
        _ => Err(CoreError::Config(format!(
            "invalid transfer operation: {}",
            operation
        ))),
    }
}

fn parse_transfer_conflict_policy(
    conflict_policy: &str,
) -> Result<operations::TransferConflictPolicy, CoreError> {
    match conflict_policy {
        "fail" => Ok(operations::TransferConflictPolicy::Fail),
        "overwrite" => Ok(operations::TransferConflictPolicy::Overwrite),
        "skip" | "discard" => Ok(operations::TransferConflictPolicy::Skip),
        "rename" | "keep_both" => Ok(operations::TransferConflictPolicy::Rename),
        _ => Err(CoreError::Config(format!(
            "invalid transfer conflict policy: {}",
            conflict_policy
        ))),
    }
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn plan_transfer_entries(
    state: State<'_, AppState>,
    fromSourceId: String,
    toSourceId: String,
    paths: Vec<String>,
    targetDir: String,
    operation: String,
    conflictPolicy: String,
    jobId: Option<String>,
) -> Result<operations::TransferPlan, CoreError> {
    let from_op = state.operator_for_storage_id(&fromSourceId)?;
    let to_op = state.operator_for_storage_id(&toSourceId)?;
    reject_namespace_conflicts(&state, &fromSourceId, &toSourceId, &paths, &targetDir)?;
    if let Some(job_id) = jobId {
        let cancel_job_id = job_id.clone();
        let result = operations::plan_transfer_entries_cancellable(
            &from_op,
            &to_op,
            paths,
            &targetDir,
            parse_transfer_operation(&operation)?,
            fromSourceId == toSourceId,
            parse_transfer_conflict_policy(&conflictPolicy)?,
            || state.is_transfer_cancelled(&cancel_job_id),
        )
        .await;
        // Do not clear a cancellation marker here: transfer execution checks it before
        // starting, preventing a race between plan completion and the next command.
        result
    } else {
        operations::plan_transfer_entries(
            &from_op,
            &to_op,
            paths,
            &targetDir,
            parse_transfer_operation(&operation)?,
            fromSourceId == toSourceId,
            parse_transfer_conflict_policy(&conflictPolicy)?,
        )
        .await
    }
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn transfer_entries(
    app: AppHandle,
    state: State<'_, AppState>,
    fromSourceId: String,
    toSourceId: String,
    paths: Vec<String>,
    targetDir: String,
    operation: String,
    conflictPolicy: String,
    jobId: Option<String>,
) -> Result<(), CoreError> {
    let from_op = state.operator_for_storage_id(&fromSourceId)?;
    let to_op = state.operator_for_storage_id(&toSourceId)?;
    reject_namespace_conflicts(&state, &fromSourceId, &toSourceId, &paths, &targetDir)?;

    let op = parse_transfer_operation(&operation)?;
    let policy = parse_transfer_conflict_policy(&conflictPolicy)?;

    if let Some(job_id) = jobId {
        let emit_app = app.clone();
        let emit_job_id = job_id.clone();
        let cancel_job_id = job_id.clone();
        let result = operations::transfer_entries_with_progress(
            &from_op,
            &to_op,
            paths,
            &targetDir,
            op,
            fromSourceId == toSourceId,
            policy,
            move |progress| {
                let _ = emit_app.emit(
                    "infimount://transfer-progress",
                    TransferProgressPayload {
                        job_id: emit_job_id.clone(),
                        completed_items: progress.completed_items,
                        total_items: progress.total_items,
                        bytes_transferred: progress.bytes_transferred,
                        total_bytes: progress.total_bytes,
                        current_path: progress.current_path,
                    },
                );
            },
            || state.is_transfer_cancelled(&cancel_job_id),
        )
        .await;
        state.clear_transfer_cancel(&job_id);
        result
    } else {
        operations::transfer_entries(
            &from_op,
            &to_op,
            paths,
            &targetDir,
            op,
            fromSourceId == toSourceId,
            policy,
        )
        .await
    }
}

#[tauri::command]
pub fn cancel_transfer_job(state: State<'_, AppState>, jobId: String) {
    state.request_transfer_cancel(&jobId);
}
