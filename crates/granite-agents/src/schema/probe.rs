//! The recorder behind [`super::GraniteSchema::of`]: a `serde::Deserializer` that answers every request with the
//! least value of its kind and writes down what was asked. The port of Swift Granite's `SchemaDecoder`.
//!
//! PIN: A REQUIRED FIELD IT CANNOT MAKE ENDS THE WALK. When a field's own deserialization fails, the fields read
//! so far are kept and the rest are not read, as Swift's walk stops.

use std::fmt;

use serde::de::{
    self, DeserializeOwned, DeserializeSeed, EnumAccess, MapAccess, SeqAccess, VariantAccess,
    Visitor,
};

use super::{Field, GraniteSchema};

const DEPTH: usize = 8;

#[derive(Debug)]
pub(crate) struct ProbeError(String);

impl fmt::Display for ProbeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ProbeError {}

impl de::Error for ProbeError {
    fn custom<T: fmt::Display>(message: T) -> Self {
        ProbeError(message.to_string())
    }
}

/// What one value's deserialization asked for.
#[derive(Default)]
struct Shape {
    schema: Option<GraniteSchema>,
    optional: bool,
}

impl Shape {
    fn schema(self) -> GraniteSchema {
        self.schema.unwrap_or(GraniteSchema::Any)
    }
}

pub(crate) fn record<T: DeserializeOwned>() -> GraniteSchema {
    let mut top = Shape::default();
    let _ = T::deserialize(Probe {
        depth: 0,
        shape: &mut top,
    });
    top.schema()
}

struct Probe<'a> {
    depth: usize,
    shape: &'a mut Shape,
}

macro_rules! scalar {
    ($($method:ident => $schema:expr, $visit:ident($($value:expr)?);)*) => {
        $(fn $method<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, ProbeError> {
            self.shape.schema = Some($schema);
            visitor.$visit($($value)?)
        })*
    };
}

impl<'de> de::Deserializer<'de> for Probe<'_> {
    type Error = ProbeError;

    scalar! {
        deserialize_bool => GraniteSchema::Boolean, visit_bool(false);
        deserialize_i8 => GraniteSchema::Integer, visit_i8(1);
        deserialize_i16 => GraniteSchema::Integer, visit_i16(1);
        deserialize_i32 => GraniteSchema::Integer, visit_i32(1);
        deserialize_i64 => GraniteSchema::Integer, visit_i64(1);
        deserialize_i128 => GraniteSchema::Integer, visit_i128(1);
        deserialize_u8 => GraniteSchema::Integer, visit_u8(1);
        deserialize_u16 => GraniteSchema::Integer, visit_u16(1);
        deserialize_u32 => GraniteSchema::Integer, visit_u32(1);
        deserialize_u64 => GraniteSchema::Integer, visit_u64(1);
        deserialize_u128 => GraniteSchema::Integer, visit_u128(1);
        deserialize_f32 => GraniteSchema::Number, visit_f32(0.0);
        deserialize_f64 => GraniteSchema::Number, visit_f64(0.0);
        deserialize_char => GraniteSchema::String, visit_char('a');
        deserialize_str => GraniteSchema::String, visit_str("");
        deserialize_string => GraniteSchema::String, visit_string(String::new());
        deserialize_bytes => GraniteSchema::Any, visit_bytes(&[]);
        deserialize_byte_buf => GraniteSchema::Any, visit_byte_buf(Vec::new());
        deserialize_unit => GraniteSchema::Any, visit_unit();
        deserialize_any => GraniteSchema::Any, visit_unit();
        deserialize_ignored_any => GraniteSchema::Any, visit_unit();
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, ProbeError> {
        self.shape.optional = true;
        visitor.visit_some(Probe {
            depth: self.depth,
            shape: self.shape,
        })
    }

    fn deserialize_unit_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, ProbeError> {
        self.shape.schema = Some(GraniteSchema::Any);
        visitor.visit_unit()
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, ProbeError> {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, ProbeError> {
        let mut element = Shape::default();
        let result = visitor.visit_seq(OneElement {
            depth: self.depth + 1,
            element: &mut element,
            done: false,
        });
        self.shape.schema = Some(GraniteSchema::Array(Box::new(element.schema())));
        result
    }

    fn deserialize_tuple<V: Visitor<'de>>(
        self,
        _len: usize,
        visitor: V,
    ) -> Result<V::Value, ProbeError> {
        self.deserialize_seq(visitor)
    }

    fn deserialize_tuple_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _len: usize,
        visitor: V,
    ) -> Result<V::Value, ProbeError> {
        self.deserialize_seq(visitor)
    }

    fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, ProbeError> {
        self.shape.schema = Some(GraniteSchema::Any);
        visitor.visit_map(NoEntries)
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, ProbeError> {
        if self.depth >= DEPTH {
            self.shape.schema = Some(GraniteSchema::Any);
            return Err(ProbeError("too deep to read".into()));
        }
        let mut read = Vec::new();
        let result = visitor.visit_map(Fields {
            depth: self.depth,
            names: fields,
            next: 0,
            read: &mut read,
        });
        self.shape.schema = Some(GraniteSchema::Object(read));
        result
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _name: &'static str,
        variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, ProbeError> {
        let Some(first) = variants.first() else {
            self.shape.schema = Some(GraniteSchema::Any);
            return Err(ProbeError("an enum with no variants".into()));
        };
        let mut unit = true;
        let result = visitor.visit_enum(FirstVariant {
            depth: self.depth,
            name: first,
            unit: &mut unit,
        });
        self.shape.schema = Some(if unit {
            GraniteSchema::Choice(
                variants
                    .iter()
                    .map(|variant| (*variant).to_owned())
                    .collect(),
            )
        } else {
            GraniteSchema::Any
        });
        result
    }

    fn deserialize_identifier<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, ProbeError> {
        visitor.visit_str("")
    }
}

struct OneElement<'a> {
    depth: usize,
    element: &'a mut Shape,
    done: bool,
}

impl<'de> SeqAccess<'de> for OneElement<'_> {
    type Error = ProbeError;

    fn next_element_seed<T: DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<Option<T::Value>, ProbeError> {
        if self.done {
            return Ok(None);
        }
        self.done = true;
        seed.deserialize(Probe {
            depth: self.depth,
            shape: self.element,
        })
        .map(Some)
    }
}

struct NoEntries;

impl<'de> MapAccess<'de> for NoEntries {
    type Error = ProbeError;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        _seed: K,
    ) -> Result<Option<K::Value>, ProbeError> {
        Ok(None)
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(
        &mut self,
        _seed: V,
    ) -> Result<V::Value, ProbeError> {
        Err(ProbeError("no value".into()))
    }
}

struct Fields<'a> {
    depth: usize,
    names: &'static [&'static str],
    next: usize,
    read: &'a mut Vec<Field>,
}

impl<'de> MapAccess<'de> for Fields<'_> {
    type Error = ProbeError;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, ProbeError> {
        let Some(name) = self.names.get(self.next) else {
            return Ok(None);
        };
        seed.deserialize(de::value::StrDeserializer::<ProbeError>::new(name))
            .map(Some)
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(
        &mut self,
        seed: V,
    ) -> Result<V::Value, ProbeError> {
        let name = self.names[self.next];
        self.next += 1;
        let mut shape = Shape::default();
        let result = seed.deserialize(Probe {
            depth: self.depth + 1,
            shape: &mut shape,
        });
        let required = !shape.optional;
        self.read.push(Field {
            name: name.to_owned(),
            schema: shape.schema(),
            required,
        });
        result
    }
}

struct FirstVariant<'a> {
    depth: usize,
    name: &'static str,
    unit: &'a mut bool,
}

impl<'de, 'a> EnumAccess<'de> for FirstVariant<'a> {
    type Error = ProbeError;
    type Variant = VariantProbe<'a>;

    fn variant_seed<S: DeserializeSeed<'de>>(
        self,
        seed: S,
    ) -> Result<(S::Value, VariantProbe<'a>), ProbeError> {
        let value = seed.deserialize(de::value::StrDeserializer::<ProbeError>::new(self.name))?;
        Ok((
            value,
            VariantProbe {
                depth: self.depth,
                unit: self.unit,
            },
        ))
    }
}

struct VariantProbe<'a> {
    depth: usize,
    unit: &'a mut bool,
}

impl<'de> VariantAccess<'de> for VariantProbe<'_> {
    type Error = ProbeError;

    fn unit_variant(self) -> Result<(), ProbeError> {
        Ok(())
    }

    fn newtype_variant_seed<T: DeserializeSeed<'de>>(
        self,
        seed: T,
    ) -> Result<T::Value, ProbeError> {
        *self.unit = false;
        let mut shape = Shape::default();
        seed.deserialize(Probe {
            depth: self.depth + 1,
            shape: &mut shape,
        })
    }

    fn tuple_variant<V: Visitor<'de>>(
        self,
        _len: usize,
        visitor: V,
    ) -> Result<V::Value, ProbeError> {
        *self.unit = false;
        let mut shape = Shape::default();
        de::Deserializer::deserialize_seq(
            Probe {
                depth: self.depth + 1,
                shape: &mut shape,
            },
            visitor,
        )
    }

    fn struct_variant<V: Visitor<'de>>(
        self,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, ProbeError> {
        *self.unit = false;
        let mut shape = Shape::default();
        de::Deserializer::deserialize_struct(
            Probe {
                depth: self.depth + 1,
                shape: &mut shape,
            },
            "",
            fields,
            visitor,
        )
    }
}
