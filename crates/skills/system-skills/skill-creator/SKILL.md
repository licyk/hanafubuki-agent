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
2. Find the managed Skill root: call `skills_list`, take any Skill whose
   authority is `user`, and read the `directory` it reports. Its parent is the
   root. If there is no user Skill yet, ask the user where their Skills live.
3. Create the directory and write `SKILL.md` with `apply_patch`. Open with the
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
5. Read the result back with `skills_read` and check two things: that the
   catalog picked it up, and that the description alone would tell you when to
   reach for it.

## Notes

- A new Skill is on as soon as its `SKILL.md` parses; there is nothing to
  publish and no catalog to register with. The catalog rescans on its own.
- A Skill outside the managed root — one discovered in a repository, or one
  shipped with the application — is not yours to edit. Copy the directory and
  edit the copy.
- Never put secrets, credentials, or install hooks in a Skill.
