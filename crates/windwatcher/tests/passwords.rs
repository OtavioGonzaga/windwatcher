use futures_executor::block_on;
use windwatcher::{
    Argon2idPasswordHasher, PasswordHash, PasswordHashError, PasswordHasher, PasswordPolicyError,
    SecretPassword,
};

fn secret(value: &str) -> SecretPassword {
    SecretPassword::new(value.to_owned()).unwrap()
}

#[test]
fn password_policy_counts_scalars_and_bytes_without_modifying_input() {
    for value in ["1234567", "ééééééé", ""] {
        assert_eq!(
            secret(value).validate_new_password(),
            Err(PasswordPolicyError::TooShort)
        );
    }
    for value in [
        "12345678",
        "éééééééé",
        "password",
        " PASSWORD ",
        "!@#$%^&*",
        "        ",
        "e\u{301}e\u{301}e\u{301}e\u{301}",
    ] {
        let password = secret(value);
        assert_eq!(password.validate_new_password(), Ok(()));
        assert_eq!(password.expose_secret(), value);
    }
    for value in ["a".repeat(1024), "é".repeat(512)] {
        assert!(
            SecretPassword::new(value)
                .unwrap()
                .validate_new_password()
                .is_ok()
        );
    }
    for value in ["a".repeat(1025), format!("{}a", "é".repeat(512))] {
        assert!(matches!(
            SecretPassword::new(value),
            Err(PasswordPolicyError::TooLong)
        ));
    }
    assert_ne!(
        secret(" password").expose_secret(),
        secret("password ").expose_secret()
    );
    assert!(!format!("{:?}", secret("distinct secret")).contains("distinct secret"));
}

#[test]
fn argon2_profile_salts_verification_and_redaction() {
    block_on(async {
        let hasher = Argon2idPasswordHasher;
        let password = secret(" Case-sensitive é! ");
        let first = hasher.hash(&password).await.unwrap();
        let second = hasher.hash(&password).await.unwrap();
        let parsed = argon2::password_hash::PasswordHash::new(first.as_encoded()).unwrap();
        assert_eq!(parsed.algorithm.as_str(), "argon2id");
        assert_eq!(parsed.version, Some(19));
        assert_eq!(parsed.params.get_decimal("m"), Some(19456));
        assert_eq!(parsed.params.get_decimal("t"), Some(2));
        assert_eq!(parsed.params.get_decimal("p"), Some(1));
        assert_eq!(parsed.hash.unwrap().len(), 32);
        let other = argon2::password_hash::PasswordHash::new(second.as_encoded()).unwrap();
        assert_ne!(parsed.salt, other.salt);
        assert_ne!(first.as_encoded(), second.as_encoded());
        let restored = PasswordHash::parse(first.as_encoded()).unwrap();
        assert!(hasher.verify(&password, Some(&restored)).await.unwrap());
        assert!(
            !hasher
                .verify(&secret(" Case-sensitive é!"), Some(&restored))
                .await
                .unwrap()
        );
        assert!(!hasher.verify(&password, None).await.unwrap());
        let debug = format!("{first:?}");
        assert!(!debug.contains(first.as_encoded()));
        assert!(!debug.contains(&parsed.hash.unwrap().to_string()));
        assert!(!debug.contains(password.expose_secret()));
    });
}

#[test]
fn malformed_or_incomplete_persisted_hashes_are_rejected() {
    for encoded in [
        "",
        "not a hash",
        "$argon2id$v=19$m=19456,t=2,p=1",
        "$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ",
        "$argon2id$v=19$m=bad$c2FsdHNhbHQ$!!!",
    ] {
        assert!(matches!(
            PasswordHash::parse(encoded),
            Err(PasswordHashError::InvalidEncodedHash(_))
        ));
    }
}

#[test]
fn verification_uses_persisted_costs_and_reports_unsupported_hashes() {
    use argon2::password_hash::PasswordHasher as _;
    block_on(async {
        let password = secret("short"); // Old credentials do not inherit registration policy.
        let engine = argon2::Argon2::new(
            argon2::Algorithm::Argon2id,
            argon2::Version::V0x13,
            argon2::Params::new(32, 1, 1, Some(16)).unwrap(),
        );
        let salt = argon2::password_hash::SaltString::encode_b64(b"test-only-salt!!").unwrap();
        let encoded = engine
            .hash_password(password.expose_secret().as_bytes(), &salt)
            .unwrap()
            .to_string();
        let hash = PasswordHash::parse(&encoded).unwrap();
        assert!(
            Argon2idPasswordHasher
                .verify(&password, Some(&hash))
                .await
                .unwrap()
        );
        for encoded in [
            encoded.replace("argon2id", "unsupported"),
            encoded.replace("m=32", "m=1"),
        ] {
            let hash = PasswordHash::parse(&encoded).unwrap();
            assert!(matches!(
                Argon2idPasswordHasher.verify(&password, Some(&hash)).await,
                Err(PasswordHashError::VerificationFailed(_))
            ));
        }
    });
}
