//! Resolve FCS files without changing the caller's ordering policy.

use std::{
    collections::HashSet,
    fs, io,
    path::{Path, PathBuf},
};

use globset::{GlobBuilder, GlobMatcher};
use thiserror::Error;

/// Ordering after expanding each input (each directory/glob is always sorted).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputOrder {
    Global,
    PerInput,
}

/// Compatibility choices for explicit inputs and expanded input groups.
#[derive(Clone, Copy, Debug)]
pub struct ResolveOptions {
    pub order: InputOrder,
    pub require_regular_file: bool,
}

impl Default for ResolveOptions {
    fn default() -> Self {
        Self {
            order: InputOrder::Global,
            require_regular_file: true,
        }
    }
}

/// Input errors stay separate from errors decoding the FCS format.
#[derive(Debug, Error)]
pub enum InputError {
    #[error("at least one FCS input is required")]
    EmptyInputs,
    #[error("input path `{path}` does not exist")]
    MissingInput { path: PathBuf },
    #[error("input path `{path}` is not an FCS file")]
    NonFcsInput { path: PathBuf },
    #[error("directory input `{path}` contains no `.fcs` files")]
    EmptyDirectory { path: PathBuf },
    #[error("glob pattern `{pattern}` matched no FCS files")]
    EmptyGlob { pattern: String },
    #[error("glob pattern `{pattern}` is unsupported; only file names may contain wildcards")]
    UnsupportedGlob { pattern: String },
    #[error("failed to read directory `{path}`: {source}")]
    DirectoryRead {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to resolve input file `{path}`: {source}")]
    InputCanonicalize {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("input file `{path}` was provided more than once")]
    DuplicateInput { path: PathBuf },
}

/// Expand files, nonrecursive directories and filename-only `*`/`?` patterns.
///
/// # Errors
/// Rejects missing/empty inputs, unsupported patterns, unreadable entries and
/// duplicate canonical paths. Returned paths retain their original spelling.
pub fn resolve_inputs(
    inputs: &[PathBuf],
    options: ResolveOptions,
) -> Result<Vec<PathBuf>, InputError> {
    let mut paths = Vec::new();
    for input in inputs {
        let raw = input.to_string_lossy();
        if raw.contains(['*', '?']) {
            paths.extend(resolve_glob(input, &raw)?);
        } else if !input.exists() {
            return Err(InputError::MissingInput {
                path: input.clone(),
            });
        } else if input.is_dir() {
            let files = directory_files(input, None)?;
            if files.is_empty() {
                return Err(InputError::EmptyDirectory {
                    path: input.clone(),
                });
            }
            paths.extend(files);
        } else if is_fcs_path(input) && (!options.require_regular_file || input.is_file()) {
            paths.push(input.clone());
        } else {
            return Err(InputError::NonFcsInput {
                path: input.clone(),
            });
        }
    }
    if paths.is_empty() {
        return Err(InputError::EmptyInputs);
    }
    if options.order == InputOrder::Global {
        paths.sort();
    }
    let mut seen = HashSet::new();
    for path in &paths {
        let canonical = fs::canonicalize(path).map_err(|source| InputError::InputCanonicalize {
            path: path.clone(),
            source,
        })?;
        if !seen.insert(canonical) {
            return Err(InputError::DuplicateInput { path: path.clone() });
        }
    }
    Ok(paths)
}

fn resolve_glob(input: &Path, raw: &str) -> Result<Vec<PathBuf>, InputError> {
    let unsupported = || InputError::UnsupportedGlob {
        pattern: raw.to_owned(),
    };
    let pattern = input
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(unsupported)?;
    let parent = input
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if parent.to_string_lossy().contains(['*', '?']) {
        return Err(unsupported());
    }
    let matcher = filename_matcher(pattern).map_err(|_| unsupported())?;
    let paths = directory_files(parent, Some(&matcher))?;
    if paths.is_empty() {
        return Err(InputError::EmptyGlob {
            pattern: raw.to_owned(),
        });
    }
    Ok(paths)
}

fn filename_matcher(pattern: &str) -> Result<GlobMatcher, globset::Error> {
    // Preserve the old byte-oriented language. Braces/classes/backslashes are
    // literal, and repeated stars must never become recursive glob syntax.
    let mut escaped = String::new();
    for character in pattern.chars() {
        if character == '*' && escaped.ends_with('*') {
            continue;
        }
        if matches!(character, '[' | ']' | '{' | '}' | '\\') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    GlobBuilder::new(&escaped)
        .backslash_escape(true)
        .literal_separator(true)
        .build()
        .map(|glob| glob.compile_matcher())
}

fn directory_files(path: &Path, matcher: Option<&GlobMatcher>) -> Result<Vec<PathBuf>, InputError> {
    let entries = fs::read_dir(path).map_err(|source| InputError::DirectoryRead {
        path: path.to_path_buf(),
        source,
    })?;
    collect_files(
        entries.map(|entry| entry.map(|entry| entry.path())),
        path,
        matcher,
    )
}

fn collect_files(
    entries: impl Iterator<Item = io::Result<PathBuf>>,
    directory: &Path,
    matcher: Option<&GlobMatcher>,
) -> Result<Vec<PathBuf>, InputError> {
    let mut paths = Vec::new();
    for entry in entries {
        let path = entry.map_err(|source| InputError::DirectoryRead {
            path: directory.to_path_buf(),
            source,
        })?;
        if path.is_file()
            && is_fcs_path(&path)
            && matcher.is_none_or(|matcher| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| matcher.is_match(name))
            })
        {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

fn is_fcs_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("fcs"))
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("cytiq-fcs-glob-{}-{}", std::process::id(), {
                    static NEXT: std::sync::atomic::AtomicU64 =
                        std::sync::atomic::AtomicU64::new(0);
                    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                }));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn globs_select_only_matching_files_in_sorted_order() {
        let dir = Directory::new();
        for name in [
            "tube-b.fcs",
            "tube-a.FCS",
            "tube-ab.fcs",
            "other.fcs",
            "tube-c.txt",
        ] {
            fs::write(dir.0.join(name), []).unwrap();
        }
        fs::create_dir(dir.0.join("tube-d.fcs")).unwrap();
        let pattern = dir.0.join("tube-?.*");
        let result = resolve_glob(&pattern, &pattern.to_string_lossy()).unwrap();
        assert_eq!(result, [dir.0.join("tube-a.FCS"), dir.0.join("tube-b.fcs")]);
        let pattern = dir.0.join("tube-*.fcs");
        let result = resolve_glob(&pattern, &pattern.to_string_lossy()).unwrap();
        assert_eq!(
            result,
            [dir.0.join("tube-ab.fcs"), dir.0.join("tube-b.fcs")]
        );
        let pattern = dir.0.join("missing*.fcs");
        assert!(
            matches!(resolve_glob(&pattern, &pattern.to_string_lossy()), Err(InputError::EmptyGlob { pattern: value }) if value == pattern.to_string_lossy())
        );
        let pattern = dir.0.join("missing").join("*.fcs");
        assert!(matches!(
            resolve_glob(&pattern, "missing/*.fcs"),
            Err(InputError::DirectoryRead { .. })
        ));
        for parent in ["wild*", "wild?"] {
            let pattern = dir.0.join(parent).join("*.fcs");
            assert!(
                matches!(resolve_glob(&pattern, "unsupported"), Err(InputError::UnsupportedGlob { pattern }) if pattern == "unsupported")
            );
        }
        assert!(matches!(
            resolve_glob(Path::new("/__cytiq_no_matches_92f6*.fcs"), "root"),
            Err(InputError::EmptyGlob { .. })
        ));
        assert!(matches!(
            resolve_glob(Path::new("__cytiq_no_matches_92f6*.fcs"), "relative"),
            Err(InputError::EmptyGlob { .. })
        ));
        assert!(matches!(
            resolve_glob(Path::new("/"), "root"),
            Err(InputError::UnsupportedGlob { .. })
        ));
    }

    #[test]
    fn wildcard_matching_handles_empty_repeated_and_backtracking_patterns() {
        for (pattern, value, expected) in [
            ("", "", true),
            ("", "a", false),
            ("*", "", true),
            ("**", "abc", true),
            ("?", "", false),
            ("?", "a", true),
            ("?", "ab", false),
            ("abc", "ab", false),
            ("abc", "abc", true),
            ("abc", "abd", false),
            ("a*b?c", "abbbc", true),
            ("a*b?c", "ac", false),
            ("*ab*bc", "ababcbc", true),
            ("*x", "abc", false),
            ("a", "ab", false),
        ] {
            assert_eq!(
                filename_matcher(pattern).unwrap().is_match(value),
                expected,
                "{pattern:?} {value:?}"
            );
        }
    }
    #[test]
    fn ordering_duplicates_and_empty_inputs_retain_their_policy() {
        let dir = Directory::new();
        let a = dir.0.join("a.fcs");
        let z = dir.0.join("z.FCS");
        fs::write(&a, []).unwrap();
        fs::write(&z, []).unwrap();
        let inputs = [z.clone(), a.clone()];
        assert_eq!(
            resolve_inputs(&inputs, ResolveOptions::default()).unwrap(),
            [a.clone(), z.clone()]
        );
        assert_eq!(
            resolve_inputs(
                &inputs,
                ResolveOptions {
                    order: InputOrder::PerInput,
                    require_regular_file: false
                }
            )
            .unwrap(),
            inputs
        );
        assert!(
            matches!(resolve_inputs(&[a.clone(), dir.0.clone()], ResolveOptions::default()), Err(InputError::DuplicateInput { path }) if path == a)
        );
        assert!(matches!(
            resolve_inputs(&[], ResolveOptions::default()),
            Err(InputError::EmptyInputs)
        ));
        assert!(matches!(
            resolve_inputs(&[dir.0.join("missing.fcs")], ResolveOptions::default()),
            Err(InputError::MissingInput { .. })
        ));
        let empty = dir.0.join("empty");
        fs::create_dir(&empty).unwrap();
        assert!(matches!(
            resolve_inputs(&[empty], ResolveOptions::default()),
            Err(InputError::EmptyDirectory { .. })
        ));
        let text = dir.0.join("a.txt");
        fs::write(&text, []).unwrap();
        assert!(matches!(
            resolve_inputs(&[text], ResolveOptions::default()),
            Err(InputError::NonFcsInput { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_aliases_are_duplicates() {
        let dir = Directory::new();
        let a = dir.0.join("a.fcs");
        let alias = dir.0.join("alias.fcs");
        fs::write(&a, []).unwrap();
        std::os::unix::fs::symlink(&a, &alias).unwrap();
        assert!(matches!(
            resolve_inputs(&[a, alias], ResolveOptions::default()),
            Err(InputError::DuplicateInput { .. })
        ));
    }

    #[test]
    fn directory_entry_errors_are_not_skipped() {
        let error = io::Error::new(io::ErrorKind::PermissionDenied, "entry denied");
        assert!(
            matches!(collect_files([Err(error)].into_iter(),Path::new("directory"),None),Err(InputError::DirectoryRead{path,source}) if path==Path::new("directory") && source.kind()==io::ErrorKind::PermissionDenied)
        );
    }

    #[test]
    fn filename_language_is_literal_except_for_byte_wildcards() {
        for (pattern, name, expected) in [
            ("[ab]*.fcs", "[ab]x.fcs", true),
            ("[ab]*.fcs", "ax.fcs", false),
            ("{a,b}?.fcs", "{a,b}x.fcs", true),
            (r"a\?.fcs", r"a\x.fcs", true),
            ("?.fcs", "é.fcs", false),
            ("??.fcs", "é.fcs", true),
            ("*.fcs", "a\nb.fcs", true),
            ("**", "a\nb", true),
            ("a**b.fcs", "ab.fcs", true),
        ] {
            assert_eq!(
                filename_matcher(pattern).unwrap().is_match(name),
                expected,
                "{pattern:?} {name:?}"
            );
        }
        let pattern = "*a".repeat(200) + "b.fcs";
        assert!(
            !filename_matcher(&pattern)
                .unwrap()
                .is_match("a".repeat(1000) + "c.fcs")
        );
    }
    #[cfg(unix)]
    #[test]
    fn explicit_special_files_retain_the_callers_regular_file_policy() {
        let directory = Directory::new();
        let fifo = directory.0.join("pipe.fcs");
        assert!(
            std::process::Command::new("mkfifo")
                .arg(&fifo)
                .status()
                .unwrap()
                .success()
        );
        assert!(matches!(
            resolve_inputs(std::slice::from_ref(&fifo), ResolveOptions::default()),
            Err(InputError::NonFcsInput { .. })
        ));
        assert_eq!(
            resolve_inputs(
                std::slice::from_ref(&fifo),
                ResolveOptions {
                    order: InputOrder::PerInput,
                    require_regular_file: false
                }
            )
            .unwrap(),
            [fifo]
        );
    }
}
