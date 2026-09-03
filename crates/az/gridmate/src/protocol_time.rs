macro_rules! define_protocol_time {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[repr(transparent)]
        #[derive(
            crate::Marshaler,
            Debug,
            Default,
            Clone,
            Copy,
            PartialEq,
            Eq,
            PartialOrd,
            Ord,
            Hash,
        )]
        pub struct $name(u64);

        impl $name {
            #[inline]
            #[must_use]
            pub const fn from_nanoseconds(nanoseconds: u64) -> Self {
                Self(nanoseconds)
            }

            #[inline]
            #[must_use]
            pub const fn as_nanoseconds(self) -> u64 {
                self.0
            }

            #[inline]
            #[must_use]
            pub fn from_std(duration: std::time::Duration) -> Self {
                Self(u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX))
            }
        }

        impl From<u64> for $name {
            fn from(nanoseconds: u64) -> Self {
                Self::from_nanoseconds(nanoseconds)
            }
        }

        impl From<$name> for u64 {
            fn from(value: $name) -> Self {
                value.as_nanoseconds()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(formatter)
            }
        }
    };
}

pub(crate) use define_protocol_time;
