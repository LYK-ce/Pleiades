//Presented by KeJi
//Created Date ： 2026-09-08
//Modified Date ： 2026-09-08

//! YOLO-v8 目标检测（candle 生态，与 GGUF 管线平行隔离）
//!
//! - `model`：网络结构（YoloV8 / YoloV8Pose，移植自 candle-examples）
//! - `detector`：检测器（Yolo_Detector + 预处理/后处理 + Lua UserData）
//! - `coco_names`：COCO 80 类名

pub mod model;
pub mod detector;
pub mod coco_names;

pub use detector::{Yolo_Detector, Detection};
