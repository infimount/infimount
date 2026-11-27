//! Additional conservative alias checks for portable local policy enforcement.
//! Storage I/O remains in OpenDAL. These checks establish path spelling only.
use std::path::Path;

use infimount_mcp::StorageRecord;
use unicode_normalization::UnicodeNormalization;

use crate::error::{Error, Result};

const MAX_DIRECTORY_NAMES: usize = 16_384;

fn folded(path: &str) -> String {
    path.nfd().flat_map(char::to_uppercase).collect()
}

fn inside(path: &str, prefix: &str) -> bool {
    path == prefix
        || path
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with('/'))
}

pub fn verify_spelling(record: &StorageRecord, relative: &str) -> Result<()> {
    if record.backend != "local" {
        return Ok(());
    }
    let folded_path = folded(relative);
    // Never turn a near match into a grant. Reject ambiguous aliases of both
    // grants and exclusions, even on a case-sensitive volume. This prevents a
    // future case/decomposition variant from bypassing a not-yet-created deny.
    for prefix in record
        .mcp_policy
        .denied_paths
        .iter()
        .chain(record.mcp_policy.rules.iter().map(|rule| &rule.prefix))
    {
        if !inside(relative, prefix) && inside(&folded_path, &folded(prefix)) {
            return Err(Error::PermissionDenied);
        }
    }
    let root = record
        .config
        .get("root")
        .and_then(serde_json::Value::as_str)
        .ok_or(Error::Configuration)?;
    let mut parent = Path::new(root).to_path_buf();
    for segment in relative.split('/').filter(|segment| !segment.is_empty()) {
        if segment.ends_with(['.', ' ']) {
            return Err(Error::InvalidRequest);
        }
        #[cfg(windows)]
        {
            let stem = segment
                .split('.')
                .next()
                .unwrap_or_default()
                .to_ascii_uppercase();
            if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
                || stem
                    .strip_prefix("COM")
                    .or_else(|| stem.strip_prefix("LPT"))
                    .is_some_and(|number| {
                        matches!(number, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
                    })
            {
                return Err(Error::InvalidRequest);
            }
        }
        let entries = match std::fs::read_dir(&parent) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(_) => return Err(Error::PermissionDenied),
        };
        let mut exact = false;
        for (index, entry) in entries.enumerate() {
            if index >= MAX_DIRECTORY_NAMES {
                return Err(Error::LimitExceeded);
            }
            if entry.map_err(|_| Error::Unavailable)?.file_name() == segment {
                exact = true;
                break;
            }
        }
        parent.push(segment);
        if !exact {
            return match std::fs::symlink_metadata(&parent) {
                // The OS found something not present under this exact spelling,
                // such as a case alias or Windows short filename.
                Ok(_) => Err(Error::PermissionDenied),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(_) => Err(Error::PermissionDenied),
            };
        }
    }
    Ok(())
}
