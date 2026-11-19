//! Arbitrary JSON (as an MQTT client could send it) encoded against every
//! shape of the data model: never a panic, and whatever encodes decodes back.
#![no_main]

use std::sync::OnceLock;

use libfuzzer_sys::fuzz_target;
use matter_model::Shape;
use rs_matter_tlv::TLVElement;

#[path = "shapes.rs"]
mod shapes;

static SHAPES: OnceLock<Vec<Shape>> = OnceLock::new();

fuzz_target!(|data: &[u8]| {
    let shapes = SHAPES.get_or_init(shapes::all);
    let (shape, json) = shapes::pick(shapes, data);
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(json) else {
        return;
    };
    if let Ok(tlv) = matter_model::encode_to_vec(shape, &value, 4096) {
        matter_model::to_json(&TLVElement::new(&tlv), shape)
            .expect("what the encoder writes, the decoder reads");
    }
});
