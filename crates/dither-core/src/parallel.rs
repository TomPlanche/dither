//! One import that decides whether the hot loops spread across threads.
//!
//! The parallel stages all share a shape: split a mutable output buffer into
//! equal rows or chunks, then fill each one from a disjoint slice of the input.
//! [`rayon`] gives that for free on a desktop, but `wasm32-unknown-unknown` has
//! no threads for it to use, so the `parallel` feature swaps in the sequential
//! `chunks_mut` from the standard library behind the same name and the call
//! sites stay identical.

#[cfg(feature = "parallel")]
pub use rayon::prelude::*;

#[cfg(not(feature = "parallel"))]
pub use sequential::ParallelSliceMut;

#[cfg(not(feature = "parallel"))]
mod sequential {
    use std::slice::ChunksMut;

    /// Stands in for the `rayon` trait of the same name, one method deep.
    ///
    /// The iterator it hands back is an ordinary [`Iterator`], and `enumerate`
    /// and `for_each` mean there what they mean on a `ParallelIterator`, so the
    /// loops read the same whichever half of the `cfg` is compiled.
    pub trait ParallelSliceMut<T> {
        fn par_chunks_mut(&mut self, chunk_size: usize) -> ChunksMut<'_, T>;
    }

    impl<T> ParallelSliceMut<T> for [T] {
        fn par_chunks_mut(&mut self, chunk_size: usize) -> ChunksMut<'_, T> {
            self.chunks_mut(chunk_size)
        }
    }
}
