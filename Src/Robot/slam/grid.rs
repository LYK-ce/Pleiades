//Presented by KeJi
//Created Date ： 2026-07-21
//Modified Date ： 2026-08-15

//! 占据栅格地图 — 概率 log-odds（三态）
//!
//! 对标 Cartographer / GMapping 标准做法。
//! 单 Chunk (256×256, 0.5m/cell)，初始全 0（Unknown）。
//! Occupied: +3/次, 夹断 +8,  >+6 视为 Occupied
//! Free:     -1/次, 夹断 -8,  <-6 视为 Free
//! 中间 [-6, +6] → Unknown
//! 不对称增量（3:1）对齐 OctoMap 风格：一次命中可抵消三次掠过


/// Chunk 大小 (cells)
pub const CHUNK_SIZE: usize = 256;

/// 分辨率 (米/格)
pub const CELL_RESOLUTION: f32 = 0.5;

/// log-odds 参数
const OCCUPIED_INCREMENT: i8 = 3;
const FREE_DECREMENT: i8 = 1;
pub const OCCUPIED_CLAMP: i8 = 8;
pub const FREE_CLAMP: i8 = -8;
const OCCUPIED_THRESHOLD: i8 = 6;
const FREE_THRESHOLD: i8 = -6;

/// 动态障碍掩蔽衰减量（Task 15 C 节）：= OCCUPIED_INCREMENT，对「他车格」做 -3 抵消 LiDAR 命中
pub const DYNAMIC_OBSTACLE_DECAY: i8 = OCCUPIED_INCREMENT;

/// 格子宏观状态（供外部使用）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i8)]
pub enum CellState {
    /// 可通行
    Free = 0,
    /// 占据（MAVLink 惯例 100）
    Occupied = 100,
    /// 未知（i8 -1，u8 线上 = 255；2026-08-07 协议统一 0/100/255）
    Unknown = -1,
}

/// 变化的格子（Task 13_2：state 三态 → delta 数值差分）
#[derive(Debug, Clone)]
pub struct Delta {
    pub gx: i32,
    pub gy: i32,
    pub delta: i8,
}

/// 一个 Chunk: 256×256 cells（i8 log-odds 概率分）
#[derive(Clone)]
pub struct Chunk {
    cells: Box<[i8; CHUNK_SIZE * CHUNK_SIZE]>,
    pub origin_gx: i32,
    pub origin_gy: i32,
}

impl Chunk {
    pub fn new(origin_gx: i32, origin_gy: i32) -> Self {
        Self {
            cells: Box::new([0i8; CHUNK_SIZE * CHUNK_SIZE]),
            origin_gx,
            origin_gy,
        }
    }

    /// 读取概率分
    pub fn get(&self, gx: i32, gy: i32) -> Option<i8> {
        let lx = gx - self.origin_gx;
        let ly = gy - self.origin_gy;
        if lx < 0 || lx >= CHUNK_SIZE as i32 || ly < 0 || ly >= CHUNK_SIZE as i32 {
            return None;
        }
        Some(self.cells[ly as usize * CHUNK_SIZE + lx as usize])
    }

    /// 概率更新：occupied=true → +3 (夹断+8), false → -1 (夹断-8)
    /// 返回 (宏观状态是否变化, 新宏观状态, Δ = new_log − old_log)
    /// （Task 13_2：Δ 用于差分广播；clamp 边界处 Δ 可能非 ±3，如 6→8 时 Δ=+2）
    pub fn update(&mut self, gx: i32, gy: i32, occupied: bool) -> Option<(bool, u8, i8)> {
        let lx = gx - self.origin_gx;
        let ly = gy - self.origin_gy;
        if lx < 0 || lx >= CHUNK_SIZE as i32 || ly < 0 || ly >= CHUNK_SIZE as i32 {
            return None;
        }
        let idx = ly as usize * CHUNK_SIZE + lx as usize;
        let old_log = self.cells[idx];
        let old_state = log_to_state(old_log);

        self.cells[idx] = if occupied {
            old_log.saturating_add(OCCUPIED_INCREMENT).min(OCCUPIED_CLAMP)
        } else {
            old_log.saturating_sub(FREE_DECREMENT).max(FREE_CLAMP)
        };

        let new_state = log_to_state(self.cells[idx]);
        let delta = self.cells[idx] - old_log;
        Some((old_state != new_state, new_state, delta))
    }

    /// 单格应用任意 Δ（入站增量 / 对账应用；Task 13_2）
    ///
    /// `merged += delta`，clamp ±8（与 update 相同的边界）；越界返回 false 不修改。
    pub fn apply_delta(&mut self, gx: i32, gy: i32, delta: i8) -> bool {
        let lx = gx - self.origin_gx;
        let ly = gy - self.origin_gy;
        if lx < 0 || lx >= CHUNK_SIZE as i32 || ly < 0 || ly >= CHUNK_SIZE as i32 {
            return false;
        }
        let idx = ly as usize * CHUNK_SIZE + lx as usize;
        let old = self.cells[idx];
        self.cells[idx] = old.saturating_add(delta).clamp(FREE_CLAMP, OCCUPIED_CLAMP);
        true
    }

    /// 概率衰减：log-odds 减 `amount`（夹断 FREE_CLAMP），返回 (宏观状态是否变化, 新宏观状态, Δ)
    ///
    /// Task 15 C 节：动态障碍掩蔽——对「他车格」做 -DYNAMIC_OBSTACLE_DECAY 抵消 LiDAR 命中，
    /// 中和 +3 后他车格稳定在 Unknown（log ≤ 5），永不 Occupied；也能清历史残留（+8 → 5）。
    pub fn decay(&mut self, gx: i32, gy: i32, amount: i8) -> Option<(bool, u8, i8)> {
        let lx = gx - self.origin_gx;
        let ly = gy - self.origin_gy;
        if lx < 0 || lx >= CHUNK_SIZE as i32 || ly < 0 || ly >= CHUNK_SIZE as i32 {
            return None;
        }
        let idx = ly as usize * CHUNK_SIZE + lx as usize;
        let old_log = self.cells[idx];
        let old_state = log_to_state(old_log);
        self.cells[idx] = old_log.saturating_sub(amount).max(FREE_CLAMP);
        let new_state = log_to_state(self.cells[idx]);
        let delta = self.cells[idx] - old_log;
        Some((old_state != new_state, new_state, delta))
    }

    /// 整表原始 log-odds（i8）导出（own 上传 / 对账下发数据源）
    pub fn log_odds_bytes(&self) -> Box<[i8; CHUNK_SIZE * CHUNK_SIZE]> {
        self.cells.clone()
    }

    /// 整表 log-odds（i8）导入（对账下发替换用）；长度非法返回 false
    pub fn set_log_odds(&mut self, data: &[i8]) -> bool {
        if data.len() != CHUNK_SIZE * CHUNK_SIZE {
            return false;
        }
        let mut cells = Box::new([0i8; CHUNK_SIZE * CHUNK_SIZE]);
        cells.copy_from_slice(data);
        self.cells = cells;
        true
    }

    /// 读取宏观状态
    pub fn state(&self, gx: i32, gy: i32) -> Option<u8> {
        self.get(gx, gy).map(log_to_state)
    }

    /// 全部 cell 的宏观状态字节 (65536B)，用于 map_full 二进制帧
    pub fn state_bytes(&self) -> Box<[u8; CHUNK_SIZE * CHUNK_SIZE]> {
        let mut out = Box::new([CellState::Unknown as u8; CHUNK_SIZE * CHUNK_SIZE]);
        for (i, &log) in self.cells.iter().enumerate() {
            out[i] = log_to_state(log);
        }
        out
    }
}

/// 概率分 → 宏观状态（三态）
fn log_to_state(log: i8) -> u8 {
    if log > OCCUPIED_THRESHOLD {
        CellState::Occupied as u8
    } else if log < FREE_THRESHOLD {
        CellState::Free as u8
    } else {
        CellState::Unknown as u8
    }
}

/// 占据栅格地图
///
/// Task 13_2 双表结构：
/// - `chunk`（merged）: 本车观测 + 远端增量（当前单车场景 = own）
/// - `own`（own 表）  : 本车观测累积贡献（对账上传的数据源）
#[derive(Clone)]
pub struct OccupancyGrid {
    pub chunk: Chunk,
    pub own: Chunk,
}

impl OccupancyGrid {
    pub fn new() -> Self {
        Self {
            chunk: Chunk::new(0, 0),
            own: Chunk::new(0, 0),
        }
    }

    /// 概率更新：同时更新 own（本车贡献）与 chunk（merged）
    /// 返回 (宏观状态是否变化, 新宏观状态, Δ = own 的数值差分)
    pub fn update(&mut self, gx: i32, gy: i32, occupied: bool) -> Option<(bool, u8, i8)> {
        let own_result = self.own.update(gx, gy, occupied)?;
        let chunk_result = self.chunk.update(gx, gy, occupied)?;
        Some((chunk_result.0, chunk_result.1, own_result.2))
    }

    /// 概率衰减：同时更新 own（本车贡献）与 chunk（merged），返回 own 的 Δ
    /// Task 15 C 节：动态障碍掩蔽（与 update 相同的双写语义）
    pub fn decay(&mut self, gx: i32, gy: i32, amount: i8) -> Option<(bool, u8, i8)> {
        let own_result = self.own.decay(gx, gy, amount)?;
        let chunk_result = self.chunk.decay(gx, gy, amount)?;
        Some((chunk_result.0, chunk_result.1, own_result.2))
    }

    /// 读取宏观状态（merged）
    pub fn state(&self, gx: i32, gy: i32) -> Option<u8> {
        self.chunk.state(gx, gy)
    }

    /// merged 整表 log-odds 导出
    pub fn log_odds_bytes(&self) -> Box<[i8; CHUNK_SIZE * CHUNK_SIZE]> {
        self.chunk.log_odds_bytes()
    }

    /// merged 整表导入（对账下发替换）
    pub fn set_log_odds(&mut self, data: &[i8]) -> bool {
        self.chunk.set_log_odds(data)
    }

    /// own 表整表 log-odds 导出（对账上传 / WS full map 数据源）
    pub fn own_log_odds_bytes(&self) -> Box<[i8; CHUNK_SIZE * CHUNK_SIZE]> {
        self.own.log_odds_bytes()
    }

    /// own 表整表导入
    pub fn set_own_log_odds(&mut self, data: &[i8]) -> bool {
        self.own.set_log_odds(data)
    }

    /// 应用远端增量：**只写 merged（chunk），own 不动**（Task 13_2）
    ///
    /// own = 本车观测累积贡献（对账上传数据源），混入他人贡献会导致终端 Σ 双倍计数，
    /// 故入站 Δ 一律走本方法，禁止直接调用 `update()`（那是双表观测语义）。
    pub fn apply_delta(&mut self, gx: i32, gy: i32, delta: i8) -> bool {
        self.chunk.apply_delta(gx, gy, delta)
    }

}

impl Default for OccupancyGrid {
    fn default() -> Self { Self::new() }
}

/// 世界坐标 → 全局网格坐标
pub fn world_to_grid(wx: f32, wy: f32) -> (i32, i32) {
    ((wx / CELL_RESOLUTION).floor() as i32, (wy / CELL_RESOLUTION).floor() as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunk_initial_state() {
        let chunk = Chunk::new(0, 0);
        assert_eq!(chunk.state(0, 0), Some(CellState::Unknown as u8));
        assert_eq!(chunk.state(127, 127), Some(CellState::Unknown as u8));
    }

    #[test]
    fn test_chunk_out_of_bounds() {
        let chunk = Chunk::new(0, 0);
        assert_eq!(chunk.get(-1, 0), None);
        assert_eq!(chunk.get(256, 0), None);
    }

    #[test]
    fn test_probabilistic_update() {
        let mut chunk = Chunk::new(0, 0);

        // 1 次命中：0+3=3，≤6 → Unknown
        let (changed, old, delta) = chunk.update(100, 200, true).unwrap();
        assert!(!changed);
        assert_eq!(old, CellState::Unknown as u8);
        assert_eq!(delta, 3);
        assert_eq!(chunk.state(100, 200), Some(CellState::Unknown as u8));

        // 2 次命中：6，6>6 否 → 仍 Unknown
        let (changed, _, delta) = chunk.update(100, 200, true).unwrap(); // 6
        assert!(!changed);
        assert_eq!(delta, 3);
        assert_eq!(chunk.state(100, 200), Some(CellState::Unknown as u8));

        // 第 3 次命中：9 → 夹断 8，8 > 6 → Occupied；Δ = 8−6 = +2（clamp 吃掉 1）
        let (changed, _, delta) = chunk.update(100, 200, true).unwrap(); // 8(clamp)
        assert!(changed);
        assert_eq!(delta, 2);
        assert_eq!(chunk.state(100, 200), Some(CellState::Occupied as u8));

        // 1 次漏打：8-1=7，>6 → 仍 Occupied
        let (changed, _, delta) = chunk.update(100, 200, false).unwrap();
        assert!(!changed);
        assert_eq!(delta, -1);
        assert_eq!(chunk.state(100, 200), Some(CellState::Occupied as u8));

        // 第 2 次漏打：7-1=6，6>6 否 → Unknown
        let (changed, _, _) = chunk.update(100, 200, false).unwrap();
        assert!(changed);
        assert_eq!(chunk.state(100, 200), Some(CellState::Unknown as u8));
    }

    #[test]
    fn test_delta_saturation_zero() {
        // 饱和格再更新：Δ = 0
        let mut chunk = Chunk::new(0, 0);
        for _ in 0..20 { chunk.update(50, 50, true); }
        let (_, _, delta) = chunk.update(50, 50, true).unwrap();
        assert_eq!(delta, 0, "饱和 +8 后命中 Δ 应为 0");

        for _ in 0..20 { chunk.update(60, 60, false); }
        let (_, _, delta) = chunk.update(60, 60, false).unwrap();
        assert_eq!(delta, 0, "饱和 -8 后掠过 Δ 应为 0");
    }

    #[test]
    fn test_grid_own_sync() {
        // OccupancyGrid::update 同时更新 own 与 chunk（单车场景二者一致）
        let mut grid = OccupancyGrid::new();
        grid.update(10, 10, true).unwrap();
        assert_eq!(grid.own.get(10, 10).unwrap(), 3);
        assert_eq!(grid.chunk.get(10, 10).unwrap(), 3);
        // Δ 来自 own 的差分
        let (_, _, delta) = grid.update(10, 10, true).unwrap();
        assert_eq!(delta, 3);
        assert_eq!(grid.own.get(10, 10).unwrap(), 6);
    }

    #[test]
    fn test_log_odds_roundtrip() {
        let mut chunk = Chunk::new(0, 0);
        chunk.update(1, 1, true).unwrap();
        chunk.update(1, 1, true).unwrap();
        chunk.update(2, 2, false).unwrap();
        chunk.update(2, 2, false).unwrap();
        chunk.update(2, 2, false).unwrap();

        let bytes = chunk.log_odds_bytes();
        assert_eq!(bytes.len(), CHUNK_SIZE * CHUNK_SIZE);
        assert_eq!(bytes[1 * CHUNK_SIZE + 1], 6);   // (1,1) 两次命中
        assert_eq!(bytes[2 * CHUNK_SIZE + 2], -3);  // (2,2) 三次掠过
        assert_eq!(bytes[0], 0);                     // 未观测保持 0

        // roundtrip
        let mut chunk2 = Chunk::new(0, 0);
        assert!(chunk2.set_log_odds(bytes.as_ref()));
        assert_eq!(chunk2.get(1, 1).unwrap(), 6);
        assert_eq!(chunk2.get(2, 2).unwrap(), -3);

        // 长度非法拒绝
        assert!(!chunk2.set_log_odds(&[0i8; 10]));
        assert_eq!(chunk2.get(1, 1).unwrap(), 6, "失败时不应修改内容");
    }

    #[test]
    fn test_grid_own_log_odds_api() {
        let mut grid = OccupancyGrid::new();
        grid.update(5, 5, true).unwrap();
        grid.update(5, 5, true).unwrap();
        let own_bytes = grid.own_log_odds_bytes();
        assert_eq!(own_bytes[5 * CHUNK_SIZE + 5], 6);
        let merged_bytes = grid.log_odds_bytes();
        assert_eq!(merged_bytes[5 * CHUNK_SIZE + 5], 6);
        // set_log_odds 替换 merged
        let data = vec![0i8; CHUNK_SIZE * CHUNK_SIZE];
        assert!(grid.set_log_odds(&data));
        assert_eq!(grid.chunk.get(5, 5).unwrap(), 0);
    }

    #[test]
    fn test_apply_delta_accumulate() {
        // 任意 Δ 累加（非固定 +3/-1）
        let mut grid = OccupancyGrid::new();
        assert!(grid.apply_delta(10, 10, 3));
        assert_eq!(grid.chunk.get(10, 10).unwrap(), 3);
        assert!(grid.apply_delta(10, 10, 2));
        assert_eq!(grid.chunk.get(10, 10).unwrap(), 5);
        assert!(grid.apply_delta(10, 10, -1));
        assert_eq!(grid.chunk.get(10, 10).unwrap(), 4);
    }

    #[test]
    fn test_apply_delta_clamp() {
        // clamp ±8：与 update 相同的边界语义
        let mut grid = OccupancyGrid::new();
        for _ in 0..10 { grid.apply_delta(50, 50, 3); }
        assert_eq!(grid.chunk.get(50, 50).unwrap(), 8);
        for _ in 0..10 { grid.apply_delta(50, 50, -3); }
        assert_eq!(grid.chunk.get(50, 50).unwrap(), -8);
    }

    #[test]
    fn test_apply_delta_out_of_bounds() {
        let mut grid = OccupancyGrid::new();
        assert!(!grid.apply_delta(-1, 0, 3));
        assert!(!grid.apply_delta(256, 0, 3));
        assert!(!grid.apply_delta(0, 256, 3));
        // 越界不修改任何内容
        assert_eq!(grid.chunk.get(0, 0).unwrap(), 0);
    }

    #[test]
    fn test_apply_delta_does_not_touch_own() {
        // 远端增量只写 merged，own 必须保持本车观测值（CRDT 双倍计数防线）
        let mut grid = OccupancyGrid::new();
        grid.update(5, 5, true).unwrap(); // own=3, chunk=3
        grid.apply_delta(5, 5, 3);        // 远端 Δ
        assert_eq!(grid.chunk.get(5, 5).unwrap(), 6);
        assert_eq!(grid.own.get(5, 5).unwrap(), 3, "own 不得被远端增量污染");
    }

    #[test]
    fn test_threshold_boundary() {
        let mut chunk = Chunk::new(0, 0);

        // 3 次命中：9 → 夹断 8 > 6 → Occupied
        for _ in 0..3 { chunk.update(10, 10, true); }
        assert_eq!(chunk.state(10, 10), Some(CellState::Occupied as u8));

        // 边界：8-1=7 仍 Occupied，7-1=6 恰好 6 不 > 6 → Unknown
        chunk.update(10, 10, false); // 7
        assert_eq!(chunk.state(10, 10), Some(CellState::Occupied as u8));
        chunk.update(10, 10, false); // 6
        assert_eq!(chunk.state(10, 10), Some(CellState::Unknown as u8));

        // Free 边界：8 次 miss 饱和到 -8，-8 < -6 → Free
        for _ in 0..8 { chunk.update(20, 20, false); }
        assert_eq!(chunk.get(20, 20).unwrap(), -8);
        assert_eq!(chunk.state(20, 20), Some(CellState::Free as u8));
    }

    #[test]
    fn test_saturation() {
        let mut chunk = Chunk::new(0, 0);

        // Occupied 夹断到 +8
        for _ in 0..20 {
            chunk.update(50, 50, true);
        }
        assert_eq!(chunk.get(50, 50).unwrap(), 8);
        assert_eq!(chunk.state(50, 50), Some(CellState::Occupied as u8));

        // Free 夹断到 -8
        for _ in 0..20 {
            chunk.update(60, 60, false);
        }
        assert_eq!(chunk.get(60, 60).unwrap(), -8);
        assert_eq!(chunk.state(60, 60), Some(CellState::Free as u8));
    }

    #[test]
    fn test_world_to_grid() {
        assert_eq!(world_to_grid(0.0, 0.0), (0, 0));
        assert_eq!(world_to_grid(0.5, 0.5), (1, 1));
        assert_eq!(world_to_grid(50.0, 50.0), (100, 100));
        assert_eq!(world_to_grid(-0.1, -0.1), (-1, -1));
    }

    #[test]
    fn test_decay_from_zero() {
        // 0 → -3 → -6 → -8 逐帧递减（动态障碍掩蔽 -3 力度）
        let mut chunk = Chunk::new(0, 0);
        let (_, _, d1) = chunk.decay(10, 10, 3).unwrap();
        assert_eq!(d1, -3);
        assert_eq!(chunk.get(10, 10).unwrap(), -3);
        let (_, _, d2) = chunk.decay(10, 10, 3).unwrap();
        assert_eq!(d2, -3);
        assert_eq!(chunk.get(10, 10).unwrap(), -6);
        let (_, _, d3) = chunk.decay(10, 10, 3).unwrap();
        assert_eq!(d3, -2); // -6 → clamp -8
        assert_eq!(chunk.get(10, 10).unwrap(), -8);
    }

    #[test]
    fn test_decay_clears_history_occupied() {
        // 历史残留 +8（Occupied）被一次 -3 降到 5（Unknown），清历史残留
        let mut chunk = Chunk::new(0, 0);
        for _ in 0..3 {
            chunk.update(10, 10, true);
        }
        assert_eq!(chunk.get(10, 10).unwrap(), 8);
        let (changed, state, delta) = chunk.decay(10, 10, 3).unwrap();
        assert_eq!(chunk.get(10, 10).unwrap(), 5);
        assert_eq!(delta, -3);
        assert!(changed);
        assert_eq!(state, CellState::Unknown as u8);
    }

    #[test]
    fn test_decay_idempotent_at_free_clamp() {
        // 压到底 -8 后再 decay 无变化（Δ=0，不广播）
        let mut chunk = Chunk::new(0, 0);
        for _ in 0..4 {
            chunk.decay(5, 5, 3);
        }
        assert_eq!(chunk.get(5, 5).unwrap(), -8);
        let (changed, _, delta) = chunk.decay(5, 5, 3).unwrap();
        assert_eq!(delta, 0);
        assert!(!changed);
    }

    #[test]
    fn test_decay_out_of_bounds() {
        let mut chunk = Chunk::new(0, 0);
        assert!(chunk.decay(-1, 0, 3).is_none());
        assert!(chunk.decay(256, 0, 3).is_none());
    }
}
