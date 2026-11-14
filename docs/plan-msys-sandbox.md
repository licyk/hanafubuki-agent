# MSYS sandbox compatibility

Base: 6335f336; reference: local Codex 57a808a392.

MSYS signal pipes use account-owned kernel objects. Match Codex's elevated token:
include the dedicated account SID, while ordinary filesystem grants use a separate
sandbox group and restricted filesystem grants still use per-root capability SIDs.
Never add the signed-in user's SID to the unelevated token.

The initial elevated accounts received direct write grants. Reusing them would
make those grants reachable through the new token, so setup creates fresh accounts
using secrets/group-accounts.json. Previous credentials, accounts and firewall
rules remain untouched. Users must run /setup-default-sandbox again.

Native tests cover Git for Windows Bash and MSYS2 Bash: startup, pipelines/fork,
workspace writes, outside-root denials, protected metadata and network on/off;
also switching workspaces and read-only after earlier writes. Use only the sandbox
workflow on test/sandbox-msys. ConPTY and terminal resizing remain separate work.

Validation: Windows cross-target clippy passed; native CI failed as described below.

## Native result: unresolved isolation regression

Run 43629976609 (commit 3fd2a733) passed Git for Windows and MSYS2 startup,
pipes/fork, current-workspace writes, protected metadata and network on/off.
It FAILED the prior-workspace check: after creating a file in workspace A,
a command scoped to workspace B could still overwrite that file. The account
SID restriction admits access to account-owned files as well as kernel objects.
Changing ordinary grants to a group alone does not preserve the root boundary.

https://github.com/licyk/hanafubuki-agent/actions/runs/43629976609

The user explicitly requested merging this implementation into main with the
known limitation documented. The ordinary-user WFP setup fix is retained.
Public documentation describes the account-owned-file write regression and
its effect on every shell using the elevated backend, not just MSYS. The failing
regression assertions remain enabled. This merge does not fix that boundary or
establish full native sandbox acceptance.

Local cargo fmt --all --check, Windows cross-target clippy and cargo xtask check
passed. No full-workspace tests ran. Additional MSYS-specific tests now also
check writes to earlier workspace files and read-only mode.
