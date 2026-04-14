//Presented by KeJi
//Date ： 2026-04-13

//! ML 执行引擎寄存器系统
//!
//! 为指令泵（Engine）提供一套粗粒度寄存器，用于在指令间传递数据。
//! 寄存器由 Session 层挂载，每个 Session 持有独立的寄存器组。
//!
//! ## 寄存器分类
//! - **TEXT1~4**：文本寄存器，存储字符串（预分配容量）
//! - **TOKENID1~4**：Token ID 寄存器，存储 `Vec<u32>`（预分配容量）
//! - **TENSOR1~4**：Tensor 寄存器，存储 `Option<Tensor>`（Candle Tensor，move 零拷贝）
//! - **FLAG1~4**：标志寄存器，存储 `bool`
//! - **META1~8**：元数据寄存器，存储 `f64`
//!
//! ## 设计原则
//! - Candle Tensor 内部为 `Arc<Storage>` + shape 元数据，move 零拷贝
//! - TEXT / TOKENID 寄存器通过 Init 预分配容量，减少运行时重新分配
//! - TENSOR 寄存器不做预分配，由推理后端自行管理底层内存

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use candle_core::Tensor;

// ============================================================
// 寄存器索引 — newtype 包装，编译期类型安全
// ============================================================

/// 文本寄存器索引（0~3 → TEXT1~TEXT4）
#[derive(Debug, Clone, Copy)]
pub struct Text_Reg(pub u8);

/// Token ID 寄存器索引（0~3 → TOKENID1~TOKENID4）
#[derive(Debug, Clone, Copy)]
pub struct Token_Reg(pub u8);

/// Tensor 寄存器索引（0~3 → TENSOR1~TENSOR4）
#[derive(Debug, Clone, Copy)]
pub struct Tensor_Reg(pub u8);

/// 标志寄存器索引（0~3 → FLAG1~FLAG4）
#[derive(Debug, Clone, Copy)]
pub struct Flag_Reg(pub u8);

/// 元数据寄存器索引（0~7 → META1~META8）
#[derive(Debug, Clone, Copy)]
pub struct Meta_Reg(pub u8);

// ============================================================
// 寄存器常量
// ============================================================

pub const TEXT1: Text_Reg = Text_Reg(0);
pub const TEXT2: Text_Reg = Text_Reg(1);
pub const TEXT3: Text_Reg = Text_Reg(2);
pub const TEXT4: Text_Reg = Text_Reg(3);

pub const TOKENID1: Token_Reg = Token_Reg(0);
pub const TOKENID2: Token_Reg = Token_Reg(1);
pub const TOKENID3: Token_Reg = Token_Reg(2);
pub const TOKENID4: Token_Reg = Token_Reg(3);

pub const TENSOR1: Tensor_Reg = Tensor_Reg(0);
pub const TENSOR2: Tensor_Reg = Tensor_Reg(1);
pub const TENSOR3: Tensor_Reg = Tensor_Reg(2);
pub const TENSOR4: Tensor_Reg = Tensor_Reg(3);

pub const FLAG1: Flag_Reg = Flag_Reg(0);
pub const FLAG2: Flag_Reg = Flag_Reg(1);
pub const FLAG3: Flag_Reg = Flag_Reg(2);
pub const FLAG4: Flag_Reg = Flag_Reg(3);

pub const META1: Meta_Reg = Meta_Reg(0);
pub const META2: Meta_Reg = Meta_Reg(1);
pub const META3: Meta_Reg = Meta_Reg(2);
pub const META4: Meta_Reg = Meta_Reg(3);
pub const META5: Meta_Reg = Meta_Reg(4);
pub const META6: Meta_Reg = Meta_Reg(5);
pub const META7: Meta_Reg = Meta_Reg(6);
pub const META8: Meta_Reg = Meta_Reg(7);

// ============================================================
// 寄存器容量常量
// ============================================================

const TEXT_REG_COUNT: usize = 4;
const TOKEN_REG_COUNT: usize = 4;
const TENSOR_REG_COUNT: usize = 4;
const FLAG_REG_COUNT: usize = 4;
const META_REG_COUNT: usize = 8;

// ============================================================
// 寄存器组
// ============================================================

/// ML 执行引擎寄存器组
///
/// 由 Session 层通过 `Init()` 创建并挂载。
/// 所有字段为线程私有，不需要 `Arc<Mutex<>>`。
pub struct Register_File {
    /// 文本寄存器（TEXT1~TEXT4），每个槽位为预分配的 String
    text: Vec<(String,)>,
    /// Token ID 寄存器（TOKENID1~TOKENID4），每个槽位为预分配的 Vec<u32>
    token: Vec<(Vec<u32>,)>,
    /// Tensor 寄存器（TENSOR1~TENSOR4），Option 语义，move 零拷贝
    tensor: Vec<Option<Tensor>>,
    /// 标志寄存器（FLAG1~FLAG4）
    flag: Vec<(bool,)>,
    /// 元数据寄存器（META1~META8）
    meta: Vec<(f64,)>,
}

impl Register_File {
    // ============================================================
    // 初始化
    // ============================================================

    /// 初始化寄存器组
    ///
    /// 根据传入的 `text_len` 和 `token_len` 为 TEXT、TOKENID 寄存器提前预分配空间。
    /// TENSOR 寄存器初始化为 None，FLAG 初始化为 false，META 初始化为 0.0。
    ///
    /// # 参数
    /// - `text_len`: 每个 TEXT 寄存器的预分配字符容量
    /// - `token_len`: 每个 TOKENID 寄存器的预分配 token 容量
    pub fn Init(text_len: usize, token_len: usize) -> Self {
        let text = (0..TEXT_REG_COUNT)
            .map(|_| (String::with_capacity(text_len),))
            .collect();

        let token = (0..TOKEN_REG_COUNT)
            .map(|_| (Vec::with_capacity(token_len),))
            .collect();

        let tensor = (0..TENSOR_REG_COUNT).map(|_| None).collect();

        let flag = (0..FLAG_REG_COUNT).map(|_| (false,)).collect();

        let meta = (0..META_REG_COUNT).map(|_| (0.0,)).collect();

        Register_File {
            text,
            token,
            tensor,
            flag,
            meta,
        }
    }

    // ============================================================
    // Text 操作
    // ============================================================

    /// 向 TEXT 寄存器写入值（清空后写入）
    pub fn Set_Text(&mut self, reg: Text_Reg, value: &str) {
        let slot = &mut self.text[reg.0 as usize];
        slot.0.clear();
        slot.0.push_str(value);
    }

    /// 获取 TEXT 寄存器的值
    pub fn Get_Text(&self, reg: Text_Reg) -> &str {
        &self.text[reg.0 as usize].0
    }

    /// 向 TEXT 寄存器追加文本
    pub fn Append_Text(&mut self, reg: Text_Reg, value: &str) {
        self.text[reg.0 as usize].0.push_str(value);
    }

    /// 清空 TEXT 寄存器
    pub fn Reset_Text(&mut self, reg: Text_Reg) {
        self.text[reg.0 as usize].0.clear();
    }

    // ============================================================
    // Token 操作
    // ============================================================

    /// 向 TOKENID 寄存器写入值（替换整个 Vec）
    pub fn Set_Token(&mut self, reg: Token_Reg, value: Vec<u32>) {
        let slot = &mut self.token[reg.0 as usize];
        slot.0 = value;
    }

    /// 获取 TOKENID 寄存器的值
    pub fn Get_Token(&self, reg: Token_Reg) -> &[u32] {
        &self.token[reg.0 as usize].0
    }

    /// 向 TOKENID 寄存器追加单个 token
    pub fn Append_Token(&mut self, reg: Token_Reg, value: u32) {
        self.token[reg.0 as usize].0.push(value);
    }

    /// 向 TOKENID 寄存器批量追加 token
    pub fn Extend_Token(&mut self, reg: Token_Reg, value: &[u32]) {
        self.token[reg.0 as usize].0.extend_from_slice(value);
    }

    /// 清空 TOKENID 寄存器
    pub fn Reset_Token(&mut self, reg: Token_Reg) {
        self.token[reg.0 as usize].0.clear();
    }

    // ============================================================
    // Tensor 操作（所有权容器，存储 Candle Tensor）
    // ============================================================
    // Candle Tensor 内部为 Arc<Storage> + shape 元数据，move 零拷贝。
    // 推理输出直接 move 进寄存器，下一轮推理直接引用寄存器中的 Tensor。
    // 网络发送时通过 tensor.data() 获取零拷贝 &[f32] 视图进行序列化。
    // 网络接收时 Tensor::new() 构造后 move 进寄存器（接收侧 1 次 memcpy 不可避免）。

    /// 将 Tensor 写入寄存器（move 语义，零拷贝）
    pub fn Set_Tensor(&mut self, reg: Tensor_Reg, value: Tensor) {
        self.tensor[reg.0 as usize] = Some(value);
    }

    /// 获取 Tensor 的不可变引用（零拷贝）
    pub fn Get_Tensor(&self, reg: Tensor_Reg) -> Option<&Tensor> {
        self.tensor[reg.0 as usize].as_ref()
    }

    /// 从寄存器中取走 Tensor 的所有权（move out，零拷贝）
    /// 取走后寄存器变为 None
    pub fn Take_Tensor(&mut self, reg: Tensor_Reg) -> Option<Tensor> {
        self.tensor[reg.0 as usize].take()
    }

    /// 清空 Tensor 寄存器（drop Tensor，Arc 引用计数减一，释放底层内存）
    pub fn Reset_Tensor(&mut self, reg: Tensor_Reg) {
        self.tensor[reg.0 as usize] = None;
    }

    // ============================================================
    // Flag 操作
    // ============================================================

    /// 设置 FLAG 寄存器的值
    pub fn Set_Flag(&mut self, reg: Flag_Reg, value: bool) {
        self.flag[reg.0 as usize].0 = value;
    }

    /// 获取 FLAG 寄存器的值
    pub fn Get_Flag(&self, reg: Flag_Reg) -> bool {
        self.flag[reg.0 as usize].0
    }

    /// 翻转 FLAG 寄存器的值
    pub fn Toggle_Flag(&mut self, reg: Flag_Reg) {
        self.flag[reg.0 as usize].0 = !self.flag[reg.0 as usize].0;
    }

    // ============================================================
    // Meta 操作
    // ============================================================

    /// 设置 META 寄存器的值
    pub fn Set_Meta(&mut self, reg: Meta_Reg, value: f64) {
        self.meta[reg.0 as usize].0 = value;
    }

    /// 获取 META 寄存器的值
    pub fn Get_Meta(&self, reg: Meta_Reg) -> f64 {
        self.meta[reg.0 as usize].0
    }

    /// 向 META 寄存器的值增加 delta
    pub fn Increment_Meta(&mut self, reg: Meta_Reg, delta: f64) {
        self.meta[reg.0 as usize].0 += delta;
    }

    // ============================================================
    // 全局操作
    // ============================================================

    /// 清空所有寄存器的值
    ///
    /// TEXT：清空字符串（保留容量）
    /// TOKENID：清空 Vec（保留容量）
    /// TENSOR：设为 None（drop Tensor）
    /// FLAG：设为 false
    /// META：设为 0.0
    pub fn Reset_All(&mut self) {
        for i in 0..TEXT_REG_COUNT {
            self.Reset_Text(Text_Reg(i as u8));
        }
        for i in 0..TOKEN_REG_COUNT {
            self.Reset_Token(Token_Reg(i as u8));
        }
        for i in 0..TENSOR_REG_COUNT {
            self.Reset_Tensor(Tensor_Reg(i as u8));
        }
        for i in 0..FLAG_REG_COUNT {
            self.Set_Flag(Flag_Reg(i as u8), false);
        }
        for i in 0..META_REG_COUNT {
            self.Set_Meta(Meta_Reg(i as u8), 0.0);
        }
    }
}
