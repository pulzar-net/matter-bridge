//! The shapes of the generated tables.

use crate::names_match;

/// A cluster: its attributes, commands and events.
#[derive(Debug)]
pub struct Cluster {
    /// Cluster id.
    pub id: u32,
    /// IDL name, `UpperCamelCase`.
    pub name: &'static str,
    /// Cluster revision the IDL describes.
    pub revision: u16,
    /// Attributes, sorted by id, global attributes included.
    pub attributes: &'static [Attribute],
    /// Commands the server accepts, sorted by id.
    pub commands: &'static [Command],
    /// Events, sorted by id.
    pub events: &'static [Event],
    /// Response commands the server sends, by command id.
    pub responses: &'static [(u32, &'static StructDef)],
}

impl Cluster {
    /// The attribute with `id`.
    pub fn attribute(&self, id: u32) -> Option<&'static Attribute> {
        self.attributes
            .binary_search_by_key(&id, |a| a.id)
            .ok()
            .map(|i| &self.attributes[i])
    }

    /// The attribute named `name` (case and `_` ignored).
    pub fn attribute_by_name(&self, name: &str) -> Option<&'static Attribute> {
        self.attributes.iter().find(|a| names_match(a.name, name))
    }

    /// The command with `id`.
    pub fn command(&self, id: u32) -> Option<&'static Command> {
        self.commands
            .binary_search_by_key(&id, |c| c.id)
            .ok()
            .map(|i| &self.commands[i])
    }

    /// The command named `name` (case and `_` ignored).
    pub fn command_by_name(&self, name: &str) -> Option<&'static Command> {
        self.commands.iter().find(|c| names_match(c.name, name))
    }

    /// The event with `id`.
    pub fn event(&self, id: u32) -> Option<&'static Event> {
        self.events
            .binary_search_by_key(&id, |e| e.id)
            .ok()
            .map(|i| &self.events[i])
    }

    /// The event named `name` (case and `_` ignored).
    pub fn event_by_name(&self, name: &str) -> Option<&'static Event> {
        self.events.iter().find(|e| names_match(e.name, name))
    }

    /// The structure of the response command `id` the server sends.
    pub fn response(&self, id: u32) -> Option<&'static StructDef> {
        self.responses
            .iter()
            .find(|(rid, _)| *rid == id)
            .map(|(_, s)| *s)
    }
}

/// An attribute of a cluster.
#[derive(Debug)]
pub struct Attribute {
    /// Attribute id.
    pub id: u32,
    /// IDL name, `lowerCamelCase`.
    pub name: &'static str,
    /// Value type (of each element, for a list).
    pub ty: Type,
    /// Whether the value is a list of `ty`.
    pub list: bool,
    /// Whether the value can be null.
    pub nullable: bool,
    /// Whether a server may leave the attribute out.
    pub optional: bool,
    /// Whether a client may write it.
    pub writable: bool,
    /// Whether a write must be timed.
    pub timed_write: bool,
}

impl Attribute {
    /// The attribute's shape, for conversion.
    pub const fn shape(&self) -> Shape {
        Shape {
            ty: Some(self.ty),
            list: self.list,
        }
    }
}

/// A command a server accepts.
#[derive(Debug)]
pub struct Command {
    /// Command id.
    pub id: u32,
    /// IDL name, `UpperCamelCase`.
    pub name: &'static str,
    /// The request's fields, or `None` for a command without any.
    pub request: Option<&'static StructDef>,
    /// The response command's fields, or `None` when the server answers with
    /// a status only.
    pub response: Option<&'static StructDef>,
    /// Whether the command must be invoked in a timed interaction.
    pub timed: bool,
}

/// An event of a cluster.
#[derive(Debug)]
pub struct Event {
    /// Event id.
    pub id: u32,
    /// IDL name, `UpperCamelCase`.
    pub name: &'static str,
    /// The event's payload: a structure of its fields.
    pub payload: &'static StructDef,
}

impl Event {
    /// The payload's shape, for conversion.
    pub const fn shape(&self) -> Shape {
        Shape {
            ty: Some(Type::Struct(self.payload)),
            list: false,
        }
    }
}

/// A field of a structure, a command or an event.
#[derive(Debug)]
pub struct Field {
    /// Context tag.
    pub tag: u8,
    /// IDL name, `lowerCamelCase`.
    pub name: &'static str,
    /// Value type (of each element, for a list).
    pub ty: Type,
    /// Whether the value is a list of `ty`.
    pub list: bool,
    /// Whether the value can be null.
    pub nullable: bool,
    /// Whether the field can be left out.
    pub optional: bool,
}

impl Field {
    /// The field's shape, for conversion.
    pub const fn shape(&self) -> Shape {
        Shape {
            ty: Some(self.ty),
            list: self.list,
        }
    }
}

/// A structure.
#[derive(Debug)]
pub struct StructDef {
    /// IDL name.
    pub name: &'static str,
    /// Fields, sorted by tag.
    pub fields: &'static [Field],
    /// Whether the structure is fabric-scoped.
    pub fabric_scoped: bool,
}

impl StructDef {
    /// The field with context tag `tag`.
    pub fn field(&self, tag: u8) -> Option<&'static Field> {
        self.fields.iter().find(|f| f.tag == tag)
    }

    /// The field named `name` (case and `_` ignored).
    pub fn field_by_name(&self, name: &str) -> Option<&'static Field> {
        self.fields.iter().find(|f| names_match(f.name, name))
    }

    /// The structure's shape, for conversion.
    pub fn shape(&'static self) -> Shape {
        Shape {
            ty: Some(Type::Struct(self)),
            list: false,
        }
    }
}

/// An enumeration.
#[derive(Debug)]
pub struct EnumDef {
    /// IDL name.
    pub name: &'static str,
    /// Values and their names.
    pub values: &'static [(u64, &'static str)],
}

impl EnumDef {
    /// The name of `value`.
    pub fn name_of(&self, value: u64) -> Option<&'static str> {
        self.values
            .iter()
            .find(|(v, _)| *v == value)
            .map(|(_, n)| *n)
    }

    /// The value named `name` (case and `_` ignored, with or without a `k`
    /// prefix as the IDL writes them).
    pub fn value_of(&self, name: &str) -> Option<u64> {
        self.values
            .iter()
            .find(|(_, n)| {
                names_match(n, name) || n.strip_prefix('k').is_some_and(|n| names_match(n, name))
            })
            .map(|(v, _)| *v)
    }
}

/// A bitmap.
#[derive(Debug)]
pub struct BitmapDef {
    /// IDL name.
    pub name: &'static str,
    /// Bit masks and their names.
    pub bits: &'static [(u64, &'static str)],
}

/// The type of a value, as far as JSON conversion needs it.
#[derive(Debug, Clone, Copy)]
pub enum Type {
    /// `boolean`.
    Bool,
    /// An unsigned integer of any width, or a semantic type over one.
    Uint,
    /// A signed integer of any width, or a semantic type over one.
    Int,
    /// `single`.
    Float,
    /// `double`.
    Double,
    /// A UTF-8 string.
    String,
    /// An octet string; hex in JSON.
    Octets,
    /// An enumeration: a number, or its name when encoding.
    Enum(&'static EnumDef),
    /// A bitmap: a number.
    Bitmap(&'static BitmapDef),
    /// A structure: an object keyed by field name.
    Struct(&'static StructDef),
    /// A type the tables do not know; converted by its TLV type.
    Unknown(&'static str),
}

/// What a value is expected to be: its type (if known) and whether it is a
/// list of it.
#[derive(Debug, Clone, Copy)]
pub struct Shape {
    /// The element type, if known.
    pub ty: Option<Type>,
    /// Whether the value is a list.
    pub list: bool,
}

impl Shape {
    /// A value of unknown shape, converted by its TLV or JSON type alone.
    pub const ANY: Shape = Shape {
        ty: None,
        list: false,
    };

    /// A single value of `ty`.
    pub const fn of(ty: Type) -> Shape {
        Shape {
            ty: Some(ty),
            list: false,
        }
    }
}
