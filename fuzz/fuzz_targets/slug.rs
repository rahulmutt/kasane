#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    kasane_core::fuzz_entry::slug(data);
});
