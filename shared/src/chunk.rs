use serde::{Deserialize, Serialize};
use serde_big_array::BigArray;
use std::fmt::Debug;

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    subchunks: [SubChunk; 16],
}

/// And POD type holding block data for 16x16x16 areas, row-major
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubChunk {
    #[serde(with = "BigArray")]
    blocks: [BlockId; 16 * 16 * 16],
}

impl Chunk {
    pub fn set(&mut self, (x, y, z): (usize, usize, usize), block: BlockId) {
        let subchunk_index = y.div_euclid(16);
        let sy = y.rem_euclid(16);
        self.subchunks[subchunk_index].blocks[sy * 16 * 16 + z * 16 + x] = block;
    }

    pub fn get(&self, (x, y, z): (usize, usize, usize)) -> BlockId {
        let subchunk_index = y.div_euclid(16);
        let sy = y.rem_euclid(16);
        self.subchunks[subchunk_index].blocks[sy * 16 * 16 + z * 16 + x]
    }
}

impl Default for SubChunk {
    fn default() -> Self {
        Self {
            blocks: [BlockId::Empty; 16 * 16 * 16],
        }
    }
}

#[derive(Default, Debug, Clone, Copy, Serialize, Deserialize)]
#[repr(u8)]
pub enum BlockId {
    #[default]
    Empty,
    Grass,
}

impl BlockId {
    pub fn is_opaque(&self) -> bool {
        use BlockId::*;
        match self {
            Empty => false,
            _ => true,
        }
    }
}
