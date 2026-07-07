//Presented by KeJi
//Date ： 2026-06-17

//! 设备层共用的数据类型

/// 车型码
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CarType {
    X3 = 0x01,
    X3Plus = 0x02,
    X1 = 0x04,
    R2 = 0x05,
}

impl CarType {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0x01 => Some(Self::X3),
            0x02 => Some(Self::X3Plus),
            0x04 => Some(Self::X1),
            0x05 => Some(Self::R2),
            _ => None,
        }
    }

    pub fn motion_limits(&self) -> (f32, f32, f32) {
        match self {
            Self::X3 => (1.0, 1.0, 5.0),
            Self::X3Plus => (0.7, 0.7, 3.2),
            Self::X1 => (1.0, 1.0, 5.0),
            Self::R2 => (1.8, 0.045, 3.0),
        }
    }
}
