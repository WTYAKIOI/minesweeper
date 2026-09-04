use serde::{Deserialize, Serialize};
use std::fmt;

/// 棋盘坐标 (0-indexed)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
pub struct Coord {
    pub x: u32,
    pub y: u32,
}

impl Coord {
    pub fn new(x: u32, y: u32) -> Self {
        Self { x, y }
    }

    /// 返回以自身为中心的周围 8 邻居坐标 (棋盘内)
    pub fn neighbors(&self, width: u32, height: u32) -> Vec<Coord> {
        let mut result = Vec::with_capacity(8);
        for dy in -1i32..=1 {
            for dx in -1i32..=1 {
                if dx == 0 && dy == 0 {
                    continue;
                }
                let nx = self.x as i32 + dx;
                let ny = self.y as i32 + dy;
                if nx >= 0 && ny >= 0 && (nx as u32) < width && (ny as u32) < height {
                    result.push(Coord { x: nx as u32, y: ny as u32 });
                }
            }
        }
        result
    }
}

impl fmt::Display for Coord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({}, {})", self.x, self.y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_neighbors_corner() {
        let c = Coord::new(0, 0);
        let n = c.neighbors(10, 10);
        assert_eq!(n.len(), 3);
        assert!(n.contains(&Coord::new(0, 1)));
        assert!(n.contains(&Coord::new(1, 0)));
        assert!(n.contains(&Coord::new(1, 1)));
    }

    #[test]
    fn test_neighbors_center() {
        let c = Coord::new(5, 5);
        let n = c.neighbors(10, 10);
        assert_eq!(n.len(), 8);
    }

    #[test]
    fn test_neighbors_edge() {
        let c = Coord::new(0, 5);
        let n = c.neighbors(10, 10);
        assert_eq!(n.len(), 5);
    }
}
