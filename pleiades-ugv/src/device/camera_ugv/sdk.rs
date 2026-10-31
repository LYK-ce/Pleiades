//Presented by KeJi
//Created Date ： 2026-10-31
//Modified Date ： 2026-10-31

#![allow(non_snake_case)]
#![allow(non_camel_case_types)]
// 非 unix 平台只编译 stub（BRIEF 硬约束 3：本机也能 cargo check）：其余代码是"给静态检查
// 看的"，定义了没人用是预期的。这里**不能**无条件 allow(dead_code)，否则会把 Linux 侧
// 真正的死代码问题一起盖掉。
#![cfg_attr(not(unix), allow(dead_code))]

//! HP60C（Angstrong / ascamera）FFI 层（task_43 Step 1）
//!
//! 参考实现：`rust_uav_camera_test/ugv_camera/rust_probe/src/main.rs`（只读、不入库）
//! + 同目录 `RESULTS.md`（实机结论）+ `.engine/BRIEF.md`（硬约束清单）。
//!
//! 三条不可退让（对应 `RESULTS.md` §4 / BRIEF 硬约束 4~7）：
//! 1) **不镜像 C++ 结构体**：`AS_SDK_Data_s` 末成员是 `std::vector`，只按实测偏移
//!    `ptr::read_unaligned` 读值（帧结构 48 B，`size` +24、`ts` +40）；
//! 2) **回调三纪律**：整条回调链 `catch_unwind(AssertUnwindSafe(..))`、零日志、只用 `try_lock`、
//!    当场 `to_vec()`（缓冲是借用的，回调返回后 SDK 立刻清零 size）；
//! 3) **清理顺序固定且幂等**：`StopStream`（只一次）→ `CloseCamera` → `DestoryCamHandle` →
//!    `Deinit` → `dlclose`；**不调用** `AS_SDK_FreeCameraList`（实机 double free → SIGABRT）。
//!
//! 本文件按"验收脚本按横幅切段"的习惯分节，横幅文字见每节的 `==== §n ... ====`。

use std::os::raw::c_void;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tracing::warn;

use crate::device::camera_ugv::slot::{FrameSlot, Monotonic_Ms};

// ===== §2 常量与布局断言 =====
//
// 【偏移的来源，逐条可核对】
// AS_Frame_s（as_camera_sdk_def.h:48-57）是纯 POD：
//     type(i32) width(u32) height(u32) data(void*) size(u32) bufferSize(u32)
//     frameId(u32) ts(u64)
// 在 aarch64 SysV 下它的 sizeof = 48，字段偏移不是"算出来的"，而是被反汇编实测校验过的：
//     AS_SDK_Data_s 是 camera 对象内偏移 +0x170 的常驻成员（add x1, x19, #0x170）；
//     回调返回后 SDK 清 size 的绝对地址是 #392 / #440 / #488 / #536 / #680；
//     392-368 = 24  → depthImg.size      = 基址 0   + 24
//     440-368 = 72  → rgbImg.size        = 基址 48  + 24
//     488-368 = 120 → irImg.size         = 基址 96  + 24
//     536-368 = 168 → pointCloud.size    = 基址 144 + 24
//     680-368 = 312 → mjpegImg.size      = 基址 288 + 24   ← 本设备只读这一路
// 其中 368 = 0x170，正是 pstData 相对相机对象的偏移。
// 7 路 AS_Frame_s 顺序排列，每路 48 B；第 8 个成员 pointCloud2 是 std::vector，
// 位于 +336，本设备**一个字节都不读**。

/// AS_Frame_s 的大小（7 路帧紧排，每路 48 B）
pub const FRAME_S_SIZE: usize = 48;

/// AS_SDK_Data_s 里 7 路帧的基址（AS_Frame_s 顺序排列，每路 FRAME_S_SIZE）
pub const OFF_DATA_DEPTH: usize = 0;
pub const OFF_DATA_RGB: usize = 48;
pub const OFF_DATA_IR: usize = 96;
pub const OFF_DATA_POINTCLOUD: usize = 144;
pub const OFF_DATA_YUYV: usize = 192;
pub const OFF_DATA_PEAK: usize = 240;
/// ← 本设备**只读这一路**：`StartStream(cam, 0x83)` 之后这里已是完整 JPEG
pub const OFF_DATA_MJPEG: usize = 288;
/// std::vector：**仅用于布局断言，绝不读**
pub const OFF_DATA_POINTCLOUD2_VEC: usize = 336;

/// AS_Frame_s 内部字段偏移（sizeof = 48）
pub const OFF_FRAME_TYPE: usize = 0; // i32  AS_FRAME_Type_e
pub const OFF_FRAME_WIDTH: usize = 4; // u32
pub const OFF_FRAME_HEIGHT: usize = 8; // u32
pub const OFF_FRAME_DATA: usize = 16; // *const u8（8 字节对齐，故 +12 有 4 B padding）
pub const OFF_FRAME_SIZE: usize = 24; // u32 ← 有效字节数，拷贝长度只信它
pub const OFF_FRAME_BUFFER_SIZE: usize = 28; // u32 只记录，**不做闸门**
pub const OFF_FRAME_ID: usize = 32; // u32 只锁布局，不读值
pub const OFF_FRAME_TS: usize = 40; // u64（实测单位毫秒）

// libstdc++ std::list<void*> 的布局（手搓空 list 用，见 §6）
// std::__cxx11::list<void*> = { _M_impl{ _List_node_base _M_node{_M_next,_M_prev},
//                                        size_type _M_size } } → 3 个机器字，24 B
pub const OFF_LIST_NEXT: usize = 0;
pub const OFF_LIST_PREV: usize = 8;
pub const OFF_LIST_SIZE: usize = 16;
/// std::_List_node<void*> = { _List_node_base(2 个机器字) ; void* _M_value }
/// ⇒ 节点 payload（真正要的相机句柄）在节点首地址 +16
pub const OFF_LIST_NODE_PAYLOAD: usize = 16;

// MEDIA_TYPE_E（as_camera_sdk_def.h:180-186）：无固定底层类型的小值枚举，
// GCC 取 unsigned int（4 B，走 w1）。Rust 侧用 i32 传，位模式相同。
pub const MEDIA_TYPE_DEPTH: i32 = 0; // AS_MEDIA_TYPE_DEPTH
pub const MEDIA_TYPE_RGB: i32 = 1; // AS_MEDIA_TYPE_RGB

// *_IMG_FLG 位掩码（as_camera_sdk_def.h:25-33）
pub const IMG_FLG_DEPTH: i32 = 0x01;
pub const IMG_FLG_RGB: i32 = 0x02;
pub const IMG_FLG_MJPEG: i32 = 0x80;
/// `StartStream` 的位掩码：DEPTH|RGB|MJPEG = **0x83**。
///
/// ⚠ **不做 0x03 降级**（与参考实现的差异，刻意的）：本设备只读 `mjpegImg`，
/// 降级到 0x03 后这一路 size 恒 0，等于"白开流"；宁可 `Open` 失败并点名 flag。
pub const STREAM_FLAG: i32 = IMG_FLG_DEPTH | IMG_FLG_RGB | IMG_FLG_MJPEG; // = 0x83
/// `StopStream` 的第二个参数：`RESULTS.md` §4 实机验证「`StopStream(cam, 0)` 只调一次」
pub const STOP_STREAM_MASK: i32 = 0;

/// 防脏数据闸门：实测 JPEG 约 13 KB，8 MiB 是"明显不是一帧"的上限。
/// 意义：万一偏移读错拿到垃圾 size，也不会拿它去 from_raw_parts 造成巨量越界读。
pub const MAX_JPEG_BYTES: usize = 8 * 1024 * 1024;
/// 遍历 std::list 的节点数上限（只有 1 台相机；上限用来防止链表被读歪后死循环）
pub const LIST_MAX_NODES: usize = 64;
// dlopen 标志（glibc：dlfcn.h 里 RTLD_NOW = 0x2，RTLD_LOCAL = 0x0）
pub const RTLD_NOW: i32 = 2;
pub const RTLD_LOCAL: i32 = 0;

// ---- 编译期布局断言（写错就编译不过）----
const _: () = assert!(FRAME_S_SIZE == 48);
const _: () = assert!(OFF_DATA_MJPEG + OFF_FRAME_SIZE == 312); // = 反汇编 #680 - 368
const _: () = assert!(OFF_DATA_MJPEG + FRAME_S_SIZE == OFF_DATA_POINTCLOUD2_VEC);
const _: () = assert!(OFF_FRAME_TS + 8 == FRAME_S_SIZE);

/// 配置期布局自检（debug 构建才有；release 下整段被裁掉）。
///
/// 作用：把"偏移表"和"反汇编实测地址"的对应关系**写死在代码里**，将来有人改一个
/// 常量就会在 debug 跑的时候立刻炸掉，而不是静默读错内存。`Sdk::Open` 第一步调用。
pub fn Debug_Assert_Layout() {
    // 每路帧 48 B 紧密排列
    debug_assert_eq!(OFF_DATA_RGB - OFF_DATA_DEPTH, FRAME_S_SIZE);
    debug_assert_eq!(OFF_DATA_IR - OFF_DATA_RGB, FRAME_S_SIZE);
    debug_assert_eq!(OFF_DATA_POINTCLOUD - OFF_DATA_IR, FRAME_S_SIZE);
    debug_assert_eq!(OFF_DATA_YUYV - OFF_DATA_POINTCLOUD, FRAME_S_SIZE);
    debug_assert_eq!(OFF_DATA_PEAK - OFF_DATA_YUYV, FRAME_S_SIZE);
    debug_assert_eq!(OFF_DATA_MJPEG - OFF_DATA_PEAK, FRAME_S_SIZE);
    // 7 路帧之后紧接着就是那个 std::vector（+336）
    debug_assert_eq!(OFF_DATA_MJPEG + FRAME_S_SIZE, OFF_DATA_POINTCLOUD2_VEC);

    // 与反汇编里的清零地址逐条对上（368 = 0x170 = pstData 偏移）
    debug_assert_eq!(OFF_DATA_MJPEG + OFF_FRAME_SIZE, 312); // #680 - 368
    debug_assert_eq!(OFF_DATA_DEPTH + OFF_FRAME_SIZE, 24); // #392 - 368
    debug_assert_eq!(OFF_DATA_RGB + OFF_FRAME_SIZE, 72); // #440 - 368
    debug_assert_eq!(OFF_DATA_IR + OFF_FRAME_SIZE, 120); // #488 - 368
    debug_assert_eq!(OFF_DATA_POINTCLOUD + OFF_FRAME_SIZE, 168); // #536 - 368

    // 帧内字段偏移（sizeof = 48）
    debug_assert_eq!(OFF_FRAME_TYPE, 0);
    debug_assert_eq!(OFF_FRAME_WIDTH, 4);
    debug_assert_eq!(OFF_FRAME_HEIGHT, 8);
    debug_assert_eq!(OFF_FRAME_DATA, 16);
    debug_assert_eq!(OFF_FRAME_SIZE, 24);
    debug_assert_eq!(OFF_FRAME_BUFFER_SIZE, 28);
    debug_assert_eq!(OFF_FRAME_ID, 32);
    debug_assert_eq!(OFF_FRAME_TS, 40);
    debug_assert_eq!(OFF_FRAME_TS + 8, FRAME_S_SIZE);

    // libstdc++ std::list 的三个机器字
    debug_assert_eq!(OFF_LIST_NEXT, 0);
    debug_assert_eq!(OFF_LIST_PREV, 8);
    debug_assert_eq!(OFF_LIST_SIZE, 16);
    debug_assert_eq!(OFF_LIST_SIZE + std::mem::size_of::<usize>(), 24);
    debug_assert_eq!(OFF_LIST_NODE_PAYLOAD, 16);

    debug_assert_eq!(std::mem::size_of::<Cxx_List>(), 24);
    debug_assert_eq!(std::mem::size_of::<Stream_Param>(), 12);
    debug_assert_eq!(std::mem::size_of::<Stream_Callback>(), 16);
}

// ===== §3 FFI 镜像类型与读帧工具 =====
//
// 这三个类型都只有标量/裸指针，是真正的 POD，用 #[repr(C)] 镜像它们是安全的。
// 注意：**没有** AS_SDK_Data_s 的镜像（原因见文件头 1)）。

/// libstdc++ `std::list<void*>` 的对象布局：3 个机器字。
///
/// 只做两件事：① 作为 `AS_SDK_GetCameraList` 的实参；② 按偏移读 next/prev 取节点。
/// **必须放在 `Box` 里**（见 `Sdk.dev_list` 的注释）：list 一旦被填充，节点里的
/// prev/next 指向的是**这个对象自己的地址**，按值移动 = 野指针。
#[repr(C)]
pub struct Cxx_List {
    /// +0  _M_next
    next: *mut c_void,
    /// +8  _M_prev
    prev: *mut c_void,
    /// +16 _M_size
    size: usize,
}

/// `STREAM_PARAM_S`（as_camera_sdk_def.h:132）`{ int width; int height; int fps; }`
/// sizeof = 12，三个 4 B 整型紧排、无 padding。
#[repr(C)]
pub struct Stream_Param {
    width: i32,
    height: i32,
    fps: i32,
}

/// `STREAM_CALLBACK_S`（as_camera_sdk_def.h:84-87）`{ callback; privateData; }`
/// 两个指针，sizeof = 16，是 POD。
///
/// 回调签名用 `*const u8` 而不是 `*const AS_SDK_Data_s`：aarch64 SysV 下所有对象指针
/// 的传参方式完全一致（x1 寄存器），`*const u8` 能表达"这里只是一块不可变内存的首地址"，
/// 同时避免为 C++ 类型造一个 Rust 镜像。
/// `Option<extern "C" fn>` 与裸函数指针同大小（空指针优化），故 sizeof 仍是 16。
#[repr(C)]
pub struct Stream_Callback {
    callback: Option<extern "C" fn(cam: *mut c_void, data: *const u8, private_data: *mut c_void)>,
    private_data: *mut c_void,
}

/// 从 AS_SDK_Data_s 里读出来的"某一路帧"的快照（只拷贝值，不借用任何结构体）。
#[derive(Clone, Copy)]
pub struct Frame_View {
    /// 期望 6 = AS_FRAME_TYPE_MJPEG（只随帧读出，回调里不做判断）
    #[allow(dead_code)] // 只做现场取证/布局自检，回调里刻意不据此拒帧（拒帧只看 size）
    pub frame_type: i32,
    pub width: u32,
    pub height: u32,
    pub data: *const u8,
    pub size: u32,
    /// 随帧带出、只记录（**不做闸门**：这一路的 bufferSize 没有实测数据，
    /// 万一恒为 0，拿 "< size 就拒" 会把正常帧全部拒掉）
    #[allow(dead_code)]
    pub buffer_size: u32,
    /// SDK 侧时间戳（实测毫秒）；槽位里存的是**本机单调毫秒**，两者不同源
    #[allow(dead_code)]
    pub ts_ms: u64,
}

// ---- 按偏移读的原子操作 ----
// 全部用 read_unaligned：AS_Frame_s 实际是 8 字节对齐的，这里"从严"处理，
// 这样即使将来有人把 Frame_View 读到一块手工拼的缓冲上也不会踩 UB。
// SAFETY（对下面三个函数的统一前提）：base 指向一块至少可读 off+sizeof 字节的内存，
// 由调用方保证——唯二的调用方是 Read_Frame_View（base 是 SDK 传来的 pstData）
// 与 §7 的单测（base 是手搓的 336 B blob），off 都来自 §2 里经反汇编实测的常量。
#[inline]
unsafe fn Read_U32(base: *const u8, off: usize) -> u32 {
    std::ptr::read_unaligned(base.add(off) as *const u32)
}

#[inline]
unsafe fn Read_U64(base: *const u8, off: usize) -> u64 {
    std::ptr::read_unaligned(base.add(off) as *const u64)
}

#[inline]
unsafe fn Read_Ptr(base: *const u8, off: usize) -> *const c_void {
    std::ptr::read_unaligned(base.add(off) as *const *const c_void)
}

/// 从 `pstData` 基址 + 某一路帧的偏移，读出一份 [`Frame_View`]。
///
/// # Safety
/// `base` 必须是回调收到的 `const AS_SDK_Data_s*`（非空），`off` 必须是
/// §2 里那 7 个 `OFF_DATA_*` 之一。
pub unsafe fn Read_Frame_View(base: *const u8, off: usize) -> Frame_View {
    let p = base.add(off);
    Frame_View {
        frame_type: Read_U32(p, OFF_FRAME_TYPE) as i32,
        width: Read_U32(p, OFF_FRAME_WIDTH),
        height: Read_U32(p, OFF_FRAME_HEIGHT),
        data: Read_Ptr(p, OFF_FRAME_DATA) as *const u8,
        size: Read_U32(p, OFF_FRAME_SIZE),
        buffer_size: Read_U32(p, OFF_FRAME_BUFFER_SIZE),
        ts_ms: Read_U64(p, OFF_FRAME_TS),
    }
}

// ===== §4 dlopen 与符号表 =====
//
// 为什么是 dlopen + dlsym（而不是 #[link(name = "AngstrongCameraSdk")]）：
// 厂商头文件里**没有任何 extern "C"**，50 个 AS_SDK_* 符号**全部**是 C++ mangled，
// 且目标机不保证装 SDK 开发包、也不做 aarch64 交叉链接。运行时按 mangled 字符串
// 取符号的代价是"符号名写错只在运行期暴露"，因此下面把 11 个必需符号名逐条列出。
//
// 关于 #[link(name = "dl")]：glibc < 2.34（Jetson Ubuntu 20.04 是 2.31）里
// dlopen/dlsym 在 libdl.so.2，需要显式 -ldl；glibc >= 2.34 起这些符号并入 libc，
// libdl 变成空壳但仍存在，链接不会失败。
#[cfg(unix)]
#[link(name = "dl")]
extern "C" {
    fn dlopen(filename: *const std::os::raw::c_char, flag: std::os::raw::c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const std::os::raw::c_char) -> *mut c_void;
    fn dlclose(handle: *mut c_void) -> std::os::raw::c_int;
    fn dlerror() -> *mut std::os::raw::c_char;
}

// ---- 11 个必需函数的 Rust 侧签名 ----
// 依据 as_camera_sdk_api.h 的声明与 `nm -D` 清单。
// 关于引用参数：aarch64 SysV 下"引用即指针"（mangling 里的 R 只影响名字，不影响调用约定），
// 因此 void*& / MEDIA_TYPE_E 一律声明成 *mut / i32。
#[cfg(unix)]
type Fn_Init = unsafe extern "C" fn() -> i32;
#[cfg(unix)]
type Fn_Deinit = unsafe extern "C" fn() -> i32;
#[cfg(unix)]
type Fn_GetSwVersion = unsafe extern "C" fn(psz_version: *mut std::os::raw::c_char, size: u32) -> i32;
#[cfg(unix)]
type Fn_GetCameraList = unsafe extern "C" fn(dev_list: *mut Cxx_List) -> i32;
#[cfg(unix)]
type Fn_OpenCamera =
    unsafe extern "C" fn(cam: *mut c_void, para_file_path: *const std::os::raw::c_char) -> i32;
#[cfg(unix)]
type Fn_SetStreamParam =
    unsafe extern "C" fn(cam: *mut c_void, media_type: i32, param: *const Stream_Param) -> i32;
#[cfg(unix)]
type Fn_RegisterStreamCallback =
    unsafe extern "C" fn(cam: *mut c_void, cb: *mut Stream_Callback) -> i32;
#[cfg(unix)]
type Fn_StartStream = unsafe extern "C" fn(cam: *mut c_void, type_mask: i32) -> i32;
#[cfg(unix)]
type Fn_StopStream = unsafe extern "C" fn(cam: *mut c_void, type_mask: i32) -> i32;
#[cfg(unix)]
type Fn_CloseCamera = unsafe extern "C" fn(cam: *mut c_void) -> i32;
#[cfg(unix)]
type Fn_DestoryCamHandle = unsafe extern "C" fn(cam: *mut c_void) -> i32;

/// 全部已解析的函数指针：**11 个**（⑤ FreeCameraList 与 ⑬ 三个可选符号刻意不解析）。
#[cfg(unix)]
pub struct Symbols {
    /// ① AS_SDK_Init
    init: Fn_Init,
    /// ② AS_SDK_Deinit
    deinit: Fn_Deinit,
    /// ③ AS_SDK_GetSwVersion
    get_sw_version: Fn_GetSwVersion,
    /// ④ AS_SDK_GetCameraList
    get_camera_list: Fn_GetCameraList,
    /// ⑥ AS_SDK_OpenCamera
    open_camera: Fn_OpenCamera,
    /// ⑦ AS_SDK_SetStreamParam
    set_stream_param: Fn_SetStreamParam,
    /// ⑧ AS_SDK_RegisterStreamCallback
    register_stream_callback: Fn_RegisterStreamCallback,
    /// ⑨ AS_SDK_StartStream
    start_stream: Fn_StartStream,
    /// ⑩ AS_SDK_StopStream
    stop_stream: Fn_StopStream,
    /// ⑪ AS_SDK_CloseCamera
    close_camera: Fn_CloseCamera,
    /// ⑫ AS_SDK_DestoryCamHandle（`Destory` 是厂商拼写，不是笔误）
    destory_cam_handle: Fn_DestoryCamHandle,
}

/// 把 8 字节的函数指针从 dlsym 的 `*mut c_void` 转成具体签名。
///
/// 用 transmute_copy 而不是 transmute：transmute 要求两端大小在**类型里**就能对得上，
/// 泛型 F 无法满足；transmute_copy 是按字节拷贝 + 运行时长度检查。
///
/// # Safety
/// 调用方必须保证 F 与该 mangled 符号的真实签名一致。
#[cfg(unix)]
unsafe fn Cast_Sym<F: Copy>(raw: *mut c_void) -> F {
    debug_assert_eq!(std::mem::size_of::<F>(), std::mem::size_of::<*mut c_void>());
    std::mem::transmute_copy::<*mut c_void, F>(&raw)
}

/// 把 `b"...\0"` 这种带结尾 NUL 的字节串转成可打印的符号名（去掉 NUL）。
#[cfg(unix)]
fn Symbol_Text(mangled: &'static [u8]) -> String {
    let body = if mangled.last() == Some(&0) {
        &mangled[..mangled.len() - 1]
    } else {
        mangled
    };
    String::from_utf8_lossy(body).into_owned()
}

/// 取一个必需符号；缺失则返回带 mangled 名的可读错误。
///
/// mangled 一律传"带 \0 的字节串字面量"（例：`b"_Z11AS_SDK_Initv\0"`）：
/// 编译期就落成 &'static [u8]，不分配、不 CString、不可能有内嵌 NUL。
///
/// # Safety
/// dl 必须是 dlopen 成功返回的句柄。
#[cfg(unix)]
unsafe fn Sym_Required<F: Copy>(dl: *mut c_void, mangled: &'static [u8]) -> Result<F, String> {
    let raw = dlsym(dl, mangled.as_ptr() as *const std::os::raw::c_char);
    if raw.is_null() {
        return Err(format!(
            "缺失符号 {}（厂商 SDK 版本不符？用 nm -D 核对 .so）",
            Symbol_Text(mangled)
        ));
    }
    Ok(Cast_Sym::<F>(raw))
}

/// 解析 11 个必需符号。
///
/// 名字逐条来自 `nm -D` 原始清单（参考实现 §4 的对照表），**必须字节级一致**：
/// 多一个字母少一个字母都取不到。刻意**不解析**的两个：
/// - ⑤ `AS_SDK_FreeCameraList`（`_Z21AS_SDK_FreeCameraListRNSt7__cxx114listIPvSaIS1_EEE`）：
///   实机调用会触发 `free(): double free detected in tcache 2` → SIGABRT（RESULTS.md §6），
///   所以**不解析、不调用**；本设备是短命进程，枚举出来的节点（几十字节）由 OS 回收；
/// - ⑬ 诊断三连 `AS_SDK_GetCameraModel`（`_Z21AS_SDK_GetCameraModelPvR14CAMERA_MODEL_E`）/
///   `AS_SDK_GetSerialNumber`（`_Z22AS_SDK_GetSerialNumberPvPci`）/
///   `AS_SDK_GetFwVersion`（`_Z19AS_SDK_GetFwVersionPvPci`）：本设备不做机型/序列号/固件诊断；
///   另外库里有两个 GetFwVersion 重载，`std::string&` 那个签名无法安全调用，索性都不解析。
///
/// # Safety
/// dl 必须是 dlopen 成功返回的句柄。
#[cfg(unix)]
unsafe fn Load_Symbols(dl: *mut c_void) -> Result<Symbols, String> {
    Ok(Symbols {
        // ① AS_SDK_Init()
        init: Sym_Required::<Fn_Init>(dl, b"_Z11AS_SDK_Initv\0")?,
        // ② AS_SDK_Deinit()
        deinit: Sym_Required::<Fn_Deinit>(dl, b"_Z13AS_SDK_Deinitv\0")?,
        // ③ AS_SDK_GetSwVersion(char*, unsigned)
        get_sw_version: Sym_Required::<Fn_GetSwVersion>(dl, b"_Z19AS_SDK_GetSwVersionPcj\0")?,
        // ④ AS_SDK_GetCameraList(std::list<void*>&)
        get_camera_list: Sym_Required::<Fn_GetCameraList>(
            dl,
            b"_Z20AS_SDK_GetCameraListRNSt7__cxx114listIPvSaIS1_EEE\0",
        )?,
        // ⑥ AS_SDK_OpenCamera(void*, char const*)
        open_camera: Sym_Required::<Fn_OpenCamera>(dl, b"_Z17AS_SDK_OpenCameraPvPKc\0")?,
        // ⑦ AS_SDK_SetStreamParam(void*, MEDIA_TYPE_E, STREAM_PARAM_S const*)
        set_stream_param: Sym_Required::<Fn_SetStreamParam>(
            dl,
            b"_Z21AS_SDK_SetStreamParamPv12MEDIA_TYPE_EPK14STREAM_PARAM_S\0",
        )?,
        // ⑧ AS_SDK_RegisterStreamCallback(void*, STREAM_CALLBACK_S*)
        register_stream_callback: Sym_Required::<Fn_RegisterStreamCallback>(
            dl,
            b"_Z29AS_SDK_RegisterStreamCallbackPvP17STREAM_CALLBACK_S\0",
        )?,
        // ⑨ AS_SDK_StartStream(void*, int)
        start_stream: Sym_Required::<Fn_StartStream>(dl, b"_Z18AS_SDK_StartStreamPvi\0")?,
        // ⑩ AS_SDK_StopStream(void*, int)
        stop_stream: Sym_Required::<Fn_StopStream>(dl, b"_Z17AS_SDK_StopStreamPvi\0")?,
        // ⑪ AS_SDK_CloseCamera(void*)
        close_camera: Sym_Required::<Fn_CloseCamera>(dl, b"_Z18AS_SDK_CloseCameraPv\0")?,
        // ⑫ AS_SDK_DestoryCamHandle(void*)  ← 厂商拼写：Destory
        destory_cam_handle: Sym_Required::<Fn_DestoryCamHandle>(
            dl,
            b"_Z23AS_SDK_DestoryCamHandlePv\0",
        )?,
    })
}

/// 取 dlerror() 的最后一句话（拷贝成 String 立刻脱离 C 字符串）。
///
/// # Safety
/// 可以在任意线程调用；返回值可能为空指针。
#[cfg(unix)]
unsafe fn Last_DlError() -> String {
    let e = dlerror();
    if e.is_null() {
        "（dlerror 未提供更多信息）".to_string()
    } else {
        std::ffi::CStr::from_ptr(e).to_string_lossy().into_owned()
    }
}

/// dlopen（`RTLD_NOW | RTLD_LOCAL`）。
///
/// # Safety
/// 只在调用线程调用；句柄的释放由 `Sdk` 负责。
#[cfg(unix)]
unsafe fn Dl_Open(path: &str) -> Result<*mut c_void, String> {
    // 先清一次 dlerror：否则后面失败时可能读到上一次遗留的错误串。
    let _ = dlerror();

    let c_path = match std::ffi::CString::new(path) {
        Ok(p) => p,
        Err(_) => {
            return Err(format!(
                "dlopen 失败：路径里含 NUL 字节，无法作为 C 字符串传入（sdk_lib={path}）"
            ));
        }
    };

    let handle = dlopen(
        c_path.as_ptr(),
        (RTLD_NOW as std::os::raw::c_int) | (RTLD_LOCAL as std::os::raw::c_int),
    );
    if handle.is_null() {
        return Err(format!(
            "dlopen 失败：{}（sdk_lib={path}）；检查 sdk_lib 或 LD_LIBRARY_PATH",
            Last_DlError()
        ));
    }
    Ok(handle)
}

// ===== §5 回调 =====
//
// 这一节刻意**不做 cfg 门控**：Mutex/Atomic 全平台都有，而"按偏移读帧"恰恰是最容易
// 写错的一段 —— 让 cargo check 也类型检查它，是最有价值的静态检查。
//
// 三条纪律的落地：
//   1. 整条回调体 catch_unwind（panic 穿进 C++ 栈是 UB）；
//   2. 零日志、只 try_lock、不编码、不等待（回调阻塞多久，SDK 采集线程就停多久）；
//   3. 当场 to_vec()（缓冲是借用的，回调一返回 SDK 立刻清零 size 并复用缓冲）。

/// 回调链的计数器快照（只读一次全拷出来，供 `Open` 后的诊断日志用）
#[derive(Clone, Copy, Debug, Default)]
pub struct Cb_Stats {
    /// 进入回调总次数
    pub cb_total: u64,
    /// 成功写入槽位的次数
    pub frames: u64,
    /// `try_lock` 失败而丢帧的次数
    pub dropped: u64,
    /// size 异常被拒的次数
    pub bad_size: u64,
    /// `mjpegImg.size == 0`（0x80 没生效的典型表现）的次数
    pub no_mjpeg: u64,
    /// `catch_unwind` 捕到的 panic 次数
    pub panics: u64,
}

/// 回调与主线程共享的状态。由 `Arc<Cb_Ctx>` 持有。
///
/// 交给 SDK 的 privateData 是 `Arc::as_ptr(&ctx)` —— **借用，不转移所有权**。
/// 不用 `Arc::into_raw`：SDK 永远不会还回来（没有 "unregister" 回调保证调用次数），
/// 必然泄漏；更糟的是若提前 drop 那个 Arc，回调就会踩到悬垂指针。
pub struct Cb_Ctx {
    slot: Arc<FrameSlot>,
    /// 帧内宽高为 0 时的兜底（来自 `[camera_ugv].width/height`）
    fallback_width: u32,
    fallback_height: u32,
    cb_total: AtomicU64,
    frames: AtomicU64,
    dropped: AtomicU64,
    bad_size: AtomicU64,
    no_mjpeg: AtomicU64,
    panics: AtomicU64,
}

impl Cb_Ctx {
    pub fn New(slot: Arc<FrameSlot>, fallback_width: u32, fallback_height: u32) -> Self {
        Self {
            slot,
            fallback_width,
            fallback_height,
            cb_total: AtomicU64::new(0),
            frames: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            bad_size: AtomicU64::new(0),
            no_mjpeg: AtomicU64::new(0),
            panics: AtomicU64::new(0),
        }
    }

    /// 读一份计数快照（全 Relaxed：这里不需要任何跨线程顺序保证）
    pub fn Stats(&self) -> Cb_Stats {
        Cb_Stats {
            cb_total: self.cb_total.load(Ordering::Relaxed),
            frames: self.frames.load(Ordering::Relaxed),
            dropped: self.dropped.load(Ordering::Relaxed),
            bad_size: self.bad_size.load(Ordering::Relaxed),
            no_mjpeg: self.no_mjpeg.load(Ordering::Relaxed),
            panics: self.panics.load(Ordering::Relaxed),
        }
    }
}

/// SDK 采集线程上的入口。
///
/// 除"两次空指针比较"外，本函数唯一语句就是 `catch_unwind(|| On_Frame_Body(..))`：
/// 所有可能 panic 的操作（to_vec 分配、索引、锁）都在那层以内。
///
/// panic 绝对不能穿进 C++ 栈：SDK 是 C++ 编的，栈上还有它的 RAII 对象；
/// Rust 的 unwind 穿过没有对应 personality routine 的帧是 UB。
///
/// 空指针比较放在 catch_unwind 外面是安全的：比较指针不可能 panic，而且必须先判
/// user 非空，才敢在 panic 分支里解引用它。
pub extern "C" fn On_Frame(cam: *mut c_void, data: *const u8, user: *mut c_void) {
    if data.is_null() || user.is_null() {
        return;
    }

    // AssertUnwindSafe：闭包只捕获两个裸指针，语义上当然是 unwind-safe 的；
    // 显式断言只是为了不依赖"裸指针是否 impl UnwindSafe"的细节，无运行时开销。
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        On_Frame_Body(data, user);
    }));

    if outcome.is_err() {
        // panic 已被吞掉。这里只做一次原子自增：不打日志（回调里不许打印）、
        // 不加锁、不分配，保证"兜底分支"本身也不可能再 panic。
        // SAFETY: user 已经判过非空，且 Cb_Ctx 的生命周期覆盖所有回调
        //         （见 Ugv_Camera 的字段声明顺序注释）。
        unsafe {
            (*(user as *const Cb_Ctx))
                .panics
                .fetch_add(1, Ordering::Relaxed);
        }
    }

    // 单设备：句柄本身不用管（多个相机才需要按 cam 区分）。
    let _ = cam;
}

/// 回调的实际逻辑。**只能被 [`On_Frame`] 在 `catch_unwind` 里调用。**
///
/// 顺序固定（每一步都有理由，别重排）：
/// 读帧 → 计数 → 闸门①（空/零长）→ 闸门②（超上限）→ 宽高兜底 → 当场拷贝 →
/// `try_lock` 写槽位 → 计数。
fn On_Frame_Body(data: *const u8, user: *mut c_void) {
    // SAFETY: privateData 是 Arc<Cb_Ctx> 的借用指针（Sdk::Open 里用 Arc::as_ptr 得到），
    //         在 Ugv_Camera 持有那个 Arc 期间一直有效；On_Frame 已判 user 非空。
    let ctx = unsafe { &*(user as *const Cb_Ctx) };

    // 1) 只读 mjpegImg 这一路：基址 = pstData + 288。
    // SAFETY: data 非空（On_Frame 已判），偏移是 §2 里经反汇编实测的常量。
    let mj = unsafe { Read_Frame_View(data, OFF_DATA_MJPEG) };

    ctx.cb_total.fetch_add(1, Ordering::Relaxed);

    // 2) 闸门①：这一帧没有 MJPEG 属**正常**情况（0x80 没生效，或该帧只填了别的路），
    //    不算错误，直接返回。
    if mj.data.is_null() || mj.size == 0 {
        ctx.no_mjpeg.fetch_add(1, Ordering::Relaxed);
        return;
    }
    // 闸门②：防脏数据。万一偏移读歪拿到垃圾 size，绝不拿它去 from_raw_parts。
    // ⚠ 刻意**不**用 bufferSize 做闸门（也不做 "buffer_size < size 就拒"）：
    //   这一路的 bufferSize 没有实测数据，万一恒为 0 会把正常帧全部拒掉，
    //   最终表现为"人为超时"——比脏数据更糟的失败模式。
    if mj.size as usize > MAX_JPEG_BYTES {
        ctx.bad_size.fetch_add(1, Ordering::Relaxed);
        return;
    }

    // 3) 宽高：帧内给了就用帧内的，全 0 才退回配置值。
    let width = if mj.width > 0 {
        mj.width
    } else {
        ctx.fallback_width
    };
    let height = if mj.height > 0 {
        mj.height
    } else {
        ctx.fallback_height
    };

    // 4) 当场复制：缓冲是**借用**的。回调一返回 SDK 立刻把 size 清零
    //    并复用那块常驻缓冲 → 此刻不拷，之后就永远拿不到了。
    // SAFETY: size 已过闸门（>0 且 <= 8 MiB），data 非空；这块内存在本回调返回前
    //         由 SDK 保证有效。
    let jpeg = unsafe { std::slice::from_raw_parts(mj.data, mj.size as usize) }.to_vec();

    // 5) 写槽位：**只** try_lock（抢不到就丢这一帧），绝不阻塞 SDK 采集线程。
    //    投毒在本 crate 里不可能发生（持锁期间没有能 panic 的语句：Vec 分配失败走
    //    abort，不 unwind），因此不需要在回调里处理 PoisonError。
    match ctx
        .slot
        .Try_Publish(Arc::from(jpeg), width, height, Monotonic_Ms())
    {
        Some(_) => {
            ctx.frames.fetch_add(1, Ordering::Relaxed);
        }
        None => {
            ctx.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

// ===== §6 Sdk =====
//
// 设计要点：所有 SDK 资源都由 Sdk 持有，Drop 兜底；显式 Shutdown() 与 Drop 共用
// 同一实现，靠 shutdown_done 幂等。这样 Open 里任何一条 `?` 提前返回都会走到清理。

/// HP60C SDK 的唯一资源所有者。
#[cfg(unix)]
pub struct Sdk {
    /// dlopen 句柄
    dl: *mut c_void,
    sym: Symbols,
    /// 相机句柄（GetCameraList → 首节点 payload）
    cam: *mut c_void,
    /// ★ 必须是 Box：GetCameraList 填充后，节点里的 prev/next 指向**这个对象
    /// 自己的地址**。按值放在会被移动的位置（或按值传参/返回）就成了野指针。
    dev_list: Box<Cxx_List>,
    /// 注册回调时那块 STREAM_CALLBACK_S 的存放处。
    ///
    /// 反汇编显示 SDK 会把 callback/privateData **拷贝**进相机对象（+152/+160），
    /// 并不长期持有这个结构体的指针；Box 只是为了"注册后地址不再随 Sdk 移动而变"。
    cb: Box<Stream_Callback>,
    /// AS_SDK_Init 成功
    inited: bool,
    /// AS_SDK_OpenCamera 成功
    opened: bool,
    /// Some(实际成功的 flag) ⇒ StopStream 只调一次（`take()` 结构性保证）
    started: Option<i32>,
    /// DestoryCamHandle 已调过
    destroyed: bool,
    /// Shutdown 已走完（幂等位）
    shutdown_done: bool,
    /// 本次实际使用的 flag（Shutdown 之后仍保留，供 `Info()` 答出）
    used_flag: i32,
    /// AS_SDK_GetSwVersion 的版本号（失败为 None，不阻断）
    sdk_version: Option<String>,
}

// SAFETY: Sdk 里的裸指针只在 `Ugv_Camera::sdk`（`Mutex<Option<Sdk>>`）之下被
// Open/Close 触碰，同一时刻至多一个线程；回调线程只读 `Cb_Ctx`，从不碰 Sdk 字段。
#[cfg(unix)]
unsafe impl Send for Sdk {}

#[cfg(unix)]
impl Sdk {
    /// dlopen → 解析符号 → 构造 Sdk → Init → 版本 → GetCameraList 取首句柄 →
    /// OpenCamera(加密 json) → SetStreamParam(DEPTH/RGB) → RegisterCallback →
    /// StartStream(0x83)。
    ///
    /// **任何一步失败都由 `Drop` 走完清理**（所以本函数里没有手写 goto cleanup）。
    ///
    /// # Safety
    /// 只在调用线程调用；`user` 必须是 `Arc<Cb_Ctx>` 的借用指针，且其生命周期必须
    /// 覆盖从本次成功返回到 `Shutdown` 完成之间的全部回调。
    pub unsafe fn Open(
        lib_path: &str,
        config_json: &str,
        width: u32,
        height: u32,
        fps: u32,
        user: *mut c_void,
    ) -> Result<Sdk, String> {
        // 步 0：布局自检（debug 构建；release 下整段被裁掉）
        Debug_Assert_Layout();

        // 步 1：dlopen
        let dl = Dl_Open(lib_path)?;

        // 步 2：11 个必需符号
        let sym = match Load_Symbols(dl) {
            Ok(sym) => sym,
            Err(message) => {
                // 符号缺失时 dl 句柄已经拿到了 —— 必须在这里关掉，否则泄漏。
                // 这是唯一一条"Sdk 还没构造出来但已经持有资源"的路径。
                let _ = dlclose(dl);
                return Err(message);
            }
        };

        // 步 3：手搓一个合法的空 std::list<void*>：
        //   哨兵节点自指（next = prev = &self），size = 0。
        // 为什么必须自指：SDK 往里 insert 时会写 node->prev->next / node->next->prev，
        // 若三个字全清零，第一次插入就会解引用空哨兵 → SIGSEGV。
        let mut dev_list = Box::new(Cxx_List {
            next: std::ptr::null_mut(),
            prev: std::ptr::null_mut(),
            size: 0,
        });
        let self_addr = (&mut *dev_list) as *mut Cxx_List as *mut c_void;
        dev_list.next = self_addr;
        dev_list.prev = self_addr;

        let mut sdk = Sdk {
            dl,
            sym,
            cam: std::ptr::null_mut(),
            dev_list,
            cb: Box::new(Stream_Callback {
                callback: None,
                private_data: std::ptr::null_mut(),
            }),
            inited: false,
            opened: false,
            started: None,
            destroyed: false,
            shutdown_done: false,
            used_flag: 0,
            sdk_version: None,
        };

        // 步 4~11
        sdk.Init()?;
        sdk.Fetch_SwVersion();
        sdk.Enumerate_First_Cam()?;
        sdk.Open_Camera(config_json)?;
        sdk.Set_Stream_Param(width, height, fps)?;
        sdk.Register_Callback(user)?;
        sdk.Start_Stream(STREAM_FLAG)?;

        // 步 12
        sdk.used_flag = STREAM_FLAG;
        Ok(sdk)
    }

    /// 本次实际使用的流掩码（`Info()` 用）
    pub fn Used_Flag(&self) -> i32 {
        self.used_flag
    }

    /// SDK 软件版本（`GetSwVersion` 失败则为 None）
    pub fn Version(&self) -> Option<String> {
        self.sdk_version.clone()
    }

    /// 步 4：AS_SDK_Init
    unsafe fn Init(&mut self) -> Result<(), String> {
        let code = (self.sym.init)();
        if code != 0 {
            return Err(format!("AS_SDK_Init 返回 {code}"));
        }
        self.inited = true;
        Ok(())
    }

    /// 步 5：AS_SDK_GetSwVersion —— 失败只 warn，不阻断（版本号只用于日志）
    unsafe fn Fetch_SwVersion(&mut self) {
        let mut buf = [0 as std::os::raw::c_char; 64];
        let code = (self.sym.get_sw_version)(buf.as_mut_ptr(), buf.len() as u32);
        if code != 0 {
            warn!("[camera_ugv] AS_SDK_GetSwVersion 返回 {code}（跳过版本号，不影响取帧）");
            self.sdk_version = None;
            return;
        }
        // SAFETY: SDK 成功时保证 buf 里是 NUL 结尾的字符串
        let text = std::ffi::CStr::from_ptr(buf.as_ptr())
            .to_string_lossy()
            .into_owned();
        self.sdk_version = Some(text);
    }

    /// 步 6+7：GetCameraList + 取第 0 个句柄
    unsafe fn Enumerate_First_Cam(&mut self) -> Result<(), String> {
        // ⚠ 这个函数的返回值是**设备数**，不是错误码，所以绝不能当错误码判 0。
        let code = (self.sym.get_camera_list)(&mut *self.dev_list);
        if code < 0 {
            return Err(format!("AS_SDK_GetCameraList 返回 {code}"));
        }
        if code == 0 {
            return Err(
                "未枚举到相机（HP60C 是否插好 / 是否被 ROS ascamera 节点占用）".to_string(),
            );
        }

        let handle = First_Handle(&mut *self.dev_list);
        if handle.is_null() {
            return Err(format!(
                "设备列表布局不符（SDK 报 size={}）",
                (*self.dev_list).size
            ));
        }
        self.cam = handle;
        Ok(())
    }

    /// 步 8：OpenCamera（吃加密 json 的绝对路径）
    unsafe fn Open_Camera(&mut self, config_json: &str) -> Result<(), String> {
        let c_path = match std::ffi::CString::new(config_json) {
            Ok(p) => p,
            Err(_) => {
                return Err(format!(
                    "配置路径里含 NUL 字节，无法作为 C 字符串传给 AS_SDK_OpenCamera：{config_json}"
                ));
            }
        };
        let code = (self.sym.open_camera)(self.cam, c_path.as_ptr());
        if code != 0 {
            return Err(format!(
                "AS_SDK_OpenCamera 返回 {code}（config_json={config_json}）"
            ));
        }
        self.opened = true;
        Ok(())
    }

    /// 步 9：SetStreamParam(DEPTH) + SetStreamParam(RGB)，都是 w×h@fps。
    ///
    /// 【顺序硬要求】这两步必须在 OpenCamera **之后**、RegisterStreamCallback
    /// **之前**（厂商 C++ 探针与 ascamera 节点的真实顺序都是这样）。
    unsafe fn Set_Stream_Param(&mut self, width: u32, height: u32, fps: u32) -> Result<(), String> {
        let param = Stream_Param {
            width: width as i32,
            height: height as i32,
            fps: fps as i32,
        };

        let code = (self.sym.set_stream_param)(self.cam, MEDIA_TYPE_DEPTH, &param);
        if code != 0 {
            return Err(format!(
                "AS_SDK_SetStreamParam(DEPTH) 返回 {code}（{}x{}@{}）",
                param.width, param.height, param.fps
            ));
        }

        let code = (self.sym.set_stream_param)(self.cam, MEDIA_TYPE_RGB, &param);
        if code != 0 {
            return Err(format!(
                "AS_SDK_SetStreamParam(RGB) 返回 {code}（{}x{}@{}）",
                param.width, param.height, param.fps
            ));
        }
        Ok(())
    }

    /// 步 10：RegisterStreamCallback（必须在 OpenCamera 之后）
    unsafe fn Register_Callback(&mut self, user: *mut c_void) -> Result<(), String> {
        self.cb.callback = Some(On_Frame);
        self.cb.private_data = user;

        let cb_ptr: *mut Stream_Callback = &mut *self.cb;
        let code = (self.sym.register_stream_callback)(self.cam, cb_ptr);
        if code != 0 {
            return Err(format!("AS_SDK_RegisterStreamCallback 返回 {code}"));
        }
        Ok(())
    }

    /// 步 11：StartStream(0x83) —— **不做 0x03 降级**（见 [`STREAM_FLAG`] 的注释）
    unsafe fn Start_Stream(&mut self, flag: i32) -> Result<(), String> {
        let code = (self.sym.start_stream)(self.cam, flag);
        if code != 0 {
            return Err(format!(
                "AS_SDK_StartStream(0x{flag:x}) 返回 {code}（该固件可能不接受 0x83；本设备不做降级）"
            ));
        }
        self.started = Some(flag);
        Ok(())
    }

    /// 清理。顺序固定且**幂等**：
    /// `StopStream(cam, 0)`（只一次）→ `CloseCamera` → `DestoryCamHandle` →
    /// 跳过 `FreeCameraList` → `Deinit` → `dlclose`。
    ///
    /// 每一步都先看状态位，保证"该步没做过就不做"，也就保证 StopStream 不会被调到
    /// 第二次（`RESULTS.md` §4：二次调用会告警，且相机可能已被销毁）。
    /// 清理期失败**只 warn**：不改变 `Open` 的成功与否，也不阻断后续步骤。
    ///
    /// # Safety
    /// 只在持有 `&mut self` 时调用；函数内部只做 C 函数调用与自身状态位读写。
    pub unsafe fn Shutdown(&mut self) {
        if self.shutdown_done {
            return;
        }
        // 先置位：即使下面某步 panic（不该发生），Drop 也不会重入
        self.shutdown_done = true;

        // ① StopStream —— `started.take()` 结构性地保证"只一次"。
        match self.started.take() {
            Some(_flag) => {
                if self.cam.is_null() {
                    warn!("[camera_ugv] 跳过 AS_SDK_StopStream（相机句柄为空）");
                } else {
                    let code = (self.sym.stop_stream)(self.cam, STOP_STREAM_MASK);
                    if code != 0 {
                        warn!("[camera_ugv] AS_SDK_StopStream 返回 {code}（清理阶段不阻断）");
                    }
                }
            }
            None => {
                warn!("[camera_ugv] 跳过 AS_SDK_StopStream（本次未成功开流）");
            }
        }

        // ② CloseCamera —— 仅当确实打开过
        if self.opened {
            let code = (self.sym.close_camera)(self.cam);
            if code != 0 {
                warn!("[camera_ugv] AS_SDK_CloseCamera 返回 {code}（清理阶段不阻断）");
            }
            self.opened = false;
        }

        // ③ 销毁句柄
        if !self.cam.is_null() && !self.destroyed {
            let code = (self.sym.destory_cam_handle)(self.cam);
            if code != 0 {
                warn!("[camera_ugv] AS_SDK_DestoryCamHandle 返回 {code}（清理阶段不阻断）");
            }
            self.destroyed = true;
        }

        // ④ **跳过 `AS_SDK_FreeCameraList`**（2026-10-31 实机观测，RESULTS.md §6）：
        //    在 DestoryCamHandle 之后调用它会触发
        //    `free(): double free detected in tcache 2` → SIGABRT。
        //    本实现用的是 24 B 手搓镜像（哨兵自指）喂给 SDK，释放路径上仍有所有权歧义；
        //    影响面只是泄漏一次枚举出来的节点（几十字节），而不是把"已经取到帧"变成 abort。
        //    该符号也因此**不解析**（见 Load_Symbols 的注释）。

        // ⑤ Deinit
        if self.inited {
            let code = (self.sym.deinit)();
            if code != 0 {
                warn!("[camera_ugv] AS_SDK_Deinit 返回 {code}（清理阶段不阻断）");
            }
            self.inited = false;
        }

        // ⑥ dlclose 放最后：前面的 SDK 调用都还需要这个库的代码在内存里
        if !self.dl.is_null() {
            let code = dlclose(self.dl);
            if code != 0 {
                warn!("[camera_ugv] dlclose 返回 {code}（清理阶段不阻断）");
            }
            self.dl = std::ptr::null_mut();
        }
    }
}

#[cfg(unix)]
impl Drop for Sdk {
    fn drop(&mut self) {
        // SAFETY: Shutdown 只做 C 函数调用与自身状态位读写，幂等。
        unsafe {
            self.Shutdown();
        }
    }
}

/// 遍历 GetCameraList 填好的 `std::list<void*>`，取第一个节点里的相机句柄。
///
/// libstdc++ 的布局（bits/stl_list.h）：
///   `_List_node_base { _M_next; _M_prev; }`  ← 2 个机器字（16 B）
///   `_List_node<void*> : _List_node_base { void* _M_value; }`  ← payload 在 +16
///   `std::list` 的对象头部就是哨兵 `_List_node_header`：`{_M_next, _M_prev, _M_size}`
///
/// 【关于 next/prev 谁在前】libstdc++ 里是 `{_M_next, _M_prev}`。但字段顺序一旦记反，
/// 单元素链表照样能取对（此时 next 与 prev 同指唯一的节点），所以首节点取到"空指针或
/// 指回哨兵"时会改用另一个字重试 —— 连这种万一也兜住。
///
/// # Safety
/// `list` 必须是 GetCameraList 返回 > 0 之后、且从未被移动过的那个对象。
#[cfg(unix)]
unsafe fn First_Handle(list: *mut Cxx_List) -> *mut c_void {
    let header = list as *mut c_void;

    let mut first = (*list).next;
    if first.is_null() || first == header {
        first = (*list).prev;
    }
    if first.is_null() || first == header {
        return std::ptr::null_mut();
    }

    // 顺着 _M_next 数一遍节点（只做一致性校验，不参与取句柄）。
    // 上限 + "回到哨兵即停" 两道保险，防止链表被读歪后死循环。
    let mut node = first;
    let mut count = 0usize;
    while !node.is_null() && node != header && count < LIST_MAX_NODES {
        count += 1;
        node = std::ptr::read_unaligned((node as *const u8).add(OFF_LIST_NEXT) as *const *mut c_void);
    }

    // SAFETY: first 指向 _List_node<void*>，payload 在 +16。
    std::ptr::read_unaligned((first as *const u8).add(OFF_LIST_NODE_PAYLOAD) as *const *mut c_void)
}

// 非 unix：只留 stub，保证本机也能 `cargo check`（BRIEF 硬约束 3）。
#[cfg(not(unix))]
pub struct Sdk;

#[cfg(not(unix))]
impl Sdk {
    /// 非 unix 永远失败（本设备的存在意义就是 dlopen 那个 aarch64 .so）
    pub unsafe fn Open(
        _lib_path: &str,
        _config_json: &str,
        _width: u32,
        _height: u32,
        _fps: u32,
        _user: *mut c_void,
    ) -> Result<Sdk, String> {
        Err("仅支持 Linux：本设备通过 dlopen 加载 libAngstrongCameraSdk.so".to_string())
    }

    pub fn Used_Flag(&self) -> i32 {
        0
    }

    pub fn Version(&self) -> Option<String> {
        None
    }

    pub unsafe fn Shutdown(&mut self) {}
}

// ===== §7 单元测试 =====

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::camera_ugv::slot::FrameSlot;

    /// 手搓一块 336 B 的 `AS_SDK_Data_s` 镜像：只在 MJPEG 那一路（+288）按 §2 的偏移写字段。
    ///
    /// 返回的 `Vec` 内部地址在移动后不变（堆分配），所以写进 blob 的 `data` 指针始终有效。
    fn Make_Blob(data: *const u8, size: u32, width: u32, height: u32, ts_ms: u64) -> Vec<u8> {
        let mut blob = vec![0u8; OFF_DATA_POINTCLOUD2_VEC];
        let base = blob.as_mut_ptr();
        unsafe {
            let frame = base.add(OFF_DATA_MJPEG);
            std::ptr::write_unaligned(frame.add(OFF_FRAME_TYPE) as *mut i32, 6);
            std::ptr::write_unaligned(frame.add(OFF_FRAME_WIDTH) as *mut u32, width);
            std::ptr::write_unaligned(frame.add(OFF_FRAME_HEIGHT) as *mut u32, height);
            std::ptr::write_unaligned(frame.add(OFF_FRAME_DATA) as *mut *const u8, data);
            std::ptr::write_unaligned(frame.add(OFF_FRAME_SIZE) as *mut u32, size);
            std::ptr::write_unaligned(frame.add(OFF_FRAME_BUFFER_SIZE) as *mut u32, size);
            std::ptr::write_unaligned(frame.add(OFF_FRAME_ID) as *mut u32, 1);
            std::ptr::write_unaligned(frame.add(OFF_FRAME_TS) as *mut u64, ts_ms);
        }
        blob
    }

    /// 造一个 `Arc<Cb_Ctx>`，返回 (Arc, 交给 SDK 的借用指针)
    fn Ctx(slot: Arc<FrameSlot>, w: u32, h: u32) -> (Arc<Cb_Ctx>, *mut c_void) {
        let ctx = Arc::new(Cb_Ctx::New(slot, w, h));
        let ptr = Arc::as_ptr(&ctx) as *mut c_void;
        (ctx, ptr)
    }

    /// 一小段「像 JPEG」的字节
    fn Jpeg(marker: u8, len: usize) -> Vec<u8> {
        let mut bytes = vec![0xFF_u8, 0xD8, marker];
        bytes.resize(len, marker);
        bytes.push(0xFF);
        bytes.push(0xD9);
        bytes
    }

    #[test]
    fn layout_assertions_hold() {
        // debug 构建下每条 debug_assert 都要成立；release 下本调用是空操作
        Debug_Assert_Layout();
    }

    #[test]
    fn read_frame_view_uses_documented_offsets() {
        let jpeg = Jpeg(0x11, 32);
        let blob = Make_Blob(jpeg.as_ptr(), jpeg.len() as u32, 640, 480, 1_724_000_123_456);
        let view = unsafe { Read_Frame_View(blob.as_ptr(), OFF_DATA_MJPEG) };

        assert_eq!(view.frame_type, 6, "type 必须从 +288+0 读出");
        assert_eq!((view.width, view.height), (640, 480), "宽高必须从 +288+4/+8 读出");
        assert_eq!(view.data, jpeg.as_ptr(), "data 指针必须从 +288+16 读出");
        assert_eq!(view.size, jpeg.len() as u32, "size 必须从 +288+24 读出");
        assert_eq!(view.buffer_size, view.size, "bufferSize 必须从 +288+28 读出");
        assert_eq!(view.ts_ms, 1_724_000_123_456, "ts 必须从 +288+40 读出");
        // 读出来的字节就是写的字节（偏移没错位）
        let got = unsafe { std::slice::from_raw_parts(view.data, view.size as usize) };
        assert_eq!(got, jpeg.as_slice());
    }

    #[test]
    fn callback_rejects_null_and_empty() {
        let slot = Arc::new(FrameSlot::New());
        let (ctx, user) = Ctx(slot.clone(), 640, 480);

        // ① data / user 任一为空：直接返回，不 panic、不计数
        On_Frame(std::ptr::null_mut(), std::ptr::null(), std::ptr::null_mut());
        On_Frame(std::ptr::null_mut(), std::ptr::null(), user);
        On_Frame(std::ptr::null_mut(), std::ptr::null_mut() as *const u8, std::ptr::null_mut());
        assert_eq!(ctx.Stats().cb_total, 0, "空指针路径不得进入回调体");

        // ② size == 0：记 no_mjpeg，槽位仍空
        let jpeg = Jpeg(0x21, 8);
        let blob = Make_Blob(jpeg.as_ptr(), 0, 640, 480, 0);
        On_Frame(std::ptr::null_mut(), blob.as_ptr(), user);
        let stats = ctx.Stats();
        assert_eq!(stats.cb_total, 1);
        assert_eq!(stats.no_mjpeg, 1, "size=0 记 no_mjpeg");
        assert_eq!(stats.frames, 0);
        assert!(slot.Latest().is_none(), "零长帧不得写进槽位");

        // ③ data == nullptr 但 size > 0：同样按 no_mjpeg 处理
        let blob = Make_Blob(std::ptr::null(), 16, 640, 480, 0);
        On_Frame(std::ptr::null_mut(), blob.as_ptr(), user);
        assert_eq!(ctx.Stats().no_mjpeg, 2);
        assert!(slot.Latest().is_none());
    }

    #[test]
    fn callback_rejects_oversize() {
        let slot = Arc::new(FrameSlot::New());
        let (ctx, user) = Ctx(slot.clone(), 640, 480);
        // 只写 size 字段、不真的分配 8 MiB：闸门②在 from_raw_parts 之前就返回。
        // data 必须非空（否则先被闸门①按 no_mjpeg 拦下，测不到 bad_size）。
        let jpeg = Jpeg(0x51, 8);
        let blob = Make_Blob(jpeg.as_ptr(), (MAX_JPEG_BYTES + 1) as u32, 640, 480, 0);
        On_Frame(std::ptr::null_mut(), blob.as_ptr(), user);

        let stats = ctx.Stats();
        assert_eq!(stats.cb_total, 1);
        assert_eq!(stats.bad_size, 1, "超过 8 MiB 记 bad_size");
        assert_eq!(stats.frames, 0);
        assert!(slot.Latest().is_none(), "超限帧不得写进槽位");
    }

    #[test]
    fn callback_publishes_jpeg_into_slot() {
        let slot = Arc::new(FrameSlot::New());
        let (ctx, user) = Ctx(slot.clone(), 111, 222);
        let jpeg = Jpeg(0x31, 64);
        let blob = Make_Blob(jpeg.as_ptr(), jpeg.len() as u32, 640, 480, 7);
        On_Frame(std::ptr::null_mut(), blob.as_ptr(), user);

        let frame = slot.Latest().expect("合法帧必须写进槽位");
        assert_eq!(frame.jpeg.as_ref(), jpeg.as_slice(), "槽位里必须是同一份字节");
        assert_eq!((frame.width, frame.height), (640, 480), "宽高取帧内值");
        assert_eq!(frame.seq, 1, "首帧帧号是 1");
        assert_eq!(ctx.Stats().frames, 1);
        assert_eq!(ctx.Stats().dropped, 0);

        // 帧内宽高为 0 → 退回 Cb_Ctx 的 fallback（来自 [camera_ugv].width/height）
        let blob = Make_Blob(jpeg.as_ptr(), jpeg.len() as u32, 0, 0, 8);
        On_Frame(std::ptr::null_mut(), blob.as_ptr(), user);
        let frame = slot.Latest().expect("第二帧必须写进槽位");
        assert_eq!((frame.width, frame.height), (111, 222), "宽高 0 时退回配置值");
        assert_eq!(frame.seq, 2, "帧号逐次 +1");
        assert_eq!(ctx.Stats().frames, 2);
    }

    #[test]
    fn callback_drops_frame_when_slot_locked() {
        let slot = Arc::new(FrameSlot::New());
        let (ctx, user) = Ctx(slot.clone(), 640, 480);
        let jpeg = Jpeg(0x41, 16);
        let blob = Make_Blob(jpeg.as_ptr(), jpeg.len() as u32, 640, 480, 0);

        // 读者正持锁：回调必须**当场丢帧**并返回，而不是等锁（等锁会停住 SDK 采集线程）
        slot.Hold_Latest_Lock_For_Test(|| {
            On_Frame(std::ptr::null_mut(), blob.as_ptr(), user);
        });

        let stats = ctx.Stats();
        assert_eq!(stats.dropped, 1, "抢不到锁记 dropped");
        assert_eq!(stats.frames, 0, "持锁期间一帧都写不进去");
        assert_eq!(stats.panics, 0, "丢帧不是 panic");
    }
}
