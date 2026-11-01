//! Bounds on sizes that untrusted FCS metadata declares. The file length bounds
//! every segment before it is read; these constants bound what it cannot.

/// Combined bytes of primary and supplemental TEXT. The largest TEXT among
/// 1,444 real acquisitions is 48 KB, and a `$SPILLOVER` for ~184 detectors is
/// ~340 KB on its own. Parsing copies TEXT into owned strings several times,
/// so this keeps that cost in the hundreds of MB.
pub(crate) const TEXT_BYTES_MAX: u64 = 16 << 20;

/// `$PAR`: parameter indexes are `u16`, and `Parameter::index` stores them.
pub(crate) const PARAMETERS_MAX: usize = 65_535;
