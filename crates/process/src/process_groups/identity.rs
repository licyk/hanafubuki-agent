//! The identity of a process group's leader, which tells a recorded group
//! from a later process that reuses its id.

/// On Linux, the boot id and the leader's start time (clock ticks after
/// boot, `/proc/<pid>/stat` field 22), when `pgid` is still a running
/// process that leads its own group. `None` when it is not, or cannot be
/// read.
#[cfg(target_os = "linux")]
pub(super) fn leader_identity(pgid: u32) -> Option<String> {
    let stat = std::fs::read_to_string(format!("/proc/{pgid}/stat")).ok()?;
    // The command name is in parentheses and may contain anything; the
    // fields after its last `)` start with field 3 (state).
    let fields = stat
        .get(stat.rfind(')')? + 1..)?
        .split_whitespace()
        .collect::<Vec<_>>();
    let group: u32 = fields.get(5 - 3)?.parse().ok()?;
    if group != pgid {
        return None;
    }
    let started: u64 = fields.get(22 - 3)?.parse().ok()?;
    let boot = std::fs::read_to_string("/proc/sys/kernel/random/boot_id").ok()?;
    Some(format!("{}:{started}", boot.trim()))
}

/// No identity is read here (macOS has one through `proc_pidinfo`, not wired
/// yet), so nothing is recorded and nothing is reaped.
#[cfg(not(target_os = "linux"))]
pub(super) fn leader_identity(_pgid: u32) -> Option<String> {
    None
}
