//! Primitives related to chunks and blocks.

use hashbrown::HashMap;
use itertools::Itertools;
use noise::{NoiseFn, OpenSimplex};
use tracing::info;

pub use wgpu_block_shared::chunk::BlockId;
use wgpu_block_shared::chunk::Chunk;

/// A collection of chunks, indexed by their chunk coordinates `(cx, cz)`.
pub struct ChunkCollection {
    chunks: HashMap<(i64, i64), ClientChunk>,
}

#[derive(Clone, Copy)]
pub enum MaybeLoadedBlock {
    Loaded(BlockId),
    Unloaded,
}

impl ChunkCollection {
    pub fn new() -> Self {
        Self {
            chunks: HashMap::new(),
        }
    }

    pub fn load_chunk(&mut self, (cx, cz): (i64, i64), chunk: Chunk) {
        self.chunks.insert(
            (cx, cz),
            ClientChunk {
                chunk,
                dirty: [true; 16],
            },
        );
    }

    pub fn unload_chunk(&mut self, (cx, cz): (i64, i64)) {
        self.chunks.remove(&(cx, cz));
    }

    /// # Panics
    ///
    /// Panics if the chunk is nonexistent.
    pub fn get_chunk(&self, (cx, cz): (i64, i64)) -> &ClientChunk {
        &self.chunks[&(cx, cz)]
    }

    /// # Panics
    ///
    /// Panics if the chunk is nonexistent.
    pub fn get_chunk_mut(&mut self, (cx, cz): (i64, i64)) -> &mut ClientChunk {
        self.chunks.get_mut(&(cx, cz)).unwrap()
    }

    /// For coordinates that are OOB above or below, the output is always [`Block::Empty`],
    /// despite the fact that we can't "load" a chunk that contains the block.
    pub fn get_block(&self, (x, y, z): (i64, i64, i64)) -> MaybeLoadedBlock {
        if (0..256).contains(&y) == false {
            return MaybeLoadedBlock::Loaded(BlockId::Empty);
        }

        let cx = x.div_euclid(16);
        let cz = z.div_euclid(16);

        let lx = x.rem_euclid(16) as usize;
        let ly = y as usize;
        let lz = z.rem_euclid(16) as usize;

        let chunk = match self.chunks.get(&(cx, cz)) {
            Some(chunk) => chunk,
            None => return MaybeLoadedBlock::Unloaded,
        };

        MaybeLoadedBlock::Loaded(chunk.get((lx, ly, lz)))
    }

    pub fn set_block(&mut self, (x, y, z): (i64, i64, i64), block: BlockId) {
        if (0..256).contains(&y) == false {
            return;
        }

        let cx = x.div_euclid(16);
        let cz = z.div_euclid(16);
        let s = y.div_euclid(16) as usize;

        let lx = x.rem_euclid(16) as usize;
        let ly = y as usize;
        let lz = z.rem_euclid(16) as usize;

        if let Some(chunk) = self.chunks.get_mut(&(cx, cz)) {
            chunk.set((lx, ly, lz), block);
            chunk.dirty[s] = true;
        }
    }

    pub fn loaded_chunk_coordinates(&self) -> Vec<(i64, i64)> {
        self.chunks.keys().cloned().collect_vec()
    }
}

#[derive(Default)]
pub struct ClientChunk {
    chunk: Chunk,
    dirty: [bool; 16],
}

impl ClientChunk {
    pub fn set(&mut self, (x, y, z): (usize, usize, usize), block: BlockId) {
        self.chunk.set((x, y, z), block)
    }

    pub fn get(&self, (x, y, z): (usize, usize, usize)) -> BlockId {
        self.chunk.get((x, y, z))
    }

    pub fn is_subchunk_dirty(&self, s: usize) -> bool {
        self.dirty[s]
    }

    pub fn unmark_subchunk_dirty(&mut self, s: usize) {
        self.dirty[s] = false;
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_chunk_collection_new() {
        tracing_subscriber::fmt::init();
        ChunkCollection::new();
    }
}
