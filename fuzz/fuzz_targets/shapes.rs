// Every shape the data model has: each attribute, each command's request and
// response, each event, and the shapeless one. The first two input bytes
// pick one, so the fuzzer explores them all.

use matter_model::{Shape, CLUSTERS};

pub fn all() -> Vec<Shape> {
    let mut shapes = vec![Shape::ANY];
    for c in CLUSTERS {
        shapes.extend(c.attributes.iter().map(|a| a.shape()));
        for cmd in c.commands {
            shapes.extend(cmd.request.map(|s| s.shape()));
            shapes.extend(cmd.response.map(|s| s.shape()));
        }
        shapes.extend(c.events.iter().map(|e| e.shape()));
    }
    shapes
}

pub fn pick<'a>(shapes: &[Shape], data: &'a [u8]) -> (Shape, &'a [u8]) {
    if data.len() < 2 {
        return (Shape::ANY, data);
    }
    let i = u16::from_le_bytes([data[0], data[1]]) as usize % shapes.len();
    (shapes[i], &data[2..])
}
