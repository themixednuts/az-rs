//! Wwise soundbank metadata.

mod header;
mod hierarchy;
mod media;
mod section;
mod sound_bank;

pub use header::WwiseBankHeader;
pub use hierarchy::parse_event_body;
pub use hierarchy::{
    WwiseEventActionIds, WwiseEventObject, WwiseHierarchyObject, WwiseHierarchyObjectKind,
};
pub use media::WwiseMediaEntry;
pub use section::WwiseBankSection;
pub use sound_bank::WwiseSoundBank;
