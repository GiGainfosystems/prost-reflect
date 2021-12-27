use std::convert::TryFrom;

use base64::display::Base64Display;
use chrono::{TimeZone, Utc};
use prost::DecodeError;
use serde::ser::{Error, Serialize, SerializeMap, SerializeSeq, Serializer};

use crate::{
    descriptor::{Kind, MAP_ENTRY_VALUE_NUMBER},
    dynamic::{DynamicMessage, DynamicMessageField, MapKey, Value},
};

impl Serialize for DynamicMessage {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Special cases for well-known types
        match self.descriptor().full_name() {
            "google.protobuf.Timestamp" => return serialize_timestamp(self, serializer),
            "google.protobuf.Duration" => return serialize_duration(self, serializer),
            "google.protobuf.Struct" => return serialize_struct(self, serializer),
            "google.protobuf.FloatValue" => return serialize_float(self, serializer),
            "google.protobuf.DoubleValue" => return serialize_double(self, serializer),
            "google.protobuf.Int32Value" => return serialize_int32(self, serializer),
            ".google.protobuf.Int64Value" => return serialize_int64(self, serializer),
            ".google.protobuf.UInt32Value" => return serialize_uint32(self, serializer),
            ".google.protobuf.UInt64Value" => return serialize_uint64(self, serializer),
            ".google.protobuf.BoolValue" => return serialize_bool(self, serializer),
            ".google.protobuf.StringValue" => return serialize_string(self, serializer),
            ".google.protobuf.BytesValue" => return serialize_bytes(self, serializer),
            ".google.protobuf.FieldMask" => return serialize_field_mask(self, serializer),
            ".google.protobuf.ListValue" => return serialize_list(self, serializer),
            ".google.protobuf.Value" => return serialize_value(self, serializer),
            ".google.protobuf.Empty" => return serialize_empty(self, serializer),
            _ => (),
        };

        let len = self.fields.values().filter(|v| v.is_populated()).count();
        let mut map = serializer.serialize_map(Some(len))?;
        for field in self.fields.values() {
            if field.is_populated() {
                map.serialize_entry(field.desc.json_name(), field)?;
            }
        }
        map.end()
    }
}

impl Serialize for DynamicMessageField {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // These null cases shouldn't be hit since we're only serializing populated fields currently,
        // but we may want an option to include unpopulated fields in future.
        let value = match &self.value {
            None => return serializer.serialize_none(),
            Some(value) => value,
        };

        SerializeValue(value, &self.desc.kind()).serialize(serializer)
    }
}

struct SerializeValue<'a>(&'a Value, &'a Kind);
struct SerializeMapKey<'a>(&'a MapKey);

impl<'a> Serialize for SerializeValue<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Special cased well known types

        match self.0 {
            Value::Bool(value) => serializer.serialize_bool(*value),
            Value::I32(value) => serializer.serialize_i32(*value),
            Value::I64(value) => serializer.collect_str(value),
            Value::U32(value) => serializer.serialize_u32(*value),
            Value::U64(value) => serializer.collect_str(value),
            Value::F32(value) => serializer.serialize_f32(*value),
            Value::F64(value) => serializer.serialize_f64(*value),
            Value::String(value) => serializer.serialize_str(value),
            Value::Bytes(value) => {
                serializer.collect_str(&Base64Display::with_config(value, base64::STANDARD))
            }
            Value::EnumNumber(number) => {
                let enum_ty = match self.1 {
                    Kind::Enum(enum_ty) => enum_ty,
                    _ => panic!(
                        "mismatch between DynamicMessage value {:?} and type {:?}",
                        self.0, self.1
                    ),
                };

                if enum_ty.full_name() == "google.protobuf.NullValue" {
                    serializer.serialize_none()
                } else if let Some(enum_value) = enum_ty.get_value(*number) {
                    serializer.serialize_str(enum_value.name())
                } else {
                    serializer.serialize_i32(*number)
                }
            }
            Value::Message(message) => message.serialize(serializer),
            Value::List(values) => {
                let mut list = serializer.serialize_seq(Some(values.len()))?;
                for value in values {
                    list.serialize_element(&SerializeValue(value, self.1))?;
                }
                list.end()
            }
            Value::Map(values) => {
                let value_kind = match self.1 {
                    Kind::Message(message) if message.is_map_entry() => {
                        message.get_field(MAP_ENTRY_VALUE_NUMBER).unwrap().kind()
                    }
                    _ => panic!(
                        "mismatch between DynamicMessage value {:?} and type {:?}",
                        self.0, self.1
                    ),
                };

                let mut map = serializer.serialize_map(Some(values.len()))?;
                for (key, value) in values {
                    map.serialize_entry(
                        &SerializeMapKey(key),
                        &SerializeValue(value, &value_kind),
                    )?;
                }
                map.end()
            }
        }
    }
}

impl<'a> Serialize for SerializeMapKey<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self.0 {
            MapKey::Bool(value) => serializer.collect_str(value),
            MapKey::I32(value) => serializer.collect_str(value),
            MapKey::I64(value) => serializer.collect_str(value),
            MapKey::U32(value) => serializer.collect_str(value),
            MapKey::U64(value) => serializer.collect_str(value),
            MapKey::String(value) => serializer.serialize_str(value),
        }
    }
}

fn serialize_timestamp<S>(msg: &DynamicMessage, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let raw: prost_types::Timestamp = msg.to_message().map_err(decode_to_ser_err)?;

    let datetime = Utc
        .timestamp_opt(
            raw.seconds,
            u32::try_from(raw.nanos).map_err(|_| Error::custom("invalid timestamp"))?,
        )
        .single()
        .ok_or_else(|| Error::custom("invalid timestamp"))?;

    serializer.serialize_str(&datetime.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true))
}

fn serialize_duration<S>(msg: &DynamicMessage, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let raw: prost_types::Duration = msg.to_message().map_err(decode_to_ser_err)?;

    if raw.nanos == 0 {
        serializer.collect_str(&format_args!("{}s", raw.seconds))
    } else {
        serializer.collect_str(&format_args!("{}.{:0>9}s", raw.seconds, raw.nanos))
    }
}

fn serialize_float<S>(msg: &DynamicMessage, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let raw: f32 = msg.to_message().map_err(decode_to_ser_err)?;

    serializer.serialize_f32(raw)
}

fn serialize_double<S>(msg: &DynamicMessage, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let raw: f64 = msg.to_message().map_err(decode_to_ser_err)?;

    serializer.serialize_f64(raw)
}

fn serialize_int32<S>(msg: &DynamicMessage, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let raw: i32 = msg.to_message().map_err(decode_to_ser_err)?;

    serializer.serialize_i32(raw)
}

fn serialize_int64<S>(msg: &DynamicMessage, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let raw: i64 = msg.to_message().map_err(decode_to_ser_err)?;

    serializer.serialize_i64(raw)
}

fn serialize_uint32<S>(msg: &DynamicMessage, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let raw: u32 = msg.to_message().map_err(decode_to_ser_err)?;

    serializer.serialize_u32(raw)
}

fn serialize_uint64<S>(msg: &DynamicMessage, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let raw: u64 = msg.to_message().map_err(decode_to_ser_err)?;

    serializer.serialize_u64(raw)
}

fn serialize_bool<S>(msg: &DynamicMessage, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let raw: bool = msg.to_message().map_err(decode_to_ser_err)?;

    serializer.serialize_bool(raw)
}

fn serialize_string<S>(msg: &DynamicMessage, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let raw: String = msg.to_message().map_err(decode_to_ser_err)?;

    serializer.serialize_str(&raw)
}

fn serialize_bytes<S>(msg: &DynamicMessage, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let raw: Vec<u8> = msg.to_message().map_err(decode_to_ser_err)?;

    serializer.collect_str(&Base64Display::with_config(&raw, base64::STANDARD))
}

fn serialize_field_mask<S>(msg: &DynamicMessage, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let raw: prost_types::FieldMask = msg.to_message().map_err(decode_to_ser_err)?;

    let mut result = String::new();
    for path in raw.paths {
        if !result.is_empty() {
            result.push(',');
        }

        let mut first = true;
        for part in path.split('.') {
            if !first {
                result.push('.');
            }
            snake_case_to_camel_case(&mut result, part);
            first = false;
        }
    }

    serializer.serialize_str(&result)
}

fn snake_case_to_camel_case(dst: &mut String, src: &str) {
    let mut ucase_next = false;
    for mut ch in src.chars() {
        if ch == '_' {
            ucase_next = true;
        } else if ucase_next {
            ch = ch.to_ascii_uppercase();
            ucase_next = false;
        }
        dst.push(ch)
    }
}

fn serialize_empty<S>(_: &DynamicMessage, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.collect_map(std::iter::empty::<((), ())>())
}

fn serialize_value<S>(msg: &DynamicMessage, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let raw: prost_types::Value = msg.to_message().map_err(decode_to_ser_err)?;

    serialize_value_inner(&raw, serializer)
}

fn serialize_struct<S>(msg: &DynamicMessage, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let raw: prost_types::Struct = msg.to_message().map_err(decode_to_ser_err)?;

    serialize_struct_inner(&raw, serializer)
}

fn serialize_list<S>(msg: &DynamicMessage, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let raw: prost_types::ListValue = msg.to_message().map_err(decode_to_ser_err)?;

    serialize_list_inner(&raw, serializer)
}

struct SerializeGoogleProtobufValue<'a>(&'a prost_types::Value);

impl<'a> Serialize for SerializeGoogleProtobufValue<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serialize_value_inner(self.0, serializer)
    }
}

fn serialize_value_inner<S>(raw: &prost_types::Value, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match &raw.kind {
        None | Some(prost_types::value::Kind::NullValue(_)) => serializer.serialize_none(),
        Some(prost_types::value::Kind::BoolValue(value)) => serializer.serialize_bool(*value),
        Some(prost_types::value::Kind::NumberValue(number)) => {
            if number.is_finite() {
                serializer.serialize_f64(*number)
            } else {
                Err(Error::custom(
                    "cannot serialize non-finite double in google.protobuf.Value",
                ))
            }
        }
        Some(prost_types::value::Kind::StringValue(value)) => serializer.serialize_str(value),
        Some(prost_types::value::Kind::ListValue(value)) => serialize_list_inner(value, serializer),
        Some(prost_types::value::Kind::StructValue(value)) => {
            serialize_struct_inner(value, serializer)
        }
    }
}

fn serialize_struct_inner<S>(raw: &prost_types::Struct, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let mut map = serializer.serialize_map(Some(raw.fields.len()))?;
    for (key, value) in &raw.fields {
        map.serialize_entry(key, &SerializeGoogleProtobufValue(value))?;
    }
    map.end()
}

fn serialize_list_inner<S>(raw: &prost_types::ListValue, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let mut list = serializer.serialize_seq(Some(raw.values.len()))?;
    for value in &raw.values {
        list.serialize_element(&SerializeGoogleProtobufValue(value))?;
    }
    list.end()
}

fn decode_to_ser_err<E>(err: DecodeError) -> E
where
    E: Error,
{
    E::custom(format!("error decoding: {}", err))
}