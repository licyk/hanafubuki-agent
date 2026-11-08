use super::*;

/// Returns true while a pid still refers to a live (non-reaped) process.
fn pid_is_alive(pid: i32) -> bool {
    // signal 0 performs error checking without sending a signal: 0 / EPERM
    // mean the process exists, ESRCH means it is gone.
    unsafe {
        libc::kill(pid, 0) == 0
            || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
}

/// A PTY shell (session/group leader, via portable-pty's setsid) backgrounds a
/// descendant `sleep`; terminating the group must kill that descendant, not
/// just the shell.
#[test]
fn terminate_group_kills_descendant_process_tree() {
    use portable_pty::{native_pty_system, CommandBuilder, PtySize};
    use std::io::Read;
    use std::sync::mpsc;

    fn parse_child_pid(bytes: &[u8]) -> Option<i32> {
        let text = String::from_utf8_lossy(bytes);
        text.lines()
            .find_map(|line| line.trim().strip_prefix("CHILD:"))
            .and_then(|value| value.trim().parse().ok())
    }

    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("openpty");
    let mut cmd = CommandBuilder::new("/bin/sh");
    cmd.arg("-c");
    cmd.arg("sleep 30 & printf 'CHILD:%s\\n' \"$!\"; wait");
    let mut child = pair.slave.spawn_command(cmd).expect("spawn");
    drop(pair.slave);
    let leader = child.process_id().expect("leader pid");

    // Read the descendant pid printed by the shell from the PTY output.
    let mut reader = pair.master.try_clone_reader().expect("reader");
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut buf = [0u8; 1024];
        let mut acc = Vec::new();
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    acc.extend_from_slice(&buf[..n]);
                    if acc.contains(&b'\n') {
                        if let Some(pid) = parse_child_pid(&acc) {
                            let _ = tx.send(pid);
                            break;
                        }
                    }
                }
                Err(_) => break,
            }
        }
    });
    let child_pid: i32 = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("did not capture descendant pid");

    // Force the full graceful sequence (never break early) so SIGKILL sweeps
    // the whole group.
    terminate_group(leader, || false).unwrap();
    let _ = child.wait();

    let deadline = Instant::now() + Duration::from_secs(3);
    while pid_is_alive(child_pid) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    assert!(
        !pid_is_alive(child_pid),
        "descendant pid {child_pid} still alive after terminate_group",
    );
    drop(pair.master);
}

/// When `is_done()` is already true up front, `terminate_group` must send no
/// signals: a live process in that group stays alive.
#[test]
fn terminate_group_skips_signals_when_already_done() {
    use portable_pty::{native_pty_system, CommandBuilder, PtySize};

    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("openpty");
    let mut cmd = CommandBuilder::new("/bin/sh");
    cmd.arg("-c");
    cmd.arg("sleep 5");
    let mut child = pair.slave.spawn_command(cmd).expect("spawn");
    drop(pair.slave);
    let pgid = child.process_id().expect("pid");

    // is_done() == true up front => no SIGTERM/SIGKILL should be sent.
    terminate_group(pgid, || true).unwrap();

    thread::sleep(Duration::from_millis(150));
    assert!(
        child.try_wait().unwrap().is_none(),
        "process was killed despite is_done() being true up front",
    );

    // Clean up the still-running child.
    let _ = child.clone_killer().kill();
    let _ = child.wait();
    drop(pair.master);
}
