//Presented by KeJi
//Created Date ： 2026-09-08
//Modified Date ： 2026-09-08

//! YOLO detect 验证工具（本地图 + 真实权重，不经过 Lua）。
//!
//! 用法：cargo run --example yolo_detect --no-default-features

use pleiades_base::ml_engine::yolo::Yolo_Detector;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let jpeg = std::fs::read("Pleiades_Workspace/bike.jpg")?;
    let det = Yolo_Detector::new("yolov8n.safetensors", "cpu")?;
    let dets = det.detect(&jpeg, 0.25, 0.45)?;
    println!("detected {} objects", dets.len());
    for d in &dets {
        println!(
            "{}  conf={:.4}  [{:.2}, {:.2}, {:.2}, {:.2}]",
            d.class_name, d.confidence, d.xmin, d.ymin, d.xmax, d.ymax
        );
    }
    Ok(())
}
