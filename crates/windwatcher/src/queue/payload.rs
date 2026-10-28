use serde::Deserialize;

use crate::JobPayload;

// Persisted payloads are opaque UTF-8 JSON, not database JSON values. JobPayload
// does not impose a nesting limit. Grow the parser stack rather than replacing
// valid deep objects with the invalid-payload placeholder.
pub(super) fn restore(bytes: &[u8]) -> Result<JobPayload, serde_json::Error> {
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    decoder.disable_recursion_limit();
    let value = serde_json::Value::deserialize(serde_stacker::Deserializer::new(&mut decoder))?;
    decoder.end()?;
    // Deserialize the root as a map instead of silently accepting other JSON roots.
    match value {
        serde_json::Value::Object(fields) => Ok(JobPayload::new(fields)),
        _ => Err(<serde_json::Error as serde::de::Error>::custom(
            "payload root is not an object",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_deep_payloads_and_rejects_invalid_documents() {
        let bytes = format!("{{\"nested\":{}null{}}}", "[".repeat(256), "]".repeat(256));
        assert!(restore(bytes.as_bytes()).is_ok());
        for invalid in [
            b"not JSON".as_slice(),
            b"{} trailing",
            b"{}{}",
            b"[]",
            b"{\"value\":\"\xff\"}",
        ] {
            assert!(restore(invalid).is_err());
        }
    }
}
