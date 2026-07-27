#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    kasane_adapters::fuzz_entry::guards(data);
});
