//! JSON <-> Matter TLV.
//!
//! Decoding never needs a schema (TLV carries its types); with one, structure
//! fields get their names. Encoding needs the schema for what JSON cannot say:
//! whether a number is signed, whether a string is text or octets (hex in
//! JSON), and which tag a named field has. Without one it falls back to the
//! JSON type (non-negative integers unsigned, objects keyed by tag number).
//!
//! Both directions are bounded by [`MAX_DEPTH`] and allocate only for the
//! output, so neither can be driven into unbounded recursion by a peer.

use rs_matter_tlv::{TLVElement, TLVTag, TLVValue, TLVWrite};
use rs_matter_utils::storage::WriteBuf;
use serde_json::{Map, Number, Value};

use crate::schema::{Field, Shape, StructDef, Type};

/// The deepest nesting either direction accepts. Matter data is a handful of
/// levels deep at most (a list of structures holding a list).
pub const MAX_DEPTH: usize = 16;

/// Why a conversion failed.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ConvertError {
    /// The TLV is malformed, or the writer ran out of room.
    #[error("TLV: {0}")]
    Tlv(String),
    /// The value is nested deeper than [`MAX_DEPTH`].
    #[error("nested deeper than {MAX_DEPTH} levels")]
    TooDeep,
    /// The JSON value does not fit the expected type.
    #[error("{path}: expected {expected}, got {got}")]
    Mismatch {
        /// Where in the value, e.g. `.level` or `[2]`.
        path: String,
        /// What the schema wanted.
        expected: &'static str,
        /// What the JSON had.
        got: String,
    },
    /// A structure field the schema does not have.
    #[error("{path}: no field `{field}` in {structure}")]
    UnknownField {
        /// Where in the value.
        path: String,
        /// The field asked for.
        field: String,
        /// The structure's name.
        structure: &'static str,
    },
    /// A required field is missing.
    #[error("{path}: missing field `{field}`")]
    MissingField {
        /// Where in the value.
        path: String,
        /// The field left out.
        field: &'static str,
    },
}

fn tlv_err<E: core::fmt::Debug>(e: E) -> ConvertError {
    ConvertError::Tlv(format!("{e:?}"))
}

/// Decode a TLV element to JSON, naming structure fields from `shape` where
/// it has them.
///
/// Integers are JSON numbers (exact up to 64 bits), octet strings lowercase
/// hex, structures objects keyed by field name (or tag number for a field the
/// schema does not know), lists and arrays JSON arrays.
pub fn to_json(el: &TLVElement<'_>, shape: Shape) -> Result<Value, ConvertError> {
    decode(el, shape, 0)
}

fn decode(el: &TLVElement<'_>, shape: Shape, depth: usize) -> Result<Value, ConvertError> {
    if depth > MAX_DEPTH {
        return Err(ConvertError::TooDeep);
    }

    Ok(match el.value().map_err(tlv_err)? {
        TLVValue::S8(v) => v.into(),
        TLVValue::S16(v) => v.into(),
        TLVValue::S32(v) => v.into(),
        TLVValue::S64(v) => v.into(),
        TLVValue::U8(v) => v.into(),
        TLVValue::U16(v) => v.into(),
        TLVValue::U32(v) => v.into(),
        TLVValue::U64(v) => v.into(),
        TLVValue::False => false.into(),
        TLVValue::True => true.into(),
        TLVValue::F32(v) => Number::from_f64(v.into()).map_or(Value::Null, Value::Number),
        TLVValue::F64(v) => Number::from_f64(v).map_or(Value::Null, Value::Number),
        TLVValue::Utf8l(s) | TLVValue::Utf16l(s) | TLVValue::Utf32l(s) | TLVValue::Utf64l(s) => {
            s.into()
        }
        TLVValue::Str8l(b) | TLVValue::Str16l(b) | TLVValue::Str32l(b) | TLVValue::Str64l(b) => {
            hex::encode(b).into()
        }
        TLVValue::Null => Value::Null,
        TLVValue::Array | TLVValue::List if shape.list => {
            let elem = Shape {
                ty: shape.ty,
                list: false,
            };
            let mut out = Vec::new();
            for child in el.container().map_err(tlv_err)?.iter() {
                out.push(decode(&child.map_err(tlv_err)?, elem, depth + 1)?);
            }
            Value::Array(out)
        }
        TLVValue::Struct => {
            let def = match shape.ty {
                Some(Type::Struct(def)) => Some(def),
                _ => None,
            };
            decode_struct(el, def, depth)?
        }
        TLVValue::Array | TLVValue::List => {
            let mut out = Vec::new();
            for child in el.container().map_err(tlv_err)?.iter() {
                out.push(decode(&child.map_err(tlv_err)?, Shape::ANY, depth + 1)?);
            }
            Value::Array(out)
        }
        TLVValue::EndCnt => return Err(ConvertError::Tlv("unexpected end of container".into())),
    })
}

fn decode_struct(
    el: &TLVElement<'_>,
    def: Option<&'static StructDef>,
    depth: usize,
) -> Result<Value, ConvertError> {
    let mut out = Map::new();
    for child in el.container().map_err(tlv_err)?.iter() {
        let child = child.map_err(tlv_err)?;
        let (key, shape) = match child.tag().map_err(tlv_err)? {
            TLVTag::Context(tag) => match def.and_then(|d| d.field(tag)) {
                Some(field) => (field.name.to_string(), field.shape()),
                None => (tag.to_string(), Shape::ANY),
            },
            other => (format!("{other:?}"), Shape::ANY),
        };
        out.insert(key, decode(&child, shape, depth + 1)?);
    }
    Ok(Value::Object(out))
}

/// Encode a JSON value as TLV under `tag`, as `shape` says it should be.
///
/// Enumeration values may be given by name; octet strings are hex; structure
/// fields by name or by tag number. Integers are written in the smallest TLV
/// width that holds them, which every Matter reader accepts.
pub fn from_json<W: TLVWrite>(
    w: &mut W,
    tag: &TLVTag,
    shape: Shape,
    value: &Value,
) -> Result<(), ConvertError> {
    encode(w, tag, shape, value, &mut String::new(), 0)
}

/// Encode a JSON value as one anonymous TLV element, as [`from_json`] does,
/// into a new buffer of at most `max_len` bytes.
pub fn encode_to_vec(shape: Shape, value: &Value, max_len: usize) -> Result<Vec<u8>, ConvertError> {
    let mut buf = vec![0u8; max_len];
    let mut wb = WriteBuf::new(&mut buf);
    from_json(&mut wb, &TLVTag::Anonymous, shape, value)?;
    let len = wb.get_tail();
    buf.truncate(len);
    Ok(buf)
}

fn kind(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Bool(_) => "a boolean".into(),
        Value::Number(n) => format!("the number {n}"),
        Value::String(_) => "a string".into(),
        Value::Array(_) => "an array".into(),
        Value::Object(_) => "an object".into(),
    }
}

fn mismatch(path: &str, expected: &'static str, got: &Value) -> ConvertError {
    ConvertError::Mismatch {
        path: if path.is_empty() {
            ".".into()
        } else {
            path.into()
        },
        expected,
        got: kind(got),
    }
}

fn encode<W: TLVWrite>(
    w: &mut W,
    tag: &TLVTag,
    shape: Shape,
    value: &Value,
    path: &mut String,
    depth: usize,
) -> Result<(), ConvertError> {
    if depth > MAX_DEPTH {
        return Err(ConvertError::TooDeep);
    }

    if value.is_null() {
        return w.null(tag).map_err(tlv_err);
    }

    if shape.list {
        let Value::Array(items) = value else {
            return Err(mismatch(path, "a list", value));
        };
        w.start_array(tag).map_err(tlv_err)?;
        let elem = Shape {
            ty: shape.ty,
            list: false,
        };
        for (i, item) in items.iter().enumerate() {
            let len = path.len();
            path.push_str(&format!("[{i}]"));
            encode(w, &TLVTag::Anonymous, elem, item, path, depth + 1)?;
            path.truncate(len);
        }
        return w.end_container().map_err(tlv_err);
    }

    match shape.ty {
        Some(Type::Bool) => match value {
            Value::Bool(b) => w.bool(tag, *b).map_err(tlv_err),
            _ => Err(mismatch(path, "a boolean", value)),
        },
        Some(Type::Uint | Type::Bitmap(_)) => match value.as_u64() {
            Some(v) => w.u64(tag, v).map_err(tlv_err),
            None => Err(mismatch(path, "an unsigned integer", value)),
        },
        Some(Type::Enum(def)) => {
            let v = match value {
                Value::String(name) => def.value_of(name),
                _ => value.as_u64(),
            };
            match v {
                Some(v) => w.u64(tag, v).map_err(tlv_err),
                None => Err(mismatch(path, "an enumeration value or name", value)),
            }
        }
        Some(Type::Int) => match value.as_i64() {
            Some(v) => w.i64(tag, v).map_err(tlv_err),
            None => Err(mismatch(path, "an integer", value)),
        },
        Some(Type::Float) => match value.as_f64() {
            Some(v) => w.f32(tag, v as f32).map_err(tlv_err),
            None => Err(mismatch(path, "a number", value)),
        },
        Some(Type::Double) => match value.as_f64() {
            Some(v) => w.f64(tag, v).map_err(tlv_err),
            None => Err(mismatch(path, "a number", value)),
        },
        Some(Type::String) => match value {
            Value::String(s) => w.utf8(tag, s).map_err(tlv_err),
            _ => Err(mismatch(path, "a string", value)),
        },
        Some(Type::Octets) => match value.as_str().map(hex::decode) {
            Some(Ok(bytes)) => w.str(tag, &bytes).map_err(tlv_err),
            _ => Err(mismatch(path, "a hex string", value)),
        },
        Some(Type::Struct(def)) => match value {
            Value::Object(map) => encode_struct(w, tag, Some(def), map, path, depth),
            _ => Err(mismatch(path, "an object", value)),
        },
        Some(Type::Unknown(_)) | None => encode_untyped(w, tag, value, path, depth),
    }
}

fn encode_struct<W: TLVWrite>(
    w: &mut W,
    tag: &TLVTag,
    def: Option<&'static StructDef>,
    map: &Map<String, Value>,
    path: &mut String,
    depth: usize,
) -> Result<(), ConvertError> {
    // Resolve every key first, so fields go out in tag order and an unknown
    // name is reported before anything is written.
    let mut fields: Vec<(u8, Option<&'static Field>, &Value)> = Vec::with_capacity(map.len());
    for (key, value) in map {
        let field = def.and_then(|d| d.field_by_name(key));
        let tag = match (field, key.parse::<u8>()) {
            (Some(f), _) => f.tag,
            (None, Ok(n)) => n,
            (None, Err(_)) => {
                return Err(ConvertError::UnknownField {
                    path: if path.is_empty() {
                        ".".into()
                    } else {
                        path.clone()
                    },
                    field: key.clone(),
                    structure: def.map_or("a structure without a schema", |d| d.name),
                })
            }
        };
        let field = field.or_else(|| def.and_then(|d| d.field(tag)));
        fields.push((tag, field, value));
    }

    // A required field left out is an error, unless it can be null: then it
    // goes out as null, which is what leaving it out means to a person.
    static NULL: Value = Value::Null;
    if let Some(def) = def {
        for f in def.fields.iter().filter(|f| !f.optional) {
            if !fields.iter().any(|(tag, _, _)| *tag == f.tag) {
                if !f.nullable {
                    return Err(ConvertError::MissingField {
                        path: if path.is_empty() {
                            ".".into()
                        } else {
                            path.clone()
                        },
                        field: f.name,
                    });
                }
                fields.push((f.tag, Some(f), &NULL));
            }
        }
    }

    fields.sort_by_key(|(tag, _, _)| *tag);

    w.start_struct(tag).map_err(tlv_err)?;
    for (field_tag, field, value) in fields {
        let len = path.len();
        match field {
            Some(f) => path.push_str(&format!(".{}", f.name)),
            None => path.push_str(&format!(".{field_tag}")),
        }
        let shape = field.map_or(Shape::ANY, Field::shape);
        encode(
            w,
            &TLVTag::Context(field_tag),
            shape,
            value,
            path,
            depth + 1,
        )?;
        path.truncate(len);
    }
    w.end_container().map_err(tlv_err)
}

fn encode_untyped<W: TLVWrite>(
    w: &mut W,
    tag: &TLVTag,
    value: &Value,
    path: &mut String,
    depth: usize,
) -> Result<(), ConvertError> {
    match value {
        Value::Null => w.null(tag).map_err(tlv_err),
        Value::Bool(b) => w.bool(tag, *b).map_err(tlv_err),
        Value::Number(n) => {
            if let Some(v) = n.as_u64() {
                w.u64(tag, v).map_err(tlv_err)
            } else if let Some(v) = n.as_i64() {
                w.i64(tag, v).map_err(tlv_err)
            } else {
                w.f64(tag, n.as_f64().unwrap_or_default()).map_err(tlv_err)
            }
        }
        Value::String(s) => w.utf8(tag, s).map_err(tlv_err),
        Value::Array(items) => {
            w.start_array(tag).map_err(tlv_err)?;
            for (i, item) in items.iter().enumerate() {
                let len = path.len();
                path.push_str(&format!("[{i}]"));
                encode(w, &TLVTag::Anonymous, Shape::ANY, item, path, depth + 1)?;
                path.truncate(len);
            }
            w.end_container().map_err(tlv_err)
        }
        Value::Object(map) => encode_struct(w, tag, None, map, path, depth),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn roundtrip(shape: Shape, v: Value) -> Value {
        let bytes = encode_to_vec(shape, &v, 1024).unwrap();
        to_json(&TLVElement::new(&bytes), shape).unwrap()
    }

    #[test]
    fn primitives() {
        assert_eq!(roundtrip(Shape::of(Type::Uint), json!(300)), json!(300));
        assert_eq!(roundtrip(Shape::of(Type::Int), json!(-5)), json!(-5));
        assert_eq!(roundtrip(Shape::of(Type::Bool), json!(true)), json!(true));
        assert_eq!(roundtrip(Shape::of(Type::String), json!("hé")), json!("hé"));
        assert_eq!(
            roundtrip(Shape::of(Type::Octets), json!("00ff10")),
            json!("00ff10")
        );
        assert_eq!(roundtrip(Shape::of(Type::Double), json!(1.5)), json!(1.5));
        assert_eq!(
            roundtrip(Shape::of(Type::Uint), json!(u64::MAX)),
            json!(u64::MAX)
        );
        assert_eq!(roundtrip(Shape::of(Type::Uint), Value::Null), Value::Null);
    }

    #[test]
    fn structures_by_name() {
        let level = crate::cluster_by_name("LevelControl").unwrap();
        let req = level
            .command_by_name("MoveToLevel")
            .unwrap()
            .request
            .unwrap();
        let v = json!({"level": 128, "options_mask": 0, "OptionsOverride": 0});
        let out = roundtrip(req.shape(), v);
        assert_eq!(
            out,
            json!({"level": 128, "transitionTime": null, "optionsMask": 0, "optionsOverride": 0})
        );
    }

    #[test]
    fn errors_name_the_place() {
        let level = crate::cluster_by_name("LevelControl").unwrap();
        let req = level
            .command_by_name("MoveToLevel")
            .unwrap()
            .request
            .unwrap();
        let mut buf = [0u8; 256];
        let mut wb = WriteBuf::new(&mut buf);
        let err = from_json(
            &mut wb,
            &TLVTag::Anonymous,
            req.shape(),
            &json!({"level": "high", "optionsMask": 0, "optionsOverride": 0, "transitionTime": 0}),
        )
        .unwrap_err();
        assert!(
            matches!(err, ConvertError::Mismatch { ref path, .. } if path == ".level"),
            "{err}"
        );
        let err = from_json(
            &mut wb,
            &TLVTag::Anonymous,
            req.shape(),
            &json!({"bogus": 1}),
        )
        .unwrap_err();
        assert!(matches!(err, ConvertError::UnknownField { .. }), "{err}");
        let err = from_json(&mut wb, &TLVTag::Anonymous, req.shape(), &json!({})).unwrap_err();
        assert!(
            matches!(err, ConvertError::MissingField { field: "level", .. }),
            "{err}"
        );
    }

    #[test]
    fn enums_by_name() {
        let ocw = crate::cluster_by_name("AdministratorCommissioning").unwrap();
        let attr = ocw.attribute_by_name("WindowStatus").unwrap();
        let Type::Enum(def) = attr.ty else {
            panic!("not an enum")
        };
        let mut buf = [0u8; 16];
        let mut wb = WriteBuf::new(&mut buf);
        let name = def.name_of(1).unwrap();
        from_json(&mut wb, &TLVTag::Anonymous, attr.shape(), &json!(name)).unwrap();
        let len = wb.get_tail();
        assert_eq!(
            to_json(&TLVElement::new(&buf[..len]), attr.shape()).unwrap(),
            json!(1)
        );
    }

    #[test]
    fn lists_of_structures_decode_with_names() {
        let descriptor = crate::cluster(0x001D).unwrap();
        let dtl = descriptor.attribute_by_name("DeviceTypeList").unwrap();
        let out = roundtrip(dtl.shape(), json!([{"deviceType": 256, "revision": 3}]));
        assert_eq!(out, json!([{"deviceType": 256, "revision": 3}]));
    }

    #[test]
    fn untyped_and_too_deep() {
        assert_eq!(
            roundtrip(Shape::ANY, json!({"1": [1, -2, "x", true], "0": null})),
            json!({"0": null, "1": [1, -2, "x", true]})
        );
        let mut deep = json!(1);
        for _ in 0..(MAX_DEPTH + 2) {
            deep = json!([deep]);
        }
        let mut buf = [0u8; 256];
        let mut wb = WriteBuf::new(&mut buf);
        assert_eq!(
            from_json(&mut wb, &TLVTag::Anonymous, Shape::ANY, &deep),
            Err(ConvertError::TooDeep)
        );
    }

    #[test]
    fn malformed_tlv_is_an_error() {
        for bytes in [
            &[0x15u8][..],
            &[0x0c, 0x05, b'a'],
            &[0xff],
            &[0x36, 0x15, 0x15],
        ] {
            let _ = to_json(&TLVElement::new(bytes), Shape::ANY);
        }
    }
}
