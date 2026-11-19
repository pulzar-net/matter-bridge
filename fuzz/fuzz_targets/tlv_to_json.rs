//! Hostile TLV, as a device could send it, decoded against every shape of the
//! data model: never a panic, never unbounded recursion.
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
    let (shape, tlv) = shapes::pick(shapes, data);
    let _ = matter_model::to_json(&TLVElement::new(tlv), shape);
});
