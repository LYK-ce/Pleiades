//Presented by KeJi
//Created Date ： 2026-10-27
//Modified Date ： 2026-10-27

//! 三值状态 + 失败原因码（Task 36 §12.2.4）
//!
//! 通用码 0~4 由 base 定义；设备自定义码从 1000 起（`FailCode::Device(u16)`），base **不解释**
//! 其语义。树里尽量只用通用码——用了设备码，这棵树就搬不到别的设备（§十一 #31）。

/// 失败原因码
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailCode {
    /// 0：条件不成立 / 未指定
    Unspecified,
    /// 1：`Timeout` 装饰器到期
    Timeout,
    /// 2：`Retry` 用尽
    RetryExhausted,
    /// 3：导航——地图上无路
    Unreachable,
    /// 4：导航——暂时被堵
    Blocked,
    /// 1000+：设备自定义（语义由设备端定义，base 只透传）
    Device(u16),
}

impl FailCode {
    /// 上报 / 日志用的数值编码（通用码 0~4，设备码 1000+）；**不参与控制流**
    pub fn code(self) -> u32 {
        match self {
            FailCode::Unspecified => 0,
            FailCode::Timeout => 1,
            FailCode::RetryExhausted => 2,
            FailCode::Unreachable => 3,
            FailCode::Blocked => 4,
            FailCode::Device(c) => 1000 + c as u32,
        }
    }
}

/// 三值状态；`Failure` 必带原因码（§十一 #31）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// 本拍完成
    Success,
    /// 还在进行，下拍继续
    Running,
    /// 失败（带原因码）
    Failure(FailCode),
}
