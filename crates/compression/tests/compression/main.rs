// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// the contract tests of `kernel/tests` of acp-kernel 0.0.189 at commit
// `4be21b08e`. Copyright (c) 2026 ranxianglei. MIT License with an
// additional attribution term (NOTICE).

//! The kernel's contracts for the compression core, ported test by test
//! (each module names the kernel test files it ports).

mod apply;
mod cache_report;
mod fixtures;
mod hide;
mod integrity;
mod latch;
mod merge;
mod nudge;
mod nudge_text;
mod pairs;
mod parse;
mod parse_line_form;
mod prompts;
mod protect;
mod prune;
mod recommend;
mod refold;
mod refs;
mod render;
mod report;
mod truncate;
mod turn;
