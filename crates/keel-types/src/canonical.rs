//! RFC 8785-ish canonical JSON for objects/arrays/strings/integers.
//! Floats are rejected: protocol documents must not use them as hashed fields.

use serde::Serialize;
use serde_json::{Map, Value};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ContentIdError {
    #[error("serde: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("floats are not allowed in hashed Keel documents")]
    Float,
}

pub fn canonical_json<T: Serialize>(value: &T) -> Result<Vec<u8>, ContentIdError> {
    let v = serde_json::to_value(value)?;
    let mut out = Vec::new();
    write_canonical(&v, &mut out)?;
    Ok(out)
}

fn write_canonical(v: &Value, out: &mut Vec<u8>) -> Result<(), ContentIdError> {
    match v {
        Value::Null => out.extend_from_slice(b"null"),
        Value::Bool(true) => out.extend_from_slice(b"true"),
        Value::Bool(false) => out.extend_from_slice(b"false"),
        Value::Number(n) => {
            if n.as_f64().is_some() && n.as_i64().is_none() && n.as_u64().is_none() {
                return Err(ContentIdError::Float);
            }
            out.extend_from_slice(n.to_string().as_bytes());
        }
        Value::String(s) => {
            let encoded = serde_json::to_vec(s)?;
            out.extend_from_slice(&encoded);
        }
        Value::Array(items) => {
            out.push(b'[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write_canonical(item, out)?;
            }
            out.push(b']');
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push(b'{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                let encoded_key = serde_json::to_vec(k)?;
                out.extend_from_slice(&encoded_key);
                out.push(b':');
                write_canonical(map.get(*k).expect("key from map"), out)?;
            }
            out.push(b'}');
        }
    }
    Ok(())
}

/// Drop a top-level object key before hashing (e.g. `signatures` on a manifest).
pub fn without_key(mut v: Value, key: &str) -> Value {
    if let Value::Object(ref mut map) = v {
        map.remove(key);
    }
    v
}

pub fn object_from_serialize<T: Serialize>(value: &T) -> Result<Map<String, Value>, ContentIdError> {
    match serde_json::to_value(value)? {
        Value::Object(m) => Ok(m),
        other => Ok({
            let mut m = Map::new();
            m.insert("value".into(), other);
            m
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Serialize;

    #[derive(Serialize)]
    struct Doc {
        b: u8,
        a: u8,
    }

    #[test]
    fn sorts_object_keys() {
        let bytes = canonical_json(&Doc { b: 2, a: 1 }).unwrap();
        assert_eq!(bytes, br#"{"a":1,"b":2}"#);
    }
}
