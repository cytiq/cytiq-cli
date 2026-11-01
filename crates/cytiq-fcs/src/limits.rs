//! Bounds on sizes that untrusted FCS metadata declares. The file length bounds
//! every segment before it is read; these constants bound what it cannot.

/// Combined bytes of primary and supplemental TEXT. The largest TEXT among
/// 1,444 real acquisitions is 48 KB, and a `$SPILLOVER` for ~184 detectors is
/// ~340 KB on its own. Parsing copies TEXT into owned strings several times,
/// so this keeps that cost in the hundreds of MB.
pub(crate) const TEXT_BYTES_MAX: u64 = 16 << 20;

/// `$PAR`: parameter indexes are `u16`, and `Parameter::index` stores them.
pub(crate) const PARAMETERS_MAX: usize = 65_535;

/// Decoded DATA: `$TOT` × 8 bytes per `A`, `I` or `D` value (integers widen to
/// `u64`) and 4 per `F` value, so 8-bit DATA decodes to 8× its size. The file
/// length bounds the stored size only. The largest of 389 local acquisitions
/// decodes to 139 MiB, and 10 million events of a 64-detector spectral panel
/// to ~2.6 GiB. Commands copy columns to `f64` afterwards, so a file at this
/// bound already needs tens of GB.
pub(crate) const DECODED_DATA_BYTES_MAX: u64 = 8 << 30;

/// ANALYSIS is returned raw and no cytiq command reads it. It holds keywords
/// and results, not events (none of the 389 local acquisitions has one), so
/// it gets TEXT's bound.
pub(crate) const ANALYSIS_BYTES_MAX: u64 = TEXT_BYTES_MAX;

/// What one read may allocate for the sizes a file declares. TEXT and `$PAR`
/// keep their fixed bounds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FcsReadLimits {
    /// Bytes of the decoded DATA columns; 8 GiB by default.
    pub decoded_bytes_max: u64,
    /// Bytes of the raw ANALYSIS segment; 16 MiB by default.
    pub analysis_bytes_max: u64,
}

impl Default for FcsReadLimits {
    fn default() -> Self {
        Self {
            decoded_bytes_max: DECODED_DATA_BYTES_MAX,
            analysis_bytes_max: ANALYSIS_BYTES_MAX,
        }
    }
}
