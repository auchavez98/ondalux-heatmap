//! IEEE 802.11 bands, channel numbering and occupied spectrum.
//!
//! Channel → frequency mapping follows the public IEEE 802.11 numbering:
//! 2.4 GHz: f = 2407 + 5·ch MHz (ch 14 = 2484 MHz),
//! 5 GHz:   f = 5000 + 5·ch MHz,
//! 6 GHz:   f = 5950 + 5·ch MHz.
//!
//! Wide channels (40/80/160/320 MHz) are modelled as the aligned block that
//! contains the primary 20 MHz channel. 320 MHz uses only the "320-1"
//! alignment; the overlapping "320-2" plan is not modelled yet.

use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Band {
    #[serde(rename = "2.4GHz")]
    G2_4,
    #[serde(rename = "5GHz")]
    G5,
    #[serde(rename = "6GHz")]
    G6,
}

impl Band {
    pub const ALL: [Band; 3] = [Band::G2_4, Band::G5, Band::G6];

    pub fn label(self) -> &'static str {
        match self {
            Band::G2_4 => "2.4 GHz",
            Band::G5 => "5 GHz",
            Band::G6 => "6 GHz",
        }
    }

    /// Valid primary 20 MHz channel numbers for this band.
    pub fn channels(self) -> Vec<u16> {
        match self {
            Band::G2_4 => (1..=14).collect(),
            Band::G5 => [
                36, 40, 44, 48, 52, 56, 60, 64, 100, 104, 108, 112, 116, 120, 124, 128, 132, 136,
                140, 144, 149, 153, 157, 161, 165, 169, 173, 177,
            ]
            .to_vec(),
            Band::G6 => (1..=233).step_by(4).collect(),
        }
    }

    /// Channel widths allowed in this band.
    pub fn widths(self) -> &'static [ChannelWidth] {
        use ChannelWidth::*;
        match self {
            Band::G2_4 => &[Mhz20, Mhz40],
            Band::G5 => &[Mhz20, Mhz40, Mhz80, Mhz160],
            Band::G6 => &[Mhz20, Mhz40, Mhz80, Mhz160, Mhz320],
        }
    }
}

impl fmt::Display for Band {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ChannelWidth {
    #[serde(rename = "20")]
    Mhz20,
    #[serde(rename = "40")]
    Mhz40,
    #[serde(rename = "80")]
    Mhz80,
    #[serde(rename = "160")]
    Mhz160,
    #[serde(rename = "320")]
    Mhz320,
}

impl ChannelWidth {
    pub fn mhz(self) -> u32 {
        match self {
            ChannelWidth::Mhz20 => 20,
            ChannelWidth::Mhz40 => 40,
            ChannelWidth::Mhz80 => 80,
            ChannelWidth::Mhz160 => 160,
            ChannelWidth::Mhz320 => 320,
        }
    }
}

/// Occupied spectrum `[lo, hi]` in MHz.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FreqRange {
    pub lo_mhz: f32,
    pub hi_mhz: f32,
}

impl FreqRange {
    pub fn center_mhz(&self) -> f32 {
        (self.lo_mhz + self.hi_mhz) * 0.5
    }

    pub fn width_mhz(&self) -> f32 {
        self.hi_mhz - self.lo_mhz
    }

    /// Overlapping bandwidth with `other`, in MHz (0 if disjoint).
    pub fn overlap_mhz(&self, other: &FreqRange) -> f32 {
        (self.hi_mhz.min(other.hi_mhz) - self.lo_mhz.max(other.lo_mhz)).max(0.0)
    }
}

/// A radio channel: band + primary channel number + width.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Channel {
    pub band: Band,
    pub number: u16,
    pub width: ChannelWidth,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelError {
    InvalidNumber(Band, u16),
    InvalidWidth(Band, ChannelWidth),
}

impl fmt::Display for ChannelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChannelError::InvalidNumber(b, n) => write!(f, "channel {n} is not valid in {b}"),
            ChannelError::InvalidWidth(b, w) => {
                write!(f, "{} MHz channels are not valid in {b}", w.mhz())
            }
        }
    }
}

impl std::error::Error for ChannelError {}

impl Channel {
    pub fn new(band: Band, number: u16, width: ChannelWidth) -> Result<Self, ChannelError> {
        if !band.channels().contains(&number) {
            return Err(ChannelError::InvalidNumber(band, number));
        }
        if !band.widths().contains(&width) {
            return Err(ChannelError::InvalidWidth(band, width));
        }
        Ok(Self {
            band,
            number,
            width,
        })
    }

    /// Center frequency of the primary 20 MHz channel, in MHz.
    pub fn primary_center_mhz(&self) -> f32 {
        center_of(self.band, self.number)
    }

    /// Occupied spectrum of the whole (possibly bonded) channel.
    pub fn range(&self) -> FreqRange {
        let w = self.width.mhz() as f32;
        match self.band {
            Band::G2_4 => {
                let c = self.primary_center_mhz();
                match self.width {
                    // HT40: secondary above for ch 1–7, below otherwise.
                    ChannelWidth::Mhz40 if self.number <= 7 => FreqRange {
                        lo_mhz: c - 10.0,
                        hi_mhz: c + 30.0,
                    },
                    ChannelWidth::Mhz40 => FreqRange {
                        lo_mhz: c - 30.0,
                        hi_mhz: c + 10.0,
                    },
                    _ => FreqRange {
                        lo_mhz: c - 10.0,
                        hi_mhz: c + 10.0,
                    },
                }
            }
            Band::G5 | Band::G6 => {
                let base = match self.band {
                    Band::G5 if self.number >= 149 => 149,
                    Band::G5 => 36,
                    _ => 1,
                };
                let n = (self.width.mhz() / 20) as u16;
                let idx = (self.number - base) / 4;
                let first = base + (idx / n) * n * 4;
                let lo = center_of(self.band, first) - 10.0;
                FreqRange {
                    lo_mhz: lo,
                    hi_mhz: lo + w,
                }
            }
        }
    }
}

fn center_of(band: Band, ch: u16) -> f32 {
    match band {
        Band::G2_4 if ch == 14 => 2484.0,
        Band::G2_4 => 2407.0 + 5.0 * ch as f32,
        Band::G5 => 5000.0 + 5.0 * ch as f32,
        Band::G6 => 5950.0 + 5.0 * ch as f32,
    }
}

/// Band and channel number for a 20 MHz channel center frequency, as
/// reported by WiFi scans. Returns `None` outside the 802.11 WLAN bands.
pub fn channel_from_frequency(freq_mhz: u32) -> Option<(Band, u16)> {
    match freq_mhz {
        2484 => Some((Band::G2_4, 14)),
        2412..=2472 if (freq_mhz - 2407) % 5 == 0 => {
            Some((Band::G2_4, ((freq_mhz - 2407) / 5) as u16))
        }
        5935 => Some((Band::G6, 2)),
        5955..=7115 if (freq_mhz - 5950) % 5 == 0 => {
            Some((Band::G6, ((freq_mhz - 5950) / 5) as u16))
        }
        5150..=5925 if freq_mhz % 5 == 0 => Some((Band::G5, ((freq_mhz - 5000) / 5) as u16)),
        _ => None,
    }
}

impl Channel {
    /// Builds a channel from scan data, falling back to the closest valid
    /// primary channel and to 20 MHz when the reported values are not in
    /// the channel plan (e.g. 6 GHz channel 2 or a regional 5 GHz channel).
    pub fn closest(band: Band, number: u16, width: ChannelWidth) -> Channel {
        let channels = band.channels();
        let number = *channels
            .iter()
            .min_by_key(|c| (i32::from(**c) - i32::from(number)).abs())
            .expect("every band has channels");
        Channel::new(band, number, width)
            .or_else(|_| Channel::new(band, number, ChannelWidth::Mhz20))
            .expect("closest valid channel with 20 MHz width is always valid")
    }
}

impl fmt::Display for Channel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} ch {} ({} MHz)",
            self.band,
            self.number,
            self.width.mhz()
        )
    }
}

/// A value stored separately for each band (e.g. wall attenuation).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct PerBand<T> {
    #[serde(rename = "2.4GHz")]
    pub g2_4: T,
    #[serde(rename = "5GHz")]
    pub g5: T,
    #[serde(rename = "6GHz")]
    pub g6: T,
}

impl<T: Copy> PerBand<T> {
    pub const fn new(g2_4: T, g5: T, g6: T) -> Self {
        Self { g2_4, g5, g6 }
    }

    pub fn get(&self, band: Band) -> T {
        match band {
            Band::G2_4 => self.g2_4,
            Band::G5 => self.g5,
            Band::G6 => self.g6,
        }
    }

    pub fn get_mut(&mut self, band: Band) -> &mut T {
        match band {
            Band::G2_4 => &mut self.g2_4,
            Band::G5 => &mut self.g5,
            Band::G6 => &mut self.g6,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ChannelWidth::*;

    fn ch(band: Band, n: u16, w: ChannelWidth) -> Channel {
        Channel::new(band, n, w).unwrap()
    }

    #[test]
    fn known_center_frequencies() {
        assert_eq!(ch(Band::G2_4, 1, Mhz20).primary_center_mhz(), 2412.0);
        assert_eq!(ch(Band::G2_4, 6, Mhz20).primary_center_mhz(), 2437.0);
        assert_eq!(ch(Band::G2_4, 14, Mhz20).primary_center_mhz(), 2484.0);
        assert_eq!(ch(Band::G5, 36, Mhz20).primary_center_mhz(), 5180.0);
        assert_eq!(ch(Band::G5, 149, Mhz20).primary_center_mhz(), 5745.0);
        assert_eq!(ch(Band::G6, 1, Mhz20).primary_center_mhz(), 5955.0);
        assert_eq!(ch(Band::G6, 37, Mhz20).primary_center_mhz(), 6135.0);
    }

    #[test]
    fn bonded_blocks_5ghz() {
        // 80 MHz block 36–48 is centered on channel 42 (5210 MHz).
        for primary in [36, 40, 44, 48] {
            let r = ch(Band::G5, primary, Mhz80).range();
            assert_eq!((r.lo_mhz, r.hi_mhz), (5170.0, 5250.0), "primary {primary}");
        }
        // 80 MHz block 149–161 is centered on channel 155 (5775 MHz).
        assert_eq!(ch(Band::G5, 157, Mhz80).range().center_mhz(), 5775.0);
        // 160 MHz block 36–64 is centered on channel 50 (5250 MHz).
        assert_eq!(ch(Band::G5, 60, Mhz160).range().center_mhz(), 5250.0);
    }

    #[test]
    fn bonded_blocks_6ghz() {
        // 80 MHz block 1–13 is centered on channel 7 (5985 MHz).
        assert_eq!(ch(Band::G6, 9, Mhz80).range().center_mhz(), 5985.0);
    }

    #[test]
    fn non_overlapping_24_channels() {
        let r1 = ch(Band::G2_4, 1, Mhz20).range();
        let r6 = ch(Band::G2_4, 6, Mhz20).range();
        let r3 = ch(Band::G2_4, 3, Mhz20).range();
        assert_eq!(r1.overlap_mhz(&r6), 0.0);
        assert_eq!(r1.overlap_mhz(&r3), 10.0);
    }

    #[test]
    fn frequency_to_channel() {
        assert_eq!(channel_from_frequency(2412), Some((Band::G2_4, 1)));
        assert_eq!(channel_from_frequency(2484), Some((Band::G2_4, 14)));
        assert_eq!(channel_from_frequency(5180), Some((Band::G5, 36)));
        assert_eq!(channel_from_frequency(5825), Some((Band::G5, 165)));
        assert_eq!(channel_from_frequency(5955), Some((Band::G6, 1)));
        assert_eq!(channel_from_frequency(6135), Some((Band::G6, 37)));
        assert_eq!(channel_from_frequency(5935), Some((Band::G6, 2)));
        assert_eq!(channel_from_frequency(900), None);
        // Round trip with the forward mapping.
        for band in Band::ALL {
            for n in band.channels() {
                let f = ch(band, n, Mhz20).primary_center_mhz() as u32;
                assert_eq!(channel_from_frequency(f), Some((band, n)), "{band} {n}");
            }
        }
    }

    #[test]
    fn closest_valid_channel() {
        assert_eq!(Channel::closest(Band::G6, 2, Mhz80).number, 1);
        assert_eq!(Channel::closest(Band::G5, 38, Mhz40).number, 36);
        assert_eq!(Channel::closest(Band::G2_4, 6, Mhz80).width, Mhz20);
    }

    #[test]
    fn invalid_channels_rejected() {
        assert!(Channel::new(Band::G5, 37, Mhz20).is_err());
        assert!(Channel::new(Band::G2_4, 6, Mhz80).is_err());
        assert!(Channel::new(Band::G6, 2, Mhz20).is_err());
    }
}
