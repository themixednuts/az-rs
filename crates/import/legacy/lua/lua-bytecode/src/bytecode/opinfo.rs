//! Shared opcode role and control-flow metadata.
//!
//! The Lua 5.1 entries mirror `luaP_opmodes` from PUC-Rio Lua 5.1.5.

use super::{Instruction, InstructionFormat, SemanticOp};

/// PUC-Rio operand role from `OpArgMask`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpArgMode {
    /// Operand is not used.
    N,
    /// Operand is used as an unsigned immediate or index.
    U,
    /// Operand is a register or jump offset.
    R,
    /// Operand is RK-encoded: register or constant.
    K,
}

/// B/C operand selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperandSlot {
    /// B operand.
    B,
    /// C operand.
    C,
}

/// Control-flow classification used by CFG construction.
///
/// Held as a flag set rather than seven parallel `bool` fields so that a
/// classification is one value to pass around and compare.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlFlowClass(u8);

impl ControlFlowClass {
    const JUMP: u8 = 1 << 0;
    const CONDITIONAL_TEST: u8 = 1 << 1;
    const CALL: u8 = 1 << 2;
    const RETURN: u8 = 1 << 3;
    const LOOP: u8 = 1 << 4;
    const FALLS_THROUGH: u8 = 1 << 5;
    const TERMINATOR: u8 = 1 << 6;

    /// Instruction performs an explicit jump.
    #[must_use]
    pub const fn is_jump(self) -> bool {
        self.0 & Self::JUMP != 0
    }

    /// Instruction is a conditional test/skip.
    #[must_use]
    pub const fn is_conditional_test(self) -> bool {
        self.0 & Self::CONDITIONAL_TEST != 0
    }

    /// Instruction is a call.
    #[must_use]
    pub const fn is_call(self) -> bool {
        self.0 & Self::CALL != 0
    }

    /// Instruction returns from the current function.
    #[must_use]
    pub const fn is_return(self) -> bool {
        self.0 & Self::RETURN != 0
    }

    /// Instruction participates in loop control.
    #[must_use]
    pub const fn is_loop(self) -> bool {
        self.0 & Self::LOOP != 0
    }

    /// Instruction can continue to the next instruction.
    #[must_use]
    pub const fn falls_through(self) -> bool {
        self.0 & Self::FALLS_THROUGH != 0
    }

    /// Instruction ends its basic block.
    #[must_use]
    pub const fn is_terminator(self) -> bool {
        self.0 & Self::TERMINATOR != 0
    }

    /// Fold an optional flag into a set under construction.
    const fn with(self, flag: u8, set: bool) -> Self {
        if set { Self(self.0 | flag) } else { self }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FlowKind {
    Linear,
    JumpSbx,
    ConditionalSkipNext,
    LoadBoolSkip,
    Return,
    ForLoop,
    ForPrep,
    TForLoop,
}

/// Full shared opcode descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpInfo {
    /// B operand role.
    pub b_mode: OpArgMode,
    /// C operand role.
    pub c_mode: OpArgMode,
    /// Whether the PUC opmode table marks A as assigned.
    pub sets_a: bool,
    /// Whether the PUC opmode table marks this as a test.
    pub test: bool,
    /// Raw instruction format.
    pub format: InstructionFormat,
    /// Control-flow classification.
    pub control_flow: ControlFlowClass,
    flow_kind: FlowKind,
}

impl OpInfo {
    /// Return the role for B or C.
    #[must_use]
    pub const fn operand_mode(self, slot: OperandSlot) -> OpArgMode {
        match slot {
            OperandSlot::B => self.b_mode,
            OperandSlot::C => self.c_mode,
        }
    }

    /// Return whether this B/C operand is RK-encoded.
    #[must_use]
    pub const fn is_rk_operand(self, slot: OperandSlot) -> bool {
        matches!(self.operand_mode(slot), OpArgMode::K)
    }

    /// Additional block starts introduced by this instruction.
    #[must_use]
    pub fn leader_pcs(self, pc: usize, inst: Instruction, code_len: usize) -> Vec<usize> {
        let mut leaders = Vec::with_capacity(2);
        match self.flow_kind {
            FlowKind::Linear => {}
            // All three take a computed target and can also reach `pc + 1`.
            FlowKind::JumpSbx | FlowKind::ForLoop | FlowKind::ForPrep => {
                push_target(&mut leaders, pc, inst.sbx, code_len);
                push_pc(&mut leaders, pc + 1, code_len);
            }
            FlowKind::ConditionalSkipNext | FlowKind::TForLoop => {
                push_pc(&mut leaders, pc + 1, code_len);
                push_pc(&mut leaders, pc + 2, code_len);
            }
            FlowKind::LoadBoolSkip => {
                if inst.c != 0 {
                    push_pc(&mut leaders, pc + 1, code_len);
                    push_pc(&mut leaders, pc + 2, code_len);
                }
            }
            FlowKind::Return => {
                push_pc(&mut leaders, pc + 1, code_len);
            }
        }
        leaders
    }

    /// Successor instruction PCs for a block ending with this instruction.
    #[must_use]
    pub fn successor_pcs(self, pc: usize, inst: Instruction, code_len: usize) -> Vec<usize> {
        let mut succs = Vec::with_capacity(2);
        match self.flow_kind {
            FlowKind::Linear => push_pc(&mut succs, pc + 1, code_len),
            // Both transfer control unconditionally to the computed target.
            FlowKind::JumpSbx | FlowKind::ForPrep => {
                push_target(&mut succs, pc, inst.sbx, code_len);
            }
            FlowKind::ConditionalSkipNext | FlowKind::TForLoop => {
                push_pc(&mut succs, pc + 1, code_len);
                push_pc(&mut succs, pc + 2, code_len);
            }
            FlowKind::LoadBoolSkip => {
                if inst.c != 0 {
                    push_pc(&mut succs, pc + 2, code_len);
                } else {
                    push_pc(&mut succs, pc + 1, code_len);
                }
            }
            FlowKind::Return => {}
            FlowKind::ForLoop => {
                push_target(&mut succs, pc, inst.sbx, code_len);
                push_pc(&mut succs, pc + 1, code_len);
            }
        }
        succs
    }
}

/// Return the shared descriptor for a semantic opcode.
#[must_use]
pub const fn info_for(op: SemanticOp) -> OpInfo {
    use InstructionFormat::{Abc, Abx, AsBx};
    use OpArgMode::{K, N, R, U};
    use SemanticOp as Op;

    match op {
        // luaP_opmodes row (0, 1, OpArgR, OpArgN, iABC).
        Op::Move | Op::LoadNil | Op::Unm | Op::Not | Op::Len => {
            opmode(false, true, R, N, Abc, FlowKind::Linear)
        }
        // Row (0, 1, OpArgK, OpArgN, iABx).
        Op::LoadK | Op::GetGlobal => opmode(false, true, K, N, Abx, FlowKind::Linear),
        Op::LoadBool => opmode(false, true, U, U, Abc, FlowKind::LoadBoolSkip),
        // Row (0, 1, OpArgU, OpArgN, iABC).
        Op::GetUpval | Op::VarArg => opmode(false, true, U, N, Abc, FlowKind::Linear),
        // Row (0, 1, OpArgR, OpArgK, iABC).
        Op::GetTable | Op::SelfOp => opmode(false, true, R, K, Abc, FlowKind::Linear),
        Op::SetGlobal => opmode(false, false, K, N, Abx, FlowKind::Linear),
        Op::SetUpval => opmode(false, false, U, N, Abc, FlowKind::Linear),
        Op::SetTable => opmode(false, false, K, K, Abc, FlowKind::Linear),
        Op::NewTable => opmode(false, true, U, U, Abc, FlowKind::Linear),
        Op::Add | Op::Sub | Op::Mul | Op::Div | Op::Mod | Op::Pow => {
            opmode(false, true, K, K, Abc, FlowKind::Linear)
        }
        Op::Concat => opmode(false, true, R, R, Abc, FlowKind::Linear),
        Op::Jmp => opmode(false, false, R, N, AsBx, FlowKind::JumpSbx),
        Op::Eq | Op::Lt | Op::Le => opmode(true, false, K, K, Abc, FlowKind::ConditionalSkipNext),
        Op::Test | Op::TestSet => opmode(true, true, R, U, Abc, FlowKind::ConditionalSkipNext),
        Op::Call => opmode_call(false, true, U, U, Abc, FlowKind::Linear),
        Op::TailCall => opmode_call(false, true, U, U, Abc, FlowKind::Return),
        Op::Return => opmode(false, false, U, N, Abc, FlowKind::Return),
        Op::ForLoop => opmode(false, true, R, N, AsBx, FlowKind::ForLoop),
        Op::ForPrep => opmode(false, true, R, N, AsBx, FlowKind::ForPrep),
        Op::TForLoop => opmode(true, false, N, U, Abc, FlowKind::TForLoop),
        Op::SetList => opmode(false, false, U, U, Abc, FlowKind::Linear),
        Op::Close => opmode(false, false, N, N, Abc, FlowKind::Linear),
        Op::Closure => opmode(false, true, U, N, Abx, FlowKind::Linear),
        _ => UNKNOWN_OP,
    }
}

/// Descriptor for opcodes this table does not model.
///
/// Deliberately distinct from `SETLIST`, which happens to share this shape:
/// that row is real reference data, this one is "no information".
const UNKNOWN_OP: OpInfo = opmode(
    false,
    false,
    OpArgMode::U,
    OpArgMode::U,
    InstructionFormat::Abc,
    FlowKind::Linear,
);

/// Return whether an opcode is part of the structural faithfulness signature.
#[must_use]
pub const fn is_structural_faithfulness_op(op: SemanticOp) -> bool {
    matches!(
        op,
        SemanticOp::Jmp
            | SemanticOp::Eq
            | SemanticOp::Lt
            | SemanticOp::Le
            | SemanticOp::Test
            | SemanticOp::TestSet
            | SemanticOp::ForPrep
            | SemanticOp::ForLoop
            | SemanticOp::TForLoop
            | SemanticOp::Call
            | SemanticOp::TailCall
            | SemanticOp::Return
            | SemanticOp::Closure
            | SemanticOp::SetList
    )
}

const fn opmode(
    test: bool,
    sets_a: bool,
    b_mode: OpArgMode,
    c_mode: OpArgMode,
    format: InstructionFormat,
    flow_kind: FlowKind,
) -> OpInfo {
    opmode_with_call(test, sets_a, b_mode, c_mode, format, flow_kind, false)
}

const fn opmode_call(
    test: bool,
    sets_a: bool,
    b_mode: OpArgMode,
    c_mode: OpArgMode,
    format: InstructionFormat,
    flow_kind: FlowKind,
) -> OpInfo {
    opmode_with_call(test, sets_a, b_mode, c_mode, format, flow_kind, true)
}

const fn opmode_with_call(
    test: bool,
    sets_a: bool,
    b_mode: OpArgMode,
    c_mode: OpArgMode,
    format: InstructionFormat,
    flow_kind: FlowKind,
    is_call: bool,
) -> OpInfo {
    OpInfo {
        b_mode,
        c_mode,
        sets_a,
        test,
        format,
        control_flow: control_flow(flow_kind, is_call),
        flow_kind,
    }
}

const fn control_flow(flow_kind: FlowKind, is_call: bool) -> ControlFlowClass {
    let base = match flow_kind {
        // A `LOADBOOL` with a skip is still straight-line for CFG purposes:
        // its extra leader is contributed by `leader_pcs`, not by this class.
        FlowKind::Linear | FlowKind::LoadBoolSkip => {
            ControlFlowClass(ControlFlowClass::FALLS_THROUGH)
        }
        FlowKind::JumpSbx | FlowKind::ForPrep => {
            ControlFlowClass(ControlFlowClass::JUMP | ControlFlowClass::TERMINATOR).with(
                ControlFlowClass::LOOP,
                matches!(flow_kind, FlowKind::ForPrep),
            )
        }
        FlowKind::ConditionalSkipNext | FlowKind::TForLoop => ControlFlowClass(
            ControlFlowClass::CONDITIONAL_TEST
                | ControlFlowClass::FALLS_THROUGH
                | ControlFlowClass::TERMINATOR,
        )
        .with(
            ControlFlowClass::LOOP,
            matches!(flow_kind, FlowKind::TForLoop),
        ),
        FlowKind::Return => {
            ControlFlowClass(ControlFlowClass::RETURN | ControlFlowClass::TERMINATOR)
        }
        FlowKind::ForLoop => ControlFlowClass(
            ControlFlowClass::JUMP
                | ControlFlowClass::CONDITIONAL_TEST
                | ControlFlowClass::LOOP
                | ControlFlowClass::FALLS_THROUGH
                | ControlFlowClass::TERMINATOR,
        ),
    };
    base.with(ControlFlowClass::CALL, is_call)
}

fn push_target(out: &mut Vec<usize>, pc: usize, sbx: i32, code_len: usize) {
    let Ok(pc_i32) = i32::try_from(pc) else {
        return;
    };
    let target = pc_i32 + 1 + sbx;
    if let Ok(target) = usize::try_from(target) {
        push_pc(out, target, code_len);
    }
}

fn push_pc(out: &mut Vec<usize>, pc: usize, code_len: usize) {
    if pc < code_len && !out.contains(&pc) {
        out.push(pc);
    }
}
