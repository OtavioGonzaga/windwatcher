use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};

use crate::{ConversationBoundary, MemberBoundary, UserId};
use crate::{ConversationId, MessageId};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OrderedCursorV1 {
    v: u32,
    kind: String,
    scope: String,
    at_nanos: String,
    id: String,
}

fn encode_ordered(kind: &str, scope: String, at: time::OffsetDateTime, id: String) -> String {
    URL_SAFE_NO_PAD.encode(
        serde_json::to_vec(&OrderedCursorV1 {
            v: 1,
            kind: kind.into(),
            scope,
            at_nanos: at.unix_timestamp_nanos().to_string(),
            id,
        })
        .expect("string cursor serialization"),
    )
}

fn decode_ordered(
    token: &str,
    kind: &str,
    scope: &str,
) -> Result<(time::OffsetDateTime, String), CursorError> {
    let bytes = URL_SAFE_NO_PAD
        .decode(token)
        .map_err(|_| CursorError::Malformed)?;
    let payload: OrderedCursorV1 =
        serde_json::from_slice(&bytes).map_err(|_| CursorError::Malformed)?;
    if payload.v != 1 {
        return Err(CursorError::UnsupportedVersion);
    }
    if payload.kind != kind {
        return Err(CursorError::WrongResource);
    }
    if payload.scope != scope {
        return Err(CursorError::WrongScope);
    }
    let nanos = payload
        .at_nanos
        .parse()
        .map_err(|_| CursorError::Malformed)?;
    let at = time::OffsetDateTime::from_unix_timestamp_nanos(nanos)
        .map_err(|_| CursorError::Malformed)?;
    Ok((at, payload.id))
}

pub(crate) fn encode_conversation(actor: UserId, boundary: ConversationBoundary) -> String {
    encode_ordered(
        "conversations",
        actor.to_string(),
        boundary.activity_at,
        boundary.conversation_id.to_string(),
    )
}

pub(crate) fn decode_conversation(
    token: &str,
    actor: UserId,
) -> Result<ConversationBoundary, CursorError> {
    let (activity_at, id) = decode_ordered(token, "conversations", &actor.to_string())?;
    Ok(ConversationBoundary {
        activity_at,
        conversation_id: id.parse().map_err(|_| CursorError::Malformed)?,
    })
}

pub(crate) fn encode_member(conversation: ConversationId, boundary: MemberBoundary) -> String {
    encode_ordered(
        "members",
        conversation.to_string(),
        boundary.joined_at,
        boundary.user_id.to_string(),
    )
}

pub(crate) fn decode_member(
    token: &str,
    conversation: ConversationId,
) -> Result<MemberBoundary, CursorError> {
    let (joined_at, id) = decode_ordered(token, "members", &conversation.to_string())?;
    Ok(MemberBoundary {
        joined_at,
        user_id: id.parse().map_err(|_| CursorError::Malformed)?,
    })
}

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

    #[test]
    fn ordered_cursors_preserve_precision_and_enforce_kind_scope_and_strictness() {
        let actor = UserId::new();
        let conversation = ConversationId::new();
        let at = time::OffsetDateTime::UNIX_EPOCH + time::Duration::nanoseconds(123456789);
        let boundary = ConversationBoundary {
            activity_at: at,
            conversation_id: conversation,
        };
        let token = encode_conversation(actor, boundary);
        assert_eq!(decode_conversation(&token, actor), Ok(boundary));
        assert_eq!(
            decode_conversation(&token, UserId::new()),
            Err(CursorError::WrongScope)
        );
        assert_eq!(
            decode_member(&token, conversation),
            Err(CursorError::WrongResource)
        );
        let member = MemberBoundary {
            joined_at: at,
            user_id: actor,
        };
        let token = encode_member(conversation, member);
        assert_eq!(decode_member(&token, conversation), Ok(member));
        assert_eq!(
            decode_member(&token, ConversationId::new()),
            Err(CursorError::WrongScope)
        );
        for (field, value, expected) in [
            ("v", serde_json::json!(2), CursorError::UnsupportedVersion),
            ("extra", serde_json::json!(true), CursorError::Malformed),
            (
                "at_nanos",
                serde_json::json!("not a timestamp"),
                CursorError::Malformed,
            ),
            ("id", serde_json::json!("not an ID"), CursorError::Malformed),
        ] {
            let mut payload: serde_json::Value =
                serde_json::from_slice(&URL_SAFE_NO_PAD.decode(&token).unwrap()).unwrap();
            payload[field] = value;
            assert_eq!(
                decode_member(&URL_SAFE_NO_PAD.encode(payload.to_string()), conversation),
                Err(expected)
            );
        }
    }

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
