//Presented by KeJi
//Created Date ： 2026-10-31
//Modified Date ： 2026-10-31

#![allow(non_snake_case)]
#![allow(non_camel_case_types)]

//! 最新帧槽位（task_43 Step 1）：**只留最新一帧**，覆盖式写入；读接口同步、非阻塞。
//!
//! **同形对象**：`pleiades-uav/src/device/camera_uav/slot.rs`（第 1 份；`camera_pc` 是第 2 份、
//! `camera_sim` 是第 3 份）。[`Frame`] 的五个字段、[`FrameSlot`]、[`Monotonic_Ms`]、[`Lock`]
//! 与第 1 份**逐字一致**。单独一份是口径 #1 的要求（camera 不进 base，即使逐字节相同也不共享）。
//!
//! **差异点（唯一，且是刻意的）**：写入口是 [`FrameSlot::Try_Publish`]（`try_lock`，抢不到锁
//! 就丢这一帧并返回 `None`），**不是**第 1 份的 `Publish`（阻塞 `lock`）。原因：HP60C 的写者
//! 是**厂商 SDK 采集线程**，无队列、无背压，回调阻塞多久采集就停多久（`rust_uav_camera_test/ugv_camera/RESULTS.md` §4）。
//!
//! **同步责任方**：task_43 本任务。四份中任一份改动都要评估另外三份。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

/// 一帧（槽位里唯一的存放形态）
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    /// JPEG 字节：`Arc<[u8]>` 让每次读只是引用计数 +1，不拷几百 KB
    pub jpeg: Arc<[u8]>,
    /// 帧宽（JPEG 编码后的真实宽）
    pub width: u32,
    /// 帧高
    pub height: u32,
    /// 递增帧号：槽位每写入一次 +1（`Clear` 不清零；**丢帧不占号**）
    pub seq: u64,
    /// 抓帧时刻：**本机单调毫秒**（[`Monotonic_Ms`]，与位姿 `Pose.time_boot_ms` **同源**、
    /// 进程启动为 0；`Instant` 单调，系统时钟回拨不影响）。
    ///
    /// ⚠️ 来源与仿真侧**不同**：`camera_sim` 的这个字段是**世界时间毫秒**
    /// （`CameraFrame::world_time × 1000`，世界侧时钟）。两者不可跨端直接比较；
    /// 跨端只比帧龄差（各自端内 `now - captured_at_ms`）。
    pub captured_at_ms: u64,
}

/// 覆盖式帧槽位
#[derive(Default)]
pub struct FrameSlot {
    latest: Mutex<Option<Frame>>,
    /// 下一个要分配的帧号（1 起）
    next_seq: AtomicU64,
}

impl FrameSlot {
    pub fn New() -> Self {
        Self::default()
    }

    /// 写一帧（覆盖旧帧）；返回刚写进去的那一份（`Arc` 克隆，近乎零成本）。
    ///
    /// **抢不到锁就丢帧**：返回 `None` 且**不消耗帧号**（帧号只在真正写入时 +1，
    /// 因此读者看到的 `seq` 仍严格递增且连续 +1）。
    ///
    /// 顺序固定：`try_lock` 成功 → 再 `next_seq.fetch_add` → 写 `*guard` → 返回克隆。
    /// 这个顺序是"丢帧不留空洞"的结构性保证，改顺序就会跳号。
    pub fn Try_Publish(
        &self,
        jpeg: Arc<[u8]>,
        width: u32,
        height: u32,
        captured_at_ms: u64,
    ) -> Option<Frame> {
        // `try_lock` 而不是 `lock`：写者是厂商 SDK 采集线程，阻塞一刻采集就停一刻。
        // 投毒（Poisoned）在本 crate 里不可能发生（持锁期间没有任何语句能 panic：
        // `Arc` 克隆与赋值都不会 unwind），因此两种失败一律按"丢帧"处理，不引入
        // `TryLockError` 枚举（它要求 rustc >= 1.79，设备上未必有）。
        let mut guard = match self.latest.try_lock() {
            Ok(guard) => guard,
            Err(_) => return None,
        };
        let seq = self.next_seq.fetch_add(1, Ordering::Relaxed) + 1;
        let frame = Frame {
            jpeg,
            width,
            height,
            seq,
            captured_at_ms,
        };
        *guard = Some(frame.clone());
        Some(frame)
    }

    /// 读最新帧（**同步、非阻塞**；无帧 → `None`）
    pub fn Latest(&self) -> Option<Frame> {
        Lock(&self.latest).clone()
    }

    /// 清空槽位（关闭相机时调用：关掉之后不该再答出旧帧）；帧号不清零
    pub fn Clear(&self) {
        *Lock(&self.latest) = None;
    }

    /// 持住槽位锁再执行闭包（**只给单测用**：模拟"读者正持锁"，验证回调丢帧而不阻塞）。
    ///
    /// `#[cfg(test)]` 不进产物。
    #[cfg(test)]
    pub fn Hold_Latest_Lock_For_Test<R>(&self, body: impl FnOnce() -> R) -> R {
        let guard = Lock(&self.latest);
        let out = body();
        drop(guard);
        out
    }
}

/// 取锁，毒化（某线程 panic 过）时恢复内部数据而不是二次 panic
fn Lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// 进程内单调毫秒：**与位姿同源**。
///
/// 位姿广播的时间戳 `Pose.time_boot_ms` 来自 `pleiades_base` 协议层的
/// [`now_boot_ms`]（`pleiades-base/src/Robot/core/robot.rs:254`），两者共用同一个
/// `OnceLock<Instant>` 基准 ⇒ `captured_at_ms` 与位姿时间戳可直接相减，不需要再对表。
///
/// 值域与协议一致（`u32` 毫秒，49.7 天后回绕，与 `time_boot_ms` 同样受限）；
/// 要更长就把 base 的 `now_boot_ms` 改 `u64`，不在本 step 范围。
pub fn Monotonic_Ms() -> u64 {
    u64::from(pleiades_base::robot::core::protocol::now_boot_ms())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一小段「像 JPEG」的字节（只用于槽位测试，不参与解析）
    fn Bytes(marker: u8, len: usize) -> Arc<[u8]> {
        let mut bytes = vec![0xFF_u8, 0xD8, marker];
        bytes.resize(len, marker);
        Arc::from(bytes)
    }

    #[test]
    fn slot_empty_is_none() {
        let slot = FrameSlot::New();
        assert!(slot.Latest().is_none(), "刚建的槽位必须没有帧");
        slot.Clear();
        assert!(slot.Latest().is_none(), "清空空槽位仍是 None（不 panic）");
    }

    #[test]
    fn slot_latest_wins_overwrites() {
        let slot = FrameSlot::New();
        let (first, second) = (Bytes(0x01, 8), Bytes(0x02, 16));
        let previous = slot
            .Try_Publish(first.clone(), 320, 240, 5)
            .expect("无竞争时必须写入成功");
        let published = slot
            .Try_Publish(second.clone(), 640, 480, 9)
            .expect("无竞争时必须写入成功");

        let latest = slot.Latest().expect("写过就必须有帧");
        assert_eq!(latest.jpeg, second, "槽位只留最新：旧帧必须被覆盖");
        assert_eq!((latest.width, latest.height), (640, 480), "宽高来自最新帧");
        assert_eq!(latest.seq, 2, "每写一次帧号 +1");
        assert_eq!(latest, published, "Try_Publish 的返回值就是槽位里的那一份");
        assert_ne!(latest.jpeg, first);
        assert_eq!(previous.seq, 1, "先写的那一份帧号是 1");
    }

    #[test]
    fn slot_seq_and_timestamp_monotonic() {
        let slot = FrameSlot::New();
        let mut previous: Option<Frame> = None;
        for step in 0..5u64 {
            slot.Try_Publish(Bytes(0x10, 4), 64, 48, Monotonic_Ms())
                .expect("无竞争时必须写入成功");
            let current = slot.Latest().expect("刚写过就必须有帧");
            if let Some(previous) = previous {
                assert!(
                    current.seq > previous.seq,
                    "帧号必须严格递增：{} → {}",
                    previous.seq,
                    current.seq
                );
                assert!(
                    current.captured_at_ms >= previous.captured_at_ms,
                    "时间戳必须单调不减：{} → {}",
                    previous.captured_at_ms,
                    current.captured_at_ms
                );
            }
            assert_eq!(current.seq, step + 1, "帧号从 1 起逐次 +1");
            previous = Some(current);
        }
    }

    #[test]
    fn slot_clear_keeps_seq() {
        let slot = FrameSlot::New();
        slot.Try_Publish(Bytes(0x20, 4), 10, 10, 1)
            .expect("无竞争时必须写入成功");
        slot.Try_Publish(Bytes(0x21, 4), 10, 10, 2)
            .expect("无竞争时必须写入成功");
        slot.Clear();
        assert!(slot.Latest().is_none(), "清空后必须没有帧");

        // 帧号不清零：关闭再打开不会让 seq 回绕，读者能看出「中间断过」
        let after = slot
            .Try_Publish(Bytes(0x22, 4), 10, 10, 3)
            .expect("无竞争时必须写入成功");
        assert_eq!(after.seq, 3, "Clear 不清零帧号");
    }

    #[test]
    fn slot_try_publish_drops_frame_when_locked() {
        let slot = FrameSlot::New();
        let first = slot
            .Try_Publish(Bytes(0x40, 4), 10, 10, 1)
            .expect("无竞争时必须写入成功");
        assert_eq!(first.seq, 1);

        // 读者正持锁：这一帧必须被丢掉，而不是把调用方（SDK 采集线程）堵在这里
        let dropped = slot.Hold_Latest_Lock_For_Test(|| slot.Try_Publish(Bytes(0x41, 4), 20, 20, 2));
        assert!(dropped.is_none(), "持锁期间必须丢帧（try_lock 不阻塞）");
        assert_eq!(
            slot.Latest().expect("原有帧必须还在").seq,
            1,
            "丢帧不得消耗帧号"
        );

        // 锁放开后照常可写，且帧号连续（丢过的那一帧不占号）
        let after = slot
            .Try_Publish(Bytes(0x42, 4), 30, 30, 3)
            .expect("锁释放后必须能写");
        assert_eq!(after.seq, 2, "丢帧不跳号：写入序号仍连续 +1");
    }

    #[test]
    fn monotonic_ms_is_non_decreasing() {
        let first = Monotonic_Ms();
        let second = Monotonic_Ms();
        assert!(second >= first, "单调毫秒不得回退：{first} → {second}");
        // 只做量级校验（不 sleep，避免拖慢测试）：基准是一次 `Instant`，必须远小于半小时
        assert!(second < 1_800_000, "基准应≈进程启动，不该是 epoch 毫秒：{second}");
    }

    #[test]
    fn monotonic_ms_shares_pose_epoch() {
        // 与位姿同源：位姿 time_boot_ms 也来自 now_boot_ms()，同一个 OnceLock<Instant> 基准
        let camera_ms = Monotonic_Ms();
        let pose_ms = u64::from(pleiades_base::robot::core::protocol::now_boot_ms());
        assert!(
            camera_ms.abs_diff(pose_ms) < 1_000,
            "captured_at_ms 必须与位姿时间戳同基准：{camera_ms} vs {pose_ms}"
        );
    }

    #[test]
    fn slot_publishes_from_another_thread() {
        let slot = Arc::new(FrameSlot::New());
        let writer = {
            let slot = slot.clone();
            std::thread::spawn(move || {
                for step in 0..50u32 {
                    slot.Try_Publish(Bytes(0x30, 8), step, step, u64::from(step))
                        .expect("单写者、无读者持锁时必须写入成功");
                }
            })
        };
        let _ = writer.join();

        let latest = slot.Latest().expect("跨线程写完之后必须有帧");
        assert_eq!(latest.seq, 50, "跨线程写入同样逐次 +1");
        assert_eq!((latest.width, latest.height), (49, 49), "读到的是最后一帧");
        assert_eq!(latest.captured_at_ms, 49);
    }
}
