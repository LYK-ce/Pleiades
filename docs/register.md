# ML service register设计
我们将会重构当前的ML service设计，为了方便之后设计指令，所以我们要给预计实现的session层增加一套寄存器系统。这套寄存器是给指令泵使用的，因此不需要细粒度。

## 文件
Src/ML_Engine/ml_thread_register.rs

## 具体寄存器设计
文本寄存器 文本数组
- TEXT1   
- TEXT2   
- TEXT3
- TEXT4

token id寄存器 数组
- TOKENID1
- TOKENID2
- TOKENID3
- TOKENID4

Tensor寄存器 存储Candle Tensor的所有权（Option\<Tensor\>），Candle Tensor本身是轻量级的智能指针（内部为Arc\<Storage\>+shape元数据），move零拷贝。不做预分配，由推理后端自行管理底层内存。
- TENSOR1
- TENSOR2
- TENSOR3
- TENSOR4

标志寄存器 bool值
- FLAG1
- FLAG2
- FLAG3
- FLAG4

元数据寄存器 数值
- META1
- META2
- META3
- META4
- META5
- META6
- META7
- META8
  


## 方法
Init(text_len, token_len)
根据传入的text len和token len为TEXT、TOKENID寄存器提前预分配空间，然后初始化生成一套寄存器。TENSOR寄存器初始化为None，FLAG初始化为false，META初始化为0.0。调用此方法的session将会把此寄存器挂载。

Set(REG, value)
向REG中写入value

Append(REG, value)
向REG中push一个新的值

Reset(REG)
清空REG的值

Reset_All()
清空所有REG的值

GET(REG)
获取REG的值

Take(TENSOR_REG)
从TENSOR寄存器中取走Tensor的所有权（move out），寄存器变为None。用于需要转移Tensor给推理后端或网络发送的场景。

## 参考
 pub fn set_text(&mut self, reg: TextReg, value: &str) {
        let slot = &mut self.text[reg.0 as usize];
        slot.0.clear();
        slot.0.push_str(value);
    }
    
    pub fn get_text(&self, reg: TextReg) -> &str {
        &self.text[reg.0 as usize].0
    }
    
    pub fn append_text(&mut self, reg: TextReg, value: &str) {
        self.text[reg.0 as usize].0.push_str(value);
    }
    
    pub fn reset_text(&mut self, reg: TextReg) {
        self.text[reg.0 as usize].0.clear();
    }

    // ========== Token 操作 ==========
    pub fn set_token(&mut self, reg: TokenReg, value: Vec<u32>) {
        let slot = &mut self.token[reg.0 as usize];
        slot.0 = value; // 直接替换（预分配容量可能变化，但通常调用方也会预分配）
    }
    
    pub fn get_token(&self, reg: TokenReg) -> &[u32] {
        &self.token[reg.0 as usize].0
    }
    
    pub fn append_token(&mut self, reg: TokenReg, value: u32) {
        self.token[reg.0 as usize].0.push(value);
    }
    
    pub fn extend_token(&mut self, reg: TokenReg, value: &[u32]) {
        self.token[reg.0 as usize].0.extend_from_slice(value);
    }
    
    pub fn reset_token(&mut self, reg: TokenReg) {
        self.token[reg.0 as usize].0.clear();
    }

    // ========== Tensor 操作（所有权容器，存储 Candle Tensor） ==========
    // Candle Tensor 内部为 Arc<Storage> + shape 元数据，move 零拷贝。
    // 推理输出直接 move 进寄存器，下一轮推理直接引用寄存器中的 Tensor。
    // 网络发送时通过 tensor.data() 获取零拷贝 &[f32] 视图进行序列化。
    // 网络接收时 Tensor::new() 构造后 move 进寄存器（接收侧 1 次 memcpy 不可避免）。

    /// 将 Tensor 写入寄存器（move 语义，零拷贝）
    pub fn set_tensor(&mut self, reg: TensorReg, value: Tensor) {
        self.tensor[reg.0 as usize] = Some(value);
    }

    /// 获取 Tensor 的不可变引用（零拷贝）
    pub fn get_tensor(&self, reg: TensorReg) -> Option<&Tensor> {
        self.tensor[reg.0 as usize].as_ref()
    }

    /// 从寄存器中取走 Tensor 的所有权（move out，零拷贝）
    /// 取走后寄存器变为 None
    pub fn take_tensor(&mut self, reg: TensorReg) -> Option<Tensor> {
        self.tensor[reg.0 as usize].take()
    }

    /// 清空寄存器（drop Tensor，Arc 引用计数减一，释放底层内存）
    pub fn reset_tensor(&mut self, reg: TensorReg) {
        self.tensor[reg.0 as usize] = None;
    }

    // ========== Flag 操作 ==========
    pub fn set_flag(&mut self, reg: FlagReg, value: bool) {
        self.flag[reg.0 as usize].0 = value;
    }
    
    pub fn get_flag(&self, reg: FlagReg) -> bool {
        self.flag[reg.0 as usize].0
    }
    
    pub fn toggle_flag(&mut self, reg: FlagReg) {
        self.flag[reg.0 as usize].0 = !self.flag[reg.0 as usize].0;
    }

    // ========== Meta 操作 ==========
    pub fn set_meta(&mut self, reg: MetaReg, value: f64) {
        self.meta[reg.0 as usize].0 = value;
    }
    
    pub fn get_meta(&self, reg: MetaReg) -> f64 {
        self.meta[reg.0 as usize].0
    }
    
    pub fn increment_meta(&mut self, reg: MetaReg, delta: f64) {
        self.meta[reg.0 as usize].0 += delta;
    }

    // ========== 全局操作 ==========
    pub fn reset_all(&mut self) {
        for i in 0..4 {
            self.reset_text(TextReg(i));
            self.reset_token(TokenReg(i));
            self.reset_tensor(TensorReg(i));
            self.set_flag(FlagReg(i), false);
        }
        for i in 0..8 {
            self.set_meta(MetaReg(i), 0.0);
        }
    }
