//Presented by KeJi
//Created Date ： 2026-10-24
//Modified Date ： 2026-10-24

#![allow(non_snake_case)]
// 本文件按仓库命名规范用 Pascal_snake 命名结构体（如 `Sensor_Info`），故关掉 camel-case 检查
#![allow(non_camel_case_types)]

//! 设备能力注册（sim 侧）：把最新相机帧暴露成 Lua 全局表 **`camera_sim`**
//! （task_34 §二 Step 4；契约与真机两个相机 sensor **逐字一致**）
//!
//! 两个成员，**都是同步**（数据本来就在内存里，没有"等未来帧"这一说）：
//! - `capture()` → 最新一帧的 JPEG（Lua string）；无帧 → `Err("还没有收到相机帧")`，**不阻塞、不等待**
//! - `info()` → `{width, height, format, age_ms}`，**永不抛错**
//!
//! 与真机的三级回退同构（只是没有"打开协商"这一级）：
//! - 有帧 → 宽高取 **JPEG SOF 里的真值**（`Latest_Size()`），`format = "JPEG"`，`age_ms = Age_Ms()`
//! - 无帧 → 宽高取 `[sim_world.sensor.params]`（缺键 → 0），`format = "none"`，`age_ms = 0`
//!
//! 闭包只做薄胶水（[`Capture_Of`] / [`Info_Table`]），业务逻辑在同文件的自由函数里
//! → 可单测、不依赖 Lua 运行时。
//!
//! 本文件只在 sim 侧注册一个表：不新增命令通路口、不碰控制口、不改 `on_tick`。

use std::sync::Arc;

use pleiades_base::orchestrator::DeviceCapability;

use crate::sim::sensors::camera::CameraSensor;
use crate::sim::sensors::Warn_Throttled;

/// Lua 契约里的 `format`：世界侧只出 JPEG
///
/// 与真机的 token（`MJPG` / `NV12` / `YUYV` …）区分开，日志里一眼看得出来源是仿真。
pub const FORMAT_JPEG: &str = "JPEG";

/// Lua 契约里的 `format`：**从未**收到帧（与真机两 sensor 的 `none` 同义）
pub const FORMAT_NONE: &str = "none";

/// 无帧时的错误文案（Lua `capture()` 会把它变成脚本错误；`camera_photo.lua` 刻意不 pcall，
/// 要的就是这行能出现在"执行失败: …"里）
pub const NO_FRAME_ERR: &str = "还没有收到相机帧";

/// 帧龄超过 `2 × 周期` 视为过期（**仍返回该帧**，只额外限流告警）
pub const STALE_PERIODS: u64 = 2;

/// `capture()` / `info()` 的业务结果（`info()` 的返回形状；纯数据，不含 Lua 类型）
#[derive(Debug, Clone, PartialEq)]
struct Sensor_Info {
    width: u32,
    height: u32,
    format: String,
    age_ms: i64,
}

/// 一次 capture 的取值：无帧 → `Err(NO_FRAME_ERR)`，**不等待未来帧**
fn Capture_Of(sensor: &Arc<CameraSensor>) -> Result<Vec<u8>, String> {
    match sensor.Latest() {
        Some(f) => Ok(f.jpeg),
        None => Err(NO_FRAME_ERR.to_string()),
    }
}

/// `info()` 的取值（**从不失败**）
fn Info_Of(sensor: &Arc<CameraSensor>) -> Sensor_Info {
    match sensor.Latest_Size() {
        Some((width, height)) => Sensor_Info {
            width,
            height,
            format: FORMAT_JPEG.to_string(),
            age_ms: sensor.Age_Ms(),
        },
        // 从未收到帧：宽度取配置（缺键 = 0），format = none，age_ms = 0
        // （与真机"从未抓到帧时 age_ms 取 0、format 取 none"逐字一致）
        None => Sensor_Info {
            width: sensor.Params().width,
            height: sensor.Params().height,
            format: FORMAT_NONE.to_string(),
            age_ms: 0,
        },
    }
}

/// 帧周期（毫秒）：`rate_hz <= 0` → `0`（未知周期 → 不做过期告警）
fn Period_Ms(rate_hz: f32) -> u64 {
    if rate_hz > 0.0 {
        // 1 Hz → 1000 ms；30 Hz → 33 ms（向下取整，宁可早告警也不漏告警）
        (1000.0 / rate_hz as f64) as u64
    } else {
        0
    }
}

/// 帧龄超过 [`STALE_PERIODS`] 个周期 → 限流 warning（**不影响返回值**：照样把这一帧给 Lua）
///
/// 限流 key 与 `camera.bad_frame` 分开（两个限流表项互不影响）。
fn Warn_If_Stale(sensor: &Arc<CameraSensor>) {
    let period = Period_Ms(sensor.Params().rate_hz);
    if period == 0 {
        return; // 周期未知（rate_hz 未配置/非法）：不告警
    }
    let age = sensor.Age_Ms();
    if age >= 0 && age as u64 > STALE_PERIODS * period {
        Warn_Throttled("camera.stale_frame", 5000, || {
            format!(
                "[camera] slot={} 最新帧已过期 {} ms（周期 {} ms × {}）：capture() 仍返回这一帧，请检查世界侧是否在出图",
                sensor.Slot(),
                age,
                period,
                STALE_PERIODS
            )
        });
    }
}

/// `Sensor_Info` → Lua table：**只做类型转换**（数值一律 `as i64`，避免 `u64 > i64::MAX` 在 Lua 侧报错）
///
/// 字段名**逐字固定**：`width / height / format / age_ms`（恰好 4 个键，
/// `camera_photo.lua` 与真机两个 sensor 都按这套名字取）。
fn Info_Table(lua: &mlua::Lua, info: &Sensor_Info) -> mlua::Result<mlua::Table> {
    let table = lua.create_table()?;
    table.set("width", info.width as i64)?;
    table.set("height", info.height as i64)?;
    table.set("format", info.format.clone())?;
    table.set("age_ms", info.age_ms)?;
    Ok(table)
}

/// 设备能力注册（sim 侧）：Lua 全局表 `camera_sim`
pub struct CameraSimCap {
    sensor: Arc<CameraSensor>,
}

impl CameraSimCap {
    pub fn New(sensor: Arc<CameraSensor>) -> Self {
        Self { sensor }
    }

}

/// `capture()` 的业务结果（= [`Capture_Of`] + 帧龄过期告警）
fn Capture_Jpeg(sensor: &Arc<CameraSensor>) -> Result<Vec<u8>, String> {
    let jpeg = Capture_Of(sensor)?;
    Warn_If_Stale(sensor);
    Ok(jpeg)
}

/// `info()` 的业务结果
fn Info(sensor: &Arc<CameraSensor>) -> Sensor_Info {
    Info_Of(sensor)
}

/// 设备能力注册（sim 侧）：Lua 全局表 `camera_sim`，两个成员
/// - `capture()`：**同步**，返回最新帧的 JPEG（Lua string）；无帧 → `mlua::Error::runtime("还没有收到相机帧")`
/// - `info()`：**同步**，返回 `{width, height, format, age_ms}`，永不抛错
impl DeviceCapability for CameraSimCap {
    fn register_lua_caps(&self, lua: &mlua::Lua) -> mlua::Result<()> {
        let capture_sensor = self.sensor.clone();
        let table = lua.create_table()?;
        table.set(
            "capture",
            lua.create_function(move |lua, (): ()| {
                let jpeg = Capture_Jpeg(&capture_sensor).map_err(mlua::Error::runtime)?;
                lua.create_string(jpeg)
            })?,
        )?;

        let info_sensor = self.sensor.clone();
        table.set(
            "info",
            lua.create_function(move |lua, (): ()| Info_Table(lua, &Info(&info_sensor)))?,
        )?;

        lua.globals().set("camera_sim", table)?;
        Ok(())
    }
}

// ================================================================ 单测

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::sensors::camera::frame::{Test_Frame, Test_Jpeg};
    use crate::sim::sensors::camera::CameraParams;

    fn sensor_with(params: CameraParams) -> Arc<CameraSensor> {
        Arc::new(CameraSensor::New("camera_nose", params))
    }

    fn feed(sensor: &Arc<CameraSensor>, seq: u32, jpeg: &[u8]) {
        sensor.On_Binary(&Test_Frame(3, seq, seq as f64 * 0.5, jpeg));
    }

    #[test]
    fn test_capture_without_frame_is_err() {
        let s = sensor_with(CameraParams::default());
        let e = match Capture_Jpeg(&s) {
            Ok(_) => String::new(),
            Err(e) => e,
        };
        assert_eq!(e, NO_FRAME_ERR, "无帧必须返回固定文案");
        assert_eq!(e, "还没有收到相机帧");
    }

    #[test]
    fn test_capture_returns_latest_jpeg() {
        let s = sensor_with(CameraParams::default());
        let j1 = Test_Jpeg(320, 240);
        feed(&s, 1, &j1);
        assert_eq!(Capture_Jpeg(&s), Ok(j1.clone()));
        // 再喂一张：返回的是**后者**（证明不是缓存第一帧）
        let j2 = Test_Jpeg(640, 480);
        feed(&s, 2, &j2);
        assert_eq!(Capture_Jpeg(&s), Ok(j2.clone()));
        assert_ne!(j1, j2);
    }

    #[test]
    fn test_info_without_frame() {
        let params = CameraParams { width: 640, height: 480, rate_hz: 1.0 };
        let s = sensor_with(params);
        let info = Info(&s);
        assert_eq!(info.format, FORMAT_NONE);
        assert_eq!(info.age_ms, 0);
        assert_eq!((info.width, info.height), (640, 480), "无帧时宽高取配置值");
    }

    #[test]
    fn test_info_with_frame_from_sof() {
        // 配置写 640x480，实际帧是 320x240：**以 JPEG 为准**
        let params = CameraParams { width: 640, height: 480, rate_hz: 1.0 };
        let s = sensor_with(params);
        feed(&s, 1, &Test_Jpeg(320, 240));
        let info = Info(&s);
        assert_eq!((info.width, info.height), (320, 240), "宽高必须来自 JPEG SOF");
        assert_eq!(info.format, FORMAT_JPEG);
        assert!(info.age_ms >= 0, "帧龄不得为负：{}", info.age_ms);
    }

    #[test]
    fn test_period_and_stale_threshold() {
        assert_eq!(Period_Ms(1.0), 1000);
        assert_eq!(Period_Ms(0.0), 0);
        assert_eq!(Period_Ms(-1.0), 0);
        assert_eq!(Period_Ms(2.0), 500);
        // 无帧时 Age_Ms()=0 → 不应告警（阈值 2×1000）；这里只验证不 panic
        let s = sensor_with(CameraParams::default());
        Warn_If_Stale(&s);
        feed(&s, 1, &Test_Jpeg(64, 48));
        Warn_If_Stale(&s); // 刚喂完：age≈0 < 2000 → 不告警
        assert_eq!(s.Age_Ms() >= 0, true);
    }

    #[test]
    fn test_lua_camera_sim_table_contract() {
        let lua = mlua::Lua::new();
        let s = sensor_with(CameraParams { width: 640, height: 480, rate_hz: 1.0 });
        let cap = CameraSimCap::New(s.clone());
        assert!(cap.register_lua_caps(&lua).is_ok());
        assert!(lua.globals().get::<mlua::Table>("camera_sim").is_ok(), "必须注册全局表 camera_sim");

        // ① 恰好两个成员
        let n: i64 = match lua.load("local n = 0 for _ in pairs(camera_sim) do n = n + 1 end return n").eval() {
            Ok(n) => n,
            Err(e) => {
                assert!(false, "pairs 计数失败：{e}");
                return;
            }
        };
        assert_eq!(n, 2, "camera_sim 只能有 capture / info 两个成员");

        // ② info() 恰好 4 个键，且三个数值是 Lua integer、format 是 string
        let script = r#"
            local i = camera_sim.info()
            local n = 0
            for _ in pairs(i) do n = n + 1 end
            return n, math.type(i.width), math.type(i.height), math.type(i.age_ms), type(i.format)
        "#;
        let got = match lua.load(script).eval::<(i64, String, String, String, String)>() {
            Ok(v) => v,
            Err(e) => {
                assert!(false, "info() 契约调用失败：{e}");
                return;
            }
        };
        assert_eq!(got.0, 4, "info() 恰好 4 个键");
        assert_eq!(got.1, "integer", "width 必须是 Lua integer");
        assert_eq!(got.2, "integer", "height 必须是 Lua integer");
        assert_eq!(got.3, "integer", "age_ms 必须是 Lua integer");
        assert_eq!(got.4, "string", "format 必须是 string");

        // ③ 无帧时 capture() 抛错，文案是被测常量
        let pcall = match lua.load(r#"
            local ok, err = pcall(camera_sim.capture)
            return ok, tostring(err)
        "#).eval::<(bool, String)>() {
            Ok(v) => v,
            Err(e) => {
                assert!(false, "pcall(capture) 无法求值：{e}");
                return;
            }
        };
        assert!(!pcall.0, "无帧时 capture() 必须抛错");
        assert!(pcall.1.contains(NO_FRAME_ERR), "错误文案必须含「{NO_FRAME_ERR}」：{}", pcall.1);

        // ④ 有帧时 capture() 返回 string，长度 == JPEG 字节数
        let jpeg = Test_Jpeg(320, 240);
        feed(&s, 1, &jpeg);
        let len: usize = match lua.load("return #camera_sim.capture()").eval() {
            Ok(v) => v,
            Err(e) => {
                assert!(false, "有帧时 capture() 必须成功：{e}");
                return;
            }
        };
        assert_eq!(len, jpeg.len());
        // info() 同步跟着变（以 JPEG 为准的宽高）
        let (w, h, fmt): (i64, i64, String) = match lua
            .load("local i = camera_sim.info() return i.width, i.height, i.format")
            .eval()
        {
            Ok(v) => v,
            Err(e) => {
                assert!(false, "info() 取值失败：{e}");
                return;
            }
        };
        assert_eq!((w, h), (320, 240));
        assert_eq!(fmt, FORMAT_JPEG);
    }
}
