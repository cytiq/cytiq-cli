//! Comma-separated channel lists, as every command's `--channels` takes them.

/// Split `raw` at commas into trimmed channel names, dropping empty ones.
///
/// # Errors
///
/// Returns `channel list must contain at least one channel` when no name is
/// left.
pub fn parse_channel_list(raw: &str) -> Result<Vec<String>, String> {
    let channels = raw
        .split(',')
        .map(str::trim)
        .filter(|channel| !channel.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    if channels.is_empty() {
        Err("channel list must contain at least one channel".to_owned())
    } else {
        Ok(channels)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_names_and_drops_empty_ones() {
        assert_eq!(
            parse_channel_list(" CD3, ,CD4 ,"),
            Ok(vec!["CD3".to_owned(), "CD4".to_owned()])
        );
    }

    #[test]
    fn rejects_a_list_without_names() {
        assert_eq!(
            parse_channel_list(" , "),
            Err("channel list must contain at least one channel".to_owned())
        );
    }
}
