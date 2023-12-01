use serde::{de::Visitor, forward_to_deserialize_any, Deserializer};

use crate::Error;

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Number;

macro_rules! deserialize_any {
    (@expand [$($num_string:tt)*]) => {
        #[inline]
        fn deserialize_any<V>(self, visitor: V) -> Result<V::Value, Error>
        where
            V: Visitor<'de>,
        {
            visitor.visit_unit()
        }

    };

    (owned) => {
        deserialize_any!(@expand [n]);
    };

    (ref) => {
        deserialize_any!(@expand [n.clone()]);
    };
}

macro_rules! deserialize_number {
    ($deserialize:ident => $visit:ident) => {
        fn $deserialize<V>(self, visitor: V) -> Result<V::Value, Error>
        where
            V: Visitor<'de>,
        {
            self.deserialize_any(visitor)
        }
    };
}

impl<'de> Deserializer<'de> for Number {
    type Error = Error;

    deserialize_any!(owned);

    deserialize_number!(deserialize_i8 => visit_i8);

    deserialize_number!(deserialize_i16 => visit_i16);

    deserialize_number!(deserialize_i32 => visit_i32);

    deserialize_number!(deserialize_i64 => visit_i64);

    deserialize_number!(deserialize_i128 => visit_i128);

    deserialize_number!(deserialize_u8 => visit_u8);

    deserialize_number!(deserialize_u16 => visit_u16);

    deserialize_number!(deserialize_u32 => visit_u32);

    deserialize_number!(deserialize_u64 => visit_u64);

    deserialize_number!(deserialize_u128 => visit_u128);

    deserialize_number!(deserialize_f32 => visit_f32);

    deserialize_number!(deserialize_f64 => visit_f64);

    forward_to_deserialize_any! {
        bool char str string bytes byte_buf option unit unit_struct
        newtype_struct seq tuple tuple_struct map struct enum identifier
        ignored_any
    }
}
