//! RES-4230: fixed-capacity, allocation-free buffers for embedded hosts.
//!
//! [`FixedBuffer`] is the `no_std` counterpart to the host interpreter's
//! reference-semantics `Buffer<T>` value. Its storage is owned by the caller
//! and lives inline in the value, so firmware can place it on the stack or in
//! a `static` without enabling an allocator. The explicit `&mut` API also
//! makes the single-writer boundary visible when a foreign function receives
//! [`FixedBuffer::as_mut_ptr`].
//!
//! The backing array is initialized for the full capacity, while `len` is a
//! separately checked logical length. This keeps the type panic-free and
//! avoids partially initialized storage in the default runtime.

/// Errors returned by fixed-capacity buffer operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BufferError {
    /// A requested logical length is larger than the type-level capacity.
    LengthExceedsCapacity { length: usize, capacity: usize },
    /// An access is outside the current logical length.
    IndexOutOfBounds { index: usize, len: usize },
}

/// An initialized, caller-owned buffer with a compile-time capacity.
///
/// `N` controls the inline storage size. `len` may be smaller than `N` when
/// constructed with [`Self::with_len`], but every slot remains initialized so
/// increasing the logical length never needs allocation or special
/// initialization code.
#[derive(Debug, PartialEq)]
pub struct FixedBuffer<T, const N: usize> {
    data: [T; N],
    len: usize,
}

impl<T, const N: usize> FixedBuffer<T, N> {
    /// The compile-time storage capacity.
    pub const CAPACITY: usize = N;

    /// Return the current logical element count.
    #[inline]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Return whether the logical buffer contains no elements.
    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Borrow the initialized logical elements.
    #[inline]
    pub fn as_slice(&self) -> &[T] {
        &self.data[..self.len]
    }

    /// Borrow the initialized logical elements mutably.
    #[inline]
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        &mut self.data[..self.len]
    }

    /// Return a pointer to the inline storage for a foreign call.
    ///
    /// The pointer is valid for `len()` elements while `self` remains alive.
    /// A foreign function must not read or write beyond that range.
    #[inline]
    pub fn as_ptr(&self) -> *const T {
        self.data.as_ptr()
    }

    /// Return a mutable pointer to the inline storage for a foreign call.
    ///
    /// The caller must uphold the usual raw-pointer rules and keep the
    /// exclusive borrow active for the complete foreign call.
    #[inline]
    pub fn as_mut_ptr(&mut self) -> *mut T {
        self.data.as_mut_ptr()
    }

    /// Read one logical element without panicking on an invalid index.
    #[inline]
    pub fn get(&self, index: usize) -> Result<&T, BufferError> {
        self.data
            .get(index)
            .filter(|_| index < self.len)
            .ok_or(BufferError::IndexOutOfBounds {
                index,
                len: self.len,
            })
    }

    /// Mutate one logical element without panicking on an invalid index.
    #[inline]
    pub fn get_mut(&mut self, index: usize) -> Result<&mut T, BufferError> {
        let len = self.len;
        self.data
            .get_mut(index)
            .filter(|_| index < len)
            .ok_or(BufferError::IndexOutOfBounds { index, len })
    }

    /// Replace one logical element without panicking on an invalid index.
    #[inline]
    pub fn set(&mut self, index: usize, value: T) -> Result<(), BufferError> {
        *self.get_mut(index)? = value;
        Ok(())
    }

    /// Change the logical length without reallocating.
    #[inline]
    pub fn set_len(&mut self, len: usize) -> Result<(), BufferError> {
        if len > N {
            return Err(BufferError::LengthExceedsCapacity {
                length: len,
                capacity: N,
            });
        }
        self.len = len;
        Ok(())
    }
}

impl<T: Copy, const N: usize> FixedBuffer<T, N> {
    /// Create a full-capacity buffer initialized with `value`.
    #[inline]
    pub fn filled(value: T) -> Self {
        Self {
            data: [value; N],
            len: N,
        }
    }

    /// Create a buffer with a logical length no larger than `N`.
    #[inline]
    pub fn with_len(value: T, len: usize) -> Result<Self, BufferError> {
        if len > N {
            return Err(BufferError::LengthExceedsCapacity {
                length: len,
                capacity: N,
            });
        }
        Ok(Self {
            data: [value; N],
            len,
        })
    }

    /// Wrap a fully initialized inline array as a full-capacity buffer.
    #[inline]
    pub fn from_array(data: [T; N]) -> Self {
        Self { data, len: N }
    }
}

impl<T: Copy + Default, const N: usize> Default for FixedBuffer<T, N> {
    #[inline]
    fn default() -> Self {
        Self::filled(T::default())
    }
}

/// Fixed-capacity integer storage for an embedded caller-owned buffer.
pub type IntBuffer<const N: usize> = FixedBuffer<i64, N>;

/// Fixed-capacity floating-point storage for an embedded caller-owned buffer.
pub type FloatBuffer<const N: usize> = FixedBuffer<f64, N>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_buffer_checks_length_and_mutation() {
        let mut buffer = IntBuffer::<4>::with_len(0, 2).unwrap();
        assert_eq!(buffer.len(), 2);
        assert_eq!(buffer.get(0), Ok(&0));
        buffer.set(1, 42).unwrap();
        assert_eq!(buffer.as_slice(), &[0, 42]);
        assert_eq!(
            buffer.set(2, 7),
            Err(BufferError::IndexOutOfBounds { index: 2, len: 2 })
        );
    }

    #[test]
    fn length_cannot_exceed_capacity() {
        assert_eq!(
            IntBuffer::<2>::with_len(0, 3),
            Err(BufferError::LengthExceedsCapacity {
                length: 3,
                capacity: 2
            })
        );
        let mut buffer = IntBuffer::<2>::filled(1);
        assert_eq!(buffer.set_len(0), Ok(()));
        assert!(buffer.is_empty());
        assert_eq!(
            buffer.set_len(3),
            Err(BufferError::LengthExceedsCapacity {
                length: 3,
                capacity: 2
            })
        );
    }

    #[test]
    fn float_buffer_exposes_stable_inline_storage() {
        let mut buffer = FloatBuffer::<2>::from_array([1.5, 2.5]);
        assert_eq!(buffer.as_slice(), &[1.5, 2.5]);
        assert_eq!(buffer.as_ptr(), buffer.as_mut_ptr().cast_const());
        *buffer.get_mut(0).unwrap() = 3.5;
        assert_eq!(buffer.get(0), Ok(&3.5));
    }

    #[test]
    fn zero_capacity_buffer_is_valid_and_empty() {
        let buffer = IntBuffer::<0>::filled(0);
        assert_eq!(buffer.len(), 0);
        assert!(buffer.is_empty());
        assert!(buffer.as_slice().is_empty());
    }
}
