//Presented by KeJi
//Date ： 2026-06-17
//Modified Date ： 2026-08-06

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

    /// 从字符串解析车型（大小写不敏感），未知车型返回 None
    pub fn from_str(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "x3" => Some(Self::X3),
            "x3plus" | "x3_plus" | "x3-plus" => Some(Self::X3Plus),
            "x1" => Some(Self::X1),
            "r2" => Some(Self::R2),
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
    }
}
