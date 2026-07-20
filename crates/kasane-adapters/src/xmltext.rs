//! Shared helpers for reading XML text content across the EPUB and PPTX parsers.
//!
//! quick-xml 0.41 stopped folding entity and character references into the
//! surrounding `Event::Text`; each reference now arrives as a separate
//! `Event::GeneralRef`. Every text-reading loop must therefore handle both
//! events, and the two must agree on how a reference is turned back into
//! characters -- otherwise the same document yields different text depending on
//! which adapter read it.

use kasane_ir::Inline;
use quick_xml::events::BytesRef;

/// Appends an inline, coalescing it into the preceding one when both are plain
/// text.
///
/// Under quick-xml 0.41 a single run of authored text like `A &amp; B` arrives
/// as three events, which would otherwise become three adjacent `Inline::Text`
/// nodes where 0.36 produced one. The Markdown writer concatenates adjacent
/// text identically either way, but keeping the IR in its unfragmented shape
/// means downstream consumers that index into an inline list -- and IR
/// snapshots -- do not shift under an entity.
pub fn push_inline(dst: &mut Vec<Inline>, inline: Inline) {
    if let (Some(Inline::Text(last)), Inline::Text(next)) = (dst.last_mut(), &inline) {
        last.push_str(next);
        return;
    }
    dst.push(inline);
}

/// Resolves a `GeneralRef` event to the text it stands for.
///
/// Handles the five predefined XML entities (`amp`, `lt`, `gt`, `quot`, `apos`)
/// and numeric character references (`&#233;`, `&#xE9;`) by delegating to
/// quick-xml's own `unescape`, so the result matches what an `Event::Text`
/// containing the same escape would have produced.
///
/// References this parser cannot resolve -- HTML entities such as `&nbsp;`, or
/// entities declared in a document-internal DTD -- are preserved verbatim as
/// their original source text (`&nbsp;`) rather than dropped. Silently
/// discarding them loses content, and the reference text is the only faithful
/// representation available without an entity table we do not have.
pub fn resolve_general_ref(r: &BytesRef<'_>) -> String {
    let Ok(name) = r.decode() else {
        return String::new();
    };
    let source = format!("&{name};");
    match quick_xml::escape::unescape(&source) {
        Ok(resolved) => resolved.into_owned(),
        Err(_) => source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_predefined_named_entities() {
        assert_eq!(resolve_general_ref(&BytesRef::new("amp")), "&");
        assert_eq!(resolve_general_ref(&BytesRef::new("lt")), "<");
        assert_eq!(resolve_general_ref(&BytesRef::new("gt")), ">");
        assert_eq!(resolve_general_ref(&BytesRef::new("quot")), "\"");
        assert_eq!(resolve_general_ref(&BytesRef::new("apos")), "'");
    }

    #[test]
    fn resolves_decimal_and_hex_character_references() {
        assert_eq!(resolve_general_ref(&BytesRef::new("#233")), "é");
        assert_eq!(resolve_general_ref(&BytesRef::new("#xE9")), "é");
    }

    #[test]
    fn preserves_unresolvable_references_verbatim() {
        // &nbsp; is an HTML entity, not an XML predefined one, and quick-xml
        // has no table for it. Keeping the source text is lossless; dropping
        // it is not.
        assert_eq!(resolve_general_ref(&BytesRef::new("nbsp")), "&nbsp;");
        assert_eq!(
            resolve_general_ref(&BytesRef::new("customEnt")),
            "&customEnt;"
        );
    }
}
