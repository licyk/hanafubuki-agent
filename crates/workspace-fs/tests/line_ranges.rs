use hanafubuki_agent_foundation::cancellation::CancellationToken;
use hanafubuki_agent_workspace_fs::{
    ReadFileRequest, ReadFileStart, WorkspaceFsError, WorkspaceFsPolicy, WorkspaceFsRequest,
    WorkspaceFsResult, WorkspaceFsService,
};
use std::fs;
use tempfile::TempDir;

#[test]
fn read_file_pages_only_the_requested_lines() {
    let temp = TempDir::new().unwrap();
    let prefix = "a".repeat(70_000) + "\n";
    fs::write(temp.path().join("lines"), format!("{prefix}中日\r\n\nlast")).unwrap();
    let service = WorkspaceFsService::new(WorkspaceFsPolicy::rooted_at(temp.path(), []).unwrap());
    let read = |start, end_line, max_bytes| {
        let request = WorkspaceFsRequest::ReadFile(ReadFileRequest {
            path: "lines".into(),
            start,
            end_line,
            max_bytes,
            line_numbers: true,
        });
        let WorkspaceFsResult::FileRead(page) =
            service.execute(&request, None, &CancellationToken::default())?
        else {
            panic!("a page")
        };
        Ok::<_, WorkspaceFsError>(page)
    };
    let first = read(ReadFileStart::Line(2), Some(3), 4).unwrap();
    assert_eq!((first.offset, first.content.as_str()), (prefix.len(), "中"));
    assert!(first.truncated);
    let rest = read(ReadFileStart::Offset(first.next_offset), Some(3), 6).unwrap();
    assert_eq!(rest.content, "日\r\n\n");
    assert_eq!(rest.lines.last().unwrap().number, 3);
    assert!(!rest.truncated);
    let last = read(ReadFileStart::Line(4), Some(10), 32).unwrap();
    assert_eq!(last.content, "last");
    assert!(!last.truncated);
    let past = read(ReadFileStart::Line(10), None, 32).unwrap();
    assert!(past.content.is_empty() && past.lines.is_empty() && !past.truncated);
    assert!(matches!(
        read(ReadFileStart::Line(3), Some(2), 32),
        Err(WorkspaceFsError::InvalidRequest(_))
    ));
}
