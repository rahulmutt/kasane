//! GitHub-Flavored Markdown text semantics, shared by `kasane-core` and
//! `kasane-writer`.
//!
//! A leaf crate over `kasane-ir`. It exists because two crates have to agree
//! on one thing — what a heading line renders to — and neither can own it:
//! `kasane-core` computes a heading's anchor at structuring time, and
//! `kasane-writer` emits the line that anchor has to match. Before this crate
//! the agreement was two functions kept in step by hand.
//!
//! `slug` holds both slug rules and the character class they share.

#[doc(hidden)]
pub mod fuzz_entry;
mod slug;

pub use slug::{
    anchor_slug_of, anchors_for_headings, inline_text, path_slug, path_slug_of, AnchorCounter,
};
