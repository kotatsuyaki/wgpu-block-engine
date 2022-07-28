//! Primitives related to chunks and blocks.

use hashbrown::HashMap;
use itertools::Itertools;
use noise::{NoiseFn, OpenSimplex};
use tracing::info;

pub use wgpu_block_shared::chunk::Block;
use wgpu_block_shared::chunk::Chunk;

/// A collection of chunks, indexed by their chunk coordinates `(cx, cz)`.
pub struct ChunkCollection {
    chunks: HashMap<(i64, i64), ClientChunk>,
}

#[derive(Clone, Copy)]
pub enum MaybeLoadedBlock {
    Loaded(Block),
    Unloaded,
}

impl ChunkCollection {
    pub fn new() -> Self {
        let mut chunks = HashMap::new();
        let simplex = OpenSimplex::new(0);

        let mut maxheight = 0;
        for cx in -3..3_i64 {
            for cz in -3..3_i64 {
                info!("Generating chunk ({cx}, {cz})");

                let mut chunk = ClientChunk::default();
                chunk.dirty = [true; 16];
                for lx in 0..16 {
                    for lz in 0..16 {
                        let height = (simplex
                            .get([(cx * 16 + lx) as f64 / 16.0, (cz * 16 + lz) as f64 / 16.0])
                            + 1.0)
                            * 10.0
                            + 26.0;
                        let height = height as usize;
                        info!("Height at (lx = {lx}, lz = {lz}) is {height}");
                        maxheight = maxheight.max(height);
                        for h in 0..height {
                            chunk.set((lx as usize, h, lz as usize), Block::Grass);
                        }
                    }
                }
                chunks.insert((cx, cz), chunk);
            }
        }

        info!(maxheight);

        Self { chunks }
    }

    /// Get a chunk from its chunk coordinates `(cx, cz)`.
    ///
    /// # Panics
    ///
    /// Panics if the chunk is nonexistent.
    pub fn get_chunk(&self, (cx, cz): (i64, i64)) -> &ClientChunk {
        &self.chunks[&(cx, cz)]
    }

    /// Get a chunk mutably from its chunk coordinates `(cx, cz)`.
    ///
    /// # Panics
    ///
    /// Panics if the chunk is nonexistent.
    pub fn get_chunk_mut(&mut self, (cx, cz): (i64, i64)) -> &mut ClientChunk {
        self.chunks.get_mut(&(cx, cz)).unwrap()
    }

    /// Get a block from its *world* coordinates.
    ///
    /// For coordinates that are OOB above or below, the output is always [`Block::Empty`],
    /// despite the fact that we can't "load" a chunk that contains the block.
    pub fn get_block(&self, (x, y, z): (i64, i64, i64)) -> MaybeLoadedBlock {
        if (0..256).contains(&y) == false {
            return MaybeLoadedBlock::Loaded(Block::Empty);
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

    /// Get chunk coordinates of all the loaded chunks.
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
    pub fn set(&mut self, (x, y, z): (usize, usize, usize), block: Block) {
        self.chunk.set((x, y, z), block)
    }

    pub fn get(&self, (x, y, z): (usize, usize, usize)) -> Block {
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
