use crate::{AgentMemoryContributionMode, AgentMemoryError, MemoryLease};
use rusqlite::{params, OptionalExtension};

pub(super) fn ensure_current_lease(
    transaction: &rusqlite::Transaction<'_>,
    lease: &MemoryLease,
    now_millis: u64,
    kind: &str,
) -> Result<(), AgentMemoryError> {
    let current: Option<i64> = transaction
        .query_row(
            "SELECT 1 FROM memory_jobs WHERE kind=?1 AND scope=?2 AND lease_token=?3
             AND reset_epoch=?4 AND expires_at>?5",
            params![
                kind,
                lease.scope,
                lease.token,
                to_i64(lease.reset_epoch)?,
                to_i64(now_millis)?
            ],
            |row| row.get(0),
        )
        .optional()
        .map_err(db_error)?;
    current.ok_or(AgentMemoryError::LeaseLost).map(|_| ())
}

/// How long a failed job's scope waits before it is claimed again, after
/// `retry_count` earlier failures: 30 s, doubling, at most a day.
pub(super) fn retry_backoff_millis(retry_count: i64) -> u64 {
    let exponent = u32::try_from(retry_count.clamp(0, 10)).unwrap_or(10);
    30_000u64
        .saturating_mul(1u64 << exponent)
        .min(24 * 60 * 60 * 1000)
}

pub(super) fn validate_id(value: &str, field: &str) -> Result<(), AgentMemoryError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(AgentMemoryError::InvalidInput(format!("invalid {field}")));
    }
    Ok(())
}

pub(super) fn validate_text(
    value: &str,
    maximum: usize,
    field: &str,
) -> Result<(), AgentMemoryError> {
    if value.len() > maximum || value.contains('\0') {
        return Err(AgentMemoryError::InvalidInput(format!(
            "{field} exceeds its bound"
        )));
    }
    Ok(())
}

/// A mode this build does not know contributes nothing; the kernel's
/// history resynchronizes it at the next start.
pub(super) fn parse_mode(value: &str) -> AgentMemoryContributionMode {
    match value {
        "enabled" => AgentMemoryContributionMode::Enabled,
        "polluted" => AgentMemoryContributionMode::Polluted,
        "disabled" => AgentMemoryContributionMode::Disabled,
        other => {
            log::warn!("unknown Agent memory contribution mode {other:?} read as disabled");
            AgentMemoryContributionMode::Disabled
        }
    }
}

pub(super) fn to_i64(value: u64) -> Result<i64, AgentMemoryError> {
    i64::try_from(value).map_err(|_| AgentMemoryError::InvalidInput("integer overflow".into()))
}

pub(super) fn from_i64(value: i64) -> Result<u64, AgentMemoryError> {
    u64::try_from(value).map_err(|_| AgentMemoryError::Corrupt("negative integer".into()))
}

/// Truncate on a character boundary, keeping the leading bytes.
///
/// A read bound is a context budget, not a validity rule: content past it is
/// dropped, never turned into an error that costs the caller everything.
pub(super) fn bound_text(value: &str, maximum: usize) -> String {
    if value.len() <= maximum {
        return value.to_string();
    }
    let mut end = maximum;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}

pub(super) fn bound_string(value: &str, maximum: usize) -> String {
    if value.len() <= maximum {
        return value.to_string();
    }
    let mut end = maximum;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &value[..end])
}

/// A database that is not one, or is damaged, is `Corrupt`; every other
/// failure (busy, locked, I/O) is `Io` and leaves the database as it is.
pub(super) fn db_error(error: rusqlite::Error) -> AgentMemoryError {
    match error.sqlite_error_code() {
        Some(rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase) => {
            AgentMemoryError::Corrupt(error.to_string())
        }
        _ => AgentMemoryError::Io(error.to_string()),
    }
}

/// An error of the schema set-up: besides what [`db_error`] calls corrupt,
/// an older schema that the current statements do not fit (`SQLITE_ERROR`,
/// such as a missing column) or a value of the wrong type is an incompatible
/// database, `Corrupt` too.
pub(super) fn schema_error(error: rusqlite::Error) -> AgentMemoryError {
    match &error {
        rusqlite::Error::SqliteFailure(failure, _)
            if failure.code == rusqlite::ErrorCode::Unknown =>
        {
            AgentMemoryError::Corrupt(format!("incompatible schema: {error}"))
        }
        rusqlite::Error::SqliteFailure(..) => db_error(error),
        _ => AgentMemoryError::Corrupt(format!("incompatible schema: {error}")),
    }
}

pub(super) fn json_error(error: serde_json::Error) -> AgentMemoryError {
    AgentMemoryError::InvalidInput(error.to_string())
}
