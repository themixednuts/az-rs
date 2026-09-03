//! Lift decoded Lua instructions into unversioned SSA nodes.

use crate::{
    bytecode::{Instruction, OpArgMode, OpcodeTable, OperandSlot, SemanticOp, opinfo},
    chunk::Proto,
};

use super::{BasicBlock, BinOp, LoopControl, RelOp, SsaNode, SsaOp, SsaRef, UnOp, UpvalueCapture};

/// Lift all basic blocks in-place.
pub fn lift_all(
    proto: &Proto,
    table: &OpcodeTable,
    instructions: &[Instruction],
    blocks: &mut [BasicBlock],
) {
    for block in blocks {
        lift_block(proto, table, instructions, block);
    }
}

fn lift_block(
    proto: &Proto,
    table: &OpcodeTable,
    instructions: &[Instruction],
    block: &mut BasicBlock,
) {
    let mut nodes = Vec::with_capacity(block.end_pc - block.start_pc + 1);
    let mut pc = block.start_pc;
    while pc <= block.end_pc {
        let inst = instructions[pc];
        let line = line_number(proto, pc);
        match inst.op {
            // These four read the nodes lifted so far, or consume trailing
            // instruction words, so they cannot be folded into `lift_operation`.
            SemanticOp::Call => nodes.push(lift_call(pc, line, inst, &nodes)),
            SemanticOp::TailCall => nodes.push(lift_tailcall(pc, line, inst, &nodes)),
            SemanticOp::Return => nodes.push(lift_return(pc, line, inst, &nodes)),
            SemanticOp::SetList => {
                pc += lift_setlist(instructions, block.end_pc, pc, line, inst, &mut nodes);
            }
            SemanticOp::Closure => {
                pc += lift_closure(
                    proto,
                    instructions,
                    block.end_pc,
                    pc,
                    line,
                    inst,
                    &mut nodes,
                );
            }
            _ => nodes.push(lift_operation(table, inst, pc, line)),
        }
        pc += 1;
    }
    block.nodes = nodes;
}

/// Lift one instruction that maps to exactly one SSA node with no lookback
/// into the nodes lifted so far and no trailing instruction words.
fn lift_operation(table: &OpcodeTable, inst: Instruction, pc: usize, line: i32) -> SsaNode {
    lift_load_operation(table, inst, pc, line)
        .or_else(|| lift_table_operation(table, inst, pc, line))
        .or_else(|| lift_arithmetic_operation(table, inst, pc, line))
        .or_else(|| lift_branch_operation(table, inst, pc, line))
        .or_else(|| lift_loop_operation(inst, pc, line))
        // `Unknown`, the caller-handled ops, and every op with no SSA form
        // lift to a no-op that still holds the program counter and line.
        .unwrap_or_else(|| SsaNode::new(pc_to_i32(pc), line, SsaOp::Nop))
}

/// Register moves, constant loads, and global/upvalue reads.
fn lift_load_operation(
    table: &OpcodeTable,
    inst: Instruction,
    pc: usize,
    line: i32,
) -> Option<SsaNode> {
    let pc_i32 = pc_to_i32(pc);
    let node = match inst.op {
        SemanticOp::Move => SsaNode::with_dest(
            pc_i32,
            line,
            reg_ref(inst.a),
            SsaOp::Move {
                src: operand_ref(table, inst, OperandSlot::B),
            },
        ),
        SemanticOp::LoadK => SsaNode::with_dest(
            pc_i32,
            line,
            reg_ref(inst.a),
            SsaOp::LoadK {
                idx: const_index(inst.bx),
            },
        ),
        SemanticOp::LoadBool => SsaNode::with_dest(
            pc_i32,
            line,
            reg_ref(inst.a),
            SsaOp::LoadBool {
                value: inst.b != 0,
                skip_next: inst.c != 0,
            },
        ),
        SemanticOp::LoadNil => SsaNode::with_dest(
            pc_i32,
            line,
            reg_ref(inst.a),
            SsaOp::LoadNil {
                start: reg_index(inst.a),
                end: reg_index(inst.b),
            },
        ),
        SemanticOp::GetUpval => SsaNode::with_dest(
            pc_i32,
            line,
            reg_ref(inst.a),
            SsaOp::GetUpval {
                upval: reg_index(inst.b),
            },
        ),
        SemanticOp::GetGlobal => SsaNode::with_dest(
            pc_i32,
            line,
            reg_ref(inst.a),
            SsaOp::GetGlobal {
                idx: const_index(inst.bx),
            },
        ),
        _ => return None,
    };
    Some(node)
}

/// Table reads and writes, table construction, and method lookup.
fn lift_table_operation(
    table: &OpcodeTable,
    inst: Instruction,
    pc: usize,
    line: i32,
) -> Option<SsaNode> {
    let pc_i32 = pc_to_i32(pc);
    let node = match inst.op {
        SemanticOp::GetTable => SsaNode::with_dest(
            pc_i32,
            line,
            reg_ref(inst.a),
            SsaOp::GetTable {
                table: operand_ref(table, inst, OperandSlot::B),
                key: operand_ref(table, inst, OperandSlot::C),
            },
        ),
        SemanticOp::SetGlobal => SsaNode::new(
            pc_i32,
            line,
            SsaOp::SetGlobal {
                src: reg_ref(inst.a),
                idx: const_index(inst.bx),
            },
        ),
        SemanticOp::SetUpval => SsaNode::new(
            pc_i32,
            line,
            SsaOp::SetUpval {
                src: reg_ref(inst.a),
                upval: reg_index(inst.b),
            },
        ),
        SemanticOp::SetTable => SsaNode::new(
            pc_i32,
            line,
            SsaOp::SetTable {
                table: reg_ref(inst.a),
                key: operand_ref(table, inst, OperandSlot::B),
                value: operand_ref(table, inst, OperandSlot::C),
            },
        ),
        SemanticOp::NewTable => SsaNode::with_dest(
            pc_i32,
            line,
            reg_ref(inst.a),
            SsaOp::NewTable {
                array_hint: super::TableSizeHint::from_encoded(
                    u16::try_from(inst.b).expect("decoded B operand fits in u16"),
                ),
                hash_hint: super::TableSizeHint::from_encoded(
                    u16::try_from(inst.c).expect("decoded C operand fits in u16"),
                ),
            },
        ),
        SemanticOp::SelfOp => SsaNode::with_dest(
            pc_i32,
            line,
            reg_ref(inst.a),
            SsaOp::SelfOp {
                table: reg_ref(inst.b),
                key: operand_ref(table, inst, OperandSlot::C),
                self_reg: reg_index(inst.a + 1),
            },
        ),
        _ => return None,
    };
    Some(node)
}

/// Binary and unary arithmetic, concatenation, and unconditional jumps.
fn lift_arithmetic_operation(
    table: &OpcodeTable,
    inst: Instruction,
    pc: usize,
    line: i32,
) -> Option<SsaNode> {
    let pc_i32 = pc_to_i32(pc);
    let node = match inst.op {
        SemanticOp::Add
        | SemanticOp::Sub
        | SemanticOp::Mul
        | SemanticOp::Div
        | SemanticOp::Mod
        | SemanticOp::Pow
        | SemanticOp::Idiv
        | SemanticOp::Band
        | SemanticOp::Bor
        | SemanticOp::Bxor
        | SemanticOp::Shl
        | SemanticOp::Shr => SsaNode::with_dest(
            pc_i32,
            line,
            reg_ref(inst.a),
            SsaOp::BinOp {
                op: bin_op(inst.op),
                left: operand_ref(table, inst, OperandSlot::B),
                right: operand_ref(table, inst, OperandSlot::C),
            },
        ),
        SemanticOp::Unm | SemanticOp::Not | SemanticOp::Len | SemanticOp::Bnot => {
            SsaNode::with_dest(
                pc_i32,
                line,
                reg_ref(inst.a),
                SsaOp::UnOp {
                    op: un_op(inst.op),
                    value: operand_ref(table, inst, OperandSlot::B),
                },
            )
        }
        SemanticOp::Concat => SsaNode::with_dest(
            pc_i32,
            line,
            reg_ref(inst.a),
            SsaOp::Concat {
                operands: (inst.b..=inst.c).map(reg_ref).collect(),
            },
        ),
        SemanticOp::Jmp => SsaNode::new(
            pc_i32,
            line,
            SsaOp::Jump {
                target: jump_target(pc, inst.sbx),
            },
        ),
        _ => return None,
    };
    Some(node)
}

/// Comparisons and the test opcodes, which lower to two-way branches.
fn lift_branch_operation(
    table: &OpcodeTable,
    inst: Instruction,
    pc: usize,
    line: i32,
) -> Option<SsaNode> {
    let pc_i32 = pc_to_i32(pc);
    let node = match inst.op {
        SemanticOp::Eq | SemanticOp::Lt | SemanticOp::Le => SsaNode::new(
            pc_i32,
            line,
            SsaOp::Branch {
                rel: rel_op(inst.op),
                a: operand_ref(table, inst, OperandSlot::B),
                b: operand_ref(table, inst, OperandSlot::C),
                invert: inst.a != 0,
                t_true: pc_to_i32(pc + 2),
                t_false: pc_to_i32(pc + 1),
            },
        ),
        SemanticOp::Test => SsaNode::new(
            pc_i32,
            line,
            SsaOp::Branch {
                rel: RelOp::Test,
                a: reg_ref(inst.a),
                b: SsaRef::None,
                invert: inst.c != 0,
                t_true: pc_to_i32(pc + 2),
                t_false: pc_to_i32(pc + 1),
            },
        ),
        SemanticOp::TestSet => SsaNode::with_dest(
            pc_i32,
            line,
            reg_ref(inst.a),
            SsaOp::Branch {
                rel: RelOp::TestSet,
                a: reg_ref(inst.b),
                b: SsaRef::None,
                invert: inst.c != 0,
                t_true: pc_to_i32(pc + 2),
                t_false: pc_to_i32(pc + 1),
            },
        ),
        _ => return None,
    };
    Some(node)
}

/// Loop headers, upvalue closing, and varargs.
fn lift_loop_operation(inst: Instruction, pc: usize, line: i32) -> Option<SsaNode> {
    let pc_i32 = pc_to_i32(pc);
    let node = match inst.op {
        SemanticOp::ForLoop => SsaNode::with_dest(
            pc_i32,
            line,
            reg_ref(inst.a + 3),
            SsaOp::ForLoop {
                control: LoopControl::from_base(reg_index(inst.a)),
                target: jump_target(pc, inst.sbx),
            },
        ),
        SemanticOp::ForPrep => SsaNode::with_dest(
            pc_i32,
            line,
            reg_ref(inst.a),
            SsaOp::ForPrep {
                control: LoopControl::from_base(reg_index(inst.a)),
                target: jump_target(pc, inst.sbx),
            },
        ),
        SemanticOp::TForLoop => SsaNode::new(
            pc_i32,
            line,
            SsaOp::TForLoop {
                control: LoopControl::from_base(reg_index(inst.a)),
                count: inst.c,
            },
        ),
        SemanticOp::Close => SsaNode::new(
            pc_i32,
            line,
            SsaOp::Close {
                base: reg_index(inst.a),
            },
        ),
        SemanticOp::VarArg => SsaNode::with_dest(
            pc_i32,
            line,
            reg_ref(inst.a),
            SsaOp::VarArg {
                base: reg_index(inst.a),
                count: inst.b,
            },
        ),
        _ => return None,
    };
    Some(node)
}

/// Lift `SETLIST`, which stores a trailing batch index in the next
/// instruction word when its `C` operand is zero.
///
/// Returns how many extra instruction words were consumed.
fn lift_setlist(
    instructions: &[Instruction],
    block_end_pc: usize,
    pc: usize,
    line: i32,
    inst: Instruction,
    nodes: &mut Vec<SsaNode>,
) -> usize {
    let batch = if inst.c == 0 {
        instructions
            .get(pc + 1)
            .and_then(|extra| i32::try_from(extra.raw).ok())
            .unwrap_or(inst.c)
    } else {
        inst.c
    };
    let values = setlist_values(inst.a, inst.b, nodes);
    nodes.push(SsaNode::new(
        pc_to_i32(pc),
        line,
        SsaOp::SetList {
            table: reg_ref(inst.a),
            values,
            base: reg_index(inst.a),
            count: inst.b,
            batch,
        },
    ));

    if inst.c == 0 && pc < block_end_pc {
        nodes.push(SsaNode::new(pc_to_i32(pc + 1), -1, SsaOp::Nop));
        return 1;
    }
    0
}

/// Lift `CLOSURE` together with the pseudo-instructions that follow it, one
/// per upvalue of the nested prototype.
///
/// Returns how many extra instruction words were consumed.
fn lift_closure(
    proto: &Proto,
    instructions: &[Instruction],
    block_end_pc: usize,
    pc: usize,
    line: i32,
    inst: Instruction,
    nodes: &mut Vec<SsaNode>,
) -> usize {
    let upvalues = closure_upvalues(proto, instructions, pc, inst.bx);
    nodes.push(SsaNode::with_dest(
        pc_to_i32(pc),
        line,
        reg_ref(inst.a),
        SsaOp::Closure {
            proto: const_index(inst.bx),
            upvalues,
        },
    ));

    let Some(nested) = usize::try_from(inst.bx)
        .ok()
        .and_then(|idx| proto.protos.get(idx))
    else {
        return 0;
    };

    let mut consumed = 0usize;
    for _ in 0..nested.nups {
        consumed += 1;
        if pc + consumed > block_end_pc {
            break;
        }
        nodes.push(SsaNode::new(pc_to_i32(pc + consumed), -1, SsaOp::Nop));
    }
    consumed
}

fn closure_upvalues(
    proto: &Proto,
    instructions: &[Instruction],
    closure_pc: usize,
    proto_idx: i32,
) -> Vec<UpvalueCapture> {
    let Some(nested) = usize::try_from(proto_idx)
        .ok()
        .and_then(|idx| proto.protos.get(idx))
    else {
        return Vec::new();
    };

    let mut captures = Vec::with_capacity(usize::from(nested.nups));
    for offset in 0..usize::from(nested.nups) {
        let Some(pseudo) = instructions.get(closure_pc + 1 + offset).copied() else {
            break;
        };
        match pseudo.op {
            SemanticOp::Move => captures.push(UpvalueCapture::ParentLocal(reg_ref(pseudo.b))),
            SemanticOp::GetUpval => {
                captures.push(UpvalueCapture::ParentUpvalue(reg_index(pseudo.b)));
            }
            _ => break,
        }
    }
    captures
}

fn lift_call(pc: usize, line: i32, inst: Instruction, previous: &[SsaNode]) -> SsaNode {
    let args = call_args(inst.a, inst.b, previous);
    SsaNode::with_dest(
        pc_to_i32(pc),
        line,
        reg_ref(inst.a),
        SsaOp::Call {
            func: reg_ref(inst.a),
            args,
            base: reg_index(inst.a),
            arg_count: inst.b,
            return_count: inst.c,
        },
    )
}

fn lift_tailcall(pc: usize, line: i32, inst: Instruction, previous: &[SsaNode]) -> SsaNode {
    let args = call_args(inst.a, inst.b, previous);
    SsaNode::new(
        pc_to_i32(pc),
        line,
        SsaOp::TailCall {
            func: reg_ref(inst.a),
            args,
            base: reg_index(inst.a),
            arg_count: inst.b,
            return_count: inst.c,
        },
    )
}

fn lift_return(pc: usize, line: i32, inst: Instruction, previous: &[SsaNode]) -> SsaNode {
    let values = if inst.b > 1 {
        (0..(inst.b - 1))
            .map(|offset| reg_ref(inst.a + offset))
            .collect()
    } else if inst.b == 0 {
        top_set_reg(previous)
            .filter(|top| i32::from(*top) >= inst.a)
            .map_or_else(Vec::new, |top| {
                (inst.a..=i32::from(top)).map(reg_ref).collect()
            })
    } else {
        Vec::new()
    };
    SsaNode::new(
        pc_to_i32(pc),
        line,
        SsaOp::Return {
            values,
            base: reg_index(inst.a),
            count: inst.b,
        },
    )
}

fn call_args(base: i32, arg_count: i32, previous: &[SsaNode]) -> Vec<SsaRef> {
    if arg_count > 1 {
        (0..(arg_count - 1))
            .map(|offset| reg_ref(base + 1 + offset))
            .collect()
    } else if arg_count == 0 {
        top_set_reg(previous)
            .filter(|top| i32::from(*top) > base)
            .map_or_else(Vec::new, |top| {
                ((base + 1)..=i32::from(top)).map(reg_ref).collect()
            })
    } else {
        Vec::new()
    }
}

fn setlist_values(base: i32, count: i32, previous: &[SsaNode]) -> Vec<SsaRef> {
    if count > 0 {
        (1..=count).map(|offset| reg_ref(base + offset)).collect()
    } else {
        top_set_reg(previous)
            .filter(|top| i32::from(*top) > base)
            .map_or_else(Vec::new, |top| {
                ((base + 1)..=i32::from(top)).map(reg_ref).collect()
            })
    }
}

fn top_set_reg(nodes: &[SsaNode]) -> Option<u16> {
    nodes.iter().rev().find_map(|node| match &node.op {
        SsaOp::Call {
            base,
            return_count: 0,
            ..
        }
        | SsaOp::VarArg { base, count: 0, .. } => Some(*base),
        _ => None,
    })
}

fn operand_ref(table: &OpcodeTable, inst: Instruction, slot: OperandSlot) -> SsaRef {
    let field = match slot {
        OperandSlot::B => inst.b,
        OperandSlot::C => inst.c,
    };
    match opinfo::info_for(inst.op).operand_mode(slot) {
        OpArgMode::K if table.is_k(field) => SsaRef::constant(const_index(table.rk_index(field))),
        OpArgMode::K | OpArgMode::R => reg_ref(field),
        OpArgMode::U | OpArgMode::N => SsaRef::None,
    }
}

fn line_number(proto: &Proto, pc: usize) -> i32 {
    proto.line_info.get(pc).copied().unwrap_or(-1)
}

const fn bin_op(op: SemanticOp) -> BinOp {
    match op {
        SemanticOp::Sub => BinOp::Sub,
        SemanticOp::Mul => BinOp::Mul,
        SemanticOp::Div => BinOp::Div,
        SemanticOp::Mod => BinOp::Mod,
        SemanticOp::Pow => BinOp::Pow,
        SemanticOp::Idiv => BinOp::IDiv,
        SemanticOp::Band => BinOp::BAnd,
        SemanticOp::Bor => BinOp::BOr,
        SemanticOp::Bxor => BinOp::BXor,
        SemanticOp::Shl => BinOp::Shl,
        SemanticOp::Shr => BinOp::Shr,
        // `Add` shares the fallback: it is the identity choice for any op the
        // lifter did not classify as binary.
        _ => BinOp::Add,
    }
}

const fn un_op(op: SemanticOp) -> UnOp {
    match op {
        SemanticOp::Not => UnOp::Not,
        SemanticOp::Len => UnOp::Len,
        SemanticOp::Bnot => UnOp::BNot,
        // `Unm` shares the fallback for any op that is not unary.
        _ => UnOp::Neg,
    }
}

const fn rel_op(op: SemanticOp) -> RelOp {
    match op {
        SemanticOp::Lt => RelOp::Lt,
        SemanticOp::Le => RelOp::Le,
        // `Eq` shares the fallback for any op that is not a comparison.
        _ => RelOp::Eq,
    }
}

fn jump_target(pc: usize, sbx: i32) -> i32 {
    pc_to_i32(pc) + 1 + sbx
}

fn reg_ref(reg: i32) -> SsaRef {
    u16::try_from(reg).map_or(SsaRef::None, SsaRef::reg)
}

fn reg_index(reg: i32) -> u16 {
    u16::try_from(reg).unwrap_or(0)
}

fn const_index(idx: i32) -> u32 {
    u32::try_from(idx).unwrap_or(0)
}

fn pc_to_i32(pc: usize) -> i32 {
    i32::try_from(pc).unwrap_or(i32::MAX)
}
