use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use super::Coord;

/// 格子状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CellState {
    /// 未翻开且未标旗
    Unknown,
    /// 已标旗
    Flagged,
    /// 已翻开，数字 0-8
    Revealed(u8),
}

/// 已翻开的格子
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevealedCell {
    pub coord: Coord,
    pub number: u8,
}

/// 玩家视角的局面 — 只含已翻开数字、旗帜、未知坐标，绝不包含真实雷藏
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerView {
    pub width: u32,
    pub height: u32,
    pub revealed: Vec<RevealedCell>,
    pub flagged: Vec<Coord>,
    pub unknown: Vec<Coord>,
    pub remaining_mines: u32,
}

impl PlayerView {
    /// 构建一个 HashMap，key=Coord, value=CellState，方便快速查询
    pub fn to_state_map(&self) -> HashMap<Coord, CellState> {
        let mut map = HashMap::new();
        for cell in &self.revealed {
            map.insert(cell.coord, CellState::Revealed(cell.number));
        }
        for c in &self.flagged {
            map.insert(*c, CellState::Flagged);
        }
        for c in &self.unknown {
            map.insert(*c, CellState::Unknown);
        }
        map
    }

    /// 构建数字坐标 → 已标旗邻居数的映射
    pub fn flagged_count_around(&self) -> HashMap<Coord, u32> {
        let state = self.to_state_map();
        let mut result = HashMap::new();
        for cell in &self.revealed {
            let count = cell
                .coord
                .neighbors(self.width, self.height)
                .iter()
                .filter(|n| matches!(state.get(n), Some(CellState::Flagged)))
                .count() as u32;
            result.insert(cell.coord, count);
        }
        result
    }

    /// 构建数字坐标 → 未知邻居数的映射
    pub fn unknown_count_around(&self) -> HashMap<Coord, u32> {
        let state = self.to_state_map();
        let mut result = HashMap::new();
        for cell in &self.revealed {
            let count = cell
                .coord
                .neighbors(self.width, self.height)
                .iter()
                .filter(|n| matches!(state.get(n), Some(CellState::Unknown)))
                .count() as u32;
            result.insert(cell.coord, count);
        }
        result
    }

    /// 某个数字格子剩余还需标记的雷数 = 数字 - 周围已标旗数
    pub fn remaining_mines_around(&self) -> HashMap<Coord, i32> {
        let flagged = self.flagged_count_around();
        let mut result = HashMap::new();
        for cell in &self.revealed {
            let flagged_n = *flagged.get(&cell.coord).unwrap_or(&0) as i32;
            result.insert(cell.coord, cell.number as i32 - flagged_n);
        }
        result
    }

    /// 合法性检查：每个已翻开数字周围的已标旗数不超过该数字
    pub fn validate(&self) -> bool {
        let flagged = self.flagged_count_around();
        for cell in &self.revealed {
            let fc = *flagged.get(&cell.coord).unwrap_or(&0);
            if fc > cell.number as u32 {
                return false;
            }
        }
        true
    }

    /// 从二维数组快速构建 PlayerView
    /// board[y][x] = -1 未知, -2 旗帜, 0-8 已翻开数字
    pub fn from_2d(board: &[Vec<i32>], remaining_mines: u32) -> Self {
        let height = board.len() as u32;
        let width = if height > 0 { board[0].len() as u32 } else { 0 };
        let mut revealed = Vec::new();
        let mut flagged = Vec::new();
        let mut unknown = Vec::new();
        for (y, row) in board.iter().enumerate() {
            for (x, &val) in row.iter().enumerate() {
                let coord = Coord::new(x as u32, y as u32);
                match val {
                    -2 => flagged.push(coord),
                    -1 => unknown.push(coord),
                    n if (0..=8).contains(&n) => revealed.push(RevealedCell {
                        coord,
                        number: n as u8,
                    }),
                    _ => {}
                }
            }
        }
        PlayerView {
            width,
            height,
            revealed,
            flagged,
            unknown,
            remaining_mines,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_2d() {
        let board = vec![
            vec![1, -1, -2],
            vec![-1, 2, -1],
            vec![0, -1, -1],
        ];
        let pv = PlayerView::from_2d(&board, 10);
        assert_eq!(pv.width, 3);
        assert_eq!(pv.height, 3);
        assert_eq!(pv.revealed.len(), 3);
        assert_eq!(pv.flagged.len(), 1);
        assert_eq!(pv.unknown.len(), 5);
    }

    #[test]
    fn test_validate_ok() {
        let board = vec![
            vec![1, -2, -1],
            vec![-1, 2, -1],
            vec![0, -1, -1],
        ];
        let pv = PlayerView::from_2d(&board, 10);
        assert!(pv.validate());
    }

    #[test]
    fn test_validate_fail() {
        // Number 1 at (0,0) with 2 flagged neighbors (1,0) and (0,1) → 2 > 1, invalid
        let board = vec![
            vec![1, -2, -1],
            vec![-2, 2, -1],
            vec![0, -1, -1],
        ];
        let pv = PlayerView::from_2d(&board, 10);
        assert!(!pv.validate());
    }

    #[test]
    fn test_remaining_mines_around() {
        let board = vec![
            vec![3, -2, -1],
            vec![-1, -1, -1],
            vec![0, -1, -1],
        ];
        let pv = PlayerView::from_2d(&board, 10);
        let rm = pv.remaining_mines_around();
        // (0,0) has number 3, 1 flagged neighbor → remaining = 2
        assert_eq!(*rm.get(&Coord::new(0, 0)).unwrap(), 2);
        // (0,2) has number 0, 0 flagged → remaining = 0
        assert_eq!(*rm.get(&Coord::new(0, 2)).unwrap(), 0);
    }
}
