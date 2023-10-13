use std::time::Duration;

use serde::{de::Error, Deserialize, Deserializer};

pub(crate) fn into_duration_ms<'de, D>(deserializer: D) -> std::result::Result<Duration, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Number::deserialize(deserializer)?;
    match value.as_u64() {
        Some(i) => Ok(Duration::from_millis(i)),
        None => Err(Error::custom("cannot convert value to u64")),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn into_duration_ms_ok() {
        let input = json!(123);

        let got = into_duration_ms(input).expect("value should be deserialized");

        assert_eq!(got, Duration::from_millis(123));
    }
}
