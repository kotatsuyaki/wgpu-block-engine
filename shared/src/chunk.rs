use std::fmt::Debug;

#[derive(Default, Debug, Clone)]
pub struct Chunk {
    subchunks: [SubChunk; 16],
}

/// And POD type holding block data for 16x16x16 areas, row-major
#[derive(Debug, Clone)]
pub struct SubChunk {
    blocks: [Block; 16 * 16 * 16],
}

impl Chunk {
    pub fn set(&mut self, (x, y, z): (usize, usize, usize), block: Block) {
        let subchunk_index = y.div_euclid(16);
        let sy = y.rem_euclid(16);
        self.subchunks[subchunk_index].blocks[sy * 16 * 16 + z * 16 + x] = block;
    }

    pub fn get(&self, (x, y, z): (usize, usize, usize)) -> Block {
        let subchunk_index = y.div_euclid(16);
        let sy = y.rem_euclid(16);
        self.subchunks[subchunk_index].blocks[sy * 16 * 16 + z * 16 + x]
    }
}

impl Default for SubChunk {
    fn default() -> Self {
        Self {
            blocks: [Block::Empty; 16 * 16 * 16],
        }
    }
}

#[derive(Default, Debug, Clone, Copy)]
#[repr(u8)]
pub enum Block {
    #[default]
    Empty,
    Grass,
}

impl Block {
    pub fn is_opaque(&self) -> bool {
        use Block::*;
        match self {
            Empty => false,
            _ => true,
        }
    }
}
