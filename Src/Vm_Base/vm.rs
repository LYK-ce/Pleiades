//Presented by KeJi
//Date ： 2026-05-09

//! Vm 执行引擎骨架
//!
//! 提供指令执行的基础设施：指令指针和槽位文件。
//! 包含 5 条公共指令的 handler 实现。领域指令由 ML Engine / Orchestrator
//! 在自己的执行循环中 match 处理。

use std::time::SystemTime;

use super::slot::{ConstValue, SlotFile, SlotId, SlotValue};

// ============================================================
// StepResult
// ============================================================

/// 单步执行结果。
#[derive(Debug, PartialEq, Eq)]
pub enum StepResult {
    Continue,
    Ready,
    Done,
    Abort(String),
}

// ============================================================
// BaseInstruction — 公共指令集
// ============================================================

/// 五条公共指令，由 ML Engine / Orchestrator 嵌入各自的指令枚举。
#[derive(Debug, Clone)]
pub enum BaseInstruction {
    Const { value: ConstValue, dst: SlotId },
    Move { src: SlotId, dst: SlotId },
    Add { dst: SlotId, delta: f64 },
    Sub { src: SlotId, dst: SlotId },
    Timer { slot: SlotId },
    Jump { target: usize },
    JumpIf { condition: SlotId, target: usize },
}

// ============================================================
// Vm
// ============================================================

/// 虚拟机基础结构，由各领域模块持有。
#[derive(Debug, Default)]
pub struct Vm {
    pub ip: usize,
    pub slots: SlotFile,
}

impl Vm {
    pub fn new() -> Self {
        Self {
            ip: 0,
            slots: SlotFile::new(),
        }
    }

    // ============================================================
    // 公共指令 dispatch
    // ============================================================

    /// 分发一条 BaseInstruction。
    pub fn dispatch_base(&mut self, inst: &BaseInstruction) -> StepResult {
        match inst {
            BaseInstruction::Const { value, dst } => self.handle_const(value.clone(), *dst),
            BaseInstruction::Move { src, dst } => self.handle_move(*src, *dst),
            BaseInstruction::Add { dst, delta } => self.handle_add(*dst, *delta),
            BaseInstruction::Sub { src, dst } => self.handle_sub(*src, *dst),
            BaseInstruction::Timer { slot } => self.handle_timer(*slot),
            BaseInstruction::Jump { target } => self.handle_jump(*target),
            BaseInstruction::JumpIf { condition, target } => {
                self.handle_jump_if(*condition, *target)
            }
        }
    }

    // ============================================================
    // 公共指令 handler
    // ============================================================

    /// Const: 将常量值写入目标槽位。
    pub fn handle_const(&mut self, value: ConstValue, dst: SlotId) -> StepResult {
        self.slots.set(dst, SlotValue::from(value));
        StepResult::Continue
    }

    /// Move: 从 src 槽位取出值，写入 dst 槽位（take 语义）。
    pub fn handle_move(&mut self, src: SlotId, dst: SlotId) -> StepResult {
        match self.slots.take(src) {
            Some(value) => {
                self.slots.set(dst, value);
                StepResult::Continue
            }
            None => StepResult::Abort(format!("Move: src slot {} not found", src.0)),
        }
    }

    /// Add: dst += delta（仅对 F64 槽位有效）。
    pub fn handle_add(&mut self, dst: SlotId, delta: f64) -> StepResult {
        match self.slots.get_f64(dst) {
            Ok(current) => {
                self.slots.set(dst, SlotValue::F64(current + delta));
                StepResult::Continue
            }
            Err(e) => StepResult::Abort(format!("Add: {}", e)),
        }
    }

    /// Sub: dst = dst - src（仅对 F64 槽位有效，读 src 不取走）。
    pub fn handle_sub(&mut self, src: SlotId, dst: SlotId) -> StepResult {
        match (self.slots.get_f64(src), self.slots.get_f64(dst)) {
            (Ok(s), Ok(d)) => {
                self.slots.set(dst, SlotValue::F64(d - s));
                StepResult::Continue
            }
            (Err(e), _) | (_, Err(e)) => StepResult::Abort(format!("Sub: {}", e)),
        }
    }

    /// Timer: 将当前时间戳（Unix 秒）写入目标槽位。
    pub fn handle_timer(&mut self, slot: SlotId) -> StepResult {
        let secs = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();
        self.slots.set(slot, SlotValue::F64(secs));
        StepResult::Continue
    }

    /// Jump: 无条件跳转到 target。
    pub fn handle_jump(&mut self, target: usize) -> StepResult {
        self.ip = target;
        StepResult::Continue
    }

    /// JumpIf: 若 condition 槽位为 true 则跳转到 target。
    pub fn handle_jump_if(&mut self, condition: SlotId, target: usize) -> StepResult {
        match self.slots.get_bool(condition) {
            Ok(true) => {
                self.ip = target;
                StepResult::Continue
            }
            Ok(false) => StepResult::Continue,
            Err(e) => StepResult::Abort(format!("JumpIf: {}", e)),
        }
    }
}

// ============================================================
// 测试
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_vm() -> Vm {
        Vm::new()
    }

    // ─── Const ───────────────────────────────────────────────

    #[test]
    fn const_writes_bool() {
        let mut vm = setup_vm();
        let r = vm.handle_const(ConstValue::Bool(true), SlotId(0));
        assert_eq!(r, StepResult::Continue);
        assert!(vm.slots.get_bool(SlotId(0)).unwrap());
    }

    #[test]
    fn const_writes_u64() {
        let mut vm = setup_vm();
        vm.handle_const(ConstValue::U64(42), SlotId(0));
        assert_eq!(vm.slots.get_u64(SlotId(0)).unwrap(), 42);
    }

    #[test]
    fn const_writes_f64() {
        let mut vm = setup_vm();
        vm.handle_const(ConstValue::F64(3.14), SlotId(0));
        assert_eq!(vm.slots.get_f64(SlotId(0)).unwrap(), 3.14);
    }

    #[test]
    fn const_writes_string() {
        let mut vm = setup_vm();
        vm.handle_const(ConstValue::String("hi".into()), SlotId(0));
        assert_eq!(vm.slots.get_string(SlotId(0)).unwrap(), "hi");
    }

    #[test]
    fn const_writes_nil() {
        let mut vm = setup_vm();
        vm.handle_const(ConstValue::Nil, SlotId(0));
        assert!(matches!(vm.slots.get(SlotId(0)), Some(SlotValue::Nil)));
    }

    #[test]
    fn const_overwrites_existing() {
        let mut vm = setup_vm();
        vm.handle_const(ConstValue::U64(1), SlotId(0));
        vm.handle_const(ConstValue::U64(2), SlotId(0));
        assert_eq!(vm.slots.get_u64(SlotId(0)).unwrap(), 2);
    }

    // ─── Move ────────────────────────────────────────────────

    #[test]
    fn move_copies_value() {
        let mut vm = setup_vm();
        vm.handle_const(ConstValue::U64(99), SlotId(0));
        let r = vm.handle_move(SlotId(0), SlotId(1));
        assert_eq!(r, StepResult::Continue);
        assert_eq!(vm.slots.get_u64(SlotId(1)).unwrap(), 99);
    }

    #[test]
    fn move_src_becomes_nil_after_take() {
        let mut vm = setup_vm();
        vm.handle_const(ConstValue::U64(99), SlotId(0));
        vm.handle_move(SlotId(0), SlotId(1));
        assert!(matches!(vm.slots.get(SlotId(0)), Some(SlotValue::Nil)));
    }

    #[test]
    fn move_empty_src_aborts() {
        let mut vm = setup_vm();
        let r = vm.handle_move(SlotId(0), SlotId(1));
        assert!(matches!(r, StepResult::Abort(ref msg) if msg.contains("not found")));
    }

    #[test]
    fn move_same_slot_roundtrips() {
        let mut vm = setup_vm();
        vm.handle_const(ConstValue::U64(42), SlotId(0));
        let r = vm.handle_move(SlotId(0), SlotId(0));
        assert_eq!(r, StepResult::Continue);
        assert_eq!(vm.slots.get_u64(SlotId(0)).unwrap(), 42);
    }

    // ─── Add ─────────────────────────────────────────────────

    #[test]
    fn add_increments_f64() {
        let mut vm = setup_vm();
        vm.handle_const(ConstValue::F64(10.0), SlotId(0));
        let r = vm.handle_add(SlotId(0), 5.0);
        assert_eq!(r, StepResult::Continue);
        assert_eq!(vm.slots.get_f64(SlotId(0)).unwrap(), 15.0);
    }

    #[test]
    fn add_negative_decrements() {
        let mut vm = setup_vm();
        vm.handle_const(ConstValue::F64(10.0), SlotId(0));
        vm.handle_add(SlotId(0), -3.0);
        assert_eq!(vm.slots.get_f64(SlotId(0)).unwrap(), 7.0);
    }

    #[test]
    fn add_zero_no_change() {
        let mut vm = setup_vm();
        vm.handle_const(ConstValue::F64(10.0), SlotId(0));
        vm.handle_add(SlotId(0), 0.0);
        assert_eq!(vm.slots.get_f64(SlotId(0)).unwrap(), 10.0);
    }

    #[test]
    fn add_on_non_f64_slot_aborts() {
        let mut vm = setup_vm();
        vm.handle_const(ConstValue::U64(10), SlotId(0));
        let r = vm.handle_add(SlotId(0), 1.0);
        assert!(matches!(r, StepResult::Abort(ref msg) if msg.contains("not a F64")));
    }

    #[test]
    fn add_on_empty_slot_aborts() {
        let mut vm = setup_vm();
        let r = vm.handle_add(SlotId(0), 1.0);
        assert!(matches!(r, StepResult::Abort(ref msg) if msg.contains("empty")));
    }

    #[test]
    fn add_multiple_steps_accumulate() {
        let mut vm = setup_vm();
        vm.handle_const(ConstValue::F64(0.0), SlotId(0));
        for _ in 0..5 {
            vm.handle_add(SlotId(0), 1.0);
        }
        assert_eq!(vm.slots.get_f64(SlotId(0)).unwrap(), 5.0);
    }

    // ─── Jump ────────────────────────────────────────────────

    #[test]
    fn jump_moves_ip() {
        let mut vm = setup_vm();
        let r = vm.handle_jump(5);
        assert_eq!(r, StepResult::Continue);
        assert_eq!(vm.ip, 5);
    }

    #[test]
    fn jump_to_zero() {
        let mut vm = setup_vm();
        vm.ip = 10;
        vm.handle_jump(0);
        assert_eq!(vm.ip, 0);
    }

    // ─── JumpIf ──────────────────────────────────────────────

    #[test]
    fn jump_if_true_jumps() {
        let mut vm = setup_vm();
        vm.handle_const(ConstValue::Bool(true), SlotId(0));
        vm.ip = 3;
        let r = vm.handle_jump_if(SlotId(0), 7);
        assert_eq!(r, StepResult::Continue);
        assert_eq!(vm.ip, 7);
    }

    #[test]
    fn jump_if_false_continues() {
        let mut vm = setup_vm();
        vm.handle_const(ConstValue::Bool(false), SlotId(0));
        vm.ip = 3;
        let r = vm.handle_jump_if(SlotId(0), 7);
        assert_eq!(r, StepResult::Continue);
        assert_eq!(vm.ip, 3);
    }

    #[test]
    fn jump_if_non_bool_slot_aborts() {
        let mut vm = setup_vm();
        vm.handle_const(ConstValue::U64(1), SlotId(0));
        let r = vm.handle_jump_if(SlotId(0), 0);
        assert!(matches!(r, StepResult::Abort(ref msg) if msg.contains("not a Bool")));
    }

    #[test]
    fn jump_if_empty_slot_aborts() {
        let mut vm = setup_vm();
        let r = vm.handle_jump_if(SlotId(0), 0);
        assert!(matches!(r, StepResult::Abort(ref msg) if msg.contains("empty")));
    }

    // ─── Vm 综合 ─────────────────────────────────────────────

    #[test]
    fn default_vm_ip_zero() {
        let vm = Vm::default();
        assert_eq!(vm.ip, 0);
    }

    #[test]
    fn simulate_loop_with_jump_if() {
        // 模拟 Loop: 重复 add 直到 FLAG 为 true
        //   0: Add(dst=0, value=1)
        //   1: JumpIf(FLAG, target=4)   ← 条件真时跳到 IP=4
        //   2: Jump(target=0)            ← 无条件回到 IP=0
        //   3: (unused)
        //   4: after_loop
        let mut vm = setup_vm();
        vm.handle_const(ConstValue::F64(0.0), SlotId(0)); // counter=0
        vm.handle_const(ConstValue::Bool(false), SlotId(1)); // FLAG=false

        // 执行 3 轮 Add
        for _ in 0..3 {
            vm.handle_add(SlotId(0), 1.0);
            vm.handle_jump_if(SlotId(1), 4); // FLAG=false → Continue
            vm.handle_jump(0); // → loop
        }
        // 第 4 轮 Add 后设 FLAG=true
        vm.handle_add(SlotId(0), 1.0);
        vm.handle_const(ConstValue::Bool(true), SlotId(1));
        vm.handle_jump_if(SlotId(1), 4); // FLAG=true → jump to IP=4
        assert_eq!(vm.ip, 4);
        assert_eq!(vm.slots.get_f64(SlotId(0)).unwrap(), 4.0);
    }

    #[test]
    fn simulate_move_chain() {
        let mut vm = setup_vm();
        vm.handle_const(ConstValue::U64(1), SlotId(0));
        vm.handle_move(SlotId(0), SlotId(1));
        vm.handle_move(SlotId(1), SlotId(2));
        assert_eq!(vm.slots.get_u64(SlotId(2)).unwrap(), 1);
        assert!(matches!(vm.slots.get(SlotId(0)), Some(SlotValue::Nil)));
        assert!(matches!(vm.slots.get(SlotId(1)), Some(SlotValue::Nil)));
    }

    // ─── Sub ─────────────────────────────────────────────────

    #[test]
    fn sub_basic() {
        let mut vm = setup_vm();
        vm.handle_const(ConstValue::F64(3.0), SlotId(0));
        vm.handle_const(ConstValue::F64(10.0), SlotId(1));
        let r = vm.handle_sub(SlotId(0), SlotId(1)); // dst - src = 10.0 - 3.0
        assert_eq!(r, StepResult::Continue);
        assert_eq!(vm.slots.get_f64(SlotId(1)).unwrap(), 7.0);
    }

    #[test]
    fn sub_src_unchanged() {
        let mut vm = setup_vm();
        vm.handle_const(ConstValue::F64(3.0), SlotId(0));
        vm.handle_const(ConstValue::F64(10.0), SlotId(1));
        vm.handle_sub(SlotId(0), SlotId(1));
        assert_eq!(vm.slots.get_f64(SlotId(0)).unwrap(), 3.0);
    }

    #[test]
    fn sub_on_empty_slot_aborts() {
        let mut vm = setup_vm();
        let r = vm.handle_sub(SlotId(0), SlotId(1));
        assert!(matches!(r, StepResult::Abort(_)));
    }

    // ─── Timer ───────────────────────────────────────────────

    #[test]
    fn timer_writes_timestamp() {
        let mut vm = setup_vm();
        let r = vm.handle_timer(SlotId(0));
        assert_eq!(r, StepResult::Continue);
        assert!(vm.slots.get_f64(SlotId(0)).is_ok());
    }

    #[test]
    fn timer_sub_chain_measures_duration() {
        let mut vm = setup_vm();
        vm.handle_timer(SlotId(0));
        // 用 Sub 自身验证时间戳差值为正
        vm.handle_timer(SlotId(1));
        std::thread::sleep(std::time::Duration::from_millis(10));
        vm.handle_timer(SlotId(2));
        vm.handle_sub(SlotId(1), SlotId(2)); // t2 - t1
        assert!(vm.slots.get_f64(SlotId(2)).unwrap() > 0.0);
    }
}
