---
name: skill-creator
description: Write or revise an Agent Skill when the user wants a reusable workflow, a domain guide, or a packaged set of references and scripts.
license: Apache-2.0
allowed-tools:
  - skills_list
  - skills_read
  - list_directory
  - read_file
  - apply_patch
  - delete_path
metadata:
  version: 3.0.0
---

# Skill Creator

A Skill is a directory holding a `SKILL.md`. Its YAML frontmatter carries a
`description` — the sentence that decides when the Skill is used — and
optionally a `name`. Everything else in the directory is supporting material.

That is the whole format, and it is the same one Claude and Codex read, so a
Skill written here installs there and a package downloaded from either installs
here.

## When to use this

The user wants to capture a workflow they will repeat, a house style, a domain
guide, or a set of references and helper scripts that belong together.

## How to do it

1. Ask what the Skill is for and for two or three requests that should trigger
   it. The description is written from those, not from a guess.
2. Call `skills_list` and use its `user_root` as the installation root, even
   when no user Skills are listed. Each entry's `directory` is the full native
   path of that Skill, which may resolve a symbolic link; it is not a root or
   a relative path. Never infer a root by taking its parent or stripping the
   relative path from its id. System, repo and plugin roots are independent.
   Standalone defaults to `<data-directory>/skills/packages`, with bundled
   Skills separately in `<data-directory>/skills/system`; embedded hosts may
   configure different roots. The enclosing `skills` directory is not the
   installation destination. If the tool cannot report `user_root`, ask for
   the configured user root.
3. Create `<user-root>/<name>/SKILL.md` with `apply_patch`. Open with the
   frontmatter and keep the body short and procedural: when to use this, the
   steps, and what to avoid.

   ```markdown
   ---
   name: release-notes
   description: Write release notes from a changelog. Use when the user asks for a release summary.
   ---

   # Release Notes

   ## When to use this
   ...
   ```

4. Put long supporting material in `references/`, runnable helpers in
   `scripts/`, and templates or samples in `assets/`. Reference them from
   `SKILL.md` by their package-relative path so a reader knows what to open.
5. Read the written files back and check that the description alone tells you
   when to reach for the Skill. On the next user turn, confirm discovery with
   `skills_list` and read it with `skills_read`; these tools use the current
   turn's frozen catalog and cannot see a Skill just created during that turn.

## Notes

- A new Skill is on as soon as its `SKILL.md` parses; there is nothing to
  publish and no catalog to register with. The catalog rescans on its own.
- A Skill outside the user root — one discovered in a repository or plugin, or one
  shipped with the application — is not yours to edit. Copy the directory and
  edit the copy.
- Never put secrets, credentials, or install hooks in a Skill.
