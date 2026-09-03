//! `GameData` table assets: wire encode/decode, decoded body, and indexed views.

mod asset;
mod body;
pub(crate) mod encode;
mod marker;
mod section;
#[cfg(test)]
pub(crate) mod view;
mod vlq;

pub use asset::{TableAsset, TableHeader, is_table_asset, parse_table_header, validate_header};
pub(crate) use body::TableBody;
pub use body::{
    AtomType, CellRef, CellType, DEPENDENCY_KIND_FOREIGN_KEY, ListElementType, ListRef, PairType,
    PairValue, RangeBounds, RangeEndpointType, RangeType, ScalarType, TableDependency, TextRef,
};
pub use marker::Row;

#[cfg(all(test, feature = "bevy"))]
pub(crate) use encode::fixture_table_asset_bytes;
