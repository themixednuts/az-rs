use std::fmt;

const fn valid_dotted(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return false;
    }
    let mut index = 0;
    let mut seg_len = 0usize;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'.' {
            if seg_len == 0 {
                return false;
            }
            seg_len = 0;
        } else if byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'_'
        {
            if seg_len == 0 && !byte.is_ascii_lowercase() {
                return false;
            }
            seg_len += 1;
        } else {
            return false;
        }
        index += 1;
    }
    seg_len != 0
}

macro_rules! dotted_id {
    ($name:ident, $kind:literal) => {
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(&'static str);

        impl $name {
            /// Invalid literals fail to compile in const context.
            ///
            /// # Panics
            ///
            /// Panics when `s` is not a dotted lowercase identifier whose
            /// segments start with a letter.
            #[must_use]
            pub const fn new(s: &'static str) -> Self {
                assert!(
                    valid_dotted(s),
                    concat!(
                        "invalid ",
                        $kind,
                        " id: dotted lowercase segments required; each segment must start with a letter"
                    )
                );
                Self(s)
            }

            #[must_use]
            pub const fn as_str(self) -> &'static str {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.0)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, concat!($kind, ":{}"), self.0)
            }
        }
    };
}

dotted_id!(GemId, "gem");
dotted_id!(ContributionId, "contribution");
dotted_id!(CapabilityId, "capability");
dotted_id!(NodeTypeId, "node-type");
dotted_id!(GraphTypeId, "graph-type");
dotted_id!(ClockName, "clock");

#[cfg(test)]
mod tests {
    use super::valid_dotted;

    #[test]
    fn grammar_rejects_uppercase_and_empty_segments() {
        assert!(valid_dotted("azoth.vegetation"));
        assert!(valid_dotted("anti-cheat"));
        assert!(valid_dotted("input-framing"));
        assert!(!valid_dotted(""));
        assert!(!valid_dotted("Azoth.vegetation"));
        assert!(!valid_dotted("azoth..vegetation"));
        assert!(!valid_dotted(".azoth"));
        assert!(!valid_dotted("azoth."));
        assert!(!valid_dotted("1leading"));
        assert!(!valid_dotted("has space"));
    }
}
