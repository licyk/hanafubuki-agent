use std::ffi::OsString;
use std::path::PathBuf;

use super::HelperRequest;

fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

#[test]
fn the_command_line_reads_back_as_written() {
    let request = HelperRequest {
        writable_roots: vec![PathBuf::from("/work"), PathBuf::from("/tmp")],
        network_access: true,
        command: args(&["/bin/sh", "-c", "echo --writable-root"]),
    };
    let written = request.to_args().unwrap();
    assert_eq!(
        written,
        [
            "--allow-network",
            "--writable-root",
            "/work",
            "--writable-root",
            "/tmp",
            "--",
            "/bin/sh",
            "-c",
            "echo --writable-root"
        ]
    );
    let read = HelperRequest::parse(written.into_iter().map(OsString::from).collect()).unwrap();
    assert_eq!(read, request);
}

#[test]
fn anything_but_its_own_command_line_is_refused() {
    for refused in [
        args(&[]),
        args(&["--"]),
        args(&["--writable-root"]),
        args(&["--network", "--", "true"]),
        args(&["true"]),
    ] {
        assert!(
            HelperRequest::parse(refused.clone()).is_err(),
            "{refused:?}"
        );
    }
}
