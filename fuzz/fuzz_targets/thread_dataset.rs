//! Arbitrary Thread datasets (as a border router or a person supplies them).
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = matter_host::network::thread_extended_pan_id(data);
    if let Ok(text) = std::str::from_utf8(data) {
        let _ = matter_host::network::parse_thread_dataset_hex(text);
    }
});
