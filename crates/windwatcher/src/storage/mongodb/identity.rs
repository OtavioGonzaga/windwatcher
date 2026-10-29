use mongodb::bson::{Bson, Document, doc};
use std::str::FromStr;

use time::OffsetDateTime;

use crate::{
    IdentityStore, IdentityStoreError, LocalAuthentication, PasswordHash, RefreshRotationResult,
    RefreshTokenHash, Session, SessionId, SessionStore, SessionStoreError, User, UserId, Username,
};

use super::{MongoDbStorage, transactions};

fn unavailable(error: &mongodb::error::Error) -> bool {
    error.contains_label("TransientTransactionError")
        || error.contains_label("UnknownTransactionCommitResult")
        || matches!(
            error.kind.as_ref(),
            mongodb::error::ErrorKind::Io(_)
                | mongodb::error::ErrorKind::ConnectionPoolCleared { .. }
                | mongodb::error::ErrorKind::ServerSelection { .. }
        )
}

fn identity_error(error: mongodb::error::Error) -> IdentityStoreError {
    if unavailable(&error) {
        IdentityStoreError::Unavailable(Box::new(error))
    } else {
        IdentityStoreError::Unexpected(Box::new(error))
    }
}

fn session_error(error: mongodb::error::Error) -> SessionStoreError {
    if unavailable(&error) {
        SessionStoreError::Unavailable(Box::new(error))
    } else {
        SessionStoreError::Unexpected(Box::new(error))
    }
}

fn identity_corrupt(message: &'static str) -> IdentityStoreError {
    IdentityStoreError::Unexpected(Box::new(std::io::Error::other(message)))
}

fn session_corrupt(message: &'static str) -> SessionStoreError {
    SessionStoreError::Unexpected(Box::new(std::io::Error::other(message)))
}

fn uuid(document: &Document, field: &str) -> Result<uuid::Uuid, &'static str> {
    let Some(Bson::Binary(binary)) = document.get(field) else {
        return Err("persisted UUID is not BSON binary");
    };
    if binary.subtype != mongodb::bson::spec::BinarySubtype::Uuid || binary.bytes.len() != 16 {
        return Err("persisted UUID has invalid subtype or length");
    }
    uuid::Uuid::from_slice(&binary.bytes).map_err(|_| "persisted UUID is malformed")
}

fn id(value: uuid::Uuid) -> mongodb::bson::Binary {
    mongodb::bson::Binary {
        subtype: mongodb::bson::spec::BinarySubtype::Uuid,
        bytes: value.as_bytes().to_vec(),
    }
}

fn timestamp(document: &Document, field: &str) -> Result<OffsetDateTime, &'static str> {
    let value = document
        .get_i64(field)
        .map_err(|_| "persisted timestamp is not BSON Int64")?;
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(value) * 1_000)
        .map_err(|_| "persisted timestamp is out of range")
}

fn optional_timestamp(
    document: &Document,
    field: &str,
) -> Result<Option<OffsetDateTime>, &'static str> {
    match document.get(field) {
        None | Some(Bson::Null) => Ok(None),
        Some(Bson::Int64(value)) => {
            OffsetDateTime::from_unix_timestamp_nanos(i128::from(*value) * 1_000)
                .map(Some)
                .map_err(|_| "persisted timestamp is out of range")
        }
        _ => Err("persisted optional timestamp has invalid BSON type"),
    }
}

fn micros(value: OffsetDateTime) -> Result<i64, &'static str> {
    i64::try_from(value.unix_timestamp_nanos().div_euclid(1_000))
        .map_err(|_| "timestamp out of range")
}

fn read_user(document: &Document) -> Result<User, IdentityStoreError> {
    let id = UserId::try_from(uuid(document, "_id").map_err(identity_corrupt)?)
        .map_err(|_| identity_corrupt("invalid persisted user ID"))?;
    let username = Username::from_str(
        document
            .get_str("username")
            .map_err(|_| identity_corrupt("persisted username is not a string"))?,
    )
    .map_err(|_| identity_corrupt("invalid persisted username"))?;
    Ok(User::restore(
        id,
        username,
        optional_timestamp(document, "disabled_at").map_err(identity_corrupt)?,
        timestamp(document, "created_at").map_err(identity_corrupt)?,
        timestamp(document, "updated_at").map_err(identity_corrupt)?,
    ))
}

fn read_session(document: &Document) -> Result<Session, SessionStoreError> {
    let id = SessionId::try_from(uuid(document, "_id").map_err(session_corrupt)?)
        .map_err(|_| session_corrupt("invalid persisted session ID"))?;
    let user = UserId::try_from(uuid(document, "user_id").map_err(session_corrupt)?)
        .map_err(|_| session_corrupt("invalid persisted user ID"))?;
    let hash = match document.get("refresh_token_hash") {
        Some(Bson::Binary(binary))
            if binary.subtype == mongodb::bson::spec::BinarySubtype::Generic
                && binary.bytes.len() == 32 =>
        {
            binary
        }
        _ => return Err(session_corrupt("invalid persisted refresh-token hash")),
    };
    let _ = hash;
    Session::restore(
        id,
        user,
        timestamp(document, "created_at").map_err(session_corrupt)?,
        timestamp(document, "expires_at").map_err(session_corrupt)?,
        optional_timestamp(document, "revoked_at").map_err(session_corrupt)?,
    )
    .map_err(|_| session_corrupt("invalid persisted session"))
}

fn user_document(user: &User) -> Result<Document, &'static str> {
    Ok(doc! {
        "_id": id(user.id().as_uuid()),
        "username": user.username().as_str(),
        "disabled_at": user.disabled_at().map(micros).transpose()?.map(Bson::Int64).unwrap_or(Bson::Null),
        "created_at": micros(user.created_at())?,
        "updated_at": micros(user.updated_at())?,
        "_ww_revision": 0_i64,
    })
}

fn session_document(
    session: &Session,
    refresh_hash: &RefreshTokenHash,
) -> Result<Document, &'static str> {
    Ok(doc! {
        "_id": id(session.id().as_uuid()),
        "user_id": id(session.user_id().as_uuid()),
        "refresh_token_hash": mongodb::bson::Binary { subtype: mongodb::bson::spec::BinarySubtype::Generic, bytes: refresh_hash.as_bytes().to_vec() },
        "created_at": micros(session.created_at())?,
        "expires_at": micros(session.expires_at())?,
        "revoked_at": session.revoked_at().map(micros).transpose()?.map(Bson::Int64).unwrap_or(Bson::Null),
    })
}

fn duplicate_for(error: &mongodb::error::Error, index: &str) -> bool {
    matches!(error.kind.as_ref(), mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(write)) if write.code == 11000 && write.message.contains(index))
}

impl IdentityStore for MongoDbStorage {
    async fn create_local_user(
        &self,
        user: &User,
        password_hash: &PasswordHash,
    ) -> Result<(), IdentityStoreError> {
        let user_doc = user_document(user).map_err(identity_corrupt)?;
        let credential =
            doc! { "_id": id(user.id().as_uuid()), "password_hash": password_hash.as_encoded() };
        let database = self.database();
        let result = transactions::run(&self.client, false, async move |session| {
            database
                .collection::<Document>("users")
                .insert_one(user_doc.clone())
                .session(&mut *session)
                .await?;
            database
                .collection::<Document>("local_credentials")
                .insert_one(credential.clone())
                .session(session)
                .await?;
            Ok(())
        })
        .await;
        match result {
            Ok(()) => Ok(()),
            Err(error) if duplicate_for(&error, "users_username_unique") => {
                Err(IdentityStoreError::UsernameConflict)
            }
            Err(error) => Err(identity_error(error)),
        }
    }

    async fn find_local_auth_by_username(
        &self,
        username: &Username,
    ) -> Result<Option<LocalAuthentication>, IdentityStoreError> {
        let database = self.database();
        let name = username.as_str().to_owned();
        let result = transactions::run(&self.client, true, async move |session| {
            let user = database
                .collection::<Document>("users")
                .find_one(doc! { "username": &name })
                .session(&mut *session)
                .await?;
            let Some(user) = user else { return Ok(None) };
            let credential = database
                .collection::<Document>("local_credentials")
                .find_one(doc! { "_id": user.get("_id") })
                .session(session)
                .await?;
            Ok(Some((user, credential)))
        })
        .await
        .map_err(identity_error)?;
        let Some((user, credential)) = result else {
            return Ok(None);
        };
        let Some(credential) = credential else {
            return Ok(None);
        };
        let user = read_user(&user)?;
        let encoded = credential
            .get_str("password_hash")
            .map_err(|_| identity_corrupt("persisted password hash is not a string"))?;
        let password_hash = PasswordHash::parse(encoded)
            .map_err(|_| identity_corrupt("invalid persisted password hash"))?;
        Ok(Some(LocalAuthentication::new(user, password_hash)))
    }

    async fn find_user(&self, user_id: UserId) -> Result<Option<User>, IdentityStoreError> {
        self.database()
            .collection::<Document>("users")
            .find_one(doc! { "_id": id(user_id.as_uuid()) })
            .await
            .map_err(identity_error)?
            .as_ref()
            .map(read_user)
            .transpose()
    }

    async fn update_user(&self, user: &User) -> Result<(), IdentityStoreError> {
        let updated_at = micros(user.updated_at()).map_err(identity_corrupt)?;
        let disabled_at = user
            .disabled_at()
            .map(micros)
            .transpose()
            .map_err(identity_corrupt)?;
        let result = self
            .database()
            .collection::<Document>("users")
            .update_one(
                doc! { "_id": id(user.id().as_uuid()) },
                doc! { "$set": {
                    "disabled_at": disabled_at.map(Bson::Int64).unwrap_or(Bson::Null),
                    "updated_at": updated_at,
                } },
            )
            .await
            .map_err(identity_error)?;
        if result.matched_count == 0 {
            Err(IdentityStoreError::NotFound)
        } else {
            Ok(())
        }
    }
}

impl SessionStore for MongoDbStorage {
    async fn find_session(
        &self,
        session_id: SessionId,
    ) -> Result<Option<Session>, SessionStoreError> {
        self.database()
            .collection::<Document>("sessions")
            .find_one(doc! { "_id": id(session_id.as_uuid()) })
            .await
            .map_err(session_error)?
            .as_ref()
            .map(read_session)
            .transpose()
    }

    async fn create_session(
        &self,
        session: &Session,
        refresh_hash: &RefreshTokenHash,
    ) -> Result<(), SessionStoreError> {
        let session_doc = session_document(session, refresh_hash).map_err(session_corrupt)?;
        let user_id = id(session.user_id().as_uuid());
        let database = self.database();
        let outcome = transactions::run(&self.client, false, async move |session_ctx| {
            let touched = database
                .collection::<Document>("users")
                .update_one(
                    doc! { "_id": &user_id },
                    doc! { "$inc": { "_ww_revision": 1_i64 } },
                )
                .session(&mut *session_ctx)
                .await?;
            if touched.matched_count == 0 {
                return Ok(0_u8);
            }
            let user = database
                .collection::<Document>("users")
                .find_one(doc! { "_id": &user_id })
                .session(&mut *session_ctx)
                .await?;
            match user.as_ref().and_then(|user| user.get("disabled_at")) {
                None | Some(Bson::Null) => (),
                Some(Bson::Int64(_)) => return Ok(1_u8),
                Some(_) => return Ok(3_u8),
            }
            database
                .collection::<Document>("sessions")
                .insert_one(session_doc.clone())
                .session(session_ctx)
                .await?;
            Ok(2_u8)
        })
        .await;
        match outcome {
            Ok(0) => Err(SessionStoreError::OwnerNotFound),
            Ok(1) => Err(SessionStoreError::OwnerDisabled),
            Ok(3) => Err(session_corrupt(
                "persisted disabled timestamp has invalid BSON type",
            )),
            Ok(_) => Ok(()),
            Err(error)
                if duplicate_for(&error, "_id_")
                    || duplicate_for(&error, "sessions_refresh_hash_unique") =>
            {
                Err(SessionStoreError::Conflict)
            }
            Err(error) => Err(session_error(error)),
        }
    }

    async fn find_by_refresh_token_hash(
        &self,
        refresh_hash: &RefreshTokenHash,
    ) -> Result<Option<Session>, SessionStoreError> {
        let hash = mongodb::bson::Binary {
            subtype: mongodb::bson::spec::BinarySubtype::Generic,
            bytes: refresh_hash.as_bytes().to_vec(),
        };
        self.database()
            .collection::<Document>("sessions")
            .find_one(doc! { "refresh_token_hash": hash })
            .await
            .map_err(session_error)?
            .as_ref()
            .map(read_session)
            .transpose()
    }

    async fn rotate_refresh_token(
        &self,
        session_id: SessionId,
        old: &RefreshTokenHash,
        new: &RefreshTokenHash,
        now: OffsetDateTime,
    ) -> Result<RefreshRotationResult, SessionStoreError> {
        let old = mongodb::bson::Binary {
            subtype: mongodb::bson::spec::BinarySubtype::Generic,
            bytes: old.as_bytes().to_vec(),
        };
        let new = mongodb::bson::Binary {
            subtype: mongodb::bson::spec::BinarySubtype::Generic,
            bytes: new.as_bytes().to_vec(),
        };
        let now = micros(now).map_err(session_corrupt)?;
        let result = self.database().collection::<Document>("sessions").update_one(
            doc! { "_id": id(session_id.as_uuid()), "refresh_token_hash": old, "revoked_at": Bson::Null, "expires_at": { "$gt": now } },
            doc! { "$set": { "refresh_token_hash": new } },
        ).await.map_err(session_error)?;
        Ok(if result.modified_count == 1 {
            RefreshRotationResult::Rotated
        } else {
            RefreshRotationResult::Rejected
        })
    }

    async fn revoke_session_for_user(
        &self,
        user_id: UserId,
        session_id: SessionId,
        revoked_at: OffsetDateTime,
    ) -> Result<bool, SessionStoreError> {
        let revoked_at = micros(revoked_at).map_err(session_corrupt)?;
        let sessions = self.database().collection::<Document>("sessions");
        let filter = doc! { "_id": id(session_id.as_uuid()), "user_id": id(user_id.as_uuid()) };
        let result = sessions.update_one(doc! { "$and": [filter.clone(), doc! { "$or": [ { "revoked_at": Bson::Null }, { "revoked_at": { "$exists": false } } ] }] }, doc! { "$set": { "revoked_at": revoked_at } }).await.map_err(session_error)?;
        if result.matched_count != 0 {
            return Ok(true);
        }
        Ok(sessions
            .find_one(filter)
            .await
            .map_err(session_error)?
            .is_some())
    }

    async fn revoke_all_for_user(
        &self,
        user_id: UserId,
        revoked_at: OffsetDateTime,
    ) -> Result<(), SessionStoreError> {
        let user_id = id(user_id.as_uuid());
        let revoked_at = micros(revoked_at).map_err(session_corrupt)?;
        let database = self.database();
        transactions::run(&self.client, false, async move |session| {
            database.collection::<Document>("sessions").update_many(
                doc! { "user_id": &user_id, "$or": [ { "revoked_at": Bson::Null }, { "revoked_at": { "$exists": false } } ] },
                doc! { "$set": { "revoked_at": revoked_at } },
            ).session(session).await?;
            Ok(())
        }).await.map_err(session_error)
    }
}
