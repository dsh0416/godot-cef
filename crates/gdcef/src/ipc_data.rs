use ciborium::value::Value as CborValue;
use godot::builtin::{Array, PackedByteArray, VarDictionary, Variant, VariantType};
use godot::prelude::*;

const MAX_IPC_DATA_BYTES: usize = 8 * 1024 * 1024;
const TYPE_KEY: &str = "__godot_type";
const VALUE_KEY: &str = "__godot_value";

pub fn max_ipc_data_bytes() -> usize {
    MAX_IPC_DATA_BYTES
}

pub fn encode_variant_to_cbor_bytes(value: &Variant) -> Result<Vec<u8>, String> {
    let cbor = variant_to_cbor_value(value)?;
    let mut out = Vec::new();
    ciborium::ser::into_writer(&cbor, &mut out).map_err(|e| format!("CBOR encode failed: {e}"))?;
    Ok(out)
}

pub fn decode_cbor_bytes_to_variant(bytes: &[u8]) -> Result<Variant, String> {
    let cbor: CborValue =
        ciborium::de::from_reader(bytes).map_err(|e| format!("CBOR decode failed: {e}"))?;
    cbor_value_to_variant(&cbor)
}

fn variant_to_cbor_value(value: &Variant) -> Result<CborValue, String> {
    match value.get_type() {
        VariantType::NIL => Ok(CborValue::Null),
        VariantType::BOOL => Ok(CborValue::Bool(value.to::<bool>())),
        VariantType::INT => Ok(CborValue::Integer(value.to::<i64>().into())),
        VariantType::FLOAT => Ok(CborValue::Float(value.to::<f64>())),
        VariantType::STRING => Ok(CborValue::Text(value.to::<GString>().to_string())),
        VariantType::PACKED_BYTE_ARRAY => {
            Ok(CborValue::Bytes(value.to::<PackedByteArray>().to_vec()))
        }
        VariantType::ARRAY => {
            let array = value.to::<Array<Variant>>();
            let mut out = Vec::with_capacity(array.len());
            for element in array.iter_shared() {
                out.push(variant_to_cbor_value(&element)?);
            }
            Ok(CborValue::Array(out))
        }
        VariantType::DICTIONARY => {
            let dict = value.to::<VarDictionary>();
            let mut out = Vec::new();
            for (key, val) in dict.iter_shared() {
                let key_str = if key.get_type() == VariantType::STRING {
                    key.to::<GString>().to_string()
                } else {
                    key.stringify().to_string()
                };
                out.push((CborValue::Text(key_str), variant_to_cbor_value(&val)?));
            }
            Ok(CborValue::Map(out))
        }
        // For broad Variant coverage, preserve unsupported Godot-native types by tagging
        // their string representation. This keeps transport robust without panicking.
        _ => {
            let tagged = vec![
                (
                    CborValue::Text(TYPE_KEY.to_string()),
                    CborValue::Text(value.get_type().ord().to_string()),
                ),
                (
                    CborValue::Text(VALUE_KEY.to_string()),
                    CborValue::Text(value.stringify().to_string()),
                ),
            ];
            Ok(CborValue::Map(tagged))
        }
    }
}

fn cbor_value_to_variant(value: &CborValue) -> Result<Variant, String> {
    match value {
        CborValue::Null => Ok(Variant::nil()),
        CborValue::Bool(v) => Ok(v.to_variant()),
        CborValue::Integer(v) => {
            let int_val = i128::from(*v);
            if int_val < i64::MIN as i128 || int_val > i64::MAX as i128 {
                return Err("Integer out of i64 range".to_string());
            }
            Ok((int_val as i64).to_variant())
        }
        CborValue::Float(v) => Ok(v.to_variant()),
        CborValue::Text(v) => Ok(GString::from(v).to_variant()),
        CborValue::Bytes(v) => Ok(PackedByteArray::from(v.as_slice()).to_variant()),
        CborValue::Array(v) => {
            let mut array = Array::<Variant>::new();
            for element in v {
                array.push(&cbor_value_to_variant(element)?);
            }
            Ok(array.to_variant())
        }
        CborValue::Map(v) => {
            if let Some(restored) = maybe_restore_special_map(v) {
                return Ok(restored);
            }

            let mut dict = VarDictionary::new();
            for (key, val) in v {
                let key_variant = match key {
                    CborValue::Text(text) => GString::from(text).to_variant(),
                    other => cbor_value_to_variant(other)?,
                };
                dict.set(key_variant, cbor_value_to_variant(val)?);
            }
            Ok(dict.to_variant())
        }
        CborValue::Tag(_, inner) => cbor_value_to_variant(inner),
        _ => Err("Unsupported CBOR value".to_string()),
    }
}

fn maybe_restore_special_map(entries: &[(CborValue, CborValue)]) -> Option<Variant> {
    // Only treat this as our internal tagged payload format when the map
    // contains exactly the two sentinel keys. This avoids collisions with
    // user dictionaries that happen to include these keys among others.
    if entries.len() != 2 {
        return None;
    }

    let mut ty = None::<String>;
    let mut val = None::<String>;
    for (k, v) in entries {
        if let CborValue::Text(key) = k {
            if key == TYPE_KEY {
                if let CborValue::Text(s) = v {
                    ty = Some(s.clone());
                }
            } else if key == VALUE_KEY
                && let CborValue::Text(s) = v
            {
                val = Some(s.clone());
            }
        }
    }
    if let (Some(ty), Some(val)) = (ty, val) {
        let mut dict = VarDictionary::new();
        dict.set(TYPE_KEY, GString::from(ty.as_str()).to_variant());
        dict.set(VALUE_KEY, GString::from(val.as_str()).to_variant());
        return Some(dict.to_variant());
    }
    None
}
