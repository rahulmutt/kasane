//! CommonMark pairing behaviour for `*`/`_` combinations, straight from the
//! parser. No writer hook needed. This is the apparatus behind spec §3's
//! condition-2/condition-3 table and §3.1's claim about `same_delim_to_splice`.
use pulldown_cmark::{Options, Parser, Event, Tag, TagEnd};
fn walk(md: &str) -> String {
    let mut o = Options::empty();
    o.insert(Options::ENABLE_MATH);
    let mut s = String::new();
    for e in Parser::new_ext(md, o) {
        match e {
            Event::Start(Tag::Emphasis) => s.push_str("<em>"),
            Event::End(TagEnd::Emphasis) => s.push_str("</em>"),
            Event::Start(Tag::Strong) => s.push_str("<st>"),
            Event::End(TagEnd::Strong) => s.push_str("</st>"),
            Event::Text(t) => s.push_str(&t),
            Event::Code(t) => { s.push('`'); s.push_str(&t); s.push('`'); }
            _ => {}
        }
    }
    s
}
#[test]
fn pairing() {
    for md in [
        // `_` is legal where `*` is, except intraword -- spec §3 condition 2
        "a*b*c", "a_b_c", "*x*", "_x_", "a_`x`_c", "a _b_ c", "`x`_`y`_",
        // the three spellings census-inexpressible.txt advertises
        "_*x*_", "__*x*__", "__**x**__",
        // inner `_`: safe on punctuation flanks, inert on letter flanks
        "*_x_*", "*a_b_c*", "**a__b__c**", "_a*b*c_",
        // what condition 3 exists to prevent
        "___a___",
        // the pair same_delim_to_splice says it cannot tell apart -- spec §3.1
        "*a *b* c*", "*a*b*c*",
        // the distinction §4.3 must preserve
        "*a**b**c*",
        // the em-outermost tie-break
        "***x***",
    ] {
        println!("{md:<14} => {}", walk(md));
    }
    panic!("show");
}
