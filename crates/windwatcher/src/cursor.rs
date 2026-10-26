use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};

use crate::{ConversationId, MessageId};

/// A continuation token could not be used. Errors never contain supplied tokens.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum CursorError {
    /// Invalid encoding, payload or typed identifiers.
    #[error("malformed cursor")]
    Malformed,
    /// The cursor uses an unsupported format version.
    #[error("unsupported cursor version")]
    UnsupportedVersion,
    /// The cursor belongs to another resource kind.
    #[error("cursor resource mismatch")]
    WrongResource,
    /// The cursor belongs to another resource scope.
    #[error("cursor scope mismatch")]
    WrongScope,
}

// Private v1 wire format. Keep decoding compatible with issued tokens.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MessageCursorV1 {
    v: u32,
    kind: String,
    conversation_id: String,
    message_id: String,
}

pub(crate) fn encode_message(conversation: ConversationId, boundary: MessageId) -> String {
    let payload = MessageCursorV1 {
        v: 1,
        kind: "messages".into(),
        conversation_id: conversation.to_string(),
        message_id: boundary.to_string(),
    };
    URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).expect("string cursor serialization"))
}

pub(crate) fn decode_message(
    token: &str,
    conversation: ConversationId,
) -> Result<MessageId, CursorError> {
    let bytes = URL_SAFE_NO_PAD
        .decode(token)
        .map_err(|_| CursorError::Malformed)?;
    let payload: MessageCursorV1 =
        serde_json::from_slice(&bytes).map_err(|_| CursorError::Malformed)?;
    if payload.v != 1 {
        return Err(CursorError::UnsupportedVersion);
    }
    if payload.kind != "messages" {
        return Err(CursorError::WrongResource);
    }
    let scope: ConversationId = payload
        .conversation_id
        .parse()
        .map_err(|_| CursorError::Malformed)?;
    let boundary = payload
        .message_id
        .parse()
        .map_err(|_| CursorError::Malformed)?;
    if scope != conversation {
        return Err(CursorError::WrongScope);
    }
    Ok(boundary)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCOPE: &str = "018f6f34-0000-7000-8000-000000000000";
    const BOUNDARY: &str = "018f6f35-0000-7000-8000-000000000001";
    // Issued v1 compact JSON fixture: independent of the current encoder.
    const V1: &str = "eyJ2IjoxLCJraW5kIjoibWVzc2FnZXMiLCJjb252ZXJzYXRpb25faWQiOiIwMThmNmYzNC0wMDAwLTcwMDAtODAwMC0wMDAwMDAwMDAwMDAiLCJtZXNzYWdlX2lkIjoiMDE4ZjZmMzUtMDAwMC03MDAwLTgwMDAtMDAwMDAwMDAwMDAxIn0";

    #[test]
    fn decodes_issued_v1_and_round_trips_url_safe_tokens() {
        assert_eq!(
            decode_message(V1, SCOPE.parse().unwrap()).unwrap(),
            BOUNDARY.parse().unwrap()
        );
        for _ in 0..100 {
            let scope = ConversationId::new();
            let boundary = MessageId::new();
            let token = encode_message(scope, boundary);
            assert!(!token.contains(['+', '/', '=']));
            assert_eq!(decode_message(&token, scope), Ok(boundary));
        }
    }

    #[test]
    fn malformed_tokens_and_strict_payloads_fail_without_disclosure() {
        let scope = SCOPE.parse().unwrap();
        for token in ["", "garbage", "+/==", "a", "e30=", "_w", "eyJ2Ijo"] {
            assert_eq!(decode_message(token, scope), Err(CursorError::Malformed));
        }
        let valid = serde_json::json!({"v":1,"kind":"messages","conversation_id":SCOPE,"message_id":BOUNDARY});
        for field in ["v", "kind", "conversation_id", "message_id"] {
            let mut payload = valid.clone();
            payload.as_object_mut().unwrap().remove(field);
            assert_eq!(
                decode_message(&URL_SAFE_NO_PAD.encode(payload.to_string()), scope),
                Err(CursorError::Malformed)
            );
        }
        for (field, value, expected) in [
            ("extra", serde_json::json!(true), CursorError::Malformed),
            ("v", serde_json::json!(2), CursorError::UnsupportedVersion),
            (
                "kind",
                serde_json::json!("users"),
                CursorError::WrongResource,
            ),
            (
                "conversation_id",
                serde_json::json!("invalid"),
                CursorError::Malformed,
            ),
            (
                "message_id",
                serde_json::json!("invalid"),
                CursorError::Malformed,
            ),
            (
                "message_id",
                serde_json::json!("550e8400-e29b-41d4-a716-446655440000"),
                CursorError::Malformed,
            ),
            (
                "conversation_id",
                serde_json::json!(ConversationId::new().to_string()),
                CursorError::WrongScope,
            ),
        ] {
            let mut payload = valid.clone();
            payload[field] = value;
            let token = URL_SAFE_NO_PAD.encode(payload.to_string());
            let error = decode_message(&token, scope).unwrap_err();
            assert_eq!(error, expected);
            assert!(!error.to_string().contains(&token));
        }
        for bytes in [b"arbitrary bytes".as_slice(), b"{", b"\xff\x00"] {
            assert_eq!(
                decode_message(&URL_SAFE_NO_PAD.encode(bytes), scope),
                Err(CursorError::Malformed)
            );
        }
    }
}
