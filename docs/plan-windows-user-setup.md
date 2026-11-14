# Windows setup from an ordinary user session

Baseline: 6335f336. User reproduced `read sandbox network block ... Windows error 5`
after approving UAC; starting the Agent as administrator succeeds.

The elevated installer previously created WFP filters with default security.
The ordinary caller could open the engine but lacked FWPM_ACTRL_READ on filters.
Pass the initiating user's SID across UAC and create filters with an explicit
security descriptor: Administrators own/manage them, SYSTEM manages them, and
the requesting user receives only FWPM_ACTRL_READ. Repeated setup replaces old
filters, so users can repair the original installation without deleting accounts.

Native tests remove administrator group access and privileges from the caller
before checking the real readiness API, then require deletion and DACL changes
to fail. This covers post-UAC verification, not an interactive desktop UAC prompt.
Existing native filesystem/network tests still run. Only the sandbox workflow
runs on test/sandbox-user-setup; no full project tests requested.

Validation: commit 9c5a884c passed Sandbox platforms run 43632035427 on Linux,
Windows and macOS. The Windows permission checks passed after both initial setup
and repair. Windows cross-target clippy, cargo fmt --all --check, cargo xtask check
and git diff --check also passed. No full-workspace tests ran.

https://github.com/licyk/hanafubuki-agent/actions/runs/43632035427

After updating, rerun /setup-default-sandbox from the ordinary user session and
approve UAC once to replace the old filters. Interactive desktop UAC remains a
real-device acceptance step; CI explicitly verifies the subsequent non-admin
access check rather than claiming to automate that prompt.
