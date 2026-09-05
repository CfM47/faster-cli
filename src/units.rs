//! Typed quantities for transfer measurements.
//!
//! Raw `f64` speeds are easy to mix up: bits and bytes, seconds and
//! milliseconds, download and upload. The types here make each of those a
//! distinct type, so the compiler rejects the substitution instead of the
//! reviewer having to catch it.

use std::fmt;
use std::iter::Sum;
use std::marker::PhantomData;
use std::ops::{Add, AddAssign};
use std::time::Duration;

/// A count of transferred bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Bytes(u64);

impl Bytes {
    /// Builds a byte count.
    pub const fn new(count: u64) -> Self {
        Self(count)
    }

    /// Returns the underlying count.
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Returns whether nothing was transferred.
    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }
}

impl Add for Bytes {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self(self.0 + other.0)
    }
}

impl AddAssign for Bytes {
    fn add_assign(&mut self, other: Self) {
        self.0 += other.0;
    }
}

impl Sum for Bytes {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        iter.fold(Self(0), Add::add)
    }
}

/// The magnitude a [`Bitrate`] is displayed in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Magnitude {
    Kilobits,
    Megabits,
    Gigabits,
}

impl Magnitude {
    /// Returns the unit symbol, such as `Mbps`.
    pub const fn symbol(self) -> &'static str {
        match self {
            Self::Kilobits => "Kbps",
            Self::Megabits => "Mbps",
            Self::Gigabits => "Gbps",
        }
    }

    /// Returns how many decimals keep the reading meaningful at this scale.
    pub const fn decimals(self) -> usize {
        match self {
            Self::Kilobits => 0,
            Self::Megabits => 1,
            Self::Gigabits => 2,
        }
    }

    const fn divisor(self) -> f64 {
        match self {
            Self::Kilobits => 1e3,
            Self::Megabits => 1e6,
            Self::Gigabits => 1e9,
        }
    }
}

/// A transfer rate in bits per second.
///
/// Deliberately has no constructor taking a bare number: a rate only exists as
/// the ratio of an observed byte count to the time it took, which rules out
/// accidentally treating a byte count or a megabit figure as a bitrate.
#[derive(Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
pub struct Bitrate(f64);

impl Bitrate {
    /// A rate of zero, for phases that transferred nothing.
    pub const ZERO: Self = Self(0.0);

    /// Derives the rate at which `bytes` moved over `elapsed`.
    ///
    /// A non-positive or non-finite elapsed time yields [`Bitrate::ZERO`]
    /// rather than an infinity, so callers never propagate a meaningless
    /// reading.
    pub fn from_transfer(bytes: Bytes, elapsed: Duration) -> Self {
        let seconds = elapsed.as_secs_f64();
        if seconds <= 0.0 {
            return Self::ZERO;
        }
        Self(bytes.get() as f64 * 8.0 / seconds)
    }

    /// Returns the rate in bits per second.
    pub const fn bits_per_second(self) -> f64 {
        self.0
    }

    /// Returns the rate in megabits per second.
    pub fn megabits_per_second(self) -> f64 {
        self.0 / Magnitude::Megabits.divisor()
    }

    /// Splits the rate into the magnitude it reads best in and its value there.
    ///
    /// Renderers that need the number and the unit separately, such as the
    /// large-digit display, use this instead of parsing [`fmt::Display`].
    pub fn scaled(self) -> (f64, Magnitude) {
        let magnitude = if self.0 >= Magnitude::Gigabits.divisor() {
            Magnitude::Gigabits
        } else if self.0 >= Magnitude::Megabits.divisor() {
            Magnitude::Megabits
        } else {
            Magnitude::Kilobits
        };
        (self.0 / magnitude.divisor(), magnitude)
    }
}

impl fmt::Display for Bitrate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (value, magnitude) = self.scaled();
        write!(
            formatter,
            "{value:.*} {}",
            magnitude.decimals(),
            magnitude.symbol()
        )
    }
}

/// A transfer direction used only as a compile-time tag.
pub trait Direction {
    /// Human readable name of the direction, used in reports and progress.
    const LABEL: &'static str;
}

/// Bytes flowing from the server to this machine.
pub enum Download {}

/// Bytes flowing from this machine to the server.
pub enum Upload {}

impl Direction for Download {
    const LABEL: &'static str = "Download";
}

impl Direction for Upload {
    const LABEL: &'static str = "Upload";
}

/// A completed transfer measurement, tagged with the direction it measured.
///
/// The direction lives in the type, so a download figure cannot be passed
/// where an upload one is expected.
pub struct Throughput<D: Direction> {
    bytes: Bytes,
    elapsed: Duration,
    direction: PhantomData<D>,
}

// Written out rather than derived: a derive would demand `D: Clone + Copy +
// Debug + PartialEq`, which the direction tags cannot satisfy because they are
// uninhabited. The bounds are spurious anyway, since no `D` value is stored.
impl<D: Direction> Clone for Throughput<D> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<D: Direction> Copy for Throughput<D> {}

impl<D: Direction> PartialEq for Throughput<D> {
    fn eq(&self, other: &Self) -> bool {
        self.bytes == other.bytes && self.elapsed == other.elapsed
    }
}

impl<D: Direction> fmt::Debug for Throughput<D> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Throughput")
            .field("direction", &D::LABEL)
            .field("bytes", &self.bytes)
            .field("elapsed", &self.elapsed)
            .finish()
    }
}

impl<D: Direction> Throughput<D> {
    /// Records that `bytes` were transferred over `elapsed`.
    pub const fn new(bytes: Bytes, elapsed: Duration) -> Self {
        Self {
            bytes,
            elapsed,
            direction: PhantomData,
        }
    }

    /// Returns how many bytes crossed the wire.
    pub const fn bytes(self) -> Bytes {
        self.bytes
    }

    /// Returns how long the transfer ran.
    pub const fn elapsed(self) -> Duration {
        self.elapsed
    }

    /// Returns the sustained rate over the whole transfer.
    pub fn bitrate(self) -> Bitrate {
        Bitrate::from_transfer(self.bytes, self.elapsed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bitrate_converts_bytes_per_second_to_bits_per_second() {
        let rate = Bitrate::from_transfer(Bytes::new(1_000_000), Duration::from_secs(1));
        assert_eq!(rate.bits_per_second(), 8_000_000.0);
        assert_eq!(rate.megabits_per_second(), 8.0);
    }

    #[test]
    fn bitrate_is_zero_when_no_time_elapsed() {
        let rate = Bitrate::from_transfer(Bytes::new(1_000), Duration::ZERO);
        assert_eq!(rate, Bitrate::ZERO);
    }

    #[test]
    fn bitrate_picks_the_magnitude_that_reads_best() {
        let cases = [
            (12_500u64, Magnitude::Kilobits, "100 Kbps"),
            (1_250_000, Magnitude::Megabits, "10.0 Mbps"),
            (1_250_000_000, Magnitude::Gigabits, "10.00 Gbps"),
        ];
        for (bytes, magnitude, rendered) in cases {
            let rate = Bitrate::from_transfer(Bytes::new(bytes), Duration::from_secs(1));
            assert_eq!(rate.scaled().1, magnitude);
            assert_eq!(rate.to_string(), rendered);
        }
    }

    #[test]
    fn throughput_carries_its_direction_in_the_type() {
        let download = Throughput::<Download>::new(Bytes::new(2_000_000), Duration::from_secs(2));
        assert_eq!(download.bitrate().megabits_per_second(), 8.0);
        assert_eq!(Download::LABEL, "Download");
    }

    #[test]
    fn throughput_stays_copy_despite_its_uninhabited_tag() {
        fn assert_copy<T: Copy>(_: T) {}
        let download = Throughput::<Download>::new(Bytes::new(1), Duration::from_secs(1));
        assert_copy(download);
        assert_eq!(download.bytes(), Bytes::new(1));
        assert_eq!(download.elapsed(), Duration::from_secs(1));
    }

    #[test]
    fn bytes_accumulate() {
        let total: Bytes = [Bytes::new(10), Bytes::new(32)].into_iter().sum();
        assert_eq!(total, Bytes::new(42));
        assert!(Bytes::default().is_zero());
    }
}
