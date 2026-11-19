//! Hostile ReportData messages through the controller's report decoder (the
//! path every subscription report from a device takes).
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = matter_controller::decode_report_data(1, data);
});
