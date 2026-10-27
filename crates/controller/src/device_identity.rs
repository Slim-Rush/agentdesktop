pub(crate) const MAX_PREVIOUS_DEVICE_ID_BYTES: usize = 128;
pub(crate) const MAX_HOSTNAME_BYTES: usize = 255;

/// The previous device ID is untrusted input. A malformed value is ignored
/// rather than failing the enrollment, so older or newer daemons still enroll.
pub(crate) fn valid_previous_device_id(value: Option<&str>) -> Option<&str> {
    value.filter(|id| {
        !id.is_empty()
            && id.len() <= MAX_PREVIOUS_DEVICE_ID_BYTES
            && id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    })
}

pub(crate) fn valid_hostname(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_HOSTNAME_BYTES
        && !value.trim().is_empty()
        && !value.bytes().any(|byte| byte.is_ascii_control())
}

pub(crate) fn hostnames_match(left: &str, right: &str) -> bool {
    left.trim().eq_ignore_ascii_case(right.trim())
}

#[cfg(test)]
mod tests {
    use super::{hostnames_match, valid_hostname, valid_previous_device_id};

    #[test]
    fn previous_device_ids_are_shape_and_length_bounded() {
        assert_eq!(valid_previous_device_id(Some("device-1")), Some("device-1"));
        assert_eq!(valid_previous_device_id(Some("")), None);
        assert_eq!(valid_previous_device_id(Some("bad id")), None);
        assert_eq!(
            valid_previous_device_id(Some(&"a".repeat(129))),
            None,
            "oversized previous IDs are ignored"
        );
    }

    #[test]
    fn hostnames_are_trimmed_bounded_and_case_insensitive() {
        assert!(valid_hostname(" host "));
        assert!(!valid_hostname("   "));
        assert!(!valid_hostname(&"h".repeat(256)));
        assert!(
            !valid_hostname(&format!("{}host", " ".repeat(300))),
            "whitespace padding must not bypass the raw byte bound"
        );
        assert!(!valid_hostname("host\nname"));
        assert!(hostnames_match("Host", "host "));
        assert!(!hostnames_match("host", "other-host"));
    }
}
