//! Wwise soundbank binary parsing helpers.

mod reader;
mod sections;

pub(super) use reader::{read_section_id_at, read_u32_at};
pub(super) use sections::{parse_bank_header, parse_hierarchy, parse_media_index};
