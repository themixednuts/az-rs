use std::collections::VecDeque;

use az_lua_bytecode::{
    bytecode::OpcodeTable,
    chunk::Proto,
    ir::{SsaFunction, SsaOp, build_ssa, dump::dump_function},
    parse_chunk, ssa_dump,
    version::LuaTarget,
};

const SYNTHETIC_BRANCH: &[u8] = include_bytes!("fixtures/control_flow/if_else_phi.luac");

#[test]
fn builds_ssa_for_synthetic_branch_with_well_formed_cfg() {
    let chunk = parse_chunk(SYNTHETIC_BRANCH).expect("synthetic chunk parses");
    let table = lua51_table();
    let mut proto_count = 0;
    let mut phi_count = 0;

    visit_protos(&chunk.root, &mut |proto| {
        let function = build_ssa(proto, &table);
        assert_well_formed_cfg(&function);
        for block in &function.blocks {
            for node in &block.nodes {
                if matches!(node.op, SsaOp::Phi { .. }) {
                    assert!(
                        block.preds.len() >= 2,
                        "phi in BB{} with fewer than two predecessors",
                        block.index
                    );
                    phi_count += 1;
                }
            }
        }
        proto_count += 1;
    });

    assert_eq!(proto_count, 1);
    assert!(phi_count > 0);
}

#[test]
fn ssa_dump_for_synthetic_branch_is_deterministic() {
    let first = ssa_dump(SYNTHETIC_BRANCH).expect("first SSA dump succeeds");
    let second = ssa_dump(SYNTHETIC_BRANCH).expect("second SSA dump succeeds");

    assert_eq!(first, second);
    assert!(first.contains("-- SSA Dump --"));
    assert!(first.contains("== ssa function"));
}

#[test]
fn synthetic_branch_ssa_dump_matches_snapshot() {
    let chunk = parse_chunk(SYNTHETIC_BRANCH).expect("synthetic chunk parses");
    let table = lua51_table();
    let function = build_ssa(&chunk.root, &table);
    let dump = dump_function(&function);

    let expected = r"== ssa function @src/if_else_phi.lua:0..0 ==
   params=0 is_vararg=2 maxstack=2 blocks=5
BB0 [pc 0..2] preds:[] succs:[1,2] idom=-1
  [   0] LOADK R0_1 := K0
  [   1] LOADNIL R1_1 := R1..R1
  [   2] BRANCH [<] inv=false K1 R0_1 true=4 false=3
BB1 [pc 3..3] preds:[0] succs:[3] idom=0
  [   3] JUMP target=6
BB2 [pc 4..5] preds:[0] succs:[4] idom=0
  [   4] LOADK R1_3 := K2
  [   5] JUMP target=7
BB3 [pc 6..6] preds:[1] succs:[4] idom=1
  [   6] LOADK R1_2 := K3
BB4 [pc 7..7] preds:[2,3] succs:[] idom=0
  [   7] PHI R1_4 := phi(R1_3 from BB2, R1_2 from BB3)
  [   7] RETURN base=R1 count=2 values:[R1_4]

";
    assert_eq!(dump, expected);
}

fn lua51_table() -> OpcodeTable {
    OpcodeTable::builtin(LuaTarget::V51)
}

fn visit_protos(proto: &Proto, visitor: &mut impl FnMut(&Proto)) {
    visitor(proto);
    for nested in &proto.protos {
        visit_protos(nested, visitor);
    }
}

fn assert_well_formed_cfg(function: &SsaFunction) {
    if function.blocks.is_empty() {
        return;
    }

    assert_eq!(function.blocks[0].index, 0);
    for block in &function.blocks {
        if block.index != 0 {
            assert!(
                !block.preds.is_empty(),
                "function {}..{} BB{} [pc {}..{}] has no predecessors; blocks={:?}",
                function.line_defined,
                function.last_line_defined,
                block.index,
                block.start_pc,
                block.end_pc,
                function
                    .blocks
                    .iter()
                    .map(|block| (
                        block.index,
                        block.start_pc,
                        block.end_pc,
                        &block.preds,
                        &block.succs
                    ))
                    .collect::<Vec<_>>()
            );
            assert!(
                block.idom.is_some(),
                "BB{} has no immediate dominator",
                block.index
            );
        }
        for &succ in &block.succs {
            assert!(succ < function.blocks.len());
            assert!(
                function.blocks[succ].preds.contains(&block.index),
                "BB{} -> BB{} missing reverse pred",
                block.index,
                succ
            );
        }
        for &pred in &block.preds {
            assert!(pred < function.blocks.len());
            assert!(
                function.blocks[pred].succs.contains(&block.index),
                "BB{} <- BB{} missing reverse succ",
                block.index,
                pred
            );
        }
    }

    let mut seen = vec![false; function.blocks.len()];
    let mut queue = VecDeque::from([0]);
    seen[0] = true;
    while let Some(block) = queue.pop_front() {
        for &succ in &function.blocks[block].succs {
            if !seen[succ] {
                seen[succ] = true;
                queue.push_back(succ);
            }
        }
    }
    assert!(
        seen.iter().all(|reachable| *reachable),
        "all blocks should be reachable from entry"
    );
}
