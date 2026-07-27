//Presented by KeJi
//Created Date ： 2026-07-21
//Modified Date ： 2026-07-27

//! 占据栅格地图 — 概率 log-odds（三态）
//!
//! 对标 Cartographer / GMapping 标准做法。
//! 单 Chunk (256×256, 0.5m/cell)，初始全 0（Unknown）。
//! Occupied: +3/次, 夹断 +30, >+10 视为 Occupied
//! Free:     -2/次, 夹断 -20, <-10 视为 Free
//! 中间 [-10, +10] → Unknown

use tracing::info;

/// Chunk 大小 (cells)
pub const CHUNK_SIZE: usize = 256;

/// 分辨率 (米/格)
pub const CELL_RESOLUTION: f32 = 0.5;

/// log-odds 参数
const OCCUPIED_INCREMENT: i8 = 3;
const FREE_DECREMENT: i8 = 2;
const OCCUPIED_CLAMP: i8 = 30;
const FREE_CLAMP: i8 = -20;
const OCCUPIED_THRESHOLD: i8 = 10;
const FREE_THRESHOLD: i8 = -10;

/// 格子宏观状态（供外部使用）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CellState {
    Free = 0,
    Occupied = 1,
    Unknown = 2,
}

/// 变化的格子
#[derive(Debug, Clone)]
pub struct Delta {
    pub gx: i32,
    pub gy: i32,
    pub state: u8,
}

/// 一个 Chunk: 256×256 cells（i8 log-odds 概率分）
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

    /// 概率更新：occupied=true → +3 (夹断+30), false → -2 (夹断-20)
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

    /// 构建 map_full 二进制帧（Pictor 协议）
    pub fn build_map_full(&self) -> Vec<u8> {
        let bytes = self.chunk.state_bytes();
        let (mut free, mut occupied, mut unknown) = (0usize, 0usize, 0usize);
        for &b in bytes.iter() {
            match b {
                0 => free += 1,
                1 => occupied += 1,
                _ => unknown += 1,
            }
        }
        info!("[SLAM] 地图状态: 可通行={free} 墙壁={occupied} 未知={unknown}");

        let mut buf = Vec::with_capacity(65545);
        buf.push(0u8); // type = map_full
        buf.extend_from_slice(&(self.chunk.origin_gx as i32).to_be_bytes());
        buf.extend_from_slice(&(self.chunk.origin_gy as i32).to_be_bytes());
        buf.extend_from_slice(bytes.as_ref());
        buf
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

        // 1 次命中：0+3=3，≤10 → Unknown
        let (changed, old) = chunk.update(100, 200, true).unwrap();
        assert!(!changed);
        assert_eq!(old, CellState::Unknown as u8);
        assert_eq!(chunk.state(100, 200), Some(CellState::Unknown as u8));

        // 4 次命中：12 > 10 → Occupied
        chunk.update(100, 200, true); // 6
        chunk.update(100, 200, true); // 9
        let (changed, _) = chunk.update(100, 200, true).unwrap(); // 12
        assert!(changed);
        assert_eq!(chunk.state(100, 200), Some(CellState::Occupied as u8));

        // 1 次漏打：12-2=10，>10=否 → Unknown
        let (changed, _) = chunk.update(100, 200, false).unwrap();
        assert!(changed);
        assert_eq!(chunk.state(100, 200), Some(CellState::Unknown as u8));
    }

    #[test]
    fn test_threshold_boundary() {
        let mut chunk = Chunk::new(0, 0);

        // 精确命中 +10：仍 Unknown
        // 初始 0, +3×3=9, 需要直接设值来测试边界
        for _ in 0..4 { chunk.update(10, 10, true); } // 12 > 10 → Occupied
        assert_eq!(chunk.state(10, 10), Some(CellState::Occupied as u8));

        // 拆回：3 次 -2
        chunk.update(10, 10, false); // 10
        assert_eq!(chunk.state(10, 10), Some(CellState::Unknown as u8)); // 10 不 > 10
        chunk.update(10, 10, false); // 8 → Unknown
        assert_eq!(chunk.state(10, 10), Some(CellState::Unknown as u8));

        // Free 边界：需要多轮
        for _ in 0..10 { chunk.update(20, 20, false); } // 饱和到 -20
        assert_eq!(chunk.get(20, 20).unwrap(), -20);
        assert_eq!(chunk.state(20, 20), Some(CellState::Free as u8)); // -20 < -10
    }

    #[test]
    fn test_saturation() {
        let mut chunk = Chunk::new(0, 0);

        // Occupied 夹断到 +30
        for _ in 0..20 {
            chunk.update(50, 50, true);
        }
        assert_eq!(chunk.get(50, 50).unwrap(), 30);
        assert_eq!(chunk.state(50, 50), Some(CellState::Occupied as u8));

        // Free 夹断到 -20
        for _ in 0..20 {
            chunk.update(60, 60, false);
        }
        assert_eq!(chunk.get(60, 60).unwrap(), -20);
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
