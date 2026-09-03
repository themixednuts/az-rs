//! Tagged reflected values carried inside AZSCENE Postcard payloads.

use std::{cmp::Ordering, collections::BTreeMap, fmt};

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub enum ReflectedValueWire {
    Bool(bool),
    U8(u8),
    U16(u16),
    U32(u32),
    U64(u64),
    I8(i8),
    I16(i16),
    I32(i32),
    I64(i64),
    F32(f32),
    F64(f64),
    Char(char),
    String(String),
    Unit,
    Option(Option<Box<Self>>),
    Newtype(Box<Self>),
    Seq(Vec<Self>),
    Map(Vec<(Self, Self)>),
    Bytes(Vec<u8>),
}

impl From<serde_value::Value> for ReflectedValueWire {
    fn from(value: serde_value::Value) -> Self {
        match value {
            serde_value::Value::Bool(value) => Self::Bool(value),
            serde_value::Value::U8(value) => Self::U8(value),
            serde_value::Value::U16(value) => Self::U16(value),
            serde_value::Value::U32(value) => Self::U32(value),
            serde_value::Value::U64(value) => Self::U64(value),
            serde_value::Value::I8(value) => Self::I8(value),
            serde_value::Value::I16(value) => Self::I16(value),
            serde_value::Value::I32(value) => Self::I32(value),
            serde_value::Value::I64(value) => Self::I64(value),
            serde_value::Value::F32(value) => Self::F32(value),
            serde_value::Value::F64(value) => Self::F64(value),
            serde_value::Value::Char(value) => Self::Char(value),
            serde_value::Value::String(value) => Self::String(value),
            serde_value::Value::Unit => Self::Unit,
            serde_value::Value::Option(value) => {
                Self::Option(value.map(|value| Box::new(Self::from(*value))))
            }
            serde_value::Value::Newtype(value) => Self::Newtype(Box::new(Self::from(*value))),
            serde_value::Value::Seq(values) => {
                Self::Seq(values.into_iter().map(Self::from).collect())
            }
            serde_value::Value::Map(values) => Self::Map(
                values
                    .into_iter()
                    .map(|(key, value)| (Self::from(key), Self::from(value)))
                    .collect(),
            ),
            serde_value::Value::Bytes(value) => Self::Bytes(value),
        }
    }
}

impl TryFrom<ReflectedValueWire> for serde_value::Value {
    type Error = ReflectedValueWireError;

    fn try_from(value: ReflectedValueWire) -> Result<Self, Self::Error> {
        Ok(match value {
            ReflectedValueWire::Bool(value) => Self::Bool(value),
            ReflectedValueWire::U8(value) => Self::U8(value),
            ReflectedValueWire::U16(value) => Self::U16(value),
            ReflectedValueWire::U32(value) => Self::U32(value),
            ReflectedValueWire::U64(value) => Self::U64(value),
            ReflectedValueWire::I8(value) => Self::I8(value),
            ReflectedValueWire::I16(value) => Self::I16(value),
            ReflectedValueWire::I32(value) => Self::I32(value),
            ReflectedValueWire::I64(value) => Self::I64(value),
            ReflectedValueWire::F32(value) => Self::F32(value),
            ReflectedValueWire::F64(value) => Self::F64(value),
            ReflectedValueWire::Char(value) => Self::Char(value),
            ReflectedValueWire::String(value) => Self::String(value),
            ReflectedValueWire::Unit => Self::Unit,
            ReflectedValueWire::Option(value) => Self::Option(
                value
                    .map(|value| Self::try_from(*value).map(Box::new))
                    .transpose()?,
            ),
            ReflectedValueWire::Newtype(value) => Self::Newtype(Box::new(Self::try_from(*value)?)),
            ReflectedValueWire::Seq(values) => Self::Seq(
                values
                    .into_iter()
                    .map(Self::try_from)
                    .collect::<Result<_, _>>()?,
            ),
            ReflectedValueWire::Map(values) => {
                let mut map = BTreeMap::new();
                let mut previous: Option<Self> = None;
                for (key, value) in values {
                    let key = Self::try_from(key)?;
                    if previous
                        .as_ref()
                        .is_some_and(|previous| previous.cmp(&key) != Ordering::Less)
                    {
                        return Err(ReflectedValueWireError::NonCanonicalMapOrder);
                    }
                    previous = Some(key.clone());
                    map.insert(key, Self::try_from(value)?);
                }
                Self::Map(map)
            }
            ReflectedValueWire::Bytes(value) => Self::Bytes(value),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReflectedValueWireError {
    NonCanonicalMapOrder,
}

impl fmt::Display for ReflectedValueWireError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonCanonicalMapOrder => {
                formatter.write_str("reflected map keys are not strictly canonical")
            }
        }
    }
}

impl std::error::Error for ReflectedValueWireError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn postcard_round_trip_preserves_tagged_value_kinds() {
        let value = serde_value::Value::Map(BTreeMap::from([(
            serde_value::Value::String("Active".to_owned()),
            serde_value::Value::Map(BTreeMap::from([
                (
                    serde_value::Value::String("count".to_owned()),
                    serde_value::Value::U32(41),
                ),
                (
                    serde_value::Value::String("hint".to_owned()),
                    serde_value::Value::Option(None),
                ),
            ])),
        )]));

        let bytes = postcard::to_allocvec(&ReflectedValueWire::from(value.clone())).unwrap();
        let wire: ReflectedValueWire = postcard::from_bytes(&bytes).unwrap();

        assert_eq!(serde_value::Value::try_from(wire).unwrap(), value);
    }

    #[test]
    fn rejects_noncanonical_map_order() {
        let wire = ReflectedValueWire::Map(vec![
            (ReflectedValueWire::U32(2), ReflectedValueWire::Unit),
            (ReflectedValueWire::U32(1), ReflectedValueWire::Unit),
        ]);

        assert_eq!(
            serde_value::Value::try_from(wire),
            Err(ReflectedValueWireError::NonCanonicalMapOrder)
        );
    }
}
