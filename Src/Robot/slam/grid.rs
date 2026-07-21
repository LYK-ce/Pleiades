//Presented by KeJi
//Created Date ： 2026-07-21
//Modified Date ： 2026-07-21

//! 占据栅格地图
//!
//! 单 Chunk (256×256, 0.5m/cell)，初始全未知(2)。

/// Chunk 大小 (cells)
pub const CHUNK_SIZE: usize = 256;

/// 分辨率 (米/格)
pub const CELL_RESOLUTION: f32 = 0.5;

/// 格子状态
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

/// 一个 Chunk: 256×256 cells
pub struct Chunk {
    /// 行优先，cells[y * 256 + x]
    cells: Box<[u8; CHUNK_SIZE * CHUNK_SIZE]>,
    /// Chunk 在世界坐标系中的原点 (grid 坐标)
    pub origin_gx: i32,
    pub origin_gy: i32,
}

impl Chunk {
    pub fn new(origin_gx: i32, origin_gy: i32) -> Self {
        Self {
            cells: Box::new([CellState::Unknown as u8; CHUNK_SIZE * CHUNK_SIZE]),
            origin_gx,
            origin_gy,
        }
    }

    /// 读取全局网格坐标 (gx, gy) 的格子值
    /// 返回 None 表示坐标不在本 Chunk 范围内
    pub fn get(&self, gx: i32, gy: i32) -> Option<u8> {
        let lx = gx - self.origin_gx;
        let ly = gy - self.origin_gy;
        if lx < 0 || lx >= CHUNK_SIZE as i32 || ly < 0 || ly >= CHUNK_SIZE as i32 {
            return None;
        }
        Some(self.cells[ly as usize * CHUNK_SIZE + lx as usize])
    }

    /// 设置全局网格坐标 (gx, gy)，返回旧值（用于对比是否变化）
    /// 坐标超出范围则返回 None
    pub fn set(&mut self, gx: i32, gy: i32, state: u8) -> Option<u8> {
        let lx = gx - self.origin_gx;
        let ly = gy - self.origin_gy;
        if lx < 0 || lx >= CHUNK_SIZE as i32 || ly < 0 || ly >= CHUNK_SIZE as i32 {
            return None;
        }
        let idx = ly as usize * CHUNK_SIZE + lx as usize;
        let old = self.cells[idx];
        self.cells[idx] = state;
        Some(old)
    }

    /// 全部 cell 的原始字节 (65536B)，用于 map_full 二进制帧
    pub fn raw_bytes(&self) -> &[u8; CHUNK_SIZE * CHUNK_SIZE] {
        &self.cells
    }
}

/// 占据栅格地图（当前单 Chunk）
pub struct OccupancyGrid {
    pub chunk: Chunk,
}

impl OccupancyGrid {
    /// 创建地图：Chunk(0,0)，初始全未知
    pub fn new() -> Self {
        Self {
            chunk: Chunk::new(0, 0),
        }
    }

    /// 读取格子
    pub fn get(&self, gx: i32, gy: i32) -> Option<u8> {
        self.chunk.get(gx, gy)
    }

    /// 设置格子，返回旧值
    pub fn set(&mut self, gx: i32, gy: i32, state: u8) -> Option<u8> {
        self.chunk.set(gx, gy, state)
    }

    /// 全部 cell 原始字节
    pub fn raw_bytes(&self) -> &[u8; CHUNK_SIZE * CHUNK_SIZE] {
        self.chunk.raw_bytes()
    }

    /// 构建 map_full 二进制帧（Pictor 协议）
    pub fn build_map_full(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(65545);
        buf.push(0u8); // type = map_full
        buf.extend_from_slice(&(self.chunk.origin_gx as i32).to_be_bytes());
        buf.extend_from_slice(&(self.chunk.origin_gy as i32).to_be_bytes());
        buf.extend_from_slice(self.chunk.raw_bytes().as_ref());
        buf
    }
}

impl Default for OccupancyGrid {
    fn default() -> Self {
        Self::new()
    }
}

/// 世界坐标 → 全局网格坐标
pub fn world_to_grid(wx: f32, wy: f32) -> (i32, i32) {
    (
        (wx / CELL_RESOLUTION).floor() as i32,
        (wy / CELL_RESOLUTION).floor() as i32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunk_initial_state() {
        let chunk = Chunk::new(0, 0);
        assert_eq!(chunk.get(0, 0), Some(CellState::Unknown as u8));
        assert_eq!(chunk.get(127, 127), Some(CellState::Unknown as u8));
        assert_eq!(chunk.get(255, 255), Some(CellState::Unknown as u8));
    }

    #[test]
    fn test_chunk_out_of_bounds() {
        let chunk = Chunk::new(0, 0);
        assert_eq!(chunk.get(-1, 0), None);
        assert_eq!(chunk.get(256, 0), None);
    }

    #[test]
    fn test_chunk_set_and_get() {
        let mut chunk = Chunk::new(0, 0);
        let old = chunk.set(100, 200, CellState::Occupied as u8);
        assert_eq!(old, Some(CellState::Unknown as u8));
        assert_eq!(chunk.get(100, 200), Some(CellState::Occupied as u8));
    }

    #[test]
    fn test_world_to_grid() {
        assert_eq!(world_to_grid(0.0, 0.0), (0, 0));
        assert_eq!(world_to_grid(0.5, 0.5), (1, 1));
        assert_eq!(world_to_grid(50.0, 50.0), (100, 100));
        assert_eq!(world_to_grid(-0.1, -0.1), (-1, -1));
    }
}
