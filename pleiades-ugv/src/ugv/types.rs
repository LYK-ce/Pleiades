//Presented by KeJi
//Created Date ： 2026-06-17
//Modified Date ： 2026-11-01

//! 设备层共用的数据类型

/// 车型码
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CarType {
    X3 = 0x01,
    X3Plus = 0x02,
    X1 = 0x04,
    R2 = 0x05,
    /// 麦克纳姆轮 M1（task_44 §1.2 #9：本车车型，真车缺省）
    M1 = 0x0A,
}

impl CarType {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0x01 => Some(Self::X3),
            0x02 => Some(Self::X3Plus),
            0x04 => Some(Self::X1),
            0x05 => Some(Self::R2),
            0x0A => Some(Self::M1),
            _ => None,
        }
    }

    /// 从字符串解析车型（大小写不敏感），未知车型返回 None
    pub fn from_str(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "x3" => Some(Self::X3),
            "x3plus" | "x3_plus" | "x3-plus" => Some(Self::X3Plus),
            "x1" => Some(Self::X1),
            "r2" => Some(Self::R2),
            "m1" => Some(Self::M1),
            _ => None,
        }
    }

    pub fn motion_limits(&self) -> (f32, f32, f32) {
        match self {
            Self::X3 => (1.0, 1.0, 5.0),
            Self::X3Plus => (0.7, 0.7, 3.2),
            Self::X1 => (1.0, 1.0, 5.0),
            Self::R2 => (1.8, 0.045, 3.0),
            Self::M1 => (0.7, 0.7, 3.2), // 先按 X3Plus 同档，待上车标定
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_car_type_from_str() {
        assert_eq!(CarType::from_str("X3"), Some(CarType::X3));
        assert_eq!(CarType::from_str("X3Plus"), Some(CarType::X3Plus));
        assert_eq!(CarType::from_str("x3plus"), Some(CarType::X3Plus));
        assert_eq!(CarType::from_str("X3_Plus"), Some(CarType::X3Plus));
        assert_eq!(CarType::from_str(" X1 "), Some(CarType::X1));
        assert_eq!(CarType::from_str("r2"), Some(CarType::R2));
        assert_eq!(CarType::from_str("unknown"), None);
        assert_eq!(CarType::from_str(""), None);
        // task_44 §1.2 #9：M1（大小写 / 首尾空白都认）
        assert_eq!(CarType::from_str("M1"), Some(CarType::M1));
        assert_eq!(CarType::from_str("m1"), Some(CarType::M1));
        assert_eq!(CarType::from_str(" M1 "), Some(CarType::M1));
    }

    #[test]
    fn test_car_type_from_u8_all_five() {
        assert_eq!(CarType::from_u8(0x01), Some(CarType::X3));
        assert_eq!(CarType::from_u8(0x02), Some(CarType::X3Plus));
        assert_eq!(CarType::from_u8(0x04), Some(CarType::X1));
        assert_eq!(CarType::from_u8(0x05), Some(CarType::R2));
        assert_eq!(CarType::from_u8(0x0A), Some(CarType::M1));
        for bad in [0x00, 0x03, 0x0B, 0xFF] {
            assert_eq!(CarType::from_u8(bad), None, "0x{bad:02X} 不是合法车型码");
        }
    }

    #[test]
    fn test_motion_limits_m1_same_as_x3plus() {
        // §1.2 #9：限幅先按 X3Plus 同档 (0.7, 0.7, 3.2)，待上车标定
        assert_eq!(CarType::M1.motion_limits(), (0.7, 0.7, 3.2));
        assert_eq!(CarType::M1.motion_limits(), CarType::X3Plus.motion_limits());
    }
}
