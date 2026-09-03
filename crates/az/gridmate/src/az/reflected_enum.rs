//! Reflected AZ `repr(i32)` enums with type UUID, `GridMate` marshaling, and
//! native-name conversions.
//!
//! This is the shared grammar for inventory slot aliases, sequence-event option
//! enums, and similar AZ reflected integer enums. Call sites keep hardcoded
//! UUIDs and discriminants; the macro only expands the repeated impl surface.

/// Define a reflected AZ `repr(i32)` enum with [`az_derive::AzTypeInfo`],
/// [`crate::Marshaler`], Bevy `Reflect`, and `i32` / native-name conversions.
///
/// ```ignore
/// gridmate::reflected_az_enum! {
///     PaperdollSlotTypes, "PaperdollSlotTypes", "5D42C439-A859-4133-9032-88DE31048F2C" {
///         #[default] Head = 0 => "head",
///         Chest = 1 => "chest",
///     }
/// }
///
/// // Optional: serde as i32 (sequence-event style) instead of variant names.
/// gridmate::reflected_az_enum! {
///     #[serde_as_i32]
///     TrackTimeInActionOptions, "TrackTimeInActionOptions", "29A85965-CF51-49F2-AA64-A3533D73092A" {
///         #[default] Enable = 0 => "Enable",
///         ClearHitTracking = 1 => "ClearHitTracking",
///     }
/// }
/// ```
#[macro_export]
macro_rules! reflected_az_enum {
    (
        $name:ident, $native_name:literal, $uuid:literal {
            $($(#[$variant_meta:meta])* $variant:ident = $value:literal => $native:literal),+ $(,)?
        }
    ) => {
        $crate::reflected_az_enum! {
            @impl
            named
            {}
            {}
            $name, $native_name, $uuid {
                $($(#[$variant_meta])* $variant = $value => $native),+
            }
        }
    };
    (
        #[serde_as_i32]
        $name:ident, $native_name:literal, $uuid:literal {
            $($(#[$variant_meta:meta])* $variant:ident = $value:literal => $native:literal),+ $(,)?
        }
    ) => {
        $crate::reflected_az_enum! {
            @impl
            integer
            { #[serde(try_from = "i32", into = "i32")] }
            { ::serde::Deserialize }
            $name, $native_name, $uuid {
                $($(#[$variant_meta])* $variant = $value => $native),+
            }
        }
    };
    (
        @impl
        $serde_mode:ident
        { $(#[$serde_attr:meta])* }
        { $($deserialize_derive:path),* $(,)? }
        $name:ident, $native_name:literal, $uuid:literal {
            $($(#[$variant_meta:meta])* $variant:ident = $value:literal => $native:literal),+ $(,)?
        }
    ) => {
        // Call sites must import `bevy::reflect::{ReflectDeserialize, ReflectSerialize}`
        // for `#[reflect(Serialize, Deserialize)]`.
        #[derive(
            ::az_derive::AzTypeInfo,
            $crate::Marshaler,
            ::core::fmt::Debug,
            ::core::default::Default,
            ::core::clone::Clone,
            ::core::marker::Copy,
            ::core::cmp::PartialEq,
            ::core::cmp::Eq,
            ::core::cmp::PartialOrd,
            ::core::cmp::Ord,
            ::core::hash::Hash,
            // Resolved at the call site; callers must depend on `bevy`.
            ::bevy::reflect::Reflect,
            ::serde::Serialize,
            $($deserialize_derive,)*
        )]
        #[repr(i32)]
        #[az_type_info(name = $native_name, $uuid)]
        $(#[$serde_attr])*
        #[reflect(Serialize, Deserialize)]
        pub enum $name {
            $($(#[$variant_meta])* $variant = $value),+
        }

        impl $name {
            #[inline]
            #[must_use]
            pub const fn value(self) -> i32 {
                self as i32
            }

            #[inline]
            #[must_use]
            pub const fn native_name(self) -> &'static str {
                match self {
                    $(Self::$variant => $native),+
                }
            }

            /// Compatibility alias for cooked procedural-clip helpers that used
            /// `impl_native_enum!`'s `as_str`.
            #[inline]
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                self.native_name()
            }
        }

        impl ::core::convert::From<$name> for i32 {
            #[inline]
            fn from(value: $name) -> Self {
                value.value()
            }
        }

        impl ::core::convert::TryFrom<i32> for $name {
            type Error = i32;

            #[inline]
            fn try_from(value: i32) -> ::core::result::Result<Self, Self::Error> {
                match value {
                    $($value => ::core::result::Result::Ok(Self::$variant),)+
                    _ => ::core::result::Result::Err(value),
                }
            }
        }

        impl ::core::convert::TryFrom<&str> for $name {
            type Error = ::std::string::String;

            fn try_from(value: &str) -> ::core::result::Result<Self, Self::Error> {
                match value {
                    $($native => ::core::result::Result::Ok(Self::$variant),)+
                    _ => ::core::result::Result::Err(value.to_owned()),
                }
            }
        }

        impl ::core::str::FromStr for $name {
            type Err = ::std::string::String;

            #[inline]
            fn from_str(value: &str) -> ::core::result::Result<Self, Self::Err> {
                Self::try_from(value)
            }
        }

        impl ::core::convert::AsRef<str> for $name {
            #[inline]
            fn as_ref(&self) -> &str {
                self.native_name()
            }
        }

        impl ::core::fmt::Display for $name {
            #[inline]
            fn fmt(&self, formatter: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                formatter.write_str(self.native_name())
            }
        }

        $crate::reflected_az_enum! {
            @deserialize $serde_mode
            $name {
                $($variant = $value => $native),+
            }
        }
    };
    (
        @deserialize integer
        $name:ident {
            $($variant:ident = $value:literal => $native:literal),+ $(,)?
        }
    ) => {};
    (
        @deserialize named
        $name:ident {
            $($variant:ident = $value:literal => $native:literal),+ $(,)?
        }
    ) => {
        impl<'de> ::serde::Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> ::core::result::Result<Self, D::Error>
            where
                D: ::serde::Deserializer<'de>,
            {
                struct ReflectedEnumVisitor;

                impl<'de> ::serde::de::DeserializeSeed<'de> for ReflectedEnumVisitor {
                    type Value = $name;

                    fn deserialize<D>(self, deserializer: D) -> ::core::result::Result<Self::Value, D::Error>
                    where
                        D: ::serde::Deserializer<'de>,
                    {
                        deserializer.deserialize_any(self)
                    }
                }

                impl<'de> ::serde::de::Visitor<'de> for ReflectedEnumVisitor {
                    type Value = $name;

                    fn expecting(
                        &self,
                        formatter: &mut ::core::fmt::Formatter<'_>,
                    ) -> ::core::fmt::Result {
                        formatter.write_str(concat!(
                            "an integer discriminant or variant name for ",
                            stringify!($name),
                        ))
                    }

                    fn visit_i64<E>(self, value: i64) -> ::core::result::Result<Self::Value, E>
                    where
                        E: ::serde::de::Error,
                    {
                        let discriminant = i32::try_from(value).map_err(|_| {
                            E::invalid_value(::serde::de::Unexpected::Signed(value), &self)
                        })?;
                        $name::try_from(discriminant).map_err(|_| {
                            E::invalid_value(::serde::de::Unexpected::Signed(value), &self)
                        })
                    }

                    fn visit_u64<E>(self, value: u64) -> ::core::result::Result<Self::Value, E>
                    where
                        E: ::serde::de::Error,
                    {
                        let discriminant = i32::try_from(value).map_err(|_| {
                            E::invalid_value(::serde::de::Unexpected::Unsigned(value), &self)
                        })?;
                        $name::try_from(discriminant).map_err(|_| {
                            E::invalid_value(::serde::de::Unexpected::Unsigned(value), &self)
                        })
                    }

                    fn visit_str<E>(self, value: &str) -> ::core::result::Result<Self::Value, E>
                    where
                        E: ::serde::de::Error,
                    {
                        $(
                            if value == stringify!($variant) || value == $native {
                                return ::core::result::Result::Ok($name::$variant);
                            }
                        )+
                        ::core::result::Result::Err(E::unknown_variant(
                            value,
                            &[$(stringify!($variant)),+],
                        ))
                    }

                    fn visit_enum<A>(self, data: A) -> ::core::result::Result<Self::Value, A::Error>
                    where
                        A: ::serde::de::EnumAccess<'de>,
                    {
                        let (value, variant) = ::serde::de::EnumAccess::variant_seed(
                            data,
                            ReflectedEnumVisitor,
                        )?;
                        ::serde::de::VariantAccess::unit_variant(variant)?;
                        ::core::result::Result::Ok(value)
                    }
                }

                if ::serde::Deserializer::is_human_readable(&deserializer) {
                    deserializer.deserialize_any(ReflectedEnumVisitor)
                } else {
                    deserializer.deserialize_enum(
                        stringify!($name),
                        &[$(stringify!($variant)),+],
                        ReflectedEnumVisitor,
                    )
                }
            }
        }
    };
}
