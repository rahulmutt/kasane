//! The rendered-line vocabulary: the newline fold, and the two projections of
//! an inline run to text.

use kasane_ir::Inline;

/// Fold every newline spelling to a single space, **collapsing runs**, for the
/// contexts where a newline is structurally impossible: heading lines, link
/// labels, image alt text, code spans, YAML scalars.
///
/// One fold for two crates, and the collapse is what makes that possible. The
/// writer's two heading paths reach it from opposite directions —
/// `Block::Heading` escapes first and folds after, `file_to_markdown`'s title
/// heading folds first and escapes after — so without the collapse they
/// disagreed on a blank line (`## A B` against `# A  B`) and the anchor, which
/// can only predict one rendered line, was dead for whichever path it did not
/// predict.
///
/// Literal spaces are a different mechanism and are deliberately *not*
/// collapsed: `Background & Notes` still anchors `background--notes`. Tabs are
/// untouched too — a tab survives into the rendered line, where the anchor
/// filter drops it, since a tab is in neither `\p{Word}`, `-`, nor space.
pub fn fold_newlines(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_was_newline = false;
    for c in s.chars() {
        if c == '\n' || c == '\r' {
            if !last_was_newline {
                out.push(' ');
            }
            last_was_newline = true;
        } else {
            out.push(c);
            last_was_newline = false;
        }
    }
    out
}

/// The visible text of an inline run for a **navigation surface**: a file's
/// frontmatter title, a breadcrumb entry, a TOC link label, a library index
/// row, and the plain-text fallback `refs` leaves where a link was stripped.
///
/// Skips `Inline::FootnoteRef`, and that is the point rather than an
/// approximation of [`rendered_text`]. A `[^1]` in any of those surfaces
/// renders a footnote reference pointing at a definition that, after a
/// `balance` split, is likely in another file — a dangling marker in
/// `index.md` is worse than an absent one in a title.
pub fn title_text(inlines: &[Inline]) -> String {
    let mut s = String::new();
    walk(inlines, 0, false, &mut s);
    s
}

/// The text the writer's rendering of this run renders back to.
///
/// This is what a heading's anchor is computed from, because GitHub computes
/// an id from the rendered line. It differs from [`title_text`] in exactly one
/// arm: `Inline::FootnoteRef(n)` contributes `[^n]`, which the writer emits as
/// visible text.
///
/// Correct whether or not the reference resolves. GitHub renders a resolved
/// one as a superscript `1` and leaves an unresolved one as the literal
/// `[^1]`; the id filter removes `[`, `^` and `]`, so both land on the same
/// digits. Nothing here has to know which happened — which matters, because
/// after a `balance` split the definition may be in a different file.
pub fn rendered_text(inlines: &[Inline]) -> String {
    let mut s = String::new();
    walk(inlines, 0, true, &mut s);
    s
}

/// One walk for both projections, so the single arm they differ on is visible
/// in one place. Bounded by `MAX_INLINE_DEPTH` like every other recursive walk
/// over the IR.
fn walk(inlines: &[Inline], depth: usize, notes: bool, s: &mut String) {
    if depth >= kasane_ir::MAX_INLINE_DEPTH {
        return;
    }
    for i in inlines {
        match i {
            Inline::Text(t) | Inline::Code(t) | Inline::Math(t) => s.push_str(t),
            Inline::Emph(x) | Inline::Strong(x) => walk(x, depth + 1, notes, s),
            Inline::Link { inlines, .. } => walk(inlines, depth + 1, notes, s),
            Inline::FootnoteRef(n) => {
                if notes {
                    s.push_str(&format!("[^{}]", n.0));
                }
            }
        }
    }
}
