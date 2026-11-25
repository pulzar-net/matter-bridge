//! Arbitrary bytes as an X.509 certificate to convert to Matter TLV.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = matter_host::x509::to_matter_tlv(data);
});
