//! Arbitrary onboarding payloads (QR and manual codes, as typed or scanned).
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(code) = std::str::from_utf8(data) {
        if let Ok(o) = matter_host::Onboarding::parse(code) {
            let _ = o.filter();
        }
        let _ = matter_host::Onboarding::parse(&format!("MT:{code}"));
    }
});
