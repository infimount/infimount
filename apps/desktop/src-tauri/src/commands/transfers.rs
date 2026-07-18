#![allow(non_snake_case)]

use infimount_core::{operations, CoreError};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::state::AppState;

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
) -> Result<operations::TransferPlan, CoreError> {
    let from_op = state.operator_for_storage_id(&fromSourceId)?;
    let to_op = state.operator_for_storage_id(&toSourceId)?;
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

    let op = parse_transfer_operation(&operation)?;
    let policy = parse_transfer_conflict_policy(&conflictPolicy)?;

    if let Some(job_id) = jobId {
        state.clear_transfer_cancel(&job_id);
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
