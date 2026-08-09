//! Fuzz seams for `kasane-core`.
//!
//! A test seam, not API — the same convention and the same rationale as
//! `kasane-adapters`'s module of this name: it lives inside the crate so it
//! can reach `pub(crate)` internals (`slug::path_slug`, `slug::anchor_slug`)
//! that the separate `fuzz/` workspace cannot.
//!
//! Each function takes `&[u8]` and either returns or panics. A panic **is**
//! the finding. That uniformity is what lets
//! `kasane-adapters/tests/fuzz_corpus.rs` dispatch by directory name and keeps
//! every libFuzzer wrapper identical.

use crate::slug::{anchor_slug, path_slug, MAX_PATH_SLUG_BYTES};
use kasane_ir::Inline;

/// `path_slug`'s postconditions, which are security-critical because this is
/// where untrusted adapter text becomes a filename.
///
/// The confinement argument is by construction -- `/`, `\`, `.`, NUL, the
/// fullwidth solidus and the RTL override are all outside `\p{Word}` and are
/// removed -- so this target exists to make that argument fail loudly if the
/// character class is ever widened by hand.
///
/// `anchor_slug`'s class is deliberately wider than `path_slug`'s (it keeps
/// Join_Control, which GitHub keeps), and that is exactly why the widening has
/// to be asserted here rather than reasoned about: the path alphabet must stay
/// closed even as the anchor alphabet grows.
pub fn slug(data: &[u8]) {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let inlines = [Inline::Text(text.to_string())];

    let path = path_slug(&inlines);
    assert!(
        !path.contains('/') && !path.contains('\\'),
        "path_slug emitted a separator: {path:?} from {text:?}"
    );
    assert!(
        !path.split('-').any(|s| s == ".." || s == "."),
        "path_slug emitted a traversal component: {path:?} from {text:?}"
    );
    assert!(
        !path.contains('.'),
        "path_slug emitted a dot: {path:?} from {text:?}"
    );
    assert!(
        !path.is_empty(),
        "path_slug emitted an empty name from {text:?}"
    );
    assert!(
        path.len() <= MAX_PATH_SLUG_BYTES,
        "path_slug exceeded the byte cap: {} bytes from {text:?}",
        path.len()
    );

    // Anchors are uncapped by design, but an empty one is a dead link.
    let anchor = anchor_slug(&inlines);
    assert!(
        !anchor.is_empty(),
        "anchor_slug emitted an empty anchor from {text:?}"
    );
}
