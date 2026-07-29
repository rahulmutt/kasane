//! Guards the generator itself. A property suite is only as good as what it
//! generates, and a generator that silently stops producing headings (or
//! duplicate sentinels) would leave every property passing vacuously.

mod generator;

use proptest::prelude::*;

proptest! {
    #[test]
    fn sentinels_are_unique(case in generator::case()) {
        let mut seen = std::collections::HashSet::new();
        for s in &case.sentinels {
            prop_assert!(seen.insert(s.token.clone()), "duplicate sentinel {}", s.token);
        }
    }

    #[test]
    fn every_block_carries_a_sentinel(case in generator::case()) {
        prop_assert_eq!(case.sentinels.len(), case.doc.nodes.len());
    }

    #[test]
    fn options_are_well_ordered(case in generator::case()) {
        prop_assert!(case.opts.min_tokens < case.opts.max_tokens);
    }
}
