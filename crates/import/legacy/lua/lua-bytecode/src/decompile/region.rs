//! Linear region fragments used inside structured control-flow regions.

use super::analysis::NodeId;

/// A straight-line stream of SSA nodes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinearRegion {
    pub nodes: Vec<NodeId>,
    pub covered_blocks: Vec<usize>,
}
