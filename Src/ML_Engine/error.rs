use std::fmt;
use std::error::Error;

#[derive(Debug)]
pub enum RuntimeError {
    NoModelLoaded,
    ModelLoadFailed(String),
    InferenceFailed(String),
    InvalidInput(String),
    InvalidOutput(String),
    TypeMismatch { expected: String, got: String },
    ShapeMismatch { expected: Vec<usize>, got: Vec<usize> },
    BackendNotAvailable(String),
    ConversionFailed(String),
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RuntimeError::NoModelLoaded => write!(f, "没有加载模型"),
            RuntimeError::ModelLoadFailed(msg) => write!(f, "模型加载失败: {}", msg),
            RuntimeError::InferenceFailed(msg) => write!(f, "推理失败: {}", msg),
            RuntimeError::InvalidInput(msg) => write!(f, "无效输入: {}", msg),
            RuntimeError::InvalidOutput(msg) => write!(f, "无效输出: {}", msg),
            RuntimeError::TypeMismatch { expected, got } => {
                write!(f, "类型不匹配: 期望 {}, 实际 {}", expected, got)
            }
            RuntimeError::ShapeMismatch { expected, got } => {
                write!(f, "形状不匹配: 期望 {:?}, 实际 {:?}", expected, got)
            }
            RuntimeError::BackendNotAvailable(backend) => {
                write!(f, "后端不可用: {}", backend)
            }
            RuntimeError::ConversionFailed(msg) => write!(f, "转换失败: {}", msg),
        }
    }
}

impl Error for RuntimeError {}