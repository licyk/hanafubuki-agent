// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// the contract tests of `kernel/tests` of acp-kernel 0.0.189 at commit
// `4be21b08e`. Copyright (c) 2026 ranxianglei. MIT License with an
// additional attribution term (NOTICE).

//! The kernel's contracts for the compression core, ported test by test
//! (each module names the kernel test files it ports).

mod apply;
mod cache_report;
mod decompress;
mod fixtures;
mod fork;
mod hide;
mod integrity;
mod latch;
mod media;
mod merge;
mod nudge;
mod nudge_text;
mod opt_in;
mod pairs;
mod parse;
mod parse_line_form;
mod prompts;
mod protect;
mod prune;
mod rebuild;
mod recommend;
mod refold;
mod refs;
mod render;
mod report;
mod search;
mod timestamps;
mod truncate;
mod turn;
