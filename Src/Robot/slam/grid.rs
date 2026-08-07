//Presented by KeJi
//Created Date ： 2026-07-21
//Modified Date ： 2026-08-04

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
const OCCUPIED_CLAMP: i8 = 8;
const FREE_CLAMP: i8 = -8;
const OCCUPIED_THRESHOLD: i8 = 6;
const FREE_THRESHOLD: i8 = -6;

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

/// 变化的格子
#[derive(Debug, Clone)]
pub struct Delta {
    pub gx: i32,
    pub gy: i32,
    pub state: u8,
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
    /// 返回 (宏观状态是否变化, 旧宏观状态)
    pub fn update(&mut self, gx: i32, gy: i32, occupied: bool) -> Option<(bool, u8)> {
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
        Some((old_state != new_state, new_state))
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

/// 占据栅格地图（当前单 Chunk）
#[derive(Clone)]
pub struct OccupancyGrid {
    pub chunk: Chunk,
}

impl OccupancyGrid {
    pub fn new() -> Self {
        Self { chunk: Chunk::new(0, 0) }
    }

    /// 概率更新
    pub fn update(&mut self, gx: i32, gy: i32, occupied: bool) -> Option<(bool, u8)> {
        self.chunk.update(gx, gy, occupied)
    }

    /// 读取宏观状态
    pub fn state(&self, gx: i32, gy: i32) -> Option<u8> {
        self.chunk.state(gx, gy)
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
        let (changed, old) = chunk.update(100, 200, true).unwrap();
        assert!(!changed);
        assert_eq!(old, CellState::Unknown as u8);
        assert_eq!(chunk.state(100, 200), Some(CellState::Unknown as u8));

        // 2 次命中：6，6>6 否 → 仍 Unknown
        let (changed, _) = chunk.update(100, 200, true).unwrap(); // 6
        assert!(!changed);
        assert_eq!(chunk.state(100, 200), Some(CellState::Unknown as u8));

        // 第 3 次命中：9 → 夹断 8，8 > 6 → Occupied
        let (changed, _) = chunk.update(100, 200, true).unwrap(); // 8(clamp)
        assert!(changed);
        assert_eq!(chunk.state(100, 200), Some(CellState::Occupied as u8));

        // 1 次漏打：8-1=7，>6 → 仍 Occupied
        let (changed, _) = chunk.update(100, 200, false).unwrap();
        assert!(!changed);
        assert_eq!(chunk.state(100, 200), Some(CellState::Occupied as u8));

        // 第 2 次漏打：7-1=6，6>6 否 → Unknown
        let (changed, _) = chunk.update(100, 200, false).unwrap();
        assert!(changed);
        assert_eq!(chunk.state(100, 200), Some(CellState::Unknown as u8));
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
}
