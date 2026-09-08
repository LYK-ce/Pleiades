//Presented by KeJi
//Created Date ： 2026-09-08
//Modified Date ： 2026-09-08

//! YOLO-v8 目标检测器（纯 Rust 核心，薄 Lua cap 暴露）
//!
//! - `Yolo_Detector` 长驻：load 一次、detect 多次，与 `MlContext` 同构
//! - 输入 = 图像原始字节（JPEG/PNG），解码在 detect 内部完成
//! - 输出 = 检测框（检测尺度坐标，不缩放回原图）

use candle_core::{DType, Device, IndexOp, Tensor};
use candle_nn::{Module, VarBuilder};
use candle_transformers::object_detection::{non_maximum_suppression, Bbox, KeyPoint};

use super::coco_names::COCO_NAMES;
use super::model::{Multiples, YoloV8};
use crate::ml_engine::device::Parse_Device_Str;

/// 默认 YOLO 模型（第一版写死，权重放 Pleiades_Workspace/）
const DEFAULT_YOLO_MODEL_FILE: &str = "yolov8n.safetensors";
const DEFAULT_YOLO_WHICH: &str = "n";

/// 单个检测框（坐标为检测尺度，即 resize 后尺度）
pub struct Detection {
    pub class_index: usize,
    pub class_name: &'static str,
    pub xmin: f32,
    pub ymin: f32,
    pub xmax: f32,
    pub ymax: f32,
    pub confidence: f32,
}

/// 从 config 读取 workspace 目录，拼出默认权重路径。
fn Default_Model_Path() -> Result<std::path::PathBuf, String> {
    let (config, _) = crate::config::Ensure_Config()
        .map_err(|e| format!("读取配置失败: {e}"))?;
    Ok(config.workspace_dir().join(DEFAULT_YOLO_MODEL_FILE))
}

/// 加载 YOLO 模型（safetensors 全量加载，FP16 存储转 F32 计算）。
fn Yolo_Load_Model(path: &std::path::Path, device: &Device, which: &str) -> Result<YoloV8, String> {
    let multiples = match which {
        "n" => Multiples::n(),
        "s" => Multiples::s(),
        "m" => Multiples::m(),
        "l" => Multiples::l(),
        "x" => Multiples::x(),
        other => return Err(format!("未知 YOLO 型号: {other}（支持 n/s/m/l/x）")),
    };
    let vb = unsafe {
        VarBuilder::from_mmaped_safetensors(&[path], DType::F32, device)
    }
    .map_err(|e| format!("加载权重失败 {}: {e}", path.display()))?;
    YoloV8::load(vb, multiples, /* num_classes= */ 80)
        .map_err(|e| format!("构建模型失败: {e}"))
}

/// YOLO 检测器。
pub struct Yolo_Detector {
    model: YoloV8,
    device: Device,
    class_names: &'static [&'static str],
}

impl Yolo_Detector {
    /// 创建检测器（型号/权重路径写死，权重放 `Pleiades_Workspace/`）。
    pub fn new(device: &str) -> Result<Self, String> {
        let dev = Parse_Device_Str(device)?;
        let path = Default_Model_Path()?;
        let model = Yolo_Load_Model(&path, &dev, DEFAULT_YOLO_WHICH)?;
        Ok(Self {
            model,
            device: dev,
            class_names: &COCO_NAMES,
        })
    }

    /// 检测：输入图像字节（JPEG/PNG），返回检测框（检测尺度坐标）。
    pub fn detect(&self, image_bytes: &[u8], conf_thr: f32, nms_thr: f32) -> Result<Vec<Detection>, String> {
        // 1. 预处理：解码 + resize_exact（保比例 + 32 对齐）
        let image_t = Yolo_Preprocess(image_bytes, &self.device)?;
        // 2. 前向
        let pred = self
            .model
            .forward(&image_t)
            .map_err(|e| format!("forward: {e}"))?
            .squeeze(0)
            .map_err(|e| format!("squeeze: {e}"))?;
        // 3. 后处理：置信度过滤 + NMS
        Yolo_Postprocess(&pred, conf_thr, nms_thr, self.class_names)
    }
}

/// 预处理：解码 + resize_exact（长边 640、短边等比缩到 32 对齐）。
fn Yolo_Preprocess(image_bytes: &[u8], device: &Device) -> Result<Tensor, String> {
    let original_image = image::load_from_memory(image_bytes)
        .map_err(|e| format!("图像解码失败: {e}"))?;
    let (ow, oh) = (
        original_image.width() as usize,
        original_image.height() as usize,
    );
    let (width, height) = if ow < oh {
        let w = ow * 640 / oh;
        (w / 32 * 32, 640)
    } else {
        let h = oh * 640 / ow;
        (640, h / 32 * 32)
    };
    let image_t = {
        let img = original_image.resize_exact(
            width as u32,
            height as u32,
            image::imageops::FilterType::CatmullRom,
        );
        let data = img.to_rgb8().into_raw();
        Tensor::from_vec(data, (height, width, 3), device)
            .map_err(|e| format!("from_vec: {e}"))?
            .permute((2, 0, 1))
            .map_err(|e| format!("permute: {e}"))?
    };
    let image_t = (image_t
        .unsqueeze(0)
        .map_err(|e| format!("unsqueeze: {e}"))?
        .to_dtype(DType::F32)
        .map_err(|e| format!("to_dtype: {e}"))?
        * (1. / 255.))
    .map_err(|e| format!("scale: {e}"))?;
    Ok(image_t)
}

/// 后处理：置信度过滤 + NMS，返回检测尺度坐标（不缩放回原图）。
fn Yolo_Postprocess(
    pred: &Tensor,
    conf_thr: f32,
    nms_thr: f32,
    class_names: &'static [&'static str],
) -> Result<Vec<Detection>, String> {
    let pred = pred
        .to_device(&Device::Cpu)
        .map_err(|e| format!("to_device: {e}"))?;
    let (pred_size, npreds) = pred.dims2().map_err(|e| format!("dims2: {e}"))?;
    let nclasses = pred_size - 4;
    if nclasses > class_names.len() {
        return Err(format!("类别数 {nclasses} 超出 COCO 80 类"));
    }

    let mut bboxes: Vec<Vec<Bbox<Vec<KeyPoint>>>> = (0..nclasses).map(|_| vec![]).collect();
    for index in 0..npreds {
        let pred_v = Vec::<f32>::try_from(pred.i((.., index)).map_err(|e| format!("i: {e}"))?)
            .map_err(|e| format!("try_from: {e}"))?;
        let confidence = *pred_v[4..]
            .iter()
            .max_by(|x, y| x.total_cmp(y))
            .unwrap();
        if confidence > conf_thr {
            let mut class_index = 0;
            for i in 0..nclasses {
                if pred_v[4 + i] > pred_v[4 + class_index] {
                    class_index = i;
                }
            }
            if pred_v[class_index + 4] > 0. {
                let bbox = Bbox {
                    xmin: pred_v[0] - pred_v[2] / 2.,
                    ymin: pred_v[1] - pred_v[3] / 2.,
                    xmax: pred_v[0] + pred_v[2] / 2.,
                    ymax: pred_v[1] + pred_v[3] / 2.,
                    confidence,
                    data: vec![],
                };
                bboxes[class_index].push(bbox);
            }
        }
    }

    non_maximum_suppression(&mut bboxes, nms_thr);

    let mut dets = Vec::new();
    for (class_index, bboxes_for_class) in bboxes.iter().enumerate() {
        for b in bboxes_for_class.iter() {
            dets.push(Detection {
                class_index,
                class_name: class_names[class_index],
                xmin: b.xmin,
                ymin: b.ymin,
                xmax: b.xmax,
                ymax: b.ymax,
                confidence: b.confidence,
            });
        }
    }
    Ok(dets)
}

/// 检测结果 → Lua table（独立函数，遵守「闭包只做薄胶水」规范）。
fn detections_to_lua_table(lua: &mlua::Lua, dets: &[Detection]) -> mlua::Result<mlua::Table> {
    let t = lua.create_table()?;
    for (i, d) in dets.iter().enumerate() {
        let row = lua.create_table()?;
        row.set("class_index", d.class_index)?;
        row.set("class_name", d.class_name)?;
        row.set("xmin", d.xmin)?;
        row.set("ymin", d.ymin)?;
        row.set("xmax", d.xmax)?;
        row.set("ymax", d.ymax)?;
        row.set("confidence", d.confidence)?;
        t.set(i + 1, row)?;
    }
    Ok(t)
}

impl mlua::UserData for Yolo_Detector {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        // 薄胶水：闭包只做类型转换 + 错误映射，业务逻辑在 detect()
        methods.add_method(
            "detect",
            |lua, this, (image_bytes, conf, nms): (mlua::String, f32, f32)| {
                let dets = this
                    .detect(image_bytes.as_bytes().as_ref(), conf, nms)
                    .map_err(|e| mlua::Error::runtime(e))?;
                detections_to_lua_table(lua, &dets)
            },
        );
    }
}
