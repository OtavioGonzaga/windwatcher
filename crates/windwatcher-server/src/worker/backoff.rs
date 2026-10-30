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

pub(crate) fn full_jitter(cap: Duration, random: u128) -> Duration {
    let maximum = cap.as_nanos();
    if maximum == 0 {
        return Duration::ZERO;
    }
    let nanos = random % (maximum + 1);
    let seconds = u64::try_from(nanos / 1_000_000_000).unwrap_or(u64::MAX);
    let subsecond = (nanos % 1_000_000_000) as u32;
    Duration::new(seconds, subsecond)
}

pub(crate) fn retry_delay(attempt: u32, base: Duration, maximum: Duration) -> Duration {
    let cap = cap_for_attempt(attempt.max(1), base, maximum);
    let mut bytes = [0u8; 16];
    let mut rng = getrandom::SysRng;
    let random = rand_core::TryRng::try_fill_bytes(&mut rng, &mut bytes)
        .map(|_| u128::from_ne_bytes(bytes))
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
        assert!(full_jitter(Duration::from_nanos(20), u128::MAX) <= Duration::from_nanos(20));
        let enormous = Duration::MAX;
        assert!(full_jitter(enormous, u128::MAX) <= enormous);
    }
}
