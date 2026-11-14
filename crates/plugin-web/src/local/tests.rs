// Ported from dsh-free-search 6ab5168 (MIT); adapted to hanafubuki-agent.
//
// The MIT License (MIT)
//
// Copyright (c) 2026 dsh-free-search contributors
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.

use super::*;
use hanafubuki_agent_process::executor::manager::executor_registry_counts;
#[cfg(unix)]
#[tokio::test]
async fn helper_cancel_and_future_drop_release_owned_processes() {
    use std::os::unix::fs::PermissionsExt;
    let directory = std::env::temp_dir().join(format!("hana-local-test-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let script = directory.join("agy");
    std::fs::write(&script, "#!/bin/sh\nexec sleep 30\n").unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    let previous = std::env::var_os("AGY_BIN");
    std::env::set_var("AGY_BIN", &script);
    let executor = ExecutorManager::default();
    let runner = OwnedWebRunner {
        executor: executor.clone(),
    };
    for cancel_turn in [true, false] {
        let cancel = CancellationToken::default();
        let run = runner.run(
            LocalWebRequest::Agy {
                query: "owned process fixture".into(),
                maximum: 1,
                model: "fixture".into(),
            },
            cancel.clone(),
        );
        tokio::pin!(run);
        tokio::select! {result=&mut run=>panic!("unexpected early completion: {result:?}"),_ = tokio::time::sleep(Duration::from_millis(120))=>{}}
        assert_ne!(executor_registry_counts(&executor), (0, 0, 0));
        if cancel_turn {
            cancel.cancel();
            assert!(matches!(run.await, Err(WebSearchError::Cancelled)));
        }
        // The second iteration drops the future without cancelling its token.
    }
    assert_eq!(executor_registry_counts(&executor), (0, 0, 0));
    if let Some(value) = previous {
        std::env::set_var("AGY_BIN", value);
    } else {
        std::env::remove_var("AGY_BIN");
    }
    std::fs::remove_dir_all(directory).unwrap();
}
