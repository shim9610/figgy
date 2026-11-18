//! The element types a monotonic source may hold (#271): `f64`, `f32`,
//! `i32`, `i16`, `u16` and `u8`. One generic reduction serves all of them.
//! It compares in the element type and converts only the kept values to
//! `f64`, so nothing is converted or copied in bulk (M3).

/// The sealed supertrait: the element set is the maintainer's decision
/// (#271), so hosts cannot add a type of their own.
pub(in crate::monotonic) mod sealed {
    pub trait Sealed {}
}

/// An element type of a monotonic source.
///
/// Every value the reduction inspects goes through these methods, so a value
/// the reduction does not read is never touched.
pub trait MonotonicElement: Copy + sealed::Sealed {
    /// Whether the value is a gap in the line: a non-finite float (NaN or
    /// ±infinity). Integers have no gaps.
    fn is_gap(self) -> bool;
    /// The value in the envelope's unit, `f64`. Exact for every supported
    /// type.
    fn to_f64(self) -> f64;
    /// `self < other`, for values that are not gaps.
    fn less(self, other: Self) -> bool;
    /// `self == other`, for values that are not gaps.
    fn same(self, other: Self) -> bool;
    /// Zero for a value that is not a gap, a gap otherwise (`self - self`
    /// for a float). Accumulated with [`Self::mark_or`] and tested with
    /// [`Self::is_gap`], it finds a gap in a column without a branch or a
    /// boolean lane, so the scan stays vectorized.
    fn gap_mark(self) -> Self;
    /// Joins two gap marks: a gap when either is one.
    fn mark_or(self, other: Self) -> Self;
    /// `source` as a [`MonotonicSourceRef`](super::MonotonicSourceRef) of this element type.
    fn erase<'a>(
        source: &'a super::source::DynMonotonicSource<'a, Self>,
    ) -> super::MonotonicSourceRef<'a>;
}

macro_rules! float_element {
    ($($t:ty => $variant:ident),*) => {$(
        impl sealed::Sealed for $t {}
        impl MonotonicElement for $t {
            #[inline(always)]
            fn is_gap(self) -> bool {
                !self.is_finite()
            }
            #[inline(always)]
            fn to_f64(self) -> f64 {
                f64::from(self)
            }
            #[inline(always)]
            fn less(self, other: Self) -> bool {
                self < other
            }
            #[inline(always)]
            fn same(self, other: Self) -> bool {
                self == other
            }
            #[inline(always)]
            fn gap_mark(self) -> Self {
                self - self
            }
            #[inline(always)]
            fn mark_or(self, other: Self) -> Self {
                self + other
            }
            fn erase<'a>(
                source: &'a super::source::DynMonotonicSource<'a, Self>,
            ) -> super::MonotonicSourceRef<'a> {
                super::MonotonicSourceRef::$variant(source)
            }
        }
    )*};
}

macro_rules! int_element {
    ($($t:ty => $variant:ident),*) => {$(
        impl sealed::Sealed for $t {}
        impl MonotonicElement for $t {
            #[inline(always)]
            fn is_gap(self) -> bool {
                false
            }
            #[inline(always)]
            fn to_f64(self) -> f64 {
                f64::from(self)
            }
            #[inline(always)]
            fn less(self, other: Self) -> bool {
                self < other
            }
            #[inline(always)]
            fn same(self, other: Self) -> bool {
                self == other
            }
            #[inline(always)]
            fn gap_mark(self) -> Self {
                0
            }
            #[inline(always)]
            fn mark_or(self, _other: Self) -> Self {
                0
            }
            fn erase<'a>(
                source: &'a super::source::DynMonotonicSource<'a, Self>,
            ) -> super::MonotonicSourceRef<'a> {
                super::MonotonicSourceRef::$variant(source)
            }
        }
    )*};
}

float_element!(f64 => F64, f32 => F32);
int_element!(i32 => I32, i16 => I16, u16 => U16, u8 => U8);
