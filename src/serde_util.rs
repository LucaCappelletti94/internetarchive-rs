//! Internal serde helpers for Internet Archive payload quirks.

use serde::de::Error as DeError;
use serde::{Deserialize, Deserializer};
use serde_json::Value;

pub(crate) fn deserialize_option_u64ish<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<Value>::deserialize(deserializer).and_then(|value| match value {
        None | Some(Value::Null) => Ok(None),
        Some(value) => deserialize_u64ish_from_value(value).map(Some),
    })
}

pub(crate) fn normalize_string_list(value: &Value) -> Option<Vec<String>> {
    match value {
        Value::String(text) => Some(vec![text.clone()]),
        Value::Array(values) => values
            .iter()
            .map(|value| value.as_str().map(str::to_owned))
            .collect(),
        _ => None,
    }
}

fn deserialize_u64ish_from_value<E>(value: Value) -> Result<u64, E>
where
    E: DeError,
{
    match value {
        Value::Number(number) => {
            if let Some(value) = number.as_u64() {
                Ok(value)
            } else if let Some(value) = number.as_i64() {
                u64::try_from(value).map_err(|_| E::custom("negative values are not supported"))
            } else if let Some(value) = number.as_f64() {
                whole_non_negative_float_to_u64(value)
            } else {
                Err(E::custom("unsupported numeric shape"))
            }
        }
        Value::String(text) => text.parse::<u64>().or_else(|_| {
            let float = text
                .parse::<f64>()
                .map_err(|_| E::custom(format!("invalid integer-like value: {text}")))?;
            whole_non_negative_float_to_u64(float)
        }),
        _ => Err(E::custom("expected an integer-like value")),
    }
}

fn whole_non_negative_float_to_u64<E>(value: f64) -> Result<u64, E>
where
    E: DeError,
{
    if !value.is_finite() || value < 0.0 || value.fract() != 0.0 {
        return Err(E::custom("expected a whole-number float"));
    }

    format!("{value:.0}")
        .parse()
        .map_err(|_| E::custom("whole-number float is out of range for u64"))
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;
    use serde_json::json;

    use super::{deserialize_option_u64ish, normalize_string_list};

    #[derive(Debug, Deserialize)]
    struct Wrapper {
        #[serde(deserialize_with = "deserialize_option_u64ish")]
        value: Option<u64>,
    }

    #[test]
    fn deserialize_option_u64ish_accepts_common_internet_archive_shapes() {
        assert_eq!(
            serde_json::from_value::<Wrapper>(json!({"value": null}))
                .unwrap()
                .value,
            None
        );
        assert_eq!(
            serde_json::from_value::<Wrapper>(json!({"value": 42}))
                .unwrap()
                .value,
            Some(42)
        );
        assert_eq!(
            serde_json::from_value::<Wrapper>(json!({"value": "42"}))
                .unwrap()
                .value,
            Some(42)
        );
        assert_eq!(
            serde_json::from_value::<Wrapper>(json!({"value": 42.0}))
                .unwrap()
                .value,
            Some(42)
        );
        assert_eq!(
            serde_json::from_value::<Wrapper>(json!({"value": "42.0"}))
                .unwrap()
                .value,
            Some(42)
        );
    }

    #[test]
    fn deserialize_option_u64ish_rejects_invalid_values() {
        assert!(serde_json::from_value::<Wrapper>(json!({"value": -1})).is_err());
        assert!(serde_json::from_value::<Wrapper>(json!({"value": 1.5})).is_err());
        assert!(serde_json::from_value::<Wrapper>(json!({"value": "abc"})).is_err());
        assert!(serde_json::from_value::<Wrapper>(json!({"value": []})).is_err());
    }

    #[test]
    fn normalize_string_list_supports_scalars_and_arrays() {
        assert_eq!(
            normalize_string_list(&json!("eng")),
            Some(vec!["eng".to_owned()])
        );
        assert_eq!(
            normalize_string_list(&json!(["eng", "ita"])),
            Some(vec!["eng".to_owned(), "ita".to_owned()])
        );
        assert_eq!(normalize_string_list(&json!([1, 2, 3])), None);
        assert_eq!(normalize_string_list(&json!({"language": "eng"})), None);
    }
}
