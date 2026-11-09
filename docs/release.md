# Releasing

A release is a `v<version>` tag on `main`. The tag starts
[`.github/workflows/release.yml`](../.github/workflows/release.yml), which
publishes every crate but `xtask` to crates.io, builds the `hanafubuki-agent`
binary for four targets and creates the GitHub release with the archives.

## Checklist

1. **Bump the version** (skip for `v0.1.0`, the version the workspace
   already has):

   ```sh
   cargo xtask version patch        # or minor, major; --dry-run prints the next version
   git commit -am "chore(release): hanafubuki-agent 0.1.1"
   git push
   ```

   `version` raises `[workspace.package].version` and every internal
   requirement in `[workspace.dependencies]`, then refreshes `Cargo.lock`.

2. **Wait for CI to pass** on the pushed commit on all three systems. The
   release reuses that run rather than testing again (and runs `ci.yml`
   itself when the commit has no usable run, for instance a docs-only
   commit CI skipped).

3. **Tag and push:**

   ```sh
   cargo xtask tag --dry-run        # prints the tag it would make: v<version>
   cargo xtask tag --push
   ```

   `tag` makes the annotated tag on a clean `HEAD` and pushes it.

4. **Watch the release:** `gh run watch -R licyk/hanafubuki-agent` (or the
   Actions tab). The jobs:

   | Job | What it does |
   | --- | --- |
   | `plan` | Reads the workspace version; fails unless the run is on the tag that names it. |
   | `ci-result` | Finds CI's run of the commit and waits for it; a failed run stops the release. |
   | `checks` | Runs `ci.yml` when `ci-result` found no usable run. |
   | `publish` | `cargo publish --workspace`, in dependency order, skipping versions already on crates.io and waiting out crates.io's rate limit. |
   | `licenses` | Generates `THIRD-PARTY-LICENSES.txt` with `cargo-about` (`about.toml`, `about.hbs`). |
   | `binaries` | Builds `hanafubuki-agent` with `--release --locked` for `x86_64-unknown-linux-gnu`, `x86_64-apple-darwin`, `aarch64-apple-darwin` and `x86_64-pc-windows-msvc`, and packages `hanafubuki-agent-<version>-<target>.tar.gz` (`.zip` on Windows) with the binary, LICENSE, NOTICE and the third-party licenses, and its `.sha256`. |
   | `github-release` | Creates the release with notes generated from the previous `v*` tag, and uploads the archives and their sums. |

5. **Check the result:** the crates on crates.io
   (`cargo install hanafubuki-agent-cli` works), and the release's eight
   files.

## The first release and crates.io's rate limit

crates.io lets an account create a few new crates at once and then one
every ten minutes. `v0.1.0` creates all 24 crates, so `publish` publishes the
first few, then one crate every ten minutes or so: about three and a half
hours in all. The job retries up to 30 times and is allowed 330 minutes;
`github-release` waits for it. Later releases only add versions of existing
crates, which crates.io limits far less, and take minutes.

## Resuming a release that stopped

A run that stopped part-way (the publish timed out or failed, a binary did
not build, the runner went away) is resumed with a manual run **on the tag**:

```sh
gh workflow run release.yml -R licyk/hanafubuki-agent --ref v0.1.0
```

Every job is safe to repeat: `publish` skips each crate version already on
crates.io, the binaries are built again, and `github-release` uploads the
archives over an existing release's (`--clobber`) instead of creating it
again. A manual run on a branch stops at `plan` and publishes nothing.

If a fix is needed in the code, a published version cannot be replaced:
fix it on `main`, bump the version, and release the new one.

## Secrets

- `CARGO_REGISTRY_TOKEN`: a crates.io token with the `publish-new` and
  `publish-update` scopes, set as a repository secret. Only `publish` sees
  it.
- `GITHUB_TOKEN`: the workflow's own token, with `contents: write` only on
  `github-release`.
