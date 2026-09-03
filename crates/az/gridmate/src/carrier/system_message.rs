//! System message IDs.
//!
//! The wire protocol reserves these values for connection control and
//! acknowledgements. They ride on [`super::SYSTEM_CHANNEL`] and never surface
//! to application code.
//!
//! Lumberyard reference:
//! `dev/Code/Framework/GridMate/GridMate/Carrier/Carrier.cpp` (`SystemMessageId`).

pub const SM_CONNECT_REQUEST: u8 = 1;
pub const SM_CONNECT_ACK: u8 = 2;
pub const SM_DISCONNECT: u8 = 3;
pub const SM_CLOCK_SYNC: u8 = 4;
/// Marker — carrier-thread message ids are `> SM_CT_FIRST`.
pub const SM_CT_FIRST: u8 = 5;
pub const SM_CT_ACKS: u8 = 6;
pub const SM_CT_CONN_CONTROL: u8 = 7;
pub const SM_CT_BANDWIDTH: u8 = 8;
