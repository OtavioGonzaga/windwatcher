use std::time::Duration;

pub(crate) fn cap_for_attempt(attempt: u32, base: Duration, maximum: Duration) -> Duration {
    let mut cap = base.min(maximum);
    for _ in 1..attempt {
        cap = cap.checked_mul(2).unwrap_or(maximum).min(maximum);
        if cap == maximum {
            break;
        }
    }
    cap
}

pub(crate) fn full_jitter(cap: Duration, random: u64) -> Duration {
    let maximum = cap.as_nanos();
    if maximum == 0 {
        return Duration::ZERO;
    }
    let nanos = u128::from(random) % (maximum + 1);
    Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX))
}

pub(crate) fn retry_delay(attempt: u32, base: Duration, maximum: Duration) -> Duration {
    let cap = cap_for_attempt(attempt.max(1), base, maximum);
    let mut bytes = [0u8; 8];
    let mut rng = rand_core::OsRng;
    let random = rand_core::RngCore::try_fill_bytes(&mut rng, &mut bytes)
        .map(|_| u64::from_ne_bytes(bytes))
        .unwrap_or(0);
    full_jitter(cap, random)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exponential_caps_and_jitter_stay_bounded() {
        let base = Duration::from_secs(1);
        let max = Duration::from_secs(300);
        assert_eq!(cap_for_attempt(1, base, max), base);
        assert_eq!(cap_for_attempt(3, base, max), Duration::from_secs(4));
        assert_eq!(cap_for_attempt(u32::MAX, base, max), max);
        assert_eq!(full_jitter(Duration::from_nanos(20), 0), Duration::ZERO);
        assert!(full_jitter(Duration::from_nanos(20), u64::MAX) <= Duration::from_nanos(20));
    }
}
